//! Shared compute renderer for Vulkan, Metal and Direct3D 12.
//!
//! Published frames own immutable RGBA pixels. GPUI can upload them on any of
//! its platforms; the scene-linear accumulation stays on the rendering device.
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::Path,
    sync::{Arc, mpsc},
    time::Instant,
};

use anyhow::{Context, Result, anyhow, ensure};
use bytemuck::Pod;
use forma_core::Scene;
use wgpu::util::DeviceExt;

use crate::{
    Backend, RenderMode, RenderSettings,
    bvh::Geometry,
    environment::EnvironmentKey,
    portable_preview::PreviewResources,
    render_data::{Uniforms, geometry_hash},
};

#[derive(Clone)]
pub(crate) struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    pub samples: u32,
    pub elapsed_ms: f64,
}

struct Film {
    width: u32,
    height: u32,
    rgba: wgpu::Texture,
    view: wgpu::TextureView,
    accumulation: wgpu::Buffer,
    albedo: wgpu::Buffer,
    normal: wgpu::Buffer,
    has_guides: bool,
    readback: wgpu::Buffer,
    stride: u32,
}

struct GpuGeometry {
    triangles: wgpu::Buffer,
    nodes: wgpu::Buffer,
    lights: wgpu::Buffer,
    counts: [u32; 3],
}

pub(crate) struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_info: wgpu::AdapterInfo,
    render_pipeline: wgpu::ComputePipeline,
    preview_pipeline: wgpu::ComputePipeline,
    bindings: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    preview_resources: PreviewResources,
    geometry: Option<GpuGeometry>,
    geometry_hash: Option<u64>,
    checked_revision: Option<u64>,
    film: Option<Film>,
    accumulation_key: Option<u64>,
    samples: u32,
    frame: Option<Frame>,
}

impl Renderer {
    pub(crate) fn new(backends: wgpu::Backends) -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .with_context(|| format!("No compatible GPU adapter is available for {backends:?}"))?;
        let adapter_info = adapter.get_info();
        let supported = adapter.limits();
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: supported.max_storage_buffer_binding_size,
            max_buffer_size: supported.max_buffer_size,
            ..wgpu::Limits::default().using_resolution(supported)
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Forma portable rendering device"),
            required_limits: limits,
            ..Default::default()
        }))
        .with_context(|| {
            format!(
                "Could not initialize {:?} GPU {}",
                adapter_info.backend, adapter_info.name
            )
        })?;
        let (render_pipeline, preview_pipeline, bindings, uniforms, preview_resources) =
            checked_gpu(&device, "Initialize Forma shaders", || {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("Forma viewport and path tracer"),
                    source: wgpu::ShaderSource::Wgsl(
                        format!(
                            "{}\n{}",
                            include_str!("shader.wgsl"),
                            include_str!("preview.wgsl")
                        )
                        .into(),
                    ),
                });
                let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Forma scene and film bindings"),
                    entries: &[
                        buffer_layout(0, wgpu::BufferBindingType::Uniform),
                        buffer_layout(1, wgpu::BufferBindingType::Storage { read_only: true }),
                        buffer_layout(2, wgpu::BufferBindingType::Storage { read_only: true }),
                        buffer_layout(3, wgpu::BufferBindingType::Storage { read_only: true }),
                        buffer_layout(4, wgpu::BufferBindingType::Storage { read_only: false }),
                        storage_texture_layout(5, wgpu::TextureFormat::Rgba8Unorm),
                        buffer_layout(6, wgpu::BufferBindingType::Storage { read_only: false }),
                        buffer_layout(7, wgpu::BufferBindingType::Storage { read_only: false }),
                    ],
                });
                let preview_resources = PreviewResources::new(&device)?;
                let render_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("Forma render layout"),
                        bind_group_layouts: &[&bindings],
                        push_constant_ranges: &[],
                    });
                let preview_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("Forma Material Preview layout"),
                        bind_group_layouts: &[&bindings, &preview_resources.bindings],
                        push_constant_ranges: &[],
                    });
                let render_pipeline = pipeline(&device, &module, &render_layout, "render_main");
                let preview_pipeline = pipeline(&device, &module, &preview_layout, "preview_main");
                let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Forma camera and render settings"),
                    size: std::mem::size_of::<Uniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                Ok((
                    render_pipeline,
                    preview_pipeline,
                    bindings,
                    uniforms,
                    preview_resources,
                ))
            })?;
        Ok(Self {
            device,
            queue,
            adapter_info,
            render_pipeline,
            preview_pipeline,
            bindings,
            uniforms,
            preview_resources,
            geometry: None,
            geometry_hash: None,
            checked_revision: None,
            film: None,
            accumulation_key: None,
            samples: 0,
            frame: None,
        })
    }

    pub fn device_name(&self) -> String {
        self.adapter_info.name.clone()
    }

    pub fn backend(&self) -> Backend {
        match self.adapter_info.backend {
            wgpu::Backend::Metal => Backend::Metal,
            wgpu::Backend::Dx12 => Backend::Dx12,
            wgpu::Backend::Vulkan => Backend::Vulkan,
            _ => Backend::Wgpu,
        }
    }

    /// One progressive sample per call. The stopping target can be increased
    /// without discarding the existing estimator, just as with native Metal.
    pub fn render(
        &mut self,
        scene: &Scene,
        settings: &RenderSettings,
        revision: u64,
    ) -> Result<Frame> {
        ensure!(
            settings.width > 0
                && settings.height > 0
                && settings.width <= 8192
                && settings.height <= 8192,
            "Render dimensions must be between 1 and 8192 pixels"
        );
        ensure!(settings.exposure.is_finite(), "Exposure must be finite");
        let started = Instant::now();
        let width = (settings.width + 1) & !1;
        let height = (settings.height + 1) & !1;
        let limits = self.device.limits();
        ensure!(
            width <= limits.max_texture_dimension_2d && height <= limits.max_texture_dimension_2d,
            "Render dimensions exceed this GPU's {} pixel texture limit",
            limits.max_texture_dimension_2d
        );
        check_storage_size(
            &self.device,
            u64::from(width) * u64::from(height) * 16,
            "Render film",
        )?;

        // Decode/bake before modifying the exportable film. Rejected HDRIs
        // leave both the last published frame and its linear values intact.
        let environment_key = if settings.mode == RenderMode::MaterialPreview {
            let p = &settings.preview;
            ensure!(
                p.rotation.is_finite()
                    && p.strength.is_finite()
                    && p.world_opacity.is_finite()
                    && p.background_blur.is_finite(),
                "Material Preview settings must be finite"
            );
            ensure!(
                (0.0..=1000.0).contains(&p.strength)
                    && (0.0..=1.0).contains(&p.world_opacity)
                    && (0.0..=1.0).contains(&p.background_blur),
                "Preview strength must be 0–1000; background opacity and blur must be 0–1"
            );
            let key = if p.use_scene_world {
                EnvironmentKey::Builtin(crate::StudioLight::Studio)
            } else {
                EnvironmentKey::new(p.studio, p.hdri_path.as_deref())?
            };
            self.preview_resources
                .prepare(&key, &self.device, &self.queue)?;
            Some(key)
        } else {
            None
        };

        if self.checked_revision != Some(revision) {
            scene.validate().context("Cannot render invalid scene")?;
            let hash = geometry_hash(scene);
            if self.geometry_hash != Some(hash) {
                let geometry = Geometry::from_scene(scene);
                let replacement = checked_gpu(&self.device, "Upload scene geometry", || {
                    Ok(GpuGeometry {
                        triangles: upload(&self.device, &geometry.triangles, "Forma triangles")?,
                        nodes: upload(&self.device, &geometry.nodes, "Forma BVH")?,
                        lights: upload(&self.device, &geometry.lights, "Forma emissive triangles")?,
                        counts: [
                            geometry.triangles.len() as u32,
                            geometry.nodes.len() as u32,
                            geometry.lights.len() as u32,
                        ],
                    })
                })?;
                self.geometry = Some(replacement);
                self.geometry_hash = Some(hash);
            }
            self.checked_revision = Some(revision);
        }
        if !self.film.as_ref().is_some_and(|film| {
            film.width == width
                && film.height == height
                && film.has_guides == settings.mode.progressive()
        }) {
            self.film = Some(checked_gpu(&self.device, "Allocate render film", || {
                Ok(Film::new(
                    &self.device,
                    width,
                    height,
                    settings.mode.progressive(),
                ))
            })?);
            self.accumulation_key = None;
        }
        let geometry = self.geometry.as_ref().context("Missing GPU geometry")?;
        let mut uniforms = Uniforms::new(
            scene,
            settings,
            width,
            height,
            self.samples,
            geometry.counts,
        );
        let mut hasher = DefaultHasher::new();
        let mut key_uniforms = uniforms;
        key_uniforms.image[2] = 0;
        if settings.mode == RenderMode::Rendered {
            key_uniforms.settings[1] = 0.0;
            key_uniforms.settings[2] = 0.0;
        }
        bytemuck::bytes_of(&key_uniforms).hash(&mut hasher);
        self.geometry_hash.hash(&mut hasher);
        if settings.mode == RenderMode::Rendered {
            revision.hash(&mut hasher);
        }
        environment_key.hash(&mut hasher);
        let key = hasher.finish();
        if self.accumulation_key != Some(key) {
            self.samples = 0;
            self.frame = None;
            self.accumulation_key = Some(key);
        }
        let limit = if settings.mode.progressive() {
            settings.max_samples.clamp(1, 1_000_000)
        } else {
            1
        };
        if self.samples >= limit {
            return self.frame.clone().context("Missing completed frame");
        }
        uniforms.image[2] = self.samples;
        let film = self.film.as_ref().context("Missing GPU film")?;
        let pixels = checked_gpu(&self.device, "Render viewport sample", || {
            self.queue
                .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Forma scene and film"),
                layout: &self.bindings,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: geometry.triangles.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: geometry.nodes.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: geometry.lights.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: film.accumulation.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&film.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: film.albedo.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: film.normal.as_entire_binding(),
                    },
                ],
            });
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Forma viewport sample and frame readback"),
                });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Forma viewport sample"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(if environment_key.is_some() {
                    &self.preview_pipeline
                } else {
                    &self.render_pipeline
                });
                pass.set_bind_group(0, &group, &[]);
                if let Some(key) = &environment_key {
                    let environment = self
                        .preview_resources
                        .get(key)
                        .context("Missing prepared preview environment")?;
                    pass.set_bind_group(1, &environment.group, &[]);
                }
                pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
            }
            encoder.copy_texture_to_buffer(
                film.rgba.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &film.readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(film.stride),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            self.queue.submit([encoder.finish()]);
            let padded = read_buffer(&self.device, &film.readback)?;
            let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
            for row in padded.chunks_exact(film.stride as usize) {
                pixels.extend_from_slice(&row[..width as usize * 4]);
            }
            Ok(pixels)
        })?;
        self.samples += 1;
        let frame = Frame {
            width,
            height,
            rgba: pixels.into(),
            samples: self.samples,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        };
        self.frame = Some(frame.clone());
        Ok(frame)
    }

    pub fn export_png(&mut self, path: &Path) -> Result<()> {
        let frame = self
            .frame
            .as_ref()
            .context("Render an image before exporting")?;
        image::save_buffer_with_format(
            path,
            &frame.rgba,
            frame.width,
            frame.height,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .with_context(|| format!("Could not export {}", path.display()))
    }

    /// Normalized scene-linear RGBA before exposure/tonemapping, with alpha 1.
    pub fn read_linear_pixels(&self) -> Result<Vec<[f32; 4]>> {
        let film = self
            .film
            .as_ref()
            .filter(|_| self.frame.is_some())
            .context("Render an image before reading pixels")?;
        self.read_float_buffer(&film.accumulation)
    }

    pub fn read_denoise_guides(&self) -> Result<crate::denoise::GuidePixels> {
        let film = self
            .film
            .as_ref()
            .filter(|f| f.has_guides && self.frame.is_some())
            .context("Render a path-traced image before reading denoising guides")?;
        Ok((
            self.read_float_buffer(&film.albedo)?,
            self.read_float_buffer(&film.normal)?,
        ))
    }

    fn read_float_buffer(&self, source: &wgpu::Buffer) -> Result<Vec<[f32; 4]>> {
        checked_gpu(&self.device, "Read scene-linear film", || {
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Forma scene-linear readback"),
                size: source.size(),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            encoder.copy_buffer_to_buffer(source, 0, &readback, 0, readback.size());
            self.queue.submit([encoder.finish()]);
            let bytes = read_buffer(&self.device, &readback)?;
            // Read POD values without assuming Vec<u8> has float alignment.
            Ok(bytes
                .as_chunks::<16>()
                .0
                .iter()
                .map(|bytes| {
                    let value: [f32; 4] = bytemuck::pod_read_unaligned(bytes);
                    let weight = value[3].max(1.0);
                    [value[0] / weight, value[1] / weight, value[2] / weight, 1.0]
                })
                .collect())
        })
    }
}

impl Film {
    fn new(device: &wgpu::Device, width: u32, height: u32, has_guides: bool) -> Self {
        let rgba = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Forma RGBA film"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = rgba.create_view(&wgpu::TextureViewDescriptor::default());
        let accumulation = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Forma scene-linear accumulation"),
            size: u64::from(width) * u64::from(height) * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let guide = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: if has_guides {
                    u64::from(width) * u64::from(height) * 16
                } else {
                    16
                },
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let albedo = guide("Forma denoising albedo");
        let normal = guide("Forma denoising normals");
        let stride = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Forma RGBA frame readback"),
            size: u64::from(stride) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            width,
            height,
            rgba,
            view,
            accumulation,
            albedo,
            normal,
            has_guides,
            readback,
            stride,
        }
    }
}

fn upload<T: Pod>(device: &wgpu::Device, values: &[T], label: &str) -> Result<wgpu::Buffer> {
    let size = std::mem::size_of_val(values)
        .max(std::mem::size_of::<T>())
        .max(16) as u64;
    check_storage_size(device, size, label)?;
    if values.is_empty() {
        return Ok(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        }));
    }
    Ok(
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(values),
            usage: wgpu::BufferUsages::STORAGE,
        }),
    )
}

fn check_storage_size(device: &wgpu::Device, size: u64, label: &str) -> Result<()> {
    let limit = u64::from(device.limits().max_storage_buffer_binding_size)
        .min(device.limits().max_buffer_size);
    ensure!(
        size <= limit,
        "{label} requires {size} bytes, exceeding this GPU's {limit} byte storage-buffer limit"
    );
    Ok(())
}

fn read_buffer(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Result<Vec<u8>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .context("GPU command completion failed")?;
    receiver
        .recv()
        .context("GPU readback callback was lost")?
        .context("Could not map GPU readback")?;
    let bytes = buffer.slice(..).get_mapped_range().to_vec();
    buffer.unmap();
    Ok(bytes)
}

/// wgpu reports validation/allocation failures asynchronously. Capture every
/// scope even on early returns so malformed resources produce usable errors.
pub(super) fn checked_gpu<T>(
    device: &wgpu::Device,
    label: &str,
    action: impl FnOnce() -> Result<T>,
) -> Result<T> {
    device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    device.push_error_scope(wgpu::ErrorFilter::Internal);
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let result = action();
    let mut errors = Vec::new();
    for _ in 0..3 {
        if let Some(error) = pollster::block_on(device.pop_error_scope()) {
            errors.push(error.to_string());
        }
    }
    if !errors.is_empty() {
        return Err(anyhow!("{label}: {}", errors.join("; ")));
    }
    result.with_context(|| label.to_string())
}

pub(super) fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    entry: &str,
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry),
        layout: Some(layout),
        module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

pub(super) fn buffer_layout(
    binding: u32,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub(super) fn texture_layout(
    binding: u32,
    dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}

pub(super) fn storage_texture_layout(
    binding: u32,
    format: wgpu::TextureFormat,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

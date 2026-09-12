#[cfg(test)]
#[path = "preview_acceleration_tests.rs"]
mod acceleration_tests;

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::Path,
    time::Instant,
};

use anyhow::{Context, Result, anyhow, ensure};
use bytemuck::{Pod, Zeroable};
use core_foundation::{
    base::{CFType, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use core_video::{
    metal_texture::{CVMetalTexture, CVMetalTextureGetTexture, CVMetalTextureKeys},
    metal_texture_cache::CVMetalTextureCache,
    pixel_buffer::{
        CVPixelBuffer, CVPixelBufferKeys, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
    },
    pixel_buffer_pool::CVPixelBufferPool,
};
use foreign_types::ForeignTypeRef;
use forma_core::Scene;
use glam::Vec3;
use metal::*;

use crate::{
    RenderMode, RenderSettings,
    bvh::Geometry,
    preview::{EnvironmentKey, PreviewResources},
};

#[derive(Clone)]
pub struct Frame {
    pub surface: CVPixelBuffer,
    pub samples: u32,
    pub elapsed_ms: f64,
}

// SAFETY: A published pixel buffer is immutable, GPU writes have completed, and
// CoreVideo retain/release are thread-safe. The renderer NEVER overwrites a
// published buffer. CVPixelBufferPool recycles storage only after all owners,
// including GPUI's Metal texture cache, have released their references.
unsafe impl Send for Frame {}
unsafe impl Sync for Frame {}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    cam_origin: [f32; 4],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    cam_forward: [f32; 4],
    world: [f32; 4],
    camera: [f32; 4],
    image: [u32; 4],
    scene: [u32; 4],
    settings: [f32; 4],
    preview_lighting: [f32; 4],
    preview_display: [f32; 4],
}

struct Film {
    width: u32,
    height: u32,
    rgba: Texture,
    accumulation: Buffer,
    pool: CVPixelBufferPool,
}

struct GpuGeometry {
    triangles: Buffer,
    nodes: Buffer,
    lights: Buffer,
    counts: [u32; 3],
    preview_acceleration: Option<AccelerationStructure>,
}

/// Create and use on one worker thread. GPU completion is synchronous, while the
/// application can keep its event loop responsive independently of render cost.
pub struct Renderer {
    device: Device,
    queue: CommandQueue,
    render_pipeline: ComputePipelineState,
    preview_pipeline: ComputePipelineState,
    accelerated_preview: bool,
    preview_resources: PreviewResources,
    convert_pipeline: ComputePipelineState,
    texture_cache: CVMetalTextureCache,
    geometry: Option<GpuGeometry>,
    geometry_hash: Option<u64>,
    checked_revision: Option<u64>,
    film: Option<Film>,
    accumulation_key: Option<u64>,
    samples: u32,
    frame: Option<Frame>,
}

impl Renderer {
    pub fn new() -> Result<Self> {
        Self::with_preview_acceleration(true)
    }

    fn with_preview_acceleration(enable: bool) -> Result<Self> {
        objc::rc::autoreleasepool(|| {
            let device = Device::system_default().context("No Metal GPU is available")?;
            let accelerated_preview = enable && device.supports_raytracing();
            let source = format!(
                "#define FORMA_PREVIEW_ACCELERATED {}\n{}\n{}\n{}\n{}",
                u32::from(accelerated_preview),
                include_str!("shader.metal"),
                include_str!("ibl.metal"),
                include_str!("preview.metal"),
                include_str!("conversion.metal")
            );
            let options = CompileOptions::new();
            // Preserve finite/NaN and reciprocal semantics in ray intersection.
            options.set_fast_math_enabled(false);
            let library = device
                .new_library_with_source(&source, &options)
                .map_err(|error| anyhow!("Metal shader compilation failed: {error}"))?;
            let render_function = library
                .get_function("render_main", None)
                .map_err(|error| anyhow!(error))?;
            let convert_function = library
                .get_function("rgba_to_nv12", None)
                .map_err(|error| anyhow!(error))?;
            let preview_function = library
                .get_function("preview_main", None)
                .map_err(|e| anyhow!(e))?;
            let preview_pipeline = device
                .new_compute_pipeline_state_with_function(&preview_function)
                .map_err(|e| anyhow!(e))?;
            let preview_resources = PreviewResources::new(&device, &library)?;
            let render_pipeline = device
                .new_compute_pipeline_state_with_function(&render_function)
                .map_err(|error| anyhow!(error))?;
            let convert_pipeline = device
                .new_compute_pipeline_state_with_function(&convert_function)
                .map_err(|error| anyhow!(error))?;
            let texture_cache =
                CVMetalTextureCache::new(None, device.clone(), None).map_err(|status| {
                    anyhow!("CoreVideo Metal texture cache creation failed: {status}")
                })?;
            let queue = device.new_command_queue();
            Ok(Self {
                device,
                queue,
                render_pipeline,
                preview_pipeline,
                accelerated_preview,
                preview_resources,
                convert_pipeline,
                texture_cache,
                geometry: None,
                geometry_hash: None,
                checked_revision: None,
                film: None,
                accumulation_key: None,
                samples: 0,
                frame: None,
            })
        })
    }

    pub fn device_name(&self) -> String {
        self.device.name().to_string()
    }

    /// One sample per call. Camera, world and settings invalidate the film
    /// automatically; callers increment revision after geometry/material edits.
    /// Odd dimensions round up for NV12's 2x2 chroma planes.
    pub fn render(
        &mut self,
        scene: &Scene,
        settings: &RenderSettings,
        revision: u64,
    ) -> Result<Frame> {
        objc::rc::autoreleasepool(|| self.render_inner(scene, settings, revision))
    }

    fn render_inner(
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
        // Prepare before changing the film or geometry: a rejected HDR load
        // leaves the last published and exportable frame intact.
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
        let width = (settings.width + 1) & !1;
        let height = (settings.height + 1) & !1;
        if self.checked_revision != Some(revision) {
            scene.validate().context("Cannot render invalid scene")?;
            let hash = geometry_hash(scene);
            if self.geometry_hash != Some(hash) {
                let geometry = Geometry::from_scene(scene);
                self.geometry = Some(GpuGeometry {
                    triangles: upload(&self.device, &geometry.triangles),
                    nodes: upload(&self.device, &geometry.nodes),
                    lights: upload(&self.device, &geometry.lights),
                    counts: [
                        geometry.triangles.len() as u32,
                        geometry.nodes.len() as u32,
                        geometry.lights.len() as u32,
                    ],
                    preview_acceleration: None,
                });
                self.geometry_hash = Some(hash);
            }
            self.checked_revision = Some(revision);
        }
        if !self
            .film
            .as_ref()
            .is_some_and(|film| film.width == width && film.height == height)
        {
            self.film = Some(Film::new(&self.device, width, height)?);
            self.accumulation_key = None;
            self.texture_cache.flush(0);
        }
        let geometry = self.geometry.as_mut().context("Missing GPU geometry")?;
        if environment_key.is_some() && self.accelerated_preview {
            geometry.prepare_preview_acceleration(&self.device, &self.queue)?;
        }
        let mut uniforms = Uniforms::new(
            scene,
            settings,
            width,
            height,
            self.samples,
            geometry.counts,
        );
        let mut hasher = DefaultHasher::new();
        // Sample count and stopping target don't change the estimator. Increasing
        // max_samples therefore resumes the existing film without throwing it away.
        let mut key_uniforms = uniforms;
        key_uniforms.image[2] = 0;
        if settings.mode == crate::RenderMode::Rendered {
            // Editor-only state has no effect on the rendered film.
            key_uniforms.settings[1] = 0.0;
            key_uniforms.settings[2] = 0.0;
        }
        bytemuck::bytes_of(&key_uniforms).hash(&mut hasher);
        self.geometry_hash.hash(&mut hasher);
        if settings.mode == crate::RenderMode::Rendered {
            // Preserve the path tracer's explicit revision invalidation contract.
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
        let surface = film
            .pool
            .create_pixel_buffer()
            .map_err(|status| anyhow!("CoreVideo pixel buffer allocation failed: {status}"))?;
        let texture_attributes = CFDictionary::from_CFType_pairs(&[(
            CVMetalTextureKeys::Usage.into(),
            CFNumber::from(
                (MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite).bits() as i64,
            )
            .as_CFType(),
        )]);
        let y = self
            .texture_cache
            .create_texture_from_image(
                surface.as_concrete_TypeRef(),
                Some(&texture_attributes),
                MTLPixelFormat::R8Unorm,
                width as usize,
                height as usize,
                0,
            )
            .map_err(|status| anyhow!("CoreVideo luma texture failed: {status}"))?;
        let uv = self
            .texture_cache
            .create_texture_from_image(
                surface.as_concrete_TypeRef(),
                Some(&texture_attributes),
                MTLPixelFormat::RG8Unorm,
                width as usize / 2,
                height as usize / 2,
                1,
            )
            .map_err(|status| anyhow!("CoreVideo chroma texture failed: {status}"))?;
        let command = self.queue.new_command_buffer();
        command.set_label("Forma viewport sample and surface conversion");
        {
            let encoder = command.new_compute_command_encoder();
            let pipeline = if settings.mode == RenderMode::MaterialPreview {
                &self.preview_pipeline
            } else {
                &self.render_pipeline
            };
            encoder.set_compute_pipeline_state(pipeline);
            encoder.set_buffer(0, Some(&geometry.triangles), 0);
            encoder.set_buffer(1, Some(&geometry.nodes), 0);
            encoder.set_bytes(
                2,
                std::mem::size_of::<Uniforms>() as u64,
                (&uniforms as *const Uniforms).cast(),
            );
            encoder.set_buffer(3, Some(&film.accumulation), 0);
            encoder.set_buffer(4, Some(&geometry.lights), 0);
            encoder.set_texture(0, Some(&film.rgba));
            if let Some(key) = &environment_key {
                if self.accelerated_preview {
                    encoder.set_acceleration_structure(5, geometry.preview_acceleration.as_deref());
                }
                let env = self
                    .preview_resources
                    .get(key)
                    .context("Missing prepared preview environment")?;
                encoder.set_texture(1, Some(&env.radiance));
                encoder.set_texture(2, Some(&env.diffuse));
                encoder.set_texture(3, Some(&env.specular));
                encoder.set_texture(4, self.preview_resources.brdf.as_deref());
            }
            dispatch(encoder, pipeline, width, height);
            encoder.end_encoding();
        }
        {
            let encoder = command.new_compute_command_encoder();
            encoder.set_compute_pipeline_state(&self.convert_pipeline);
            encoder.set_texture(0, Some(&film.rgba));
            encoder.set_texture(1, Some(texture_ref(&y)?));
            encoder.set_texture(2, Some(texture_ref(&uv)?));
            dispatch(encoder, &self.convert_pipeline, width / 2, height / 2);
            encoder.end_encoding();
        }
        command.commit();
        command.wait_until_completed();
        ensure!(
            command.status() == MTLCommandBufferStatus::Completed,
            "Metal render command failed ({:?})",
            command.status()
        );
        // Keep CVMetalTexture wrappers alive until completion, retaining their
        // underlying IOSurface ownership across both command encoders.
        drop((y, uv));
        self.samples += 1;
        let frame = Frame {
            surface,
            samples: self.samples,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        };
        self.frame = Some(frame.clone());
        Ok(frame)
    }

    /// Explicit GPU-to-CPU transfer, used only for export and image regression QA.
    pub fn export_png(&mut self, path: &Path) -> Result<()> {
        objc::rc::autoreleasepool(|| {
            let film = self
                .film
                .as_ref()
                .filter(|_| self.frame.is_some())
                .context("Render an image before exporting")?;
            let stride = (film.width as usize * 4).div_ceil(256) * 256;
            let readback = self.device.new_buffer(
                (stride * film.height as usize) as u64,
                MTLResourceOptions::StorageModeShared,
            );
            let command = self.queue.new_command_buffer();
            let encoder = command.new_blit_command_encoder();
            encoder.copy_from_texture_to_buffer(
                &film.rgba,
                0,
                0,
                MTLOrigin { x: 0, y: 0, z: 0 },
                MTLSize {
                    width: film.width as u64,
                    height: film.height as u64,
                    depth: 1,
                },
                &readback,
                0,
                stride as u64,
                (stride * film.height as usize) as u64,
                MTLBlitOption::None,
            );
            encoder.end_encoding();
            command.commit();
            command.wait_until_completed();
            ensure!(
                command.status() == MTLCommandBufferStatus::Completed,
                "Metal image readback failed"
            );
            // SAFETY: command completion makes the complete shared buffer visible
            // to the CPU; the buffer lives for the lifetime of this borrowed slice.
            let source = unsafe {
                std::slice::from_raw_parts(
                    readback.contents().cast::<u8>(),
                    stride * film.height as usize,
                )
            };
            let mut pixels = Vec::with_capacity(film.width as usize * film.height as usize * 4);
            for row in source.chunks_exact(stride) {
                pixels.extend_from_slice(&row[..film.width as usize * 4]);
            }
            image::save_buffer_with_format(
                path,
                &pixels,
                film.width,
                film.height,
                image::ColorType::Rgba8,
                image::ImageFormat::Png,
            )
            .with_context(|| format!("Could not export {}", path.display()))
        })
    }
}

impl GpuGeometry {
    /// Reuse the world-space triangle buffer and its ordering: primitive IDs are
    /// exactly the indices used by surface_at. Camera changes never rebuild it.
    fn prepare_preview_acceleration(
        &mut self,
        device: &DeviceRef,
        queue: &CommandQueueRef,
    ) -> Result<()> {
        if self.preview_acceleration.is_some() {
            return Ok(());
        }
        let count = self.counts[0];
        // A valid binding is required even when the shader skips an empty scene.
        let dummy = (count == 0).then(|| upload(device, &[[0.0f32; 4]; 3]));
        let indices: Vec<u32> = if count == 0 {
            vec![0, 1, 2]
        } else {
            (0..count)
                .flat_map(|i| [i * 9, i * 9 + 1, i * 9 + 2])
                .collect()
        };
        let indices = upload(device, &indices);
        let triangles = AccelerationStructureTriangleGeometryDescriptor::descriptor();
        triangles.set_vertex_buffer(Some(dummy.as_deref().unwrap_or(&self.triangles)));
        triangles.set_vertex_stride(16);
        triangles.set_index_buffer(Some(&indices));
        triangles.set_index_type(MTLIndexType::UInt32);
        triangles.set_triangle_count(count.max(1) as u64);
        triangles.set_opaque(true);
        let descriptor = PrimitiveAccelerationStructureDescriptor::descriptor();
        descriptor.set_geometry_descriptors(Array::from_owned_slice(&[triangles.into()]));
        let sizes = device.acceleration_structure_sizes_with_descriptor(&descriptor);
        let acceleration =
            device.new_acceleration_structure_with_size(sizes.acceleration_structure_size);
        acceleration.set_label("Forma preview geometry");
        let scratch = device.new_buffer(
            sizes.build_scratch_buffer_size.max(16),
            MTLResourceOptions::StorageModePrivate,
        );
        let command = queue.new_command_buffer();
        command.set_label("Forma build preview acceleration structure");
        let encoder = command.new_acceleration_structure_command_encoder();
        encoder.build_acceleration_structure(&acceleration, &descriptor, &scratch, 0);
        encoder.end_encoding();
        command.commit();
        command.wait_until_completed();
        ensure!(
            command.status() == MTLCommandBufferStatus::Completed,
            "Metal preview acceleration build failed ({:?})",
            command.status()
        );
        // Build inputs and scratch remain alive until GPU completion. The built
        // structure owns the traversal data needed by subsequent preview frames.
        self.preview_acceleration = Some(acceleration);
        Ok(())
    }
}

impl Film {
    fn new(device: &DeviceRef, width: u32, height: u32) -> Result<Self> {
        let descriptor = TextureDescriptor::new();
        descriptor.set_texture_type(MTLTextureType::D2);
        descriptor.set_pixel_format(MTLPixelFormat::RGBA8Unorm);
        descriptor.set_width(width as u64);
        descriptor.set_height(height as u64);
        descriptor.set_storage_mode(MTLStorageMode::Private);
        descriptor.set_usage(MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite);
        let rgba = device.new_texture(&descriptor);
        let accumulation = device.new_buffer(
            width as u64 * height as u64 * 16,
            MTLResourceOptions::StorageModePrivate,
        );
        let empty = CFDictionary::<CFString, CFType>::from_CFType_pairs(&[]);
        let attributes = CFDictionary::from_CFType_pairs(&[
            (
                CVPixelBufferKeys::PixelFormatType.into(),
                CFNumber::from(kCVPixelFormatType_420YpCbCr8BiPlanarFullRange as i64).as_CFType(),
            ),
            (
                CVPixelBufferKeys::Width.into(),
                CFNumber::from(width as i64).as_CFType(),
            ),
            (
                CVPixelBufferKeys::Height.into(),
                CFNumber::from(height as i64).as_CFType(),
            ),
            (
                CVPixelBufferKeys::MetalCompatibility.into(),
                CFBoolean::true_value().as_CFType(),
            ),
            (
                CVPixelBufferKeys::IOSurfaceProperties.into(),
                empty.as_CFType(),
            ),
        ]);
        let pool = CVPixelBufferPool::new(None, Some(&attributes))
            .map_err(|status| anyhow!("CoreVideo surface pool creation failed: {status}"))?;
        Ok(Self {
            width,
            height,
            rgba,
            accumulation,
            pool,
        })
    }
}

impl Uniforms {
    fn new(
        scene: &Scene,
        settings: &RenderSettings,
        width: u32,
        height: u32,
        sample: u32,
        counts: [u32; 3],
    ) -> Self {
        let camera = scene.camera;
        let view_inverse = camera.view_matrix().inverse();
        let vector = |v: Vec3| {
            let v = view_inverse.transform_vector3(v).normalize();
            [v.x, v.y, v.z, 0.0]
        };
        let position = camera.position();
        Self {
            cam_origin: [position.x, position.y, position.z, 0.0],
            cam_right: vector(Vec3::X),
            cam_up: vector(Vec3::Y),
            cam_forward: vector(Vec3::NEG_Z),
            world: if settings.mode == RenderMode::MaterialPreview
                && !settings.preview.use_scene_world
            {
                [0.0; 4]
            } else {
                [
                    scene.world.color.x,
                    scene.world.color.y,
                    scene.world.color.z,
                    scene.world.strength,
                ]
            },
            camera: [
                (camera.fov_y * 0.5).tan(),
                width as f32 / height as f32,
                camera.distance * (camera.fov_y * 0.5).tan(),
                u32::from(camera.orthographic) as f32,
            ],
            image: [width, height, sample, settings.mode as u32],
            scene: [
                counts[0],
                counts[1],
                counts[2],
                if settings.mode == RenderMode::Rendered {
                    settings.max_bounces.clamp(1, 32)
                } else {
                    1
                },
            ],
            settings: [
                settings.exposure,
                u32::from(settings.show_grid) as f32,
                settings
                    .selected
                    .and_then(|id| scene.objects.iter().position(|object| object.id == id))
                    .map_or(0.0, |index| (index + 1) as f32),
                0.0,
            ],
            preview_lighting: if settings.mode == RenderMode::MaterialPreview {
                if settings.preview.use_scene_world {
                    [1.0, 0.0, 1.0, 1.0]
                } else {
                    [
                        settings.preview.rotation.cos(),
                        settings.preview.rotation.sin(),
                        settings.preview.strength,
                        0.0,
                    ]
                }
            } else {
                [0.0; 4]
            },
            preview_display: if settings.mode == RenderMode::MaterialPreview {
                [
                    settings.preview.world_opacity,
                    if settings.preview.use_scene_world {
                        0.0
                    } else {
                        settings.preview.background_blur
                    },
                    u32::from(settings.preview.ambient_occlusion) as f32,
                    0.5,
                ]
            } else {
                [0.0; 4]
            },
        }
    }
}

pub(super) fn dispatch(
    encoder: &ComputeCommandEncoderRef,
    pipeline: &ComputePipelineStateRef,
    width: u32,
    height: u32,
) {
    let x = pipeline.thread_execution_width().min(16);
    let y = (pipeline.max_total_threads_per_threadgroup() / x).min(8);
    encoder.dispatch_thread_groups(
        MTLSize {
            width: (width as u64).div_ceil(x),
            height: (height as u64).div_ceil(y),
            depth: 1,
        },
        MTLSize {
            width: x,
            height: y,
            depth: 1,
        },
    );
}

fn texture_ref(texture: &CVMetalTexture) -> Result<&TextureRef> {
    // CVMetalTextureGetTexture follows the get rule. Borrow instead of taking
    // ownership via CVMetalTexture::get_texture, whose wrapper does not retain.
    let pointer = unsafe { CVMetalTextureGetTexture(texture.as_concrete_TypeRef()) };
    ensure!(
        !pointer.is_null(),
        "CoreVideo returned a null Metal texture"
    );
    Ok(unsafe { TextureRef::from_ptr(pointer) })
}

fn upload<T: Pod>(device: &DeviceRef, values: &[T]) -> Buffer {
    if values.is_empty() {
        return device.new_buffer(
            std::mem::size_of::<T>().max(16) as u64,
            MTLResourceOptions::StorageModeShared,
        );
    }
    let bytes: &[u8] = bytemuck::cast_slice(values);
    device.new_buffer_with_data(
        bytes.as_ptr().cast(),
        bytes.len() as u64,
        MTLResourceOptions::StorageModeShared,
    )
}

fn geometry_hash(scene: &Scene) -> u64 {
    let mut hasher = DefaultHasher::new();
    scene.collections.len().hash(&mut hasher);
    for collection in &scene.collections {
        collection.id.hash(&mut hasher);
        collection.parent.hash(&mut hasher);
        collection.visible.hash(&mut hasher);
    }
    scene.objects.len().hash(&mut hasher);
    for object in &scene.objects {
        object.id.hash(&mut hasher);
        object.visible.hash(&mut hasher);
        object.parent.hash(&mut hasher);
        object.collections.hash(&mut hasher);
        if !scene.is_effectively_visible(object.id) {
            continue;
        }
        let Some(instance) = scene.mesh_instance(object.id) else {
            continue;
        };
        for p in &instance.mesh.positions {
            for n in p.to_array() {
                n.to_bits().hash(&mut hasher);
            }
        }
        instance.mesh.faces.hash(&mut hasher);
        for n in instance.world_transform.to_cols_array() {
            n.to_bits().hash(&mut hasher);
        }
        for n in object
            .data
            .material_ids()
            .iter()
            .filter_map(|id| scene.material(*id))
            .flat_map(|data| {
                data.material
                    .base_color
                    .to_array()
                    .into_iter()
                    .chain(data.material.emission.to_array())
                    .chain([data.material.roughness, data.material.metallic])
            })
        {
            n.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RenderMode;
    use forma_core::Primitive;

    // Read the untonemapped floating-point film. This deliberate test-only
    // transfer checks radiometry, which an 8-bit screenshot cannot establish.
    fn mean_linear(renderer: &Renderer) -> Vec3 {
        objc::rc::autoreleasepool(|| {
            let film = renderer.film.as_ref().unwrap();
            let size = film.width as u64 * film.height as u64 * 16;
            let buffer = renderer
                .device
                .new_buffer(size, MTLResourceOptions::StorageModeShared);
            let command = renderer.queue.new_command_buffer();
            let encoder = command.new_blit_command_encoder();
            encoder.copy_from_buffer(&film.accumulation, 0, &buffer, 0, size);
            encoder.end_encoding();
            command.commit();
            command.wait_until_completed();
            assert_eq!(command.status(), MTLCommandBufferStatus::Completed);
            // SAFETY: completed GPU copy, aligned Metal allocation, fixed length.
            let values = unsafe {
                std::slice::from_raw_parts(
                    buffer.contents().cast::<[f32; 4]>(),
                    (film.width * film.height) as usize,
                )
            };
            values
                .iter()
                .map(|value| Vec3::from_slice(value) / value[3])
                .sum::<Vec3>()
                / values.len() as f32
        })
    }

    #[test]
    fn white_furnace_preserves_diffuse_energy_and_linear_world_scaling() {
        let mut renderer = Renderer::new().unwrap();
        let mut scene = Scene::default();
        scene.objects.clear();
        let id = scene.add(Primitive::Plane);
        let plane = scene.object_mut(id).unwrap();
        plane.transform.scale = Vec3::splat(100.0);
        let material = scene.object_material_mut(id).unwrap();
        material.base_color = Vec3::splat(0.5);
        material.roughness = 1.0;
        scene.camera.target = Vec3::ZERO;
        scene.camera.distance = 2.0;
        scene.camera.pitch = 1.3;
        scene.world.color = Vec3::ONE;
        scene.world.strength = 1.0;
        let settings = RenderSettings {
            width: 32,
            height: 32,
            mode: RenderMode::Rendered,
            max_samples: 48,
            max_bounces: 1,
            show_grid: false,
            ..Default::default()
        };
        for _ in 0..settings.max_samples {
            renderer.render(&scene, &settings, 1).unwrap();
        }
        let mean = mean_linear(&renderer);
        // A 50% diffuse dielectric has ~48% diffuse + a few percent single-
        // scattering specular energy. This catches MIS double counting and
        // terminal-bounce loss without requiring a particular noise realization.
        assert!(
            (0.46..0.54).contains(&mean.x),
            "Unexpected furnace energy: {mean:?}"
        );
        scene.world.strength = 2.0;
        for _ in 0..settings.max_samples {
            renderer.render(&scene, &settings, 1).unwrap();
        }
        let doubled = mean_linear(&renderer);
        assert!(
            (doubled - mean * 2.0).abs().max_element() < 0.002,
            "Radiance must scale linearly: {mean:?}, {doubled:?}"
        );
    }

    #[test]
    fn preview_furnace_preserves_linear_energy_and_emission() {
        let mut renderer = Renderer::new().unwrap();
        let mut scene = Scene::default();
        scene.objects.clear();
        let id = scene.add(Primitive::Plane);
        scene.object_mut(id).unwrap().transform.scale = Vec3::splat(100.0);
        scene.camera.target = Vec3::ZERO;
        scene.camera.distance = 2.0;
        scene.camera.pitch = 1.55;
        scene.world.color = Vec3::ONE;
        scene.world.strength = 1.0;
        let mut settings = RenderSettings {
            width: 16,
            height: 16,
            mode: RenderMode::MaterialPreview,
            show_grid: false,
            preview: crate::PreviewSettings {
                use_scene_world: true,
                ambient_occlusion: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let material = scene.object_material_mut(id).unwrap();
        material.base_color = Vec3::splat(0.5);
        material.roughness = 1.0;
        renderer.render(&scene, &settings, 1).unwrap();
        let diffuse = mean_linear(&renderer);
        assert!(
            (0.48..0.53).contains(&diffuse.x),
            "Diffuse furnace energy: {diffuse:?}"
        );
        let material = scene.object_material_mut(id).unwrap();
        material.base_color = Vec3::new(0.7, 0.4, 0.2);
        material.roughness = 0.025;
        material.metallic = 1.0;
        renderer.render(&scene, &settings, 2).unwrap();
        let mirror = mean_linear(&renderer);
        assert!(
            (mirror - Vec3::new(0.7, 0.4, 0.2)).abs().max_element() < 0.012,
            "Metal furnace energy: {mirror:?}"
        );
        scene.world.strength = 2.0;
        renderer.render(&scene, &settings, 2).unwrap();
        assert!((mean_linear(&renderer) - mirror * 2.0).abs().max_element() < 0.002);
        scene.world.strength = 0.0;
        scene.object_material_mut(id).unwrap().emission = Vec3::new(3.0, 1.5, 0.5);
        renderer.render(&scene, &settings, 3).unwrap();
        assert!(
            (mean_linear(&renderer) - Vec3::new(3.0, 1.5, 0.5))
                .abs()
                .max_element()
                < 0.002,
            "Emission is visible radiance independent of world energy"
        );
        settings.preview.use_scene_world = false;
        settings.preview.strength = 0.0;
        renderer.render(&scene, &settings, 3).unwrap();
        assert!(
            (mean_linear(&renderer) - Vec3::new(3.0, 1.5, 0.5))
                .abs()
                .max_element()
                < 0.002
        );
        // The baked texture route must retain the same linear radiometry as
        // an analytic constant world, including HDR values above one.
        let path =
            std::env::temp_dir().join(format!("forma-preview-furnace-{}.hdr", std::process::id()));
        let pixels = vec![image::Rgb([2.0_f32, 1.0, 0.5]); 16 * 8];
        image::codecs::hdr::HdrEncoder::new(std::fs::File::create(&path).unwrap())
            .encode(&pixels, 16, 8)
            .unwrap();
        settings.preview.hdri_path = Some(path.clone());
        settings.preview.strength = 1.0;
        scene.object_material_mut(id).unwrap().emission = Vec3::ZERO;
        renderer.render(&scene, &settings, 4).unwrap();
        let baked = mean_linear(&renderer);
        assert!(
            (baked - mirror * Vec3::new(2.0, 1.0, 0.5))
                .abs()
                .max_element()
                < 0.003,
            "Constant HDR must survive irradiance, GGX and BRDF baking: {baked:?}"
        );
        settings.preview.strength = 2.0;
        renderer.render(&scene, &settings, 4).unwrap();
        assert!((mean_linear(&renderer) - baked * 2.0).abs().max_element() < 0.003);
        std::fs::remove_file(path).unwrap();
    }
}

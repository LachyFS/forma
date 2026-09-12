//! Cached HDR environments and GPU image-based lighting preparation for wgpu.
use std::collections::VecDeque;

use anyhow::{Context, Result};
use wgpu::util::DeviceExt;

use crate::{
    environment::{
        CACHE_CAPACITY, ENV_HEIGHT, ENV_WIDTH, EnvironmentKey, SPECULAR_HEIGHT, SPECULAR_SLICES,
        SPECULAR_WIDTH, load_environment,
    },
    portable::{buffer_layout, checked_gpu, pipeline, storage_texture_layout, texture_layout},
};

pub(super) struct Environment {
    pub group: wgpu::BindGroup,
}

pub(super) struct PreviewResources {
    pub bindings: wgpu::BindGroupLayout,
    bake_bindings: wgpu::BindGroupLayout,
    empty_group: wgpu::BindGroup,
    diffuse_pipeline: wgpu::ComputePipeline,
    specular_pipeline: wgpu::ComputePipeline,
    brdf_pipeline: wgpu::ComputePipeline,
    brdf: Option<wgpu::TextureView>,
    cache: VecDeque<(EnvironmentKey, Environment)>,
}

impl PreviewResources {
    pub fn new(device: &wgpu::Device) -> Result<Self> {
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Forma environment lighting bindings"),
            entries: &[
                texture_layout(0, wgpu::TextureViewDimension::D2),
                texture_layout(1, wgpu::TextureViewDimension::D2),
                texture_layout(2, wgpu::TextureViewDimension::D2Array),
                texture_layout(3, wgpu::TextureViewDimension::D2),
            ],
        });
        let bake_bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Forma IBL preparation bindings"),
            entries: &[
                texture_layout(0, wgpu::TextureViewDimension::D2),
                storage_texture_layout(1, wgpu::TextureFormat::Rgba32Float),
                buffer_layout(2, wgpu::BufferBindingType::Uniform),
            ],
        });
        let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Forma empty bake bindings"),
            entries: &[],
        });
        let empty_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Forma empty bake group"),
            layout: &empty,
            entries: &[],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Forma IBL preparation layout"),
            bind_group_layouts: &[&empty, &empty, &bake_bindings],
            push_constant_ranges: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Forma IBL preparation"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    include_str!("shader.wgsl"),
                    include_str!("ibl.wgsl")
                )
                .into(),
            ),
        });
        Ok(Self {
            bindings,
            bake_bindings,
            empty_group,
            diffuse_pipeline: pipeline(device, &module, &layout, "bake_diffuse"),
            specular_pipeline: pipeline(device, &module, &layout, "bake_specular"),
            brdf_pipeline: pipeline(device, &module, &layout, "bake_brdf"),
            brdf: None,
            cache: VecDeque::new(),
        })
    }

    pub fn get(&self, key: &EnvironmentKey) -> Option<&Environment> {
        self.cache
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, environment)| environment)
    }

    pub fn prepare(
        &mut self,
        key: &EnvironmentKey,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<()> {
        if self.get(key).is_some() {
            return Ok(());
        }
        let pixels = load_environment(key)?;
        let (environment, new_brdf) =
            checked_gpu(device, "Prepare Material Preview environment", || {
                let radiance = texture(device, "Forma HDR radiance", ENV_WIDTH, ENV_HEIGHT, 10, 1);
                upload_mips(queue, &radiance, pixels, ENV_WIDTH, ENV_HEIGHT);
                let radiance_view = radiance.create_view(&wgpu::TextureViewDescriptor::default());
                let diffuse = texture(device, "Forma diffuse irradiance", 64, 32, 1, 1);
                let diffuse_view = diffuse.create_view(&wgpu::TextureViewDescriptor::default());
                let specular = texture(
                    device,
                    "Forma specular environment",
                    SPECULAR_WIDTH,
                    SPECULAR_HEIGHT,
                    1,
                    SPECULAR_SLICES,
                );
                let specular_view = specular.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                });
                let new_brdf = self.brdf.is_none().then(|| {
                    texture(device, "Forma integrated BRDF", 128, 128, 1, 1)
                        .create_view(&wgpu::TextureViewDescriptor::default())
                });
                let brdf = self
                    .brdf
                    .as_ref()
                    .or(new_brdf.as_ref())
                    .context("Missing BRDF texture")?;
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Forma bake Material Preview environment"),
                });
                let mut bake = |pipeline: &wgpu::ComputePipeline,
                                view: &wgpu::TextureView,
                                roughness: f32,
                                width: u32,
                                height: u32| {
                    let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Forma bake roughness"),
                        contents: bytemuck::bytes_of(&[roughness, 0.0, 0.0, 0.0]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Forma IBL bake"),
                        layout: &self.bake_bindings,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&radiance_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: uniforms.as_entire_binding(),
                            },
                        ],
                    });
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("Forma IBL preparation"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &self.empty_group, &[]);
                    pass.set_bind_group(1, &self.empty_group, &[]);
                    pass.set_bind_group(2, &group, &[]);
                    pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
                };
                if new_brdf.is_some() {
                    bake(&self.brdf_pipeline, brdf, 0.0, 128, 128);
                }
                bake(&self.diffuse_pipeline, &diffuse_view, 0.0, 64, 32);
                for layer in 0..SPECULAR_SLICES {
                    let view = specular.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2),
                        base_array_layer: layer,
                        array_layer_count: Some(1),
                        ..Default::default()
                    });
                    bake(
                        &self.specular_pipeline,
                        &view,
                        layer as f32 / (SPECULAR_SLICES - 1) as f32,
                        SPECULAR_WIDTH,
                        SPECULAR_HEIGHT,
                    );
                }
                queue.submit([encoder.finish()]);
                device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .context("Material Preview bake completion failed")?;
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Forma prepared environment"),
                    layout: &self.bindings,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&radiance_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&diffuse_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&specular_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(brdf),
                        },
                    ],
                });
                // Bind groups retain their views and textures; they outlive this bake.
                Ok((Environment { group }, new_brdf))
            })?;
        if new_brdf.is_some() {
            self.brdf = new_brdf;
        }
        if self.cache.len() == CACHE_CAPACITY {
            self.cache.pop_front();
        }
        self.cache.push_back((key.clone(), environment));
        Ok(())
    }
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    levels: u32,
    layers: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: layers,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// Box reduction on linear radiance supports all adapters, including devices
/// without float32 filtering. The shader manually interpolates these mip levels.
fn upload_mips(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mut pixels: Vec<[f32; 4]>,
    mut width: u32,
    mut height: u32,
) {
    for level in 0..texture.mip_level_count() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&pixels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 16),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        if level + 1 == texture.mip_level_count() {
            break;
        }
        let next_width = (width / 2).max(1);
        let next_height = (height / 2).max(1);
        let mut next = Vec::with_capacity((next_width * next_height) as usize);
        for y in 0..next_height {
            for x in 0..next_width {
                let mut average = [0.0; 4];
                for dy in 0..2 {
                    for dx in 0..2 {
                        let source = pixels[(((y * 2 + dy).min(height - 1)) * width
                            + (x * 2 + dx).min(width - 1))
                            as usize];
                        for channel in 0..4 {
                            average[channel] += source[channel] * 0.25;
                        }
                    }
                }
                next.push(average);
            }
        }
        pixels = next;
        width = next_width;
        height = next_height;
    }
}

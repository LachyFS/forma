//! Native Metal preparation using the common environment decoder.
pub(super) use crate::environment::EnvironmentKey;
use crate::environment::{
    CACHE_CAPACITY, ENV_HEIGHT, ENV_WIDTH, SPECULAR_HEIGHT, SPECULAR_WIDTH, load_environment,
};
use anyhow::{Result, anyhow, ensure};
use metal::*;
use std::collections::VecDeque;
const SPECULAR_SLICES: u64 = crate::environment::SPECULAR_SLICES as u64;

pub(super) struct Environment {
    pub radiance: Texture,
    pub diffuse: Texture,
    pub specular: Texture,
}

pub(super) struct PreviewResources {
    diffuse_pipeline: ComputePipelineState,
    specular_pipeline: ComputePipelineState,
    brdf_pipeline: ComputePipelineState,
    pub brdf: Option<Texture>,
    cache: VecDeque<(EnvironmentKey, Environment)>,
}

impl PreviewResources {
    pub fn new(device: &DeviceRef, library: &LibraryRef) -> Result<Self> {
        let pipeline = |name| -> Result<ComputePipelineState> {
            let function = library.get_function(name, None).map_err(|e| anyhow!(e))?;
            device
                .new_compute_pipeline_state_with_function(&function)
                .map_err(|e| anyhow!(e))
        };
        Ok(Self {
            diffuse_pipeline: pipeline("bake_diffuse")?,
            specular_pipeline: pipeline("bake_specular")?,
            brdf_pipeline: pipeline("bake_brdf")?,
            brdf: None,
            cache: VecDeque::new(),
        })
    }

    pub fn get(&self, key: &EnvironmentKey) -> Option<&Environment> {
        self.cache.iter().find(|(k, _)| k == key).map(|(_, e)| e)
    }

    pub fn prepare(
        &mut self,
        key: &EnvironmentKey,
        device: &DeviceRef,
        queue: &CommandQueueRef,
    ) -> Result<()> {
        if self.get(key).is_some() {
            return Ok(());
        }
        let pixels = load_environment(key)?;
        let radiance = texture(device, ENV_WIDTH, ENV_HEIGHT, 10, MTLStorageMode::Shared);
        radiance.replace_region(
            MTLRegion::new_2d(0, 0, ENV_WIDTH as u64, ENV_HEIGHT as u64),
            0,
            pixels.as_ptr().cast(),
            ENV_WIDTH as u64 * 16,
        );
        let diffuse = texture(device, 64, 32, 1, MTLStorageMode::Private);
        let descriptor = TextureDescriptor::new();
        descriptor.set_texture_type(MTLTextureType::D2Array);
        descriptor.set_pixel_format(MTLPixelFormat::RGBA32Float);
        descriptor.set_width(SPECULAR_WIDTH as u64);
        descriptor.set_height(SPECULAR_HEIGHT as u64);
        descriptor.set_array_length(SPECULAR_SLICES);
        descriptor.set_storage_mode(MTLStorageMode::Private);
        descriptor.set_usage(
            MTLTextureUsage::ShaderRead
                | MTLTextureUsage::ShaderWrite
                | MTLTextureUsage::PixelFormatView,
        );
        let specular = device.new_texture(&descriptor);
        let command = queue.new_command_buffer();
        command.set_label("Bake cached Material Preview environment");
        let blit = command.new_blit_command_encoder();
        blit.generate_mipmaps(&radiance);
        blit.end_encoding();
        let new_brdf = self
            .brdf
            .is_none()
            .then(|| texture(device, 128, 128, 1, MTLStorageMode::Private));
        if let Some(brdf) = &new_brdf {
            let encoder = command.new_compute_command_encoder();
            encoder.set_compute_pipeline_state(&self.brdf_pipeline);
            encoder.set_texture(0, Some(brdf));
            super::native::dispatch(encoder, &self.brdf_pipeline, 128, 128);
            encoder.end_encoding();
        }
        let encoder = command.new_compute_command_encoder();
        encoder.set_compute_pipeline_state(&self.diffuse_pipeline);
        encoder.set_texture(0, Some(&radiance));
        encoder.set_texture(1, Some(&diffuse));
        super::native::dispatch(encoder, &self.diffuse_pipeline, 64, 32);
        encoder.end_encoding();
        // Keep texture views alive until the bake command has completed.
        let mut views = Vec::new();
        for layer in 0..SPECULAR_SLICES {
            let view = specular.new_texture_view_from_slice(
                MTLPixelFormat::RGBA32Float,
                MTLTextureType::D2,
                NSRange {
                    location: 0,
                    length: 1,
                },
                NSRange {
                    location: layer,
                    length: 1,
                },
            );
            let roughness = layer as f32 / (SPECULAR_SLICES - 1) as f32;
            let encoder = command.new_compute_command_encoder();
            encoder.set_compute_pipeline_state(&self.specular_pipeline);
            encoder.set_texture(0, Some(&radiance));
            encoder.set_texture(1, Some(&view));
            encoder.set_bytes(0, 4, (&roughness as *const f32).cast());
            super::native::dispatch(
                encoder,
                &self.specular_pipeline,
                SPECULAR_WIDTH,
                SPECULAR_HEIGHT,
            );
            encoder.end_encoding();
            views.push(view);
        }
        command.commit();
        command.wait_until_completed();
        ensure!(
            command.status() == MTLCommandBufferStatus::Completed,
            "Material Preview environment bake failed ({:?})",
            command.status()
        );
        if new_brdf.is_some() {
            self.brdf = new_brdf;
        }
        if self.cache.len() == CACHE_CAPACITY {
            self.cache.pop_front();
        }
        self.cache.push_back((
            key.clone(),
            Environment {
                radiance,
                diffuse,
                specular,
            },
        ));
        Ok(())
    }
}

fn texture(
    device: &DeviceRef,
    width: u32,
    height: u32,
    levels: u64,
    storage: MTLStorageMode,
) -> Texture {
    let descriptor = TextureDescriptor::new();
    descriptor.set_texture_type(MTLTextureType::D2);
    descriptor.set_pixel_format(MTLPixelFormat::RGBA32Float);
    descriptor.set_width(width as u64);
    descriptor.set_height(height as u64);
    descriptor.set_mipmap_level_count(levels);
    descriptor.set_storage_mode(storage);
    descriptor.set_usage(
        MTLTextureUsage::ShaderRead
            | MTLTextureUsage::ShaderWrite
            | MTLTextureUsage::PixelFormatView,
    );
    device.new_texture(&descriptor)
}

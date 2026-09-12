//! Bounded, cached scene-linear environment preparation for Material Preview.
use std::{
    collections::VecDeque,
    fs::File,
    hash::Hash,
    io::BufReader,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result, anyhow, ensure};
use glam::Vec3;
use image::{ImageDecoder, Rgb32FImage, codecs::hdr::HdrDecoder};
use metal::*;

use crate::StudioLight;

const ENV_WIDTH: u32 = 512;
const ENV_HEIGHT: u32 = 256;
const SPECULAR_WIDTH: u32 = 256;
const SPECULAR_HEIGHT: u32 = 128;
const SPECULAR_SLICES: u64 = 9;
const CACHE_CAPACITY: usize = 4;

/// Validate a Radiance HDR using the same bounded decoder as the preview baker.
/// Inputs are scene-linear, nonnegative RGB, at most 16,384 x 8,192 and 32 MP,
/// with files bounded to 128 MiB and individual radiance values to 1,000,000.
pub fn validate_hdri(path: &Path) -> Result<()> {
    decode_hdr(path).map(|_| ())
}

fn decode_hdr(path: &Path) -> Result<Rgb32FImage> {
    let file = File::open(path)
        .with_context(|| format!("Could not open HDR environment {}", path.display()))?;
    ensure!(
        file.metadata()?.len() <= 128 * 1024 * 1024,
        "HDR environment exceeds the 128 MiB file limit"
    );
    let decoder = HdrDecoder::new(BufReader::new(file))
        .context("Environment must be a valid Radiance .hdr image")?;
    let (width, height) = decoder.dimensions();
    ensure!(
        width > 0
            && height > 0
            && width <= 16384
            && height <= 8192
            && u64::from(width) * u64::from(height) <= 32 * 1024 * 1024,
        "HDR environment dimensions must be nonzero, at most 16,384 x 8,192 and 32 megapixels"
    );
    let mut values = vec![0.0_f32; width as usize * height as usize * 3];
    decoder
        .read_image(bytemuck::cast_slice_mut(&mut values))
        .context("Could not decode HDR environment pixels")?;
    ensure!(
        values
            .iter()
            .all(|v| v.is_finite() && (0.0..=1_000_000.0).contains(v)),
        "HDR environment contains nonfinite, negative or excessive radiance (maximum 1,000,000)"
    );
    Rgb32FImage::from_raw(width, height, values).context("Invalid HDR environment dimensions")
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum EnvironmentKey {
    Builtin(StudioLight),
    File {
        path: PathBuf,
        length: u64,
        modified: Option<SystemTime>,
    },
}

impl EnvironmentKey {
    pub fn new(studio: StudioLight, path: Option<&Path>) -> Result<Self> {
        if let Some(path) = path {
            let path = path
                .canonicalize()
                .with_context(|| format!("HDR environment is unavailable: {}", path.display()))?;
            let metadata = path.metadata()?;
            ensure!(metadata.is_file(), "HDR environment must be a file");
            Ok(Self::File {
                path,
                length: metadata.len(),
                modified: metadata.modified().ok(),
            })
        } else {
            Ok(Self::Builtin(studio))
        }
    }
}

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
        let pixels = match key {
            EnvironmentKey::Builtin(studio) => builtin(*studio),
            EnvironmentKey::File { path, .. } => {
                // Area-weighted resampling preserves bright small emitters when
                // reducing large HDRIs to the fixed viewport lighting budget.
                resize_radiance(&decode_hdr(path)?)
            }
        };
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

fn resample_weights(source: usize, target: usize, wrap: bool) -> Vec<Vec<(usize, f32)>> {
    let scale = source as f64 / target as f64;
    (0..target)
        .map(|index| {
            if source > target {
                let start = index as f64 * scale;
                let end = (index + 1) as f64 * scale;
                (start.floor() as usize..end.ceil() as usize)
                    .filter_map(|sample| {
                        let weight = ((sample + 1) as f64).min(end) - (sample as f64).max(start);
                        (weight > 0.0 && sample < source)
                            .then_some((sample, (weight / scale) as f32))
                    })
                    .collect()
            } else {
                let center = (index as f64 + 0.5) * scale - 0.5;
                let lower = center.floor() as isize;
                let fraction = (center - lower as f64) as f32;
                let address = |i: isize| {
                    if wrap {
                        i.rem_euclid(source as isize) as usize
                    } else {
                        i.clamp(0, source as isize - 1) as usize
                    }
                };
                vec![
                    (address(lower), 1.0 - fraction),
                    (address(lower + 1), fraction),
                ]
            }
        })
        .collect()
}

fn resize_radiance(source: &Rgb32FImage) -> Vec<[f32; 4]> {
    // Image-oriented float resizers may clamp to [0,1]. These separable area
    // weights retain HDR radiance and tiny bright texels when downsampling;
    // bilinear upsampling wraps the panorama seam and clamps the poles.
    let x_weights = resample_weights(source.width() as usize, ENV_WIDTH as usize, true);
    let y_weights = resample_weights(source.height() as usize, ENV_HEIGHT as usize, false);
    let mut horizontal = vec![Vec3::ZERO; ENV_WIDTH as usize * source.height() as usize];
    for y in 0..source.height() as usize {
        for (x, weights) in x_weights.iter().enumerate() {
            horizontal[y * ENV_WIDTH as usize + x] = weights
                .iter()
                .map(|&(sx, weight)| {
                    Vec3::from_array(source.get_pixel(sx as u32, y as u32).0) * weight
                })
                .sum();
        }
    }
    let mut pixels = Vec::with_capacity((ENV_WIDTH * ENV_HEIGHT) as usize);
    for weights in y_weights {
        for x in 0..ENV_WIDTH as usize {
            let color: Vec3 = weights
                .iter()
                .map(|&(sy, weight)| horizontal[sy * ENV_WIDTH as usize + x] * weight)
                .sum();
            pixels.push([color.x, color.y, color.z, 1.0]);
        }
    }
    pixels
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn panel(direction: Vec3, center: Vec3, half_width: f32, half_height: f32) -> f32 {
    let center = center.normalize();
    let right = center.cross(Vec3::Y).normalize();
    let up = right.cross(center);
    let forward = direction.dot(center);
    if forward <= 0.0 {
        return 0.0;
    }
    let x = (direction.dot(right) / forward).abs();
    let y = (direction.dot(up) / forward).abs();
    (1.0 - smooth(half_width * 0.88, half_width, x))
        * (1.0 - smooth(half_height * 0.88, half_height, y))
}

fn builtin(studio: StudioLight) -> Vec<[f32; 4]> {
    let mut pixels = Vec::with_capacity((ENV_WIDTH * ENV_HEIGHT) as usize);
    for y in 0..ENV_HEIGHT {
        let theta = (y as f32 + 0.5) / ENV_HEIGHT as f32 * std::f32::consts::PI;
        for x in 0..ENV_WIDTH {
            let phi = ((x as f32 + 0.5) / ENV_WIDTH as f32 - 0.5) * std::f32::consts::TAU;
            let direction = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            let d = direction;
            let color = match studio {
                StudioLight::Studio => {
                    let ambient = Vec3::new(0.055, 0.065, 0.08)
                        .lerp(Vec3::new(0.24, 0.27, 0.31), smooth(-0.4, 0.85, d.y));
                    ambient
                        + Vec3::new(8.0, 7.7, 7.2) * panel(d, Vec3::new(-0.6, 0.9, 0.55), 0.5, 0.38)
                        + Vec3::new(2.2, 2.8, 3.6)
                            * panel(d, Vec3::new(0.85, 0.3, -0.45), 0.28, 0.7)
                        + Vec3::new(4.5, 3.8, 3.0) * panel(d, Vec3::new(-0.3, 0.2, -0.9), 0.16, 0.8)
                }
                StudioLight::Courtyard => {
                    let sky = Vec3::new(0.32, 0.48, 0.78)
                        .lerp(Vec3::new(0.75, 1.15, 1.85), smooth(0.0, 0.95, d.y));
                    let ground = Vec3::new(0.17, 0.21, 0.16);
                    ground.lerp(sky, smooth(-0.04, 0.12, d.y))
                        + Vec3::new(10.0, 9.6, 8.5)
                            * smooth(0.972, 0.994, d.dot(Vec3::new(-0.35, 0.8, 0.48).normalize()))
                        + Vec3::new(0.45, 0.39, 0.3)
                            * panel(d, Vec3::new(0.8, 0.2, -0.5), 0.55, 0.65)
                }
                StudioLight::Sunset => {
                    let sky = Vec3::new(1.1, 0.33, 0.12)
                        .lerp(Vec3::new(0.11, 0.23, 0.48), smooth(0.0, 0.8, d.y));
                    Vec3::new(0.07, 0.045, 0.045).lerp(sky, smooth(-0.12, 0.08, d.y))
                        + Vec3::new(28.0, 10.0, 2.5)
                            * smooth(
                                0.987,
                                0.997,
                                d.dot(Vec3::new(-0.75, 0.18, 0.64).normalize()),
                            )
                }
            };
            pixels.push([color.x, color.y, color.z, 1.0]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn built_in_environments_are_hdr_finite_and_distinct() {
        let environments = [
            StudioLight::Studio,
            StudioLight::Courtyard,
            StudioLight::Sunset,
        ]
        .map(builtin);
        for env in &environments {
            assert_eq!(env.len(), (ENV_WIDTH * ENV_HEIGHT) as usize);
            assert!(env.iter().flatten().all(|v| v.is_finite() && *v >= 0.0));
            assert!(env.iter().any(|v| v[0] > 4.0));
        }
        assert_ne!(environments[0], environments[1]);
        assert_ne!(environments[1], environments[2]);
    }

    #[test]
    fn hdr_resampling_preserves_bright_texels_and_linear_values() {
        let source = Rgb32FImage::from_pixel(16, 8, image::Rgb([8.0, 2.0, 0.5]));
        assert!(resize_radiance(&source).iter().all(|pixel| {
            (Vec3::from_slice(pixel) - Vec3::new(8.0, 2.0, 0.5))
                .abs()
                .max_element()
                < 1e-5
        }));
        let mut source = Rgb32FImage::new(ENV_WIDTH * 2, ENV_HEIGHT * 2);
        source.put_pixel(345, 173, image::Rgb([10_000.0, 2_000.0, 100.0]));
        let energy: Vec3 = resize_radiance(&source)
            .iter()
            .map(|p| Vec3::from_slice(p))
            .sum();
        assert!(
            (energy - Vec3::new(2_500.0, 500.0, 25.0))
                .abs()
                .max_element()
                < 1e-3,
            "Area resampling must retain the energy of a subpixel HDR emitter: {energy:?}"
        );
    }
}

use std::{fmt, path::Path, str::FromStr, sync::Arc};

use anyhow::{Context, Result, bail};
use forma_core::Scene;

use crate::{DenoiseInput, DenoiseQuality, Denoiser, RenderMode, RenderSettings};

/// Renderer selection. `Auto` preserves native Metal on macOS and selects wgpu
/// DirectX 12 on Windows or wgpu Vulkan on Linux. Explicit choices never fall
/// back to a different graphics API.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Auto,
    Wgpu,
    NativeMetal,
    Metal,
    Dx12,
    Vulkan,
}

impl Backend {
    /// Read the optional process-wide default without changing the environment.
    pub fn from_env() -> Result<Self> {
        match std::env::var("FORMA_RENDERER") {
            Ok(value) => value.parse().context("Invalid FORMA_RENDERER"),
            Err(std::env::VarError::NotPresent) => Ok(Self::Auto),
            Err(error) => Err(error).context("Invalid FORMA_RENDERER"),
        }
    }

    fn platform_wgpu() -> Result<Self> {
        if cfg!(target_os = "macos") {
            Ok(Self::Metal)
        } else if cfg!(target_os = "windows") {
            Ok(Self::Dx12)
        } else if cfg!(target_os = "linux") {
            Ok(Self::Vulkan)
        } else {
            bail!("Forma supports macOS, Windows, and Linux")
        }
    }
}

impl FromStr for Backend {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "wgpu" => Ok(Self::Wgpu),
            "native-metal" => Ok(Self::NativeMetal),
            "metal" => Ok(Self::Metal),
            "dx12" | "directx12" | "d3d12" => Ok(Self::Dx12),
            "vulkan" => Ok(Self::Vulkan),
            _ => bail!(
                "Unknown renderer {value:?}; choose auto, wgpu, native-metal, metal, dx12, or vulkan"
            ),
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Wgpu => "wgpu",
            Self::NativeMetal => "native-metal",
            Self::Metal => "metal",
            Self::Dx12 => "dx12",
            Self::Vulkan => "vulkan",
        })
    }
}

#[derive(Clone)]
enum FrameStorage {
    #[cfg(target_os = "macos")]
    Native(crate::native::Frame),
    Rgba(crate::portable::Frame),
}

/// A completed immutable frame that can be retained across worker handoff,
/// resize, subsequent renders, and renderer destruction.
#[derive(Clone)]
pub struct Frame {
    storage: FrameStorage,
    pub samples: u32,
    pub elapsed_ms: f64,
    pub denoised: bool,
    pub denoise_device: Option<String>,
}

impl Frame {
    pub(crate) fn from_denoised(
        input: &DenoiseInput,
        rgba: Vec<u8>,
        device: String,
        elapsed_ms: f64,
    ) -> Self {
        Self {
            storage: FrameStorage::Rgba(crate::portable::Frame {
                width: input.width,
                height: input.height,
                rgba: rgba.into(),
                samples: input.samples,
                elapsed_ms,
            }),
            samples: input.samples,
            elapsed_ms,
            denoised: true,
            denoise_device: Some(device),
        }
    }

    pub fn width(&self) -> u32 {
        match &self.storage {
            #[cfg(target_os = "macos")]
            FrameStorage::Native(frame) => frame.surface.get_width() as u32,
            FrameStorage::Rgba(frame) => frame.width,
        }
    }

    pub fn height(&self) -> u32 {
        match &self.storage {
            #[cfg(target_os = "macos")]
            FrameStorage::Native(frame) => frame.surface.get_height() as u32,
            FrameStorage::Rgba(frame) => frame.height,
        }
    }

    /// Tightly packed, top-to-bottom, sRGB-encoded RGBA8 pixels for portable
    /// presentation. Native Metal frames instead expose `native_surface`.
    pub fn rgba(&self) -> Option<&Arc<[u8]>> {
        match &self.storage {
            #[cfg(target_os = "macos")]
            FrameStorage::Native(_) => None,
            FrameStorage::Rgba(frame) => Some(&frame.rgba),
        }
    }

    #[cfg(target_os = "macos")]
    pub fn native_surface(&self) -> Option<&core_video::pixel_buffer::CVPixelBuffer> {
        match &self.storage {
            FrameStorage::Native(frame) => Some(&frame.surface),
            FrameStorage::Rgba(_) => None,
        }
    }

    /// Identity of the completed display allocation, without comparing pixels.
    pub fn same_surface(&self, other: &Self) -> bool {
        match (&self.storage, &other.storage) {
            #[cfg(target_os = "macos")]
            (FrameStorage::Native(a), FrameStorage::Native(b)) => a.surface == b.surface,
            (FrameStorage::Rgba(a), FrameStorage::Rgba(b)) => Arc::ptr_eq(&a.rgba, &b.rgba),
            #[cfg(target_os = "macos")]
            _ => false,
        }
    }
}

enum Implementation {
    #[cfg(target_os = "macos")]
    Native(Box<crate::native::Renderer>),
    Portable(Box<crate::portable::Renderer>),
}

/// Synchronous GPU work belongs on a render worker, never the UI event loop.
/// Scene revisions, film reset/resume, modes, and exports have the same contract
/// for every backend.
pub struct Renderer {
    implementation: Implementation,
    last_frame: Option<Frame>,
    last_mode: RenderMode,
    exposure: f32,
}

impl Renderer {
    pub fn new() -> Result<Self> {
        Self::with_backend(Backend::from_env()?)
    }

    pub fn with_backend(backend: Backend) -> Result<Self> {
        let selected = match backend {
            Backend::Auto if cfg!(target_os = "macos") => Backend::NativeMetal,
            Backend::Auto | Backend::Wgpu => Backend::platform_wgpu()?,
            explicit => explicit,
        };
        let implementation = match selected {
            #[cfg(target_os = "macos")]
            Backend::NativeMetal => {
                Implementation::Native(Box::new(crate::native::Renderer::new()?))
            }
            Backend::Metal if cfg!(target_os = "macos") => Implementation::Portable(Box::new(
                crate::portable::Renderer::new(wgpu::Backends::METAL)?,
            )),
            Backend::Dx12 if cfg!(target_os = "windows") => Implementation::Portable(Box::new(
                crate::portable::Renderer::new(wgpu::Backends::DX12)?,
            )),
            Backend::Vulkan if cfg!(any(target_os = "linux", target_os = "windows")) => {
                Implementation::Portable(Box::new(crate::portable::Renderer::new(
                    wgpu::Backends::VULKAN,
                )?))
            }
            _ => bail!(
                "Renderer {selected} is unavailable on {}",
                std::env::consts::OS
            ),
        };
        Ok(Self {
            implementation,
            last_frame: None,
            last_mode: RenderMode::Solid,
            exposure: 0.0,
        })
    }

    pub fn backend(&self) -> Backend {
        match &self.implementation {
            #[cfg(target_os = "macos")]
            Implementation::Native(_) => Backend::NativeMetal,
            Implementation::Portable(renderer) => renderer.backend(),
        }
    }

    pub fn device_name(&self) -> String {
        match &self.implementation {
            #[cfg(target_os = "macos")]
            Implementation::Native(renderer) => renderer.device_name(),
            Implementation::Portable(renderer) => renderer.device_name(),
        }
    }

    pub fn render(
        &mut self,
        scene: &Scene,
        settings: &RenderSettings,
        revision: u64,
    ) -> Result<Frame> {
        let storage = match &mut self.implementation {
            #[cfg(target_os = "macos")]
            Implementation::Native(renderer) => {
                FrameStorage::Native(renderer.render(scene, settings, revision)?)
            }
            Implementation::Portable(renderer) => {
                FrameStorage::Rgba(renderer.render(scene, settings, revision)?)
            }
        };
        let (samples, elapsed_ms) = match &storage {
            #[cfg(target_os = "macos")]
            FrameStorage::Native(frame) => (frame.samples, frame.elapsed_ms),
            FrameStorage::Rgba(frame) => (frame.samples, frame.elapsed_ms),
        };
        let frame = Frame {
            storage,
            samples,
            elapsed_ms,
            denoised: false,
            denoise_device: None,
        };
        self.last_frame = Some(frame.clone());
        self.last_mode = settings.mode;
        self.exposure = settings.exposure;
        Ok(frame)
    }

    pub fn export_png(&mut self, path: &Path) -> Result<()> {
        match &mut self.implementation {
            #[cfg(target_os = "macos")]
            Implementation::Native(renderer) => renderer.export_png(path),
            Implementation::Portable(renderer) => renderer.export_png(path),
        }
    }

    /// Snapshot all three linear passes without changing accumulation or display.
    pub fn read_denoise_input(&self) -> Result<DenoiseInput> {
        anyhow::ensure!(
            self.last_mode == RenderMode::Rendered,
            "Denoising requires a path-traced image"
        );
        let frame = self
            .last_frame
            .as_ref()
            .context("Render before denoising")?;
        let color = self.read_linear_pixels()?;
        let (mut albedo, mut normal) = match &self.implementation {
            #[cfg(target_os = "macos")]
            Implementation::Native(renderer) => renderer.read_denoise_guides()?,
            Implementation::Portable(renderer) => renderer.read_denoise_guides()?,
        };
        // Floating-point sums can drift a few ulps beyond the guide ranges.
        for pixel in &mut albedo {
            for value in &mut pixel[..3] {
                *value = value.clamp(0.0, 1.0);
            }
        }
        for pixel in &mut normal {
            for value in &mut pixel[..3] {
                *value = value.clamp(-1.0, 1.0);
            }
        }
        Ok(DenoiseInput {
            width: frame.width(),
            height: frame.height(),
            samples: frame.samples,
            color,
            albedo,
            normal,
        })
    }

    /// Final-quality HDR denoising with accurate guide prefiltering. Failure is
    /// explicit: a requested denoised export is never silently saved as raw.
    pub fn export_denoised_png(&self, path: &Path, denoiser: &mut Denoiser) -> Result<()> {
        let input = self.read_denoise_input()?;
        let frame = denoiser.denoise_frame(&input, DenoiseQuality::High, true, self.exposure)?;
        image::save_buffer_with_format(
            path,
            frame.rgba().unwrap(),
            frame.width(),
            frame.height(),
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .with_context(|| format!("Could not export {}", path.display()))
    }

    /// Explicit numerical readback of the untonemapped, normalized linear film.
    pub fn read_linear_pixels(&self) -> Result<Vec<[f32; 4]>> {
        match &self.implementation {
            #[cfg(target_os = "macos")]
            Implementation::Native(renderer) => renderer.read_linear_pixels(),
            Implementation::Portable(renderer) => renderer.read_linear_pixels(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_names_round_trip_and_reject_typos() {
        for backend in [
            Backend::Auto,
            Backend::Wgpu,
            Backend::NativeMetal,
            Backend::Metal,
            Backend::Dx12,
            Backend::Vulkan,
        ] {
            assert_eq!(backend.to_string().parse::<Backend>().unwrap(), backend);
        }
        assert_eq!("DirectX12".parse::<Backend>().unwrap(), Backend::Dx12);
        assert!("metla".parse::<Backend>().is_err());
        assert!("".parse::<Backend>().is_err());
    }

    #[test]
    fn unsupported_api_fails_without_silent_fallback() {
        let backend = if cfg!(target_os = "macos") {
            Backend::Dx12
        } else {
            Backend::Metal
        };
        assert!(
            Renderer::with_backend(backend)
                .err()
                .unwrap()
                .to_string()
                .contains("unavailable")
        );
    }
}

//! GPU viewport and progressive path tracer for macOS, Windows, and Linux.
//!
//! Native Metal retains the zero-copy macOS display path. The portable wgpu
//! implementation uses Metal, DirectX 12, or Vulkan behind the same render API.

mod bvh;
mod environment;
mod material;
pub use material::load_texture;
#[cfg(target_os = "macos")]
mod native;
mod portable;
mod portable_preview;
#[cfg(target_os = "macos")]
mod preview;
mod render_data;

mod backend;
pub use backend::{Backend, Frame, Renderer, validate_custom_shaders};
pub use environment::validate_hdri;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum RenderMode {
    Wireframe = 0,
    #[default]
    Solid = 1,
    MaterialPreview = 2,
    Rendered = 3,
}

impl RenderMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wireframe => "Wireframe",
            Self::Solid => "Solid",
            Self::MaterialPreview => "Material Preview",
            Self::Rendered => "Rendered",
        }
    }

    pub fn progressive(self) -> bool {
        matches!(self, Self::Rendered)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum StudioLight {
    #[default]
    Studio = 0,
    Courtyard = 1,
    Sunset = 2,
}

impl StudioLight {
    pub fn label(self) -> &'static str {
        match self {
            Self::Studio => "Studio",
            Self::Courtyard => "Courtyard",
            Self::Sunset => "Sunset",
        }
    }
}

/// Local viewport lighting, independent of a project's scene world and render
/// settings. Angles are radians; all environment radiance remains scene-linear.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewSettings {
    pub studio: StudioLight,
    pub rotation: f32,
    pub strength: f32,
    pub world_opacity: f32,
    pub background_blur: f32,
    pub ambient_occlusion: bool,
    pub use_scene_world: bool,
    pub hdri_path: Option<std::path::PathBuf>,
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            studio: StudioLight::Studio,
            rotation: 0.0,
            strength: 1.0,
            world_opacity: 0.0,
            background_blur: 0.35,
            ambient_occlusion: true,
            use_scene_world: false,
            hdri_path: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderSettings {
    pub mode: RenderMode,
    pub width: u32,
    pub height: u32,
    pub max_samples: u32,
    pub max_bounces: u32,
    pub exposure: f32,
    pub show_grid: bool,
    pub selected: Option<u64>,
    pub preview: PreviewSettings,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            mode: RenderMode::Solid,
            width: 960,
            height: 640,
            max_samples: 256,
            max_bounces: 8,
            exposure: 0.0,
            show_grid: true,
            selected: None,
            preview: PreviewSettings::default(),
        }
    }
}

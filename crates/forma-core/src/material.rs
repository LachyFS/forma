//! Portable surface definitions. Image pixels are embedded, so a saved project
//! never depends on the source file still existing on the author's machine.
use anyhow::{Result, ensure};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const MAX_SHADER_BYTES: usize = 64 * 1024;
pub const MAX_TEXTURE_PIXELS: usize = 4 * 1024 * 1024;
pub const MAX_SCENE_TEXTURE_BYTES: usize = 64 * 1024 * 1024;
pub const DEFAULT_SHADER_CODE: &str = "// Metal surface function body. Colors are scene-linear.\n// input: uv, generated, position, normal, view_direction\n// Textures have already been applied to surface.\nfloat bands = 0.5f + 0.5f * sin(input.uv.x * 30.0f);\nsurface.color = mix(float3(0.04f, 0.12f, 0.3f),\n                    float3(0.6f, 0.3f, 0.08f), bands);\nsurface.roughness = 0.28f;\n";

/// The language is persisted so changing renderers cannot silently reinterpret code.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ShaderLanguage {
    #[default]
    Metal,
    Wgsl,
}
impl ShaderLanguage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Metal => "Metal",
            Self::Wgsl => "WGSL",
        }
    }
    pub fn default_code(self) -> &'static str {
        match self {
            Self::Metal => DEFAULT_SHADER_CODE,
            Self::Wgsl => DEFAULT_WGSL_CODE,
        }
    }
}
pub const DEFAULT_WGSL_CODE: &str = "// WGSL surface function body. Colors are scene-linear.\n// input: uv, generated, position, normal, view_direction\n// Textures have already been applied to surface.\nlet bands = 0.5 + 0.5 * sin(input.uv.x * 30.0);\nsurface.color = mix(vec3(0.04, 0.12, 0.3),\n                    vec3(0.6, 0.3, 0.08), bands);\nsurface.roughness = 0.28;\n";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u32)]
pub enum ShaderKind {
    #[default]
    Pbr = 0,
    Glass = 1,
    Custom = 2,
}
impl ShaderKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pbr => "PBR",
            Self::Glass => "Glass",
            Self::Custom => "Custom",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u32)]
pub enum TextureMapping {
    #[default]
    Box = 0,
    Sphere = 1,
    Plane = 2,
}
impl TextureMapping {
    pub fn label(self) -> &'static str {
        match self {
            Self::Box => "Box",
            Self::Sphere => "Sphere",
            Self::Plane => "Plane XZ",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum TextureSlot {
    BaseColor,
    Roughness,
    Metallic,
    Normal,
    Emission,
}
impl TextureSlot {
    pub const ALL: [Self; 5] = [
        Self::BaseColor,
        Self::Roughness,
        Self::Metallic,
        Self::Normal,
        Self::Emission,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::BaseColor => "Base color",
            Self::Roughness => "Roughness",
            Self::Metallic => "Metallic",
            Self::Normal => "Normal",
            Self::Emission => "Emission",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextureImage {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Top-to-bottom RGBA8; color space is selected by the texture slot.
    #[serde(with = "pixel_bytes")]
    pub rgba: Vec<u8>,
}
impl TextureImage {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.is_empty()
                && self.name.len() <= 256
                && !self.name.chars().any(char::is_control),
            "Invalid texture name"
        );
        ensure!(
            self.width > 0 && self.height > 0 && self.width <= 8192 && self.height <= 8192,
            "Texture dimensions must be 1–8192"
        );
        let pixels = u64::from(self.width) * u64::from(self.height);
        ensure!(
            pixels <= MAX_TEXTURE_PIXELS as u64,
            "Texture exceeds four megapixels"
        );
        ensure!(
            pixels * 4 == self.rgba.len() as u64,
            "Texture pixel data does not match its dimensions"
        );
        Ok(())
    }
}
mod pixel_bytes {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        if text.len() > (super::MAX_TEXTURE_PIXELS * 4).div_ceil(3) * 4 {
            return Err(D::Error::custom("Texture exceeds four megapixels"));
        }
        STANDARD.decode(text).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Material {
    pub base_color: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub emission: Vec3,
    pub shader: ShaderKind,
    pub ior: f32,
    pub normal_strength: f32,
    pub mapping: TextureMapping,
    pub texture_scale: Vec2,
    pub texture_offset: Vec2,
    pub textures: [Option<Arc<TextureImage>>; 5],
    /// Function body retained when switching away from Custom.
    pub custom_code: String,
    pub custom_language: ShaderLanguage,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            base_color: Vec3::new(0.48, 0.53, 0.59),
            metallic: 0.0,
            roughness: 0.36,
            emission: Vec3::ZERO,
            shader: ShaderKind::Pbr,
            ior: 1.5,
            normal_strength: 1.0,
            mapping: TextureMapping::Box,
            texture_scale: Vec2::ONE,
            texture_offset: Vec2::ZERO,
            textures: Default::default(),
            custom_code: DEFAULT_SHADER_CODE.into(),
            custom_language: ShaderLanguage::Metal,
        }
    }
}
impl Material {
    pub fn glass() -> Self {
        Self {
            shader: ShaderKind::Glass,
            base_color: Vec3::ONE,
            roughness: 0.06,
            ..Self::default()
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.base_color.is_finite()
                && self.base_color.min_element() >= 0.0
                && self.base_color.max_element() <= 1.0
                && self.emission.is_finite()
                && self.emission.min_element() >= 0.0
                && self.emission.max_element() <= 1.0e6
                && self.metallic.is_finite()
                && (0.0..=1.0).contains(&self.metallic)
                && self.roughness.is_finite()
                && (0.0..=1.0).contains(&self.roughness)
                && self.ior.is_finite()
                && (1.01..=3.0).contains(&self.ior)
                && self.normal_strength.is_finite()
                && (0.0..=4.0).contains(&self.normal_strength),
            "Invalid material values"
        );
        ensure!(
            self.texture_scale.is_finite()
                && self.texture_scale.abs().min_element() >= 0.001
                && self.texture_scale.abs().max_element() <= 1000.0
                && self.texture_offset.is_finite()
                && self.texture_offset.abs().max_element() <= 1000.0,
            "Invalid texture mapping"
        );
        ensure!(
            self.custom_code.len() <= MAX_SHADER_BYTES && !self.custom_code.contains('\0'),
            "Shader source must be at most 64 KiB without NUL bytes"
        );
        if self.shader == ShaderKind::Custom {
            ensure!(
                !self.custom_code.trim().is_empty(),
                "Custom shader source is empty"
            );
        }
        for image in self.textures.iter().flatten() {
            image.validate()?;
        }
        Ok(())
    }
}

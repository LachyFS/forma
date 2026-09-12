//! Texture decoding, mip preparation and custom source assembly are CPU-only and
//! testable without a GPU. Both renderers compile custom code transactionally.
use anyhow::{Context, Result, ensure};
use bytemuck::{Pod, Zeroable};
use forma_core::{
    MAX_TEXTURE_PIXELS, Scene, ShaderKind, ShaderLanguage, TextureImage, TextureSlot,
};
use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
use std::{
    collections::HashMap,
    io::{Cursor, Read},
    path::Path,
};

pub fn load_texture(path: &Path) -> Result<TextureImage> {
    const MAX_BYTES: u64 = 32 * 1024 * 1024;
    let file = std::fs::File::open(path).context("Cannot open texture image")?;
    ensure!(
        file.metadata()?.len() <= MAX_BYTES,
        "Image file exceeds 32 MiB"
    );
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_BYTES, "Image file exceeds 32 MiB");
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    ensure!(
        matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)),
        "Choose a PNG or JPEG texture"
    );
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().context("Cannot decode texture")?;
    let (width, height) = decoder.dimensions();
    ensure!(
        u64::from(width) * u64::from(height) <= MAX_TEXTURE_PIXELS as u64,
        "Texture exceeds four megapixels"
    );
    let orientation = decoder.orientation()?;
    let mut decoded = image::DynamicImage::from_decoder(decoder)?;
    decoded.apply_orientation(orientation);
    let rgba = decoded.into_rgba8();
    let texture = TextureImage {
        name: path
            .file_name()
            .context("Image has no filename")?
            .to_string_lossy()
            .into_owned(),
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    };
    texture.validate()?;
    Ok(texture)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GpuMaterial {
    pub color: [f32; 4],
    pub emission: [f32; 4],
    pub params: [f32; 4],   // metallic, roughness, IOR, normal strength
    pub mapping: [f32; 4],  // scale.xy, offset.xy
    pub info: [u32; 4],     // shader kind, projection, custom function index, reserved
    pub textures: [u32; 8], // descriptor indices, NO_TEXTURE = u32::MAX
}

pub(crate) struct MaterialResources {
    pub materials: Vec<GpuMaterial>,
    pub pixels: Vec<[f32; 4]>,
    pub levels: Vec<[u32; 4]>, // offset, width, height, number of levels remaining
}
impl MaterialResources {
    pub fn from_scene(scene: &Scene) -> Self {
        let codes = custom_sources(scene);
        let mut result = Self {
            materials: Vec::new(),
            pixels: Vec::new(),
            levels: Vec::new(),
        };
        // Deduplicate by image content and color interpretation, including linked
        // duplicates and the same image used in several scalar slots.
        let mut images = HashMap::new();
        for data in &scene.materials {
            let m = &data.material;
            let mut textures = [u32::MAX; 8];
            for slot in TextureSlot::ALL {
                if let Some(image) = &m.textures[slot as usize] {
                    let srgb = matches!(slot, TextureSlot::BaseColor | TextureSlot::Emission);
                    let index = *images
                        .entry((image.width, image.height, image.rgba.as_slice(), srgb))
                        .or_insert_with(|| result.add_image(image, srgb));
                    textures[slot as usize] = index;
                }
            }
            result.materials.push(GpuMaterial {
                color: m.base_color.extend(0.0).to_array(),
                emission: m.emission.extend(0.0).to_array(),
                params: [m.metallic, m.roughness, m.ior, m.normal_strength],
                mapping: [
                    m.texture_scale.x,
                    m.texture_scale.y,
                    m.texture_offset.x,
                    m.texture_offset.y,
                ],
                info: [
                    m.shader as u32,
                    m.mapping as u32,
                    codes
                        .iter()
                        .position(|code| *code == m.custom_code)
                        .unwrap_or(0) as u32,
                    0,
                ],
                textures,
            });
        }
        result
    }
    fn add_image(&mut self, image: &TextureImage, srgb: bool) -> u32 {
        let first = self.levels.len() as u32;
        let (mut width, mut height) = (image.width, image.height);
        let mut pixels: Vec<[f32; 4]> = image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|i| {
                    let v = p[i] as f32 / 255.0;
                    if srgb && i < 3 { srgb_to_linear(v) } else { v }
                })
            })
            .collect();
        loop {
            self.levels
                .push([self.pixels.len() as u32, width, height, 0]);
            self.pixels.extend_from_slice(&pixels);
            if width == 1 && height == 1 {
                break;
            }
            let (w, h) = ((width / 2).max(1), (height / 2).max(1));
            let mut next = Vec::with_capacity((w * h) as usize);
            for y in 0..h {
                for x in 0..w {
                    let mut sum = [0.0; 4];
                    let x0 = x as f32 * width as f32 / w as f32;
                    let x1 = (x + 1) as f32 * width as f32 / w as f32;
                    let y0 = y as f32 * height as f32 / h as f32;
                    let y1 = (y + 1) as f32 * height as f32 / h as f32;
                    // Area filtering preserves the mean even for odd dimensions.
                    for sy in y0.floor() as u32..(y1.ceil() as u32).min(height) {
                        for sx in x0.floor() as u32..(x1.ceil() as u32).min(width) {
                            let weight = (x1.min((sx + 1) as f32) - x0.max(sx as f32))
                                * (y1.min((sy + 1) as f32) - y0.max(sy as f32));
                            for (i, value) in sum.iter_mut().enumerate() {
                                *value += pixels[(sy * width + sx) as usize][i] * weight;
                            }
                        }
                    }
                    next.push(sum.map(|v| v / ((x1 - x0) * (y1 - y0))));
                }
            }
            pixels = next;
            width = w;
            height = h;
        }
        let end = self.levels.len();
        for (i, level) in self.levels.iter_mut().enumerate().skip(first as usize) {
            level[3] = (end - i) as u32;
        }
        first
    }
}
fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

pub(crate) fn custom_sources(scene: &Scene) -> Vec<&str> {
    let mut codes: Vec<_> = scene
        .materials
        .iter()
        .filter(|m| m.material.shader == ShaderKind::Custom)
        .map(|m| m.material.custom_code.as_str())
        .collect();
    codes.sort_unstable();
    codes.dedup();
    codes
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn shader_source(codes: &[&str], accelerated: bool) -> String {
    let mut functions = String::new();
    for (index, code) in codes.iter().enumerate() {
        functions.push_str(&format!("void forma_custom_{index}(thread Surface &surface, ShaderInput input) {{\n#line 1 \"custom_{index}.metal\"\n{code}\n}}\n#line 1 \"forma_material.metal\"\n"));
    }
    functions.push_str("void forma_custom(uint index, thread Surface &surface, ShaderInput input) {\nswitch (index) {\n");
    for index in 0..codes.len() {
        functions.push_str(&format!(
            "case {index}u: forma_custom_{index}(surface, input); break;\n"
        ));
    }
    functions.push_str("default: break;\n}\n}\n");
    format!(
        "#define FORMA_PREVIEW_ACCELERATED {}\n{}\n{}\n{}\n{}",
        u32::from(accelerated),
        include_str!("shader.metal").replace(
            "// FORMA_MATERIAL_SYSTEM",
            &include_str!("material.metal").replace("// FORMA_CUSTOM_FUNCTIONS", &functions)
        ),
        include_str!("ibl.metal"),
        include_str!("preview.metal"),
        include_str!("conversion.metal")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::{Material, Primitive};
    use std::sync::Arc;

    #[test]
    fn texture_mips_decode_color_before_filtering_and_keep_data_linear() {
        let mut scene = Scene::empty();
        let id = scene.add(Primitive::Cube);
        let image = Arc::new(TextureImage {
            name: "map.png".into(),
            width: 2,
            height: 1,
            rgba: vec![128, 128, 128, 255, 255, 255, 255, 255],
        });
        let m = scene.object_material_mut(id).unwrap();
        m.textures[0] = Some(image.clone());
        m.textures[1] = Some(image.clone());
        m.textures[2] = Some(image.clone());
        m.textures[4] = Some(image);
        let resources = MaterialResources::from_scene(&scene);
        let m = resources.materials[0];
        assert_eq!(m.textures[0], m.textures[4]);
        assert_eq!(m.textures[1], m.textures[2]);
        assert_ne!(m.textures[0], m.textures[1]);
        assert!((resources.pixels[0][0] - 0.21586).abs() < 0.0001);
        let last = resources.levels[m.textures[0] as usize + 1];
        assert_eq!(&last[1..], &[1, 1, 1]);
        assert!((resources.pixels[last[0] as usize][0] - 0.60793).abs() < 0.0001);
        let data = resources.levels[m.textures[1] as usize];
        assert!((resources.pixels[data[0] as usize][0] - 128.0 / 255.0).abs() < 0.0001);
        assert_eq!(std::mem::size_of::<GpuMaterial>(), 7 * 16);
    }
    #[test]
    fn non_power_of_two_mips_include_last_row_and_column() {
        let mut resources = MaterialResources {
            materials: Vec::new(),
            pixels: Vec::new(),
            levels: Vec::new(),
        };
        resources.add_image(
            &TextureImage {
                name: "odd.png".into(),
                width: 3,
                height: 1,
                rgba: vec![0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255],
            },
            false,
        );
        assert_eq!(
            resources.levels.iter().map(|d| d[1]).collect::<Vec<_>>(),
            [3, 1]
        );
        assert!((resources.pixels[3][0] - 1.0 / 3.0).abs() < 1e-6);
    }
    #[test]
    fn sources_are_deduplicated_and_material_dispatch_is_stable() {
        let mut scene = Scene::empty();
        for code in [
            "surface.roughness = 0.5f;",
            "surface.color = float3(1);",
            "surface.roughness = 0.5f;",
        ] {
            scene
                .create_material_data(
                    "Custom",
                    Material {
                        shader: ShaderKind::Custom,
                        custom_code: code.into(),
                        ..Material::default()
                    },
                )
                .unwrap();
        }
        let codes = custom_sources(&scene);
        assert_eq!(codes.len(), 2);
        let resources = MaterialResources::from_scene(&scene);
        assert_eq!(
            resources.materials[0].info[2],
            resources.materials[2].info[2]
        );
        let source = shader_source(&codes, true);
        assert!(source.contains("#define FORMA_PREVIEW_ACCELERATED 1"));
        assert!(source.contains("#line 1 \"custom_0.metal\""));
        assert!(!source.contains("// FORMA_CUSTOM_FUNCTIONS"));
        assert!(!source.contains("// FORMA_MATERIAL_SYSTEM"));
        assert!(source.contains("kernel void preview_main"));
        assert!(source.contains("kernel void render_main"));
    }
    #[test]
    fn imported_images_are_embedded_and_bad_files_do_not_decode() {
        let directory =
            std::env::temp_dir().join(format!("forma-texture-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for extension in ["png", "jpg"] {
            let path = directory.join(format!("test.{extension}"));
            image::RgbImage::from_pixel(2, 2, image::Rgb([128, 64, 32]))
                .save(&path)
                .unwrap();
            let loaded = load_texture(&path).unwrap();
            assert_eq!((loaded.width, loaded.height, loaded.rgba.len()), (2, 2, 16));
            std::fs::remove_file(path).unwrap();
            loaded.validate().unwrap();
        }
        let path = directory.join("bad.png");
        std::fs::write(&path, b"not an image").unwrap();
        assert!(load_texture(&path).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}

/// The same source ordering must be used by CPU dispatch indices and pipelines.
pub(crate) fn validate_language(scene: &Scene, language: ShaderLanguage) -> Result<()> {
    if let Some(data) = scene.materials.iter().find(|data| {
        data.material.shader == ShaderKind::Custom && data.material.custom_language != language
    }) {
        anyhow::bail!(
            "Material '{}' contains {} code; this renderer requires {}. Select a compatible renderer or edit the material's custom code.",
            data.name,
            data.material.custom_language.label(),
            language.label()
        );
    }
    Ok(())
}

pub(crate) fn wgsl_source(codes: &[&str]) -> String {
    let mut functions = String::new();
    for (index, code) in codes.iter().enumerate() {
        functions.push_str(&format!("// custom_{index}.wgsl\nfn forma_custom_{index}(initial: Surface, input: ShaderInput) -> Surface {{\nvar surface = initial;\n{code}\nreturn surface;\n}}\n"));
    }
    functions.push_str("fn forma_custom(index: u32, surface: Surface, input: ShaderInput) -> Surface {\nswitch index {\n");
    for index in 0..codes.len() {
        functions.push_str(&format!(
            "case {index}u: {{ return forma_custom_{index}(surface, input); }}\n"
        ));
    }
    functions.push_str("default: { return surface; }\n}\n}\n");
    format!("{}\n{}", include_str!("shader.wgsl").replace(
        "fn forma_custom(index: u32, surface: Surface, input: ShaderInput) -> Surface { return surface; }",
        &functions), include_str!("preview.wgsl"))
}

pub(crate) fn custom_cache_key(scene: &Scene) -> Vec<(ShaderLanguage, String)> {
    let mut result: Vec<_> = scene
        .materials
        .iter()
        .filter(|m| m.material.shader == ShaderKind::Custom)
        .map(|m| (m.material.custom_language, m.material.custom_code.clone()))
        .collect();
    result.sort_unstable();
    result.dedup();
    result
}

#![cfg(target_os = "macos")]

use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use forma_core::{Primitive, Scene};
use forma_render::{RenderMode, RenderSettings, Renderer, StudioLight, validate_hdri};
use glam::Vec3;
use image::RgbImage;

const WIDTH: u32 = 96;
const HEIGHT: u32 = 72;

struct TemporaryFile(PathBuf);

impl TemporaryFile {
    fn new(extension: &str) -> Self {
        static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "forma-preview-test-{}-{}.{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed),
            extension
        )))
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn pixels(renderer: &mut Renderer) -> RgbImage {
    let file = TemporaryFile::new("png");
    renderer.export_png(&file.0).unwrap();
    image::open(&file.0).unwrap().into_rgb8()
}

fn sphere_scene() -> Scene {
    let mut scene = Scene::default();
    scene.objects.clear();
    scene.camera.target = Vec3::ZERO;
    scene.camera.yaw = 0.0;
    scene.camera.pitch = 0.0;
    scene.camera.distance = 4.5;
    scene.add(Primitive::Sphere);
    scene.objects[0].material.base_color = Vec3::splat(0.55);
    scene
}

fn settings() -> RenderSettings {
    RenderSettings {
        width: WIDTH,
        height: HEIGHT,
        mode: RenderMode::MaterialPreview,
        show_grid: false,
        ..Default::default()
    }
}

fn render(
    renderer: &mut Renderer,
    scene: &Scene,
    settings: &RenderSettings,
    revision: u64,
) -> RgbImage {
    let frame = renderer.render(scene, settings, revision).unwrap();
    assert_eq!(frame.samples, 1, "Preview completes in one frame");
    pixels(renderer)
}

/// An interior patch, well clear of the sphere's antialiased silhouette.
fn object_patch(image: &RgbImage) -> Vec<u8> {
    let mut values = Vec::new();
    for y in HEIGHT / 2 - 8..HEIGHT / 2 + 8 {
        for x in WIDTH / 2 - 8..WIDTH / 2 + 8 {
            values.extend_from_slice(&image.get_pixel(x, y).0);
        }
    }
    values
}

fn channel_sum(image: &RgbImage, channel: usize) -> u64 {
    object_patch(image)
        .chunks_exact(3)
        .map(|pixel| pixel[channel] as u64)
        .sum()
}

#[test]
fn preview_is_deterministic_and_independent_of_path_tracing_quality_and_scene_world() {
    assert!(!RenderMode::MaterialPreview.progressive());
    let mut renderer = Renderer::new().unwrap();
    let mut scene = sphere_scene();
    let mut settings = settings();
    let original = render(&mut renderer, &scene, &settings, 1);
    for revision in [1, 1, 2] {
        assert_eq!(original, render(&mut renderer, &scene, &settings, revision));
    }

    settings.max_samples = 1;
    settings.max_bounces = 1;
    assert_eq!(original, render(&mut renderer, &scene, &settings, 2));
    settings.max_samples = 4096;
    settings.max_bounces = 32;
    scene.world.color = Vec3::ZERO;
    scene.world.strength = 0.0;
    assert_eq!(original, render(&mut renderer, &scene, &settings, 2));
    scene.world.color = Vec3::new(8.0, 0.1, 0.0);
    scene.world.strength = 25.0;
    assert_eq!(original, render(&mut renderer, &scene, &settings, 2));
}

#[test]
fn preview_updates_base_color_metallic_roughness_and_geometry() {
    let mut renderer = Renderer::new().unwrap();
    let mut scene = sphere_scene();
    let settings = settings();
    let neutral = render(&mut renderer, &scene, &settings, 1);

    scene.objects[0].material.base_color = Vec3::new(0.8, 0.02, 0.01);
    let red = render(&mut renderer, &scene, &settings, 2);
    assert_ne!(neutral, red);
    assert!(
        channel_sum(&red, 0) > channel_sum(&red, 2) * 2,
        "A red base color must tint the visible material"
    );

    scene.objects[0].material.base_color = Vec3::splat(0.55);
    scene.objects[0].material.metallic = 1.0;
    let metal = render(&mut renderer, &scene, &settings, 3);
    assert_ne!(neutral, metal, "Metallic changes the material response");
    scene.objects[0].material.roughness = 0.95;
    let rough = render(&mut renderer, &scene, &settings, 4);
    assert_ne!(metal, rough, "Roughness changes reflected lighting");

    scene.objects[0].transform.translation.x = 0.6;
    let moved = render(&mut renderer, &scene, &settings, 5);
    assert_ne!(rough, moved, "Transforms update cached preview geometry");
    for position in &mut scene.objects[0].mesh.positions {
        position.y *= 0.55;
    }
    assert_ne!(
        moved,
        render(&mut renderer, &scene, &settings, 6),
        "Mesh edits update cached preview geometry"
    );
}

#[test]
fn preview_shows_emission_without_scene_light_bounces() {
    let mut renderer = Renderer::new().unwrap();
    let mut scene = sphere_scene();
    let mut settings = settings();
    settings.preview.ambient_occlusion = false;
    let light = scene.add(Primitive::Plane);
    scene.object_mut(light).unwrap().transform.translation = Vec3::new(0.0, 3.0, 0.0);
    let unlit = render(&mut renderer, &scene, &settings, 1);

    scene.object_mut(light).unwrap().material.emission = Vec3::splat(80.0);
    assert_eq!(
        unlit,
        render(&mut renderer, &scene, &settings, 2),
        "An off-camera scene emitter must not replace the preview's studio lighting"
    );

    scene.objects[0].material.emission = Vec3::new(5.0, 0.1, 0.0);
    let glowing = render(&mut renderer, &scene, &settings, 3);
    assert!(
        channel_sum(&glowing, 0) > channel_sum(&unlit, 0),
        "A visible material's own emission must remain visible"
    );
}

#[test]
fn studio_presets_rotation_and_strength_change_material_lighting() {
    let mut renderer = Renderer::new().unwrap();
    let mut scene = sphere_scene();
    scene.objects[0].material.metallic = 1.0;
    scene.objects[0].material.roughness = 0.16;
    let mut settings = settings();
    let studio = render(&mut renderer, &scene, &settings, 1);
    settings.preview.rotation = std::f32::consts::FRAC_PI_2;
    assert_ne!(studio, render(&mut renderer, &scene, &settings, 1));
    settings.preview.rotation = 0.0;
    settings.preview.studio = StudioLight::Courtyard;
    let courtyard = render(&mut renderer, &scene, &settings, 1);
    assert_ne!(studio, courtyard);
    settings.preview.studio = StudioLight::Sunset;
    let sunset = render(&mut renderer, &scene, &settings, 1);
    assert_ne!(studio, sunset);
    assert_ne!(courtyard, sunset);

    settings.preview.strength = 0.0;
    let dark = render(&mut renderer, &scene, &settings, 1);
    assert!(object_patch(&dark).iter().all(|value| *value == 0));
    settings.preview.strength = 2.0;
    let bright = render(&mut renderer, &scene, &settings, 1);
    assert!(channel_sum(&bright, 0) > channel_sum(&dark, 0));
}

#[test]
fn preview_background_controls_do_not_change_material_illumination() {
    let mut renderer = Renderer::new().unwrap();
    let scene = sphere_scene();
    let mut settings = settings();
    let opaque = render(&mut renderer, &scene, &settings, 1);
    settings.preview.world_opacity = 1.0;
    settings.preview.background_blur = 0.0;
    let world = render(&mut renderer, &scene, &settings, 1);
    assert_ne!(opaque.get_pixel(0, 0), world.get_pixel(0, 0));
    assert_eq!(object_patch(&opaque), object_patch(&world));

    settings.preview.background_blur = 1.0;
    let blurred = render(&mut renderer, &scene, &settings, 1);
    assert_ne!(world, blurred, "Blur must change the visible environment");
    assert_eq!(object_patch(&world), object_patch(&blurred));
}

#[test]
fn scene_world_option_uses_project_radiance_and_keeps_visible_emission() {
    let mut renderer = Renderer::new().unwrap();
    let mut scene = sphere_scene();
    let mut settings = settings();
    settings.preview.use_scene_world = true;
    settings.preview.world_opacity = 1.0;
    scene.world.color = Vec3::ZERO;
    scene.world.strength = 0.0;
    let dark = render(&mut renderer, &scene, &settings, 1);
    assert!(dark.as_raw().iter().all(|value| *value == 0));

    scene.objects[0].material.emission = Vec3::new(2.0, 0.0, 0.0);
    let glowing = render(&mut renderer, &scene, &settings, 2);
    assert!(channel_sum(&glowing, 0) > 0);
    assert_eq!(glowing.get_pixel(0, 0).0, [0, 0, 0]);

    scene.objects[0].material.emission = Vec3::ZERO;
    scene.world.color = Vec3::ONE;
    scene.world.strength = 2.0;
    let bright = render(&mut renderer, &scene, &settings, 3);
    assert!(channel_sum(&bright, 0) > 0);
    settings.preview.studio = StudioLight::Sunset;
    settings.preview.strength = 0.0;
    settings.preview.rotation = 1.5;
    assert_eq!(bright, render(&mut renderer, &scene, &settings, 3));
}

#[test]
fn preview_controls_do_not_reset_or_change_rendered_accumulation() {
    let scene = sphere_scene();
    let mut settings = settings();
    settings.mode = RenderMode::Rendered;
    settings.max_samples = 2;
    let mut renderer = Renderer::new().unwrap();
    assert_eq!(renderer.render(&scene, &settings, 1).unwrap().samples, 1);

    settings.preview.studio = StudioLight::Sunset;
    settings.preview.rotation = 1.4;
    settings.preview.strength = 3.0;
    settings.preview.world_opacity = 1.0;
    settings.preview.ambient_occlusion = false;
    assert_eq!(renderer.render(&scene, &settings, 1).unwrap().samples, 2);
    let completed = pixels(&mut renderer);
    settings.preview.background_blur = 0.0;
    settings.preview.use_scene_world = true;
    settings.preview.hdri_path = Some(TemporaryFile::new("missing.hdr").0.clone());
    assert_eq!(renderer.render(&scene, &settings, 1).unwrap().samples, 2);
    assert_eq!(completed, pixels(&mut renderer));

    settings.preview = Default::default();
    let mut reference = Renderer::new().unwrap();
    reference.render(&scene, &settings, 1).unwrap();
    reference.render(&scene, &settings, 1).unwrap();
    assert_eq!(completed, pixels(&mut reference));
}

fn hdr_fixture(color: [f32; 3]) -> TemporaryFile {
    let file = TemporaryFile::new("hdr");
    let output = std::fs::File::create(&file.0).unwrap();
    image::codecs::hdr::HdrEncoder::new(output)
        .encode(&[image::Rgb(color); 16 * 8], 16, 8)
        .unwrap();
    file
}

#[test]
fn custom_hdr_lights_materials_and_switching_files_refreshes_the_environment() {
    let red = hdr_fixture([4.0, 0.01, 0.01]);
    let blue = hdr_fixture([0.01, 0.01, 4.0]);
    let mut renderer = Renderer::new().unwrap();
    let scene = sphere_scene();
    let mut settings = settings();
    settings.preview.world_opacity = 1.0;
    settings.preview.hdri_path = Some(red.0.clone());
    let red_image = render(&mut renderer, &scene, &settings, 1);
    assert!(channel_sum(&red_image, 0) > channel_sum(&red_image, 2) * 2);
    assert!(red_image.get_pixel(0, 0)[0] > red_image.get_pixel(0, 0)[2]);

    settings.preview.hdri_path = Some(blue.0.clone());
    let blue_image = render(&mut renderer, &scene, &settings, 1);
    assert!(channel_sum(&blue_image, 2) > channel_sum(&blue_image, 0) * 2);
    assert!(blue_image.get_pixel(0, 0)[2] > blue_image.get_pixel(0, 0)[0]);
    assert_ne!(red_image, blue_image);
}

#[test]
fn hdr_validation_rejects_malformed_truncated_and_excessive_inputs() {
    let valid = hdr_fixture([2.0, 0.5, 0.2]);
    validate_hdri(&valid.0).unwrap();

    let invalid = TemporaryFile::new("hdr");
    std::fs::write(&invalid.0, b"This is not a Radiance image").unwrap();
    assert!(validate_hdri(&invalid.0).is_err());

    let truncated = TemporaryFile::new("hdr");
    let mut bytes = std::fs::read(&valid.0).unwrap();
    bytes.truncate(bytes.len() - 8);
    std::fs::write(&truncated.0, bytes).unwrap();
    assert!(validate_hdri(&truncated.0).is_err());

    let oversized = TemporaryFile::new("hdr");
    std::fs::write(
        &oversized.0,
        b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 20000\n",
    )
    .unwrap();
    let error = validate_hdri(&oversized.0).unwrap_err();
    assert!(
        format!("{error:#}").to_lowercase().contains("dimension"),
        "Oversized dimensions must be rejected before decoding the absent pixel payload: {error:#}"
    );

    // RGBE cannot encode NaN or infinity, but very large finite radiance can
    // still overflow downstream lighting calculations and must be bounded.
    let excessive = hdr_fixture([1.0e20, 0.0, 0.0]);
    assert!(validate_hdri(&excessive.0).is_err());
}

#[test]
fn failed_hdr_load_preserves_the_completed_frame_and_recovers() {
    let invalid = TemporaryFile::new("hdr");
    std::fs::write(&invalid.0, b"Invalid environment").unwrap();
    let mut renderer = Renderer::new().unwrap();
    let scene = sphere_scene();
    let mut settings = settings();
    let completed = render(&mut renderer, &scene, &settings, 1);

    settings.preview.hdri_path = Some(invalid.0.clone());
    settings.width = 128;
    assert!(renderer.render(&scene, &settings, 1).is_err());
    assert_eq!(
        completed,
        pixels(&mut renderer),
        "A failed environment load must leave the last completed film exportable"
    );

    settings.width = WIDTH;
    settings.preview.hdri_path = None;
    assert_eq!(completed, render(&mut renderer, &scene, &settings, 1));
}

use std::{f32::consts::PI, path::PathBuf};

use forma_core::{Primitive, Scene};
use forma_render::{RenderMode, RenderSettings, Renderer};
use glam::Vec3;

struct Fixture(PathBuf);

impl Fixture {
    fn new(extension: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "forma-preview-direction-review-{}.{}",
            std::process::id(),
            extension
        )))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn horizontal_surfaces_receive_constant_hdr_light_from_both_poles() {
    let hdr = Fixture::new("poles.hdr");
    image::codecs::hdr::HdrEncoder::new(std::fs::File::create(&hdr.0).unwrap())
        .encode(&vec![image::Rgb([1.0; 3]); 64 * 32], 64, 32)
        .unwrap();
    let mut scene = Scene::empty();
    let plane = scene.add(Primitive::Plane);
    scene.object_mut(plane).unwrap().transform.scale = Vec3::splat(100.0);
    scene.camera.target = Vec3::ZERO;
    scene.camera.distance = 4.0;
    scene.world.color = Vec3::ONE;
    scene.world.strength = 1.0;
    let mut settings = RenderSettings {
        width: 32,
        height: 32,
        mode: RenderMode::MaterialPreview,
        show_grid: false,
        ..Default::default()
    };
    settings.preview.hdri_path = Some(hdr.0.clone());
    settings.preview.ambient_occlusion = false;
    let mut renderer = Renderer::new().unwrap();
    for pitch in [-0.8, 0.8] {
        scene.camera.pitch = pitch;
        settings.preview.use_scene_world = false;
        renderer.render(&scene, &settings, 1).unwrap();
        let environment = renderer.read_linear_pixels().unwrap();
        settings.preview.use_scene_world = true;
        renderer.render(&scene, &settings, 1).unwrap();
        let world = renderer.read_linear_pixels().unwrap();
        assert!(world.iter().all(|pixel| pixel[0] > 0.1));
        for (a, b) in environment.iter().flatten().zip(world.iter().flatten()) {
            assert!(
                (a - b).abs() < 1e-3,
                "pitch={pitch}: HDR {a} differs from constant world {b}"
            );
        }
    }
}

/// A perfectly rough metal still reflects only its front hemisphere. Roughness
/// filtering may broaden the light, but must not collapse all environment
/// directions into a single scalar or erase environment rotation.
#[test]
fn fully_rough_metal_retains_directional_environment_lighting() {
    let hdr = Fixture::new("hdr");
    let png = Fixture::new("png");
    let (width, height) = (128, 64);
    let mut radiance = Vec::new();
    for y in 0..height {
        let theta = (y as f32 + 0.5) / height as f32 * PI;
        for x in 0..width {
            let phi = ((x as f32 + 0.5) / width as f32 - 0.5) * 2.0 * PI;
            let z = theta.sin() * phi.sin();
            radiance.push(image::Rgb([0.01 + 0.9 * z.max(0.0); 3]));
        }
    }
    image::codecs::hdr::HdrEncoder::new(std::fs::File::create(&hdr.0).unwrap())
        .encode(&radiance, width as usize, height as usize)
        .unwrap();

    let mut scene = Scene::default();
    scene.objects.clear();
    let plane = scene.add(Primitive::Plane);
    let object = scene.object_mut(plane).unwrap();
    object.transform.scale = Vec3::splat(100.0);
    object.transform.rotation.x = PI * 0.5;
    let material = scene.object_material_mut(plane).unwrap();
    material.base_color = Vec3::splat(0.8);
    material.metallic = 1.0;
    material.roughness = 1.0;
    scene.camera.target = Vec3::ZERO;
    scene.camera.yaw = 0.0;
    scene.camera.pitch = 0.0;
    scene.camera.distance = 3.0;
    scene.camera.orthographic = true;
    let mut settings = RenderSettings {
        width: 32,
        height: 32,
        mode: RenderMode::MaterialPreview,
        show_grid: false,
        ..Default::default()
    };
    settings.preview.hdri_path = Some(hdr.0.clone());
    settings.preview.ambient_occlusion = false;
    let mut renderer = Renderer::new().unwrap();
    let mut mean = |settings: &RenderSettings| {
        assert_eq!(renderer.render(&scene, settings, 1).unwrap().samples, 1);
        renderer.export_png(&png.0).unwrap();
        let pixels = image::open(&png.0).unwrap().into_rgb8();
        pixels
            .pixels()
            .map(|pixel| f32::from(pixel[0]))
            .sum::<f32>()
            / pixels.width() as f32
            / pixels.height() as f32
    };
    let facing_light = mean(&settings);
    settings.preview.rotation = PI;
    let facing_dark = mean(&settings);
    assert!(
        facing_light > facing_dark + 40.0,
        "A rough metal must retain front/back lighting after filtering: light={facing_light}, dark={facing_dark}"
    );
}

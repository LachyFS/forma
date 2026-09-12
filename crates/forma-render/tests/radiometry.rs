//! Numerical transport checks shared by native Metal and every wgpu backend.
use forma_core::{Primitive, Scene};
use forma_render::{RenderMode, RenderSettings, Renderer};
use glam::Vec3;

fn mean_linear(renderer: &Renderer) -> Vec3 {
    let pixels = renderer.read_linear_pixels().unwrap();
    assert!(pixels.iter().flatten().all(|v| v.is_finite()));
    pixels.iter().map(|p| Vec3::from_slice(p)).sum::<Vec3>() / pixels.len() as f32
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
        preview: forma_render::PreviewSettings {
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

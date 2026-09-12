use forma_core::{Primitive, Scene};
use forma_render::{DenoiseQuality, RenderMode, RenderSettings, Renderer};
use glam::Vec3;

#[test]
fn path_guides_match_surface_colors_normals_and_reset_with_the_film() {
    let mut renderer = Renderer::new().unwrap();
    let mut scene = Scene::default();
    scene.objects.clear();
    let id = scene.add(Primitive::Plane);
    scene.object_mut(id).unwrap().transform.scale = Vec3::splat(100.0);
    scene.object_material_mut(id).unwrap().base_color = Vec3::new(0.2, 0.4, 0.8);
    scene.camera.target = Vec3::ZERO;
    scene.camera.distance = 2.0;
    scene.camera.pitch = 1.3;
    let mut settings = RenderSettings {
        mode: RenderMode::Rendered,
        width: 32,
        height: 24,
        max_samples: 8,
        ..Default::default()
    };
    for _ in 0..8 {
        renderer.render(&scene, &settings, 1).unwrap();
    }
    let input = renderer.read_denoise_input().unwrap();
    assert_eq!((input.width, input.height, input.samples), (32, 24, 8));
    assert_eq!(input.color, renderer.read_linear_pixels().unwrap());
    for (albedo, normal) in input.albedo.iter().zip(&input.normal) {
        assert!((Vec3::from_slice(albedo) - Vec3::new(0.2, 0.4, 0.8)).length() < 1e-5);
        assert!((Vec3::from_slice(normal) - Vec3::Y).length() < 1e-5);
    }
    // These are presentation preferences: compare toggles at the cap for an exact film.
    settings.denoise.viewport = false;
    settings.denoise.quality = DenoiseQuality::High;
    settings.denoise.start_sample = 32;
    assert_eq!(renderer.render(&scene, &settings, 1).unwrap().samples, 8);
    assert_eq!(renderer.read_denoise_input().unwrap().color, input.color);
    settings.max_samples = 9;
    assert_eq!(renderer.render(&scene, &settings, 1).unwrap().samples, 9);
    scene.object_material_mut(id).unwrap().base_color = Vec3::new(0.8, 0.1, 0.3);
    assert_eq!(renderer.render(&scene, &settings, 2).unwrap().samples, 1);
    let changed = renderer.read_denoise_input().unwrap();
    assert!((Vec3::from_slice(&changed.albedo[0]) - Vec3::new(0.8, 0.1, 0.3)).length() < 1e-5);
    settings.mode = RenderMode::Solid;
    renderer.render(&scene, &settings, 2).unwrap();
    assert!(renderer.read_denoise_input().is_err());
    settings.mode = RenderMode::Rendered;
    settings.width = 48;
    renderer.render(&scene, &settings, 2).unwrap();
    let resized = renderer.read_denoise_input().unwrap();
    assert_eq!(resized.normal.len(), 48 * 24);
    assert_eq!(resized.samples, 1);
}

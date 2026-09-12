use std::sync::atomic::{AtomicUsize, Ordering};

use forma_core::{Primitive, Scene};
use forma_render::{RenderMode, RenderSettings, Renderer};
use glam::{Vec2, Vec3};

fn surface_bytes(frame: &forma_render::Frame) -> Vec<u8> {
    if let Some(rgba) = frame.rgba() {
        return rgba.to_vec();
    }
    #[cfg(target_os = "macos")]
    {
        use core_video::pixel_buffer::kCVPixelBufferLock_ReadOnly;
        let surface = frame.native_surface().unwrap();
        assert_eq!(surface.lock_base_address(kCVPixelBufferLock_ReadOnly), 0);
        let mut bytes = Vec::new();
        for plane in 0..2 {
            let stride = surface.get_bytes_per_row_of_plane(plane);
            let width = surface.get_width_of_plane(plane) * if plane == 0 { 1 } else { 2 };
            let height = surface.get_height_of_plane(plane);
            // SAFETY: completed Frame, read-only CoreVideo lock, plane-sized slice.
            let source = unsafe {
                std::slice::from_raw_parts(
                    surface.get_base_address_of_plane(plane).cast::<u8>(),
                    stride * height,
                )
            };
            for row in source.chunks_exact(stride) {
                bytes.extend_from_slice(&row[..width]);
            }
        }
        assert_eq!(surface.unlock_base_address(kCVPixelBufferLock_ReadOnly), 0);
        bytes
    }
    #[cfg(not(target_os = "macos"))]
    unreachable!("Portable frames contain RGBA pixels")
}

fn pixels(renderer: &mut Renderer) -> Vec<u8> {
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let file = std::env::temp_dir().join(format!(
        "forma-render-test-{}-{}.png",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    renderer.export_png(&file).unwrap();
    let pixels = image::open(&file).unwrap().into_rgb8().into_raw();
    std::fs::remove_file(file).unwrap();
    pixels
}

#[test]
fn progressive_film_caps_resumes_and_invalidates() {
    let mut renderer = Renderer::new().unwrap();
    let mut scene = Scene::default();
    let mut settings = RenderSettings {
        width: 64,
        height: 48,
        mode: RenderMode::Rendered,
        max_samples: 2,
        ..Default::default()
    };
    assert_eq!(renderer.render(&scene, &settings, 1).unwrap().samples, 1);
    assert_eq!(renderer.render(&scene, &settings, 1).unwrap().samples, 2);
    assert_eq!(renderer.render(&scene, &settings, 1).unwrap().samples, 2);
    settings.max_samples = 3;
    assert_eq!(renderer.render(&scene, &settings, 1).unwrap().samples, 3);
    settings.selected = scene.objects.first().map(|object| object.id);
    settings.show_grid = !settings.show_grid;
    assert_eq!(renderer.render(&scene, &settings, 1).unwrap().samples, 3);
    scene.camera.orbit(Vec2::new(0.1, 0.0));
    assert_eq!(renderer.render(&scene, &settings, 1).unwrap().samples, 1);
    settings.exposure = 1.0;
    assert_eq!(renderer.render(&scene, &settings, 1).unwrap().samples, 1);
    assert_eq!(renderer.render(&scene, &settings, 2).unwrap().samples, 1);
}

#[test]
fn rendered_world_has_no_fabricated_light_or_viewport_grid() {
    let mut renderer = Renderer::new().unwrap();
    let mut scene = Scene::default();
    scene.objects.clear();
    scene.world.color = Vec3::ZERO;
    scene.world.strength = 0.0;
    scene.add(Primitive::Sphere);
    let settings = RenderSettings {
        width: 48,
        height: 48,
        mode: RenderMode::Rendered,
        max_samples: 1,
        show_grid: true,
        ..Default::default()
    };
    renderer.render(&scene, &settings, 1).unwrap();
    assert!(
        pixels(&mut renderer).iter().all(|value| *value == 0),
        "A black world without emitters must stay black, including the grid"
    );
    scene.world.color = Vec3::ONE;
    scene.world.strength = 1.0;
    renderer.render(&scene, &settings, 1).unwrap();
    assert!(
        pixels(&mut renderer).iter().any(|value| *value > 128),
        "World radiance must reach the film"
    );
}

#[test]
fn viewport_modes_are_distinct_and_preview_is_independent_of_scene_world() {
    let mut renderer = Renderer::new().unwrap();
    let mut scene = Scene::default();
    let mut settings = RenderSettings {
        width: 64,
        height: 48,
        max_samples: 1,
        ..Default::default()
    };
    settings.mode = RenderMode::Wireframe;
    renderer.render(&scene, &settings, 1).unwrap();
    let wire = pixels(&mut renderer);
    settings.mode = RenderMode::Solid;
    renderer.render(&scene, &settings, 1).unwrap();
    let solid = pixels(&mut renderer);
    assert_ne!(wire, solid);
    settings.mode = RenderMode::MaterialPreview;
    renderer.render(&scene, &settings, 1).unwrap();
    let preview = pixels(&mut renderer);
    assert_ne!(solid, preview);
    scene.world.strength = 0.0;
    scene.world.color = Vec3::ZERO;
    renderer.render(&scene, &settings, 1).unwrap();
    assert_eq!(preview, pixels(&mut renderer));
}

#[test]
fn published_surface_survives_new_frames_and_worker_handoff() {
    let mut renderer = Renderer::new().unwrap();
    let mut scene = Scene::default();
    scene.objects.clear();
    let settings = RenderSettings {
        width: 33,
        height: 25,
        mode: RenderMode::Rendered,
        max_samples: 1,
        ..Default::default()
    };
    let retained = renderer.render(&scene, &settings, 1).unwrap();
    assert_eq!(retained.width(), 34);
    assert_eq!(retained.height(), 26);
    let original = surface_bytes(&retained);
    for index in 0..8 {
        scene.world.color = Vec3::splat(index as f32 * 0.1);
        renderer.render(&scene, &settings, 1).unwrap();
    }
    let handed_off = std::thread::spawn(move || surface_bytes(&retained))
        .join()
        .unwrap();
    assert_eq!(
        original, handed_off,
        "A published surface must remain immutable while retained by another thread"
    );
}

#[test]
fn invalid_requests_leave_the_last_frame_exportable() {
    let mut renderer = Renderer::new().unwrap();
    let scene = Scene::default();
    let mut settings = RenderSettings {
        width: 32,
        height: 24,
        ..Default::default()
    };
    renderer.render(&scene, &settings, 1).unwrap();
    let valid = pixels(&mut renderer);
    for dimensions in [(0, 24), (32, 0), (8193, 24), (32, u32::MAX)] {
        settings.width = dimensions.0;
        settings.height = dimensions.1;
        assert!(renderer.render(&scene, &settings, 1).is_err());
        assert_eq!(pixels(&mut renderer), valid);
    }
    settings.width = 32;
    settings.height = 24;
    settings.exposure = f32::NAN;
    assert!(renderer.render(&scene, &settings, 1).is_err());
    assert_eq!(pixels(&mut renderer), valid);
}

#[test]
fn portable_frames_survive_resize_and_renderer_destruction() {
    let mut renderer = Renderer::with_backend(forma_render::Backend::Wgpu).unwrap();
    let mut scene = Scene::default();
    scene.objects.clear();
    let mut settings = RenderSettings {
        width: 17,
        height: 9,
        mode: RenderMode::Rendered,
        max_samples: 0,
        ..Default::default()
    };
    let frame = renderer.render(&scene, &settings, 1).unwrap();
    assert_eq!((frame.width(), frame.height()), (18, 10));
    assert_eq!(frame.samples, 1);
    assert_eq!(frame.rgba().unwrap().len(), 18 * 10 * 4);
    assert!(
        frame
            .rgba()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[3] == 255)
    );
    assert!(frame.same_surface(&renderer.render(&scene, &settings, 1).unwrap()));
    let expected = frame.rgba().unwrap().to_vec();
    settings.width = 64;
    settings.height = 48;
    renderer.render(&scene, &settings, 1).unwrap();
    drop(renderer);
    assert_eq!(
        std::thread::spawn(move || frame.rgba().unwrap().to_vec())
            .join()
            .unwrap(),
        expected
    );
}

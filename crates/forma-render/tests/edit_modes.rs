//! Editing overlays must apply to every visible mesh, independent of selection.
use forma_core::{Primitive, Scene, Vec3};
use forma_render::{RenderMode, RenderSettings, Renderer};

fn display(renderer: &mut Renderer) -> Vec<[f32; 4]> {
    let path = std::env::temp_dir().join(format!("forma-edit-overlay-{}.png", std::process::id()));
    renderer.export_png(&path).unwrap();
    let pixels = image::open(&path)
        .unwrap()
        .into_rgba8()
        .pixels()
        .map(|p| p.0.map(|v| v as f32 / 255.))
        .collect();
    std::fs::remove_file(path).unwrap();
    pixels
}

#[test]
fn edit_overlay_covers_unselected_meshes_without_polygon_diagonals() {
    let mut renderer = Renderer::new().unwrap();
    let mut scene = Scene::empty();
    let left = scene.add(Primitive::Cube);
    let right = scene.add(Primitive::Cube);
    scene.object_mut(left).unwrap().transform.translation.x = -1.5;
    scene.object_mut(right).unwrap().transform.translation.x = 1.5;
    scene.camera.target = Vec3::ZERO;
    scene.camera.yaw = 0.;
    scene.camera.pitch = 0.;
    scene.camera.distance = 9.;
    scene.camera.orthographic = true;
    for mode in [
        RenderMode::Solid,
        RenderMode::MaterialPreview,
        RenderMode::Rendered,
    ] {
        let mut settings = RenderSettings {
            width: 192,
            height: 128,
            mode,
            show_grid: false,
            selected: Some(left),
            ..Default::default()
        };
        renderer.render(&scene, &settings, 1).unwrap();
        let plain = display(&mut renderer);
        settings.edit_wireframe = true;
        renderer.render(&scene, &settings, 1).unwrap();
        let wire = display(&mut renderer);
        for range in [0..96, 96..192] {
            let changed = (0..128)
                .flat_map(|y| range.clone().map(move |x| y * 192 + x))
                .filter(|&i| {
                    (Vec3::from_slice(&plain[i]) - Vec3::from_slice(&wire[i])).length() > 0.01
                })
                .count();
            assert!(
                changed > 30,
                "{mode:?}: both selected and unselected meshes need wireframe; changed={changed}"
            );
        }
        // The middle of each quad is on a triangulation diagonal. It must stay shaded.
        for x in [70, 122] {
            let i = 64 * 192 + x;
            assert!(
                (Vec3::from_slice(&plain[i]) - Vec3::from_slice(&wire[i])).length() < 0.001,
                "{mode:?}: polygon interior changed"
            );
        }
        settings.edit_vertices = true;
        renderer.render(&scene, &settings, 1).unwrap();
        let vertices = display(&mut renderer);
        assert!(
            wire.iter()
                .zip(vertices.iter())
                .any(|(a, b)| (Vec3::from_slice(a) - Vec3::from_slice(b)).length() > 0.01),
            "{mode:?}: vertex markers missing"
        );
    }
}

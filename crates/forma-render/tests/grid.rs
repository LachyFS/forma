//! Image-space regressions for the modelling grid, including density transitions.
use forma_core::Scene;
use forma_render::{RenderMode, RenderSettings, Renderer};
use glam::Vec3;

fn render(renderer: &mut Renderer, scene: &Scene, settings: &RenderSettings) -> Vec<[f32; 4]> {
    renderer.render(scene, settings, 1).unwrap();
    let pixels = renderer.read_linear_pixels().unwrap();
    assert!(pixels.iter().flatten().all(|value| value.is_finite()));
    pixels
}

fn difference(a: &[[f32; 4]], b: &[[f32; 4]]) -> (f32, f32) {
    let mut sum = 0.0;
    let mut maximum: f32 = 0.0;
    for (a, b) in a.iter().zip(b) {
        let delta = (Vec3::from_slice(a) - Vec3::from_slice(b))
            .abs()
            .max_element();
        sum += delta;
        maximum = maximum.max(delta);
    }
    (sum / a.len() as f32, maximum)
}

#[test]
fn tiny_orthographic_zoom_does_not_switch_whole_grid_levels() {
    let mut renderer = Renderer::new().unwrap();
    let mut scene = Scene::empty();
    scene.camera.orthographic = true;
    scene.camera.pitch = std::f32::consts::FRAC_PI_2;
    scene.camera.yaw = 0.0;
    let mut worst_mean: f32 = 0.0;
    let mut worst_max: f32 = 0.0;
    for mode in [
        RenderMode::Wireframe,
        RenderMode::Solid,
        RenderMode::MaterialPreview,
    ] {
        let mut settings = RenderSettings {
            width: 128,
            height: 128,
            mode,
            show_grid: true,
            ..Default::default()
        };
        // Reproduces the jumps at three successive decade boundaries. A 0.02%
        // zoom moves lines a tiny fraction of a pixel, not an entire grid level.
        for distance in [0.44147, 4.4147, 44.147] {
            scene.camera.target = Vec3::new(distance * 2.3, 0.0, distance * 1.7);
            scene.camera.distance = distance * 0.9999;
            let before = render(&mut renderer, &scene, &settings);
            scene.camera.distance = distance * 1.0001;
            let after = render(&mut renderer, &scene, &settings);
            let (mean, max) = difference(&before, &after);
            eprintln!("{mode:?} distance={distance}: mean={mean:.6}, max={max:.6}");
            worst_mean = worst_mean.max(mean);
            worst_max = worst_max.max(max);

            settings.show_grid = false;
            let hidden = render(&mut renderer, &scene, &settings);
            settings.show_grid = true;
            assert!(
                difference(&after, &hidden).0 > 0.0002,
                "The grid must remain visible"
            );
        }
    }
    assert!(
        worst_mean < 0.00015 && worst_max < 0.0015,
        "Tiny zoom produced a grid-density jump: mean={worst_mean}, max={worst_max}"
    );
}

#[test]
fn perspective_grid_stays_continuous_during_small_zoom_steps() {
    let mut renderer = Renderer::new().unwrap();
    let mut scene = Scene::empty();
    for mode in [
        RenderMode::Wireframe,
        RenderMode::Solid,
        RenderMode::MaterialPreview,
    ] {
        let settings = RenderSettings {
            width: 256,
            height: 192,
            mode,
            ..Default::default()
        };
        for (distance, pitch, yaw) in [(9.5, 0.36, 0.65), (45.0, 0.18, 0.0), (140.0, 0.45, 0.9)] {
            scene.camera.distance = distance;
            scene.camera.pitch = pitch;
            scene.camera.yaw = yaw;
            let before = render(&mut renderer, &scene, &settings);
            scene.camera.distance *= 1.0001;
            let after = render(&mut renderer, &scene, &settings);
            let (mean, max) = difference(&before, &after);
            assert!(
                mean < 0.00015 && max < 0.002,
                "{mode:?} distance={distance}: grid band popped during zoom: mean={mean}, max={max}"
            );
        }
    }
}

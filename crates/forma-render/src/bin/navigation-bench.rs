//! Repeatable full-resolution orbit benchmark. Run with --release; timings are
//! observations, not pass/fail thresholds tied to a particular GPU.
use anyhow::Result;
use forma_core::Scene;
use forma_render::{RenderMode, RenderSettings, Renderer};
use glam::Vec2;
use std::path::PathBuf;

fn main() -> Result<()> {
    let output = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("artifacts/navigation"));
    std::fs::create_dir_all(&output)?;
    let scene = std::env::args()
        .nth(2)
        .filter(|path| path != "-")
        .map(|path| Scene::load(std::path::Path::new(&path)))
        .transpose()?
        .unwrap_or_default();
    let width = std::env::args()
        .nth(3)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(2020);
    let height = std::env::args()
        .nth(4)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(1390);
    let mut renderer = Renderer::new()?;
    println!(
        "device={} resolution={width}x{height} objects={} frames=40 warmup=8",
        renderer.device_name(),
        scene.objects.len()
    );
    for (name, mode, ao, selected, grid) in [
        ("preview", RenderMode::MaterialPreview, true, true, true),
        (
            "preview-no-selection",
            RenderMode::MaterialPreview,
            true,
            false,
            true,
        ),
        (
            "preview-no-ao",
            RenderMode::MaterialPreview,
            false,
            true,
            true,
        ),
        (
            "preview-no-grid",
            RenderMode::MaterialPreview,
            true,
            true,
            false,
        ),
        ("solid", RenderMode::Solid, true, true, true),
        ("solid-no-selection", RenderMode::Solid, true, false, true),
    ] {
        let mut scene = scene.clone();
        let mut settings = RenderSettings {
            width,
            height,
            mode,
            show_grid: grid,
            selected: scene
                .objects
                .first()
                .filter(|_| selected)
                .map(|object| object.id),
            ..Default::default()
        };
        settings.preview.ambient_occlusion = ao;
        let mut timings = Vec::new();
        for i in 0..48 {
            scene.camera.orbit(Vec2::new(1.4, 0.));
            let frame = renderer.render(&scene, &settings, 1)?;
            if i >= 8 {
                timings.push(frame.elapsed_ms);
            }
        }
        timings.sort_by(f64::total_cmp);
        println!(
            "{name} median_ms={:.2} p95_ms={:.2} min_ms={:.2} max_ms={:.2}",
            timings[20], timings[37], timings[0], timings[39]
        );
        renderer.export_png(&output.join(format!("{name}.png")))?;
    }
    Ok(())
}

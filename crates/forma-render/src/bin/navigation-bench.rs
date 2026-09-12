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
    let mut renderer = Renderer::new()?;
    println!(
        "device={} resolution=2020x1390 frames=40 warmup=8",
        renderer.device_name()
    );
    for (name, mode, ao, selected) in [
        ("preview", RenderMode::MaterialPreview, true, true),
        (
            "preview-no-selection",
            RenderMode::MaterialPreview,
            true,
            false,
        ),
        ("preview-no-ao", RenderMode::MaterialPreview, false, true),
        ("solid", RenderMode::Solid, true, true),
    ] {
        let mut scene = Scene::default();
        let mut settings = RenderSettings {
            width: 2020,
            height: 1390,
            mode,
            selected: selected.then(|| scene.objects[0].id),
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

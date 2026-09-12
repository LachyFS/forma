//! Headless GPU verification and reference image generator.
//! cargo run -p forma-render --bin render-smoke -- artifacts/render 32
use std::path::PathBuf;

use anyhow::{Result, ensure};
use forma_core::Scene;
use forma_render::{RenderMode, RenderSettings, Renderer};

fn main() -> Result<()> {
    let directory = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("artifacts/render"));
    let samples = std::env::args()
        .nth(2)
        .and_then(|arg| arg.parse::<u32>().ok())
        .unwrap_or(32)
        .max(1);
    std::fs::create_dir_all(&directory)?;
    let mut renderer = Renderer::new()?;
    let scene = Scene::default();
    println!("GPU: {}", renderer.device_name());
    for (mode, name) in [
        (RenderMode::Wireframe, "wireframe"),
        (RenderMode::Solid, "solid"),
        (RenderMode::MaterialPreview, "material-preview"),
        (RenderMode::Rendered, "rendered"),
    ] {
        let settings = RenderSettings {
            mode,
            width: 640,
            height: 480,
            max_samples: samples,
            selected: scene.objects.first().map(|object| object.id),
            ..Default::default()
        };
        let count = if mode.progressive() { samples } else { 1 };
        let mut render_ms = 0.0;
        for index in 0..count {
            let frame = renderer.render(&scene, &settings, 1)?;
            render_ms += frame.elapsed_ms;
            ensure!(frame.samples == index + 1, "Sample counter failed");
        }
        let capped = renderer.render(&scene, &settings, 1)?;
        ensure!(capped.samples == count, "Sample cap failed");
        let path = directory.join(format!("{name}.png"));
        renderer.export_png(&path)?;
        let image = image::open(&path)?.to_rgb8();
        let (min, max) = image
            .pixels()
            .flat_map(|pixel| pixel.0)
            .fold((255u8, 0u8), |(min, max), value| {
                (min.min(value), max.max(value))
            });
        ensure!(
            max.saturating_sub(min) > 30,
            "{name} image is unexpectedly blank"
        );
        println!(
            "{name:18} {count:3} spp  {:7.1} ms/sample  {}",
            render_ms / count as f64,
            path.display()
        );
    }
    Ok(())
}

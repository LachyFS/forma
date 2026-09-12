//! Quantitative and visual end-to-end AI denoising verification.
//! python3 scripts/setup-denoiser.py
//! cargo run -p forma-render --bin denoise-smoke -- artifacts/denoise 8
use anyhow::{Result, ensure};
use forma_core::Scene;
use forma_render::{DenoiseQuality, Denoiser, RenderMode, RenderSettings, Renderer};
use std::{path::PathBuf, time::Instant};

fn main() -> Result<()> {
    let mut denoiser = Denoiser::new()?;
    if std::env::args().nth(1).as_deref() == Some("--probe") {
        println!("Open Image Denoise · {}", denoiser.device_name);
        return Ok(());
    }
    let directory = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "artifacts/denoise".into());
    let samples = std::env::args()
        .nth(2)
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or(8)
        .clamp(1, 64);
    std::fs::create_dir_all(&directory)?;
    let mut renderer = Renderer::new()?;
    let scene = Scene::default();
    let mut settings = RenderSettings {
        mode: RenderMode::Rendered,
        width: 512,
        height: 384,
        max_samples: samples,
        show_grid: false,
        ..Default::default()
    };
    for _ in 0..samples {
        renderer.render(&scene, &settings, 1)?;
    }
    renderer.export_png(&directory.join("raw.png"))?;
    let input = renderer.read_denoise_input()?;
    let started = Instant::now();
    let denoised = denoiser.denoise(&input, DenoiseQuality::Balanced, false)?;
    let denoise_ms = started.elapsed().as_secs_f64() * 1000.0;
    let frame =
        denoiser.denoise_frame(&input, DenoiseQuality::Balanced, false, settings.exposure)?;
    image::save_buffer(
        directory.join("viewport-denoised.png"),
        frame.rgba().unwrap(),
        frame.width(),
        frame.height(),
        image::ColorType::Rgba8,
    )?;
    renderer.export_denoised_png(&directory.join("export-denoised.png"), &mut denoiser)?;
    ensure!(
        renderer.read_linear_pixels()? == input.color,
        "Denoising changed the original film"
    );
    settings.max_samples = 256;
    for _ in samples..settings.max_samples {
        renderer.render(&scene, &settings, 1)?;
    }
    renderer.export_png(&directory.join("reference-256.png"))?;
    let reference = renderer.read_linear_pixels()?;
    let mse = |pixels: &[[f32; 4]]| {
        pixels
            .iter()
            .zip(&reference)
            .flat_map(|(p, r)| (0..3).map(move |i| f64::from(p[i] - r[i]).powi(2)))
            .sum::<f64>()
            / (reference.len() * 3) as f64
    };
    let raw_error = mse(&input.color);
    let clean_error = mse(&denoised);
    let report = format!(
        "Renderer: {} · {}\nDenoiser: Open Image Denoise · {}\nResolution: 512 × 384\nInput: {samples} spp; reference: 256 spp\nBalanced denoise including first filter setup: {denoise_ms:.1} ms\nRaw linear MSE: {raw_error:.8}\nDenoised linear MSE: {clean_error:.8}\nError reduction: {:.1}%\nOriginal film preserved; final export uses High with accurate guide prefiltering.\n",
        renderer.backend(),
        renderer.device_name(),
        denoiser.device_name,
        (1.0 - clean_error / raw_error) * 100.0
    );
    print!("{report}");
    std::fs::write(directory.join("report.txt"), report)?;
    ensure!(
        clean_error < raw_error * 0.8,
        "Denoising did not materially improve the reference error"
    );
    Ok(())
}

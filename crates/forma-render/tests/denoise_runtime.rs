//! Explicit runtime tests: python3 scripts/setup-denoiser.py, then
//! cargo test -p forma-render --test denoise_runtime -- --ignored
use forma_render::{DenoiseInput, DenoiseQuality, Denoiser};

#[test]
#[ignore = "requires the official Open Image Denoise 2.4+ runtime"]
fn neural_filter_reduces_noise_preserves_edges_and_reuses_resized_buffers() {
    let mut denoiser = Denoiser::new().expect("Install OIDN with scripts/setup-denoiser.py");
    for (width, height, quality, prefilter) in [
        (64, 48, DenoiseQuality::Balanced, false),
        (64, 48, DenoiseQuality::Balanced, false),
        (64, 48, DenoiseQuality::High, true),
        (80, 64, DenoiseQuality::Fast, false),
    ] {
        let mut input = DenoiseInput {
            width,
            height,
            samples: 8,
            color: Vec::new(),
            albedo: Vec::new(),
            normal: Vec::new(),
        };
        let mut reference = Vec::new();
        let mut rng = 17_u32;
        for _y in 0..height {
            for x in 0..width {
                let color = if x < width / 2 {
                    [0.18, 0.08, 0.04, 1.0]
                } else {
                    [1.2, 2.4, 0.6, 1.0]
                };
                rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = 0.3 + (rng as f64 / u32::MAX as f64) as f32 * 1.4;
                input
                    .color
                    .push([color[0] * noise, color[1] * noise, color[2] * noise, 1.0]);
                input.albedo.push(if x < width / 2 {
                    [0.45, 0.2, 0.1, 1.0]
                } else {
                    [0.2, 0.4, 0.1, 1.0]
                });
                input.normal.push([0.0, 0.0, -1.0, 1.0]);
                reference.push(color);
            }
        }
        let original = input.color.clone();
        let output = denoiser.denoise(&input, quality, prefilter).unwrap();
        let mse = |pixels: &[[f32; 4]]| {
            pixels
                .iter()
                .zip(&reference)
                .flat_map(|(p, r)| (0..3).map(move |i| (p[i] - r[i]).powi(2)))
                .sum::<f32>()
                / (width * height * 3) as f32
        };
        assert!(
            mse(&output) < mse(&original) * 0.2,
            "AI filter did not reduce error: {} vs {}",
            mse(&output),
            mse(&original)
        );
        // Average along each side of the discontinuity: a single Monte Carlo
        // pixel can overshoot without implying that the edge moved or blurred.
        for x in [width / 2 - 2, width / 2 + 1] {
            let mean_error = (4..height - 4)
                .map(|y| {
                    let i = (y * width + x) as usize;
                    output[i][1] - reference[i][1]
                })
                .sum::<f32>()
                / (height - 8) as f32;
            assert!(
                mean_error.abs() < (2.4 - 0.08) * 0.1,
                "Guide edge shifted: {quality:?}, prefilter={prefilter}, x={x}, mean error={mean_error}"
            );
        }
        assert_eq!(
            input.color, original,
            "Denoising modified the source estimator"
        );
        assert!(
            output
                .iter()
                .all(|p| p.iter().all(|v| v.is_finite()) && p[3] == 1.0)
        );
        let frame = denoiser
            .denoise_frame(&input, quality, prefilter, 0.5)
            .unwrap();
        assert!(frame.denoised);
        assert_eq!(
            (frame.width(), frame.height(), frame.samples),
            (width, height, 8)
        );
        assert!(
            frame
                .rgba()
                .unwrap()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[3] == 255)
        );
    }
    let black = DenoiseInput {
        width: 32,
        height: 32,
        samples: 1,
        color: vec![[0.0; 4]; 1024],
        albedo: vec![[0.5; 4]; 1024],
        normal: vec![[0.0, 1.0, 0.0, 1.0]; 1024],
    };
    let output = denoiser
        .denoise(&black, DenoiseQuality::High, true)
        .unwrap();
    assert!(
        output
            .iter()
            .all(|p| p[..3].iter().all(|&v| v.abs() < 1e-5))
    );
}

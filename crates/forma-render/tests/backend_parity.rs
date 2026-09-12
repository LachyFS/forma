//! Run on a Mac with a Metal GPU to compare the native and wgpu implementations.
#![cfg(target_os = "macos")]

use forma_core::Scene;
use forma_render::{Backend, RenderMode, RenderSettings, Renderer};

#[test]
fn native_and_wgpu_metal_keep_all_four_modes_visually_consistent() {
    let mut native = Renderer::with_backend(Backend::NativeMetal).unwrap();
    let mut portable = Renderer::with_backend(Backend::Metal).unwrap();
    let scene = Scene::default();
    for mode in [
        RenderMode::Wireframe,
        RenderMode::Solid,
        RenderMode::MaterialPreview,
        RenderMode::Rendered,
    ] {
        let settings = RenderSettings {
            mode,
            width: 64,
            height: 48,
            max_samples: 4,
            selected: Some(scene.objects[0].id),
            ..Default::default()
        };
        for _ in 0..if mode.progressive() { 4 } else { 1 } {
            native.render(&scene, &settings, 1).unwrap();
            portable.render(&scene, &settings, 1).unwrap();
        }
        let a = native.read_linear_pixels().unwrap();
        let b = portable.read_linear_pixels().unwrap();
        assert_eq!(a.len(), b.len());
        assert!(a.iter().chain(&b).flatten().all(|v| v.is_finite()));
        // Monte Carlo paths and boundary hits can differ with compiler floating
        // point arithmetic. Compare image energy, preserving HDR values rather
        // than allowing the display tone curve to hide transport differences.
        for channel in 0..3 {
            let mean = |pixels: &[[f32; 4]]| {
                pixels.iter().map(|p| p[channel]).sum::<f32>() / pixels.len() as f32
            };
            let left = mean(&a);
            let right = mean(&b);
            assert!(
                (left - right).abs() < 0.03 + left.abs() * 0.08,
                "{mode:?} channel {channel}: native={left}, wgpu={right}"
            );
        }
    }
}

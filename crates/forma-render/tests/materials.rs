use forma_core::ShaderLanguage;
use forma_core::{Material, Primitive, Scene, ShaderKind, TextureImage, TextureSlot, Vec3};
use forma_render::{Backend, RenderMode, RenderSettings, Renderer, validate_custom_shaders};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn language() -> ShaderLanguage {
    Backend::from_env().unwrap().shader_language()
}
fn code(metal: &str, wgsl: &str) -> String {
    match language() {
        ShaderLanguage::Metal => metal,
        ShaderLanguage::Wgsl => wgsl,
    }
    .into()
}
fn scene() -> (Scene, u64) {
    let mut scene = Scene::empty();
    let id = scene.add(Primitive::Sphere);
    scene.camera.target = Vec3::ZERO;
    scene.camera.distance = 4.0;
    scene.world.color = Vec3::ONE;
    scene.world.strength = 1.0;
    scene.object_material_mut(id).unwrap().base_color = Vec3::ONE;
    (scene, id)
}
fn settings(mode: RenderMode) -> RenderSettings {
    RenderSettings {
        width: 48,
        height: 48,
        mode,
        max_samples: 8,
        show_grid: false,
        ..Default::default()
    }
}
fn pixels(renderer: &mut Renderer) -> Vec<u8> {
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "forma-material-{}-{}.png",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    renderer.export_png(&path).unwrap();
    let pixels = image::open(&path).unwrap().into_rgb8().into_raw();
    std::fs::remove_file(path).unwrap();
    pixels
}
#[test]
fn textures_custom_code_and_glass_change_both_render_modes() {
    for mode in [RenderMode::MaterialPreview, RenderMode::Rendered] {
        let (mut scene, id) = scene();
        let settings = settings(mode);
        let mut renderer = Renderer::new().unwrap();
        renderer.render(&scene, &settings, 1).unwrap();
        let original = pixels(&mut renderer);
        scene.object_material_mut(id).unwrap().textures[TextureSlot::BaseColor as usize] =
            Some(Arc::new(TextureImage {
                name: "red.png".into(),
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255],
            }));
        assert_eq!(renderer.render(&scene, &settings, 2).unwrap().samples, 1);
        let textured = pixels(&mut renderer);
        assert_ne!(original, textured);
        *scene.object_material_mut(id).unwrap() = Material::glass();
        renderer.render(&scene, &settings, 3).unwrap();
        assert_ne!(textured, pixels(&mut renderer));
        let m = scene.object_material_mut(id).unwrap();
        m.shader = ShaderKind::Custom;
        m.custom_language = language();
        m.custom_code = code(
            "surface.color = float3(0); surface.emission = float3(0, 3, 0);",
            "surface.color = vec3(0.0); surface.emission = vec3(0.0, 3.0, 0.0);",
        );
        validate_custom_shaders(&scene).unwrap();
        let frame = renderer.render(&scene, &settings, 4).unwrap();
        assert!(frame.shader_error.is_none());
        let custom = pixels(&mut renderer);
        let center = (24 * 48 + 24) * 3;
        assert!(custom[center + 1] > custom[center] + 50);
    }
}
#[test]
fn bad_custom_source_has_diagnostics_pbr_fallback_and_recovers() {
    let (mut scene, id) = scene();
    let mut renderer = Renderer::new().unwrap();
    let settings = settings(RenderMode::MaterialPreview);
    renderer.render(&scene, &settings, 1).unwrap();
    let pbr = pixels(&mut renderer);
    let m = scene.object_material_mut(id).unwrap();
    m.shader = ShaderKind::Custom;
    m.custom_language = language();
    m.custom_code = code("surface.color = float3(;", "surface.color = vec3(;");
    assert!(
        validate_custom_shaders(&scene)
            .unwrap_err()
            .to_string()
            .contains(match language() {
                ShaderLanguage::Metal => "custom_0.metal",
                ShaderLanguage::Wgsl => "error",
            })
    );
    let frame = renderer.render(&scene, &settings, 2).unwrap();
    assert!(frame.shader_error.is_some());
    assert_eq!(pixels(&mut renderer), pbr);
    scene.object_material_mut(id).unwrap().custom_code = code(
        "surface.color = float3(0.05f, 0.3f, 0.8f);",
        "surface.color = vec3(0.05, 0.3, 0.8);",
    );
    assert!(
        renderer
            .render(&scene, &settings, 3)
            .unwrap()
            .shader_error
            .is_none()
    );
    assert_ne!(pixels(&mut renderer), pbr);
}
#[test]
fn default_template_and_nonfinite_outputs_compile_without_poisoning_the_film() {
    let (mut scene, id) = scene();
    let m = scene.object_material_mut(id).unwrap();
    m.shader = ShaderKind::Custom;
    m.custom_language = language();
    m.custom_code = language().default_code().into();
    validate_custom_shaders(&scene).unwrap();
    scene.object_material_mut(id).unwrap().custom_code = code(
        "surface.color = float3(NAN); surface.roughness = NAN; surface.normal = float3(0);",
        "let invalid = bitcast<f32>(0x7fc00000u); surface.color = vec3(invalid); surface.roughness = invalid; surface.normal = vec3(0.0);",
    );
    validate_custom_shaders(&scene).unwrap();
    let mut renderer = Renderer::new().unwrap();
    for mode in [RenderMode::MaterialPreview, RenderMode::Rendered] {
        let frame = renderer.render(&scene, &settings(mode), 1).unwrap();
        assert!(frame.shader_error.is_none());
        assert!(pixels(&mut renderer).iter().any(|v| *v > 0));
        assert!(
            renderer
                .read_linear_pixels()
                .unwrap()
                .iter()
                .flatten()
                .all(|v| v.is_finite())
        );
    }
}

#[test]
fn incompatible_language_reports_error_and_recovers_when_language_changes() {
    let (mut scene, id) = scene();
    let mut renderer = Renderer::new().unwrap();
    let settings = settings(RenderMode::MaterialPreview);
    renderer.render(&scene, &settings, 1).unwrap();
    let baseline = pixels(&mut renderer);
    let m = scene.object_material_mut(id).unwrap();
    m.shader = ShaderKind::Custom;
    m.custom_language = match language() {
        ShaderLanguage::Metal => ShaderLanguage::Wgsl,
        ShaderLanguage::Wgsl => ShaderLanguage::Metal,
    };
    // The code is valid in both languages: changing only its language must
    // invalidate the failed compilation cache and compile the compatible body.
    m.custom_code = "surface.roughness = 0.8;".into();
    let frame = renderer.render(&scene, &settings, 2).unwrap();
    assert!(
        frame
            .shader_error
            .unwrap()
            .contains("this renderer requires")
    );
    assert_eq!(pixels(&mut renderer), baseline);
    scene.object_material_mut(id).unwrap().custom_language = language();
    assert!(
        renderer
            .render(&scene, &settings, 3)
            .unwrap()
            .shader_error
            .is_none()
    );
    assert_ne!(pixels(&mut renderer), baseline);
}

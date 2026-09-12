use forma_core::{
    History, MAX_SHADER_BYTES, Material, Primitive, Scene, ShaderKind, ShaderLanguage,
    TextureImage, TextureMapping, TextureSlot, Vec2, Vec3,
};
use std::sync::Arc;

fn image() -> Arc<TextureImage> {
    Arc::new(TextureImage {
        name: "albedo.png".into(),
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 128, 255, 255],
    })
}

#[test]
fn materials_textures_and_code_are_portable_and_undoable() {
    let mut scene = Scene::empty();
    let id = scene.add(Primitive::Cube);
    let before = scene.clone();
    let mut history = History::default();
    history.checkpoint(&scene);
    let m = scene.object_material_mut(id).unwrap();
    *m = Material::glass();
    m.shader = ShaderKind::Custom;
    m.custom_code = "surface.glass = true;\nsurface.ior = 1.45f;".into();
    m.mapping = TextureMapping::Sphere;
    m.texture_scale = Vec2::new(2.0, 3.0);
    m.texture_offset = Vec2::new(-0.5, 0.25);
    m.textures[TextureSlot::BaseColor as usize] = Some(image());
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("textured.forma");
    scene.save(&file).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved["version"], 3);
    assert!(saved["scene"]["materials"][0]["material"]["textures"][0]["rgba"].is_string());
    assert_eq!(Scene::load(&file).unwrap(), scene);
    let edited = scene.clone();
    assert!(history.undo(&mut scene));
    assert_eq!(scene, before);
    assert!(history.redo(&mut scene));
    assert_eq!(scene, edited);
    let m = scene.object_material_mut(id).unwrap();
    m.custom_language = ShaderLanguage::Wgsl;
    m.custom_code = ShaderLanguage::Wgsl.default_code().into();
    scene.save(&file).unwrap();
    assert_eq!(Scene::load(&file).unwrap(), scene);
}

#[test]
fn version_two_opaque_materials_receive_shader_defaults() {
    let mut scene = Scene::empty();
    let id = scene.add(Primitive::Sphere);
    scene.object_material_mut(id).unwrap().base_color = Vec3::new(0.1, 0.2, 0.3);
    let mut value = serde_json::to_value(&scene).unwrap();
    let material = value["materials"][0]["material"].as_object_mut().unwrap();
    material.retain(|key, _| {
        ["base_color", "metallic", "roughness", "emission"].contains(&key.as_str())
    });
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("legacy.forma");
    std::fs::write(
        &file,
        serde_json::to_vec(&serde_json::json!({"format":"forma", "version":2, "scene":value}))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(Scene::load(&file).unwrap(), scene);
}

#[test]
fn malformed_texture_pixels_and_shader_values_are_rejected() {
    let mut m = Material {
        ior: f32::NAN,
        ..Material::default()
    };
    assert!(m.validate().is_err());
    m.ior = 0.0;
    assert!(m.validate().is_err());
    m.ior = 1.5;
    m.normal_strength = f32::INFINITY;
    assert!(m.validate().is_err());
    m.normal_strength = 1.0;
    m.texture_scale.x = 0.0;
    assert!(m.validate().is_err());
    m.texture_scale = Vec2::ONE;
    m.custom_code = "x".repeat(MAX_SHADER_BYTES + 1);
    assert!(m.validate().is_err());
    m.custom_code.clear();
    m.shader = ShaderKind::Custom;
    assert!(m.validate().is_err());
    m.custom_code = "surface.color = float3(1);".into();
    let mut bad = (*image()).clone();
    bad.rgba.pop();
    m.textures[0] = Some(Arc::new(bad));
    assert!(m.validate().is_err());
    let mut bad = (*image()).clone();
    bad.width = u32::MAX;
    assert!(bad.validate().is_err());
    let mut value = serde_json::to_value(image()).unwrap();
    value["rgba"] = "not base64!".into();
    assert!(serde_json::from_value::<TextureImage>(value).is_err());
}

#[test]
fn linked_materials_share_images_and_independent_duplicates_can_replace_them() {
    let mut scene = Scene::empty();
    let id = scene.add(Primitive::Cube);
    scene.object_material_mut(id).unwrap().textures[0] = Some(image());
    let duplicate = scene.duplicate(id).unwrap();
    let a = scene.object_material(id).unwrap().textures[0]
        .as_ref()
        .unwrap();
    let b = scene.object_material(duplicate).unwrap().textures[0]
        .as_ref()
        .unwrap();
    assert!(Arc::ptr_eq(a, b));
    scene.object_material_mut(duplicate).unwrap().textures[0] = None;
    assert!(scene.object_material(id).unwrap().textures[0].is_some());
    scene.validate().unwrap();
}

#[test]
fn custom_shader_count_is_bounded_and_identical_code_is_shared() {
    let mut scene = Scene::empty();
    for i in 0..32 {
        scene
            .create_material_data(
                format!("Shader {i}"),
                Material {
                    shader: ShaderKind::Custom,
                    custom_code: format!("surface.roughness = {}f;", i as f32 / 33.0),
                    ..Material::default()
                },
            )
            .unwrap();
    }
    assert!(
        scene
            .create_material_data(
                "One too many",
                Material {
                    shader: ShaderKind::Custom,
                    custom_code: "surface.color = float3(0);".into(),
                    ..Material::default()
                }
            )
            .is_err()
    );
    scene.validate().unwrap();
    let code = scene.materials[0].material.custom_code.clone();
    for data in &mut scene.materials {
        data.material.custom_code = code.clone();
    }
    scene.validate().unwrap();
}

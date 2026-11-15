//! CAD part data blocks: scene integration, file migration and validation.
use forma_core::{
    FeatureIssue, FeatureKind, FeatureOperation, History, Mesh, ObjectData, PartData, Placement,
    Primitive, PrimitiveFeature, PrimitiveShape, Scene,
};
use glam::Vec3;
use std::fs;

fn box_part() -> PartData {
    let mut part = PartData::new(0, "Bracket");
    part.push_feature(
        "Box",
        FeatureKind::Primitive(PrimitiveFeature {
            shape: PrimitiveShape::Box {
                size: Vec3::splat(2.0),
            },
            placement: Placement::default(),
            operation: FeatureOperation::Join,
        }),
    );
    part
}

fn grouped_quad() -> Mesh {
    let mut mesh = Mesh::primitive(Primitive::Plane);
    mesh.faces = vec![vec![0, 1, 2], vec![0, 2, 3]];
    mesh.groups = vec![0, 0];
    mesh
}

#[test]
fn parts_render_only_after_a_current_tessellation_arrives() {
    let mut scene = Scene::empty();
    let id = scene.add_part("Bracket", box_part()).unwrap();
    scene.validate().unwrap();
    let object = scene.object(id).unwrap();
    assert_eq!(object.kind(), "part");
    assert!(
        object.mesh_id().is_none(),
        "no cache before the first regeneration"
    );
    assert!(scene.mesh_instance(id).is_none());
    let part = scene.object_part(id).unwrap();
    let (part_id, stamp) = (part.id, part.stamp);
    assert!(!part.is_evaluated());

    // A result for an older stamp is dropped.
    assert!(
        !scene
            .set_part_tessellation(part_id, stamp ^ 1, Some(grouped_quad()), Vec::new())
            .unwrap()
    );
    assert!(scene.object(id).unwrap().mesh_id().is_none());
    let issue = FeatureIssue {
        feature: 1,
        message: "test".into(),
        suggestion: String::new(),
    };
    assert!(
        scene
            .set_part_tessellation(part_id, stamp, Some(grouped_quad()), vec![issue.clone()])
            .unwrap()
    );
    let instance = scene
        .mesh_instance(id)
        .expect("parts render through MeshInstance");
    assert_eq!(instance.mesh.groups, vec![0, 0]);
    assert!(scene.object_part(id).unwrap().is_evaluated());
    assert_eq!(scene.object_part(id).unwrap().issues(), &[issue]);
    // Polygon tools cannot reach a part's cached tessellation.
    assert!(scene.object_mesh_mut(id).is_none());
    assert!(scene.object_mesh(id).is_some());
    // A new result reuses the same mesh block.
    let mesh_id = scene.object(id).unwrap().mesh_id().unwrap();
    scene
        .set_part_tessellation(part_id, stamp, Some(grouped_quad()), Vec::new())
        .unwrap();
    assert_eq!(scene.object(id).unwrap().mesh_id(), Some(mesh_id));
    assert_eq!(scene.meshes.len(), 1);
    scene.validate().unwrap();
}

#[test]
fn duplicates_own_their_history_and_linked_duplicates_share_it() {
    let mut scene = Scene::empty();
    let id = scene.add_part("Bracket", box_part()).unwrap();
    let part_id = scene.object(id).unwrap().part_id().unwrap();
    let stamp = scene.part(part_id).unwrap().stamp;
    scene
        .set_part_tessellation(part_id, stamp, Some(grouped_quad()), Vec::new())
        .unwrap();
    let copy = scene.duplicate(id).unwrap();
    let linked = scene.duplicate_linked(id).unwrap();
    scene.validate().unwrap();
    let copy_part = scene.object(copy).unwrap().part_id().unwrap();
    assert_ne!(copy_part, part_id);
    assert_ne!(
        scene.object(copy).unwrap().mesh_id(),
        scene.object(id).unwrap().mesh_id()
    );
    assert_ne!(
        scene.object(copy).unwrap().data.material_ids(),
        scene.object(id).unwrap().data.material_ids()
    );
    assert_eq!(scene.object(linked).unwrap().part_id(), Some(part_id));
    assert_eq!(scene.part_users(part_id), 2);
    // Regeneration updates every object of the part.
    let mut mesh = grouped_quad();
    mesh.positions[0].y = 0.5;
    scene
        .set_part_tessellation(part_id, stamp, Some(mesh), Vec::new())
        .unwrap();
    assert_eq!(scene.object_mesh(linked).unwrap().positions[0].y, 0.5);
    assert_eq!(scene.object_mesh(copy).unwrap().positions[0].y, 0.0);
    scene.remove(copy);
    scene.purge_orphans();
    assert!(scene.part(copy_part).is_none());
    scene.validate().unwrap();
}

#[test]
fn parts_round_trip_and_stale_stamps_are_recomputed_on_load() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("part.forma");
    let mut scene = Scene::default();
    let id = scene.add_part("Bracket", box_part()).unwrap();
    let part_id = scene.object(id).unwrap().part_id().unwrap();
    let stamp = scene.part(part_id).unwrap().stamp;
    scene
        .set_part_tessellation(part_id, stamp, Some(grouped_quad()), Vec::new())
        .unwrap();
    scene.save(&path).unwrap();
    let loaded = Scene::load(&path).unwrap();
    assert_eq!(loaded, scene);
    assert!(loaded.part(part_id).unwrap().is_evaluated());

    // Editing the history in the file without restamping must not reuse the cache.
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(json["version"], 4);
    let size = &mut json["scene"]["parts"][0]["features"][0]["kind"]["shape"]["size"];
    *size = serde_json::json!([3.0, 3.0, 3.0]);
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let edited = Scene::load(&path).unwrap();
    assert!(!edited.part(part_id).unwrap().is_evaluated());
}

#[test]
fn version_three_scenes_migrate_without_parts() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("old.forma");
    let scene = Scene::default();
    scene.save(&path).unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    json["version"] = 3.into();
    json["scene"].as_object_mut().unwrap().remove("parts");
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let loaded = Scene::load(&path).unwrap();
    assert!(loaded.parts.is_empty());
    assert_eq!(loaded.objects, scene.objects);
    // A future version is refused with a clear message.
    json["version"] = 5.into();
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let error = Scene::load(&path).unwrap_err().to_string();
    assert!(error.contains("newer Forma"), "{error}");
}

#[test]
fn validation_rejects_dangling_part_references_and_bad_features() {
    let mut scene = Scene::empty();
    let id = scene.add_part("Bracket", box_part()).unwrap();
    let mut missing = scene.clone();
    missing.parts.clear();
    assert!(missing.validate().is_err());
    let mut bad_mesh = scene.clone();
    if let ObjectData::Part { mesh, .. } = &mut bad_mesh.object_mut(id).unwrap().data {
        *mesh = Some(999_999);
    }
    assert!(bad_mesh.validate().is_err());
    let mut bad_feature = scene.clone();
    bad_feature.parts[0].features[0].kind = FeatureKind::Primitive(PrimitiveFeature {
        shape: PrimitiveShape::Sphere { radius: -1.0 },
        placement: Placement::default(),
        operation: FeatureOperation::Join,
    });
    assert!(bad_feature.validate().is_err());
    // Mismatched face groups make a mesh invalid.
    let mut mesh = grouped_quad();
    mesh.groups.push(3);
    assert!(mesh.validate().is_err());
}

#[test]
fn transform_baking_refuses_parts_and_undo_restores_them() {
    let mut scene = Scene::empty();
    let id = scene.add_part("Bracket", box_part()).unwrap();
    let part_id = scene.object(id).unwrap().part_id().unwrap();
    let stamp = scene.part(part_id).unwrap().stamp;
    scene
        .set_part_tessellation(part_id, stamp, Some(grouped_quad()), Vec::new())
        .unwrap();
    scene.object_mut(id).unwrap().transform.scale = Vec3::splat(2.0);
    let ids = [id].into_iter().collect();
    let error = scene
        .apply_object_transforms(&ids, forma_core::ApplyTransform::Scale)
        .unwrap_err();
    assert!(error.to_string().contains("CAD part"), "{error}");
    let mut history = History::default();
    history.checkpoint(&scene);
    scene
        .part_mut(part_id)
        .unwrap()
        .edit_feature(1, |feature| feature.suppressed = true);
    assert!(!scene.part(part_id).unwrap().is_evaluated());
    assert!(history.undo(&mut scene));
    assert!(scene.part(part_id).unwrap().is_evaluated());
}

#[test]
fn display_edges_hide_edges_inside_groups() {
    let mesh = grouped_quad();
    let edges = mesh.display_edges();
    assert_eq!(edges.len(), 4, "the shared diagonal is interior: {edges:?}");
    assert!(!edges.contains(&[0, 2]));
    let mut split = mesh.clone();
    split.groups = vec![0, 1];
    assert_eq!(split.display_edges().len(), 5);
    let mut plain = mesh;
    plain.groups.clear();
    assert_eq!(plain.display_edges(), plain.edges());
}

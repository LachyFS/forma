use forma_core::{ApplyTransform, History, Mat4, Primitive, Scene, Transform, Vec3};
use std::collections::{BTreeMap, BTreeSet};

fn world_vertices(scene: &Scene) -> BTreeMap<u64, Vec<Vec3>> {
    scene
        .mesh_instances()
        .map(|instance| {
            (
                instance.id,
                instance
                    .mesh
                    .positions
                    .iter()
                    .map(|&p| instance.world_transform.transform_point3(p))
                    .collect(),
            )
        })
        .collect()
}

fn assert_world_geometry(scene: &Scene, expected: &BTreeMap<u64, Vec<Vec3>>) {
    let actual = world_vertices(scene);
    assert_eq!(actual.len(), expected.len());
    for (id, before) in expected {
        let after = &actual[id];
        assert_eq!(before.len(), after.len());
        for (&before, &after) in before.iter().zip(after) {
            let tolerance = before.abs().max_element().max(1.) * 3e-6;
            assert!(
                before.abs_diff_eq(after, tolerance),
                "Object {id}: {before:?} moved to {after:?}"
            );
        }
    }
}

fn assert_matrix(actual: Mat4, expected: Mat4) {
    assert!(
        actual.abs_diff_eq(expected, 2e-5),
        "World transform changed:\n{expected:?}\n{actual:?}"
    );
}

fn offset_mesh(scene: &mut Scene, id: u64, offset: Vec3) {
    for position in &mut scene.object_mesh_mut(id).unwrap().positions {
        *position += offset;
    }
}

/// IDs intentionally put descendants before ancestors, and one child uses a
/// sheared parent inverse, covering cases where decomposing world TRS fails.
fn hierarchy() -> (Scene, u64, u64, u64, u64, u64) {
    let mut scene = Scene::empty();
    let child = scene.add(Primitive::Cube);
    let grandchild = scene.add(Primitive::Cube);
    let empty_child = scene.add_empty("Child origin marker").unwrap();
    let parent = scene.add(Primitive::Cube);
    let root = scene.add_empty("Transformed root").unwrap();
    scene.object_mut(root).unwrap().transform = Transform {
        translation: Vec3::new(2., -3., 1.),
        rotation: Vec3::new(0.3, -0.7, 0.1),
        scale: Vec3::new(2., 0.6, -1.2),
    };
    scene.object_mut(parent).unwrap().transform = Transform {
        translation: Vec3::new(-2., 0.5, 0.3),
        rotation: Vec3::new(-0.6, 0.4, 0.2),
        scale: Vec3::new(1.5, -0.7, 2.2),
    };
    scene.object_mut(child).unwrap().transform = Transform {
        translation: Vec3::new(1., 2., -1.),
        rotation: Vec3::new(0.7, -0.1, 0.3),
        scale: Vec3::new(-0.8, 1.3, 0.9),
    };
    scene.object_mut(grandchild).unwrap().transform = Transform {
        translation: Vec3::new(4., -1., 3.),
        rotation: Vec3::new(0.2, 0.6, -0.9),
        scale: Vec3::new(0.6, 1.4, 0.8),
    };
    scene.object_mut(empty_child).unwrap().transform.translation = Vec3::new(-1., 3., 2.);
    scene.set_parent(parent, Some(root), false).unwrap();
    scene.set_parent(child, Some(parent), false).unwrap();
    scene.set_parent(grandchild, Some(child), true).unwrap();
    scene.set_parent(empty_child, Some(parent), false).unwrap();
    scene.validate().unwrap();
    (scene, root, parent, child, grandchild, empty_child)
}

#[test]
fn applying_scale_preserves_world_geometry_rotation_materials_and_linked_users() {
    let mut scene = Scene::empty();
    let id = scene.add(Primitive::Cube);
    let linked = scene.duplicate_linked(id).unwrap();
    scene.object_mut(id).unwrap().transform = Transform {
        translation: Vec3::new(2., -1., 4.),
        rotation: Vec3::new(0.6, -0.3, 0.8),
        scale: Vec3::new(-2., 0.5, 3.),
    };
    let before = scene.clone();
    let vertices = world_vertices(&scene);
    scene
        .apply_object_transforms(&BTreeSet::from([id]), ApplyTransform::Scale)
        .unwrap();
    scene.validate().unwrap();
    let old_object = before.object(id).unwrap();
    let object = scene.object(id).unwrap();
    assert_eq!(object.transform.scale, Vec3::ONE);
    assert_eq!(object.transform.rotation, old_object.transform.rotation);
    assert_eq!(
        object.transform.translation,
        old_object.transform.translation
    );
    assert_eq!(object.data.material_ids(), old_object.data.material_ids());
    assert_ne!(object.mesh_id(), scene.object(linked).unwrap().mesh_id());
    assert_eq!(scene.object(linked), before.object(linked));
    assert_eq!(scene.object_mesh(linked), before.object_mesh(linked));
    for (&old, &new) in before
        .object_mesh(id)
        .unwrap()
        .positions
        .iter()
        .zip(&scene.object_mesh(id).unwrap().positions)
    {
        assert_eq!(new, old * old_object.transform.scale);
    }
    assert_world_geometry(&scene, &vertices);
}

#[test]
fn applying_parent_and_child_preserves_sheared_descendants_in_any_hierarchy_id_order() {
    for mode in [ApplyTransform::Scale, ApplyTransform::RotationScale] {
        let (mut scene, root, parent, child, grandchild, empty_child) = hierarchy();
        let before = scene.clone();
        let vertices = world_vertices(&scene);
        let selected = BTreeSet::from([parent, child]);
        scene.apply_object_transforms(&selected, mode).unwrap();
        scene.validate().unwrap();
        assert_world_geometry(&scene, &vertices);
        for id in [root, grandchild, empty_child] {
            assert_matrix(
                scene.world_transform(id).unwrap(),
                before.world_transform(id).unwrap(),
            );
            assert_eq!(
                scene.object(id).unwrap().transform,
                before.object(id).unwrap().transform
            );
        }
        for id in [parent, child] {
            let object = scene.object(id).unwrap();
            assert_eq!(object.transform.scale, Vec3::ONE);
            assert_eq!(object.parent, before.object(id).unwrap().parent);
            if mode == ApplyTransform::RotationScale {
                assert_eq!(object.transform.rotation, Vec3::ZERO);
            }
        }
        // Compare with the opposite order applied as separate transactions.
        let mut other_order = before;
        other_order
            .apply_object_transforms(&BTreeSet::from([parent]), mode)
            .unwrap();
        other_order
            .apply_object_transforms(&BTreeSet::from([child]), mode)
            .unwrap();
        assert_eq!(scene, other_order);
    }
}

#[test]
fn applying_several_linked_objects_bakes_each_once_and_preserves_external_users() {
    let mut scene = Scene::empty();
    let first = scene.add(Primitive::Cube);
    let second = scene.duplicate_linked(first).unwrap();
    let untouched = scene.duplicate_linked(first).unwrap();
    scene.object_mut(first).unwrap().transform.scale = Vec3::new(2., 3., 4.);
    scene.object_mut(second).unwrap().transform.scale = Vec3::new(-0.5, 0.75, 1.5);
    let before = scene.clone();
    let vertices = world_vertices(&scene);
    scene
        .apply_object_transforms(
            &BTreeSet::from([first, second]),
            ApplyTransform::RotationScale,
        )
        .unwrap();
    assert_world_geometry(&scene, &vertices);
    assert_eq!(scene.object_mesh(untouched), before.object_mesh(untouched));
    assert_eq!(scene.object(untouched), before.object(untouched));
    let mesh_ids: BTreeSet<_> = [first, second, untouched]
        .map(|id| scene.object(id).unwrap().mesh_id().unwrap())
        .into_iter()
        .collect();
    assert_eq!(mesh_ids.len(), 3);
}

#[test]
fn centering_origins_preserves_world_geometry_children_and_other_linked_instances() {
    let (mut scene, root, parent, child, grandchild, empty_child) = hierarchy();
    let offset = Vec3::new(3., -2., 4.);
    offset_mesh(&mut scene, parent, offset);
    let linked = scene.duplicate_linked(parent).unwrap();
    let before = scene.clone();
    let vertices = world_vertices(&scene);
    scene
        .center_object_origins(&BTreeSet::from([parent]))
        .unwrap();
    scene.validate().unwrap();
    assert_world_geometry(&scene, &vertices);
    for id in [root, child, grandchild, empty_child, linked] {
        assert_matrix(
            scene.world_transform(id).unwrap(),
            before.world_transform(id).unwrap(),
        );
        assert_eq!(
            scene.object(id).unwrap().transform,
            before.object(id).unwrap().transform
        );
    }
    let old = before.object(parent).unwrap();
    let new = scene.object(parent).unwrap();
    assert_eq!(new.transform.rotation, old.transform.rotation);
    assert_eq!(new.transform.scale, old.transform.scale);
    assert_eq!(
        new.transform.translation,
        old.transform.matrix().transform_point3(offset)
    );
    let expected_origin = before
        .world_transform(parent)
        .unwrap()
        .transform_point3(offset);
    assert!(
        scene
            .world_transform(parent)
            .unwrap()
            .transform_point3(Vec3::ZERO)
            .abs_diff_eq(expected_origin, 1e-5)
    );
    assert_eq!(scene.object_mesh(linked), before.object_mesh(linked));
    assert_ne!(new.mesh_id(), old.mesh_id());
    assert_eq!(
        scene.object_mesh(parent).unwrap(),
        &forma_core::Mesh::primitive(Primitive::Cube)
    );
}

#[test]
fn centering_parent_and_child_origins_is_independent_of_hierarchy_id_order() {
    let (mut scene, _, parent, child, grandchild, empty_child) = hierarchy();
    offset_mesh(&mut scene, parent, Vec3::new(3., -2., 1.));
    offset_mesh(&mut scene, child, Vec3::new(-4., 1., 2.));
    let before = scene.clone();
    let vertices = world_vertices(&scene);
    scene
        .center_object_origins(&BTreeSet::from([parent, child]))
        .unwrap();
    assert_world_geometry(&scene, &vertices);
    for id in [grandchild, empty_child] {
        assert_matrix(
            scene.world_transform(id).unwrap(),
            before.world_transform(id).unwrap(),
        );
    }
    let mut reverse = before;
    reverse
        .center_object_origins(&BTreeSet::from([parent]))
        .unwrap();
    reverse
        .center_object_origins(&BTreeSet::from([child]))
        .unwrap();
    assert_eq!(scene, reverse);
}

#[test]
fn noops_leave_shared_data_and_id_sequence_unchanged() {
    let mut scene = Scene::empty();
    let first = scene.add(Primitive::Cube);
    let second = scene.duplicate_linked(first).unwrap();
    let ids = BTreeSet::from([first, second]);
    let before = scene.clone();
    scene
        .apply_object_transforms(&ids, ApplyTransform::Scale)
        .unwrap();
    scene
        .apply_object_transforms(&ids, ApplyTransform::RotationScale)
        .unwrap();
    scene.center_object_origins(&ids).unwrap();
    assert_eq!(scene, before);
    scene.object_mut(first).unwrap().transform.rotation = Vec3::new(0.3, 0.1, -0.2);
    let before = scene.clone();
    scene
        .apply_object_transforms(&ids, ApplyTransform::Scale)
        .unwrap();
    scene.center_object_origins(&ids).unwrap();
    assert_eq!(scene, before);
}

#[test]
fn centering_all_linked_instances_uses_original_bounds_for_every_copy() {
    let mut scene = Scene::empty();
    let first = scene.add(Primitive::Cube);
    offset_mesh(&mut scene, first, Vec3::new(3., -2., 4.));
    let second = scene.duplicate_linked(first).unwrap();
    let third = scene.duplicate_linked(first).unwrap();
    scene.object_mut(first).unwrap().transform.scale = Vec3::new(-2., 3., 0.5);
    scene.object_mut(second).unwrap().transform.rotation = Vec3::new(0.4, -0.2, 0.6);
    scene.object_mut(third).unwrap().transform.translation = Vec3::new(4., 2., -3.);
    scene.set_parent(first, Some(third), false).unwrap();
    let ids = BTreeSet::from([first, second, third]);
    let before = world_vertices(&scene);
    scene.center_object_origins(&ids).unwrap();
    assert_world_geometry(&scene, &before);
    let mut mesh_ids = BTreeSet::new();
    for id in ids.iter().copied() {
        mesh_ids.insert(scene.object(id).unwrap().mesh_id().unwrap());
        assert_eq!(
            scene.object_mesh(id).unwrap(),
            &forma_core::Mesh::primitive(Primitive::Cube)
        );
    }
    assert_eq!(mesh_ids.len(), 3);
    let centered = scene.clone();
    scene.center_object_origins(&ids).unwrap();
    assert_eq!(scene, centered, "Repeated centering must remain a no-op");
}

#[test]
fn invalid_selection_and_singular_input_are_atomic() {
    let mut scene = Scene::empty();
    let mesh = scene.add(Primitive::Cube);
    let empty = scene.add_empty("Not a mesh").unwrap();
    scene.object_mut(mesh).unwrap().transform.scale = Vec3::splat(2.);
    for ids in [
        BTreeSet::new(),
        BTreeSet::from([mesh, u64::MAX]),
        BTreeSet::from([mesh, empty]),
    ] {
        let before = scene.clone();
        assert!(
            scene
                .apply_object_transforms(&ids, ApplyTransform::Scale)
                .is_err()
        );
        assert_eq!(scene, before);
        assert!(scene.center_object_origins(&ids).is_err());
        assert_eq!(scene, before);
    }
    scene.object_mut(mesh).unwrap().transform.scale.x = 0.;
    let before = scene.clone();
    assert!(
        scene
            .apply_object_transforms(&BTreeSet::from([mesh]), ApplyTransform::Scale)
            .is_err()
    );
    assert_eq!(scene, before);
    assert!(
        scene
            .center_object_origins(&BTreeSet::from([mesh]))
            .is_err()
    );
    assert_eq!(scene, before);
}

#[test]
fn failure_after_an_earlier_bake_rolls_back_geometry_links_and_allocated_ids() {
    let mut scene = Scene::empty();
    let good = scene.add(Primitive::Cube);
    scene.duplicate_linked(good).unwrap();
    scene.object_mut(good).unwrap().transform.scale = Vec3::splat(2.);
    let too_large = scene.add(Primitive::Cube);
    offset_mesh(&mut scene, too_large, Vec3::splat(1000.));
    scene.object_mut(too_large).unwrap().transform.scale = Vec3::splat(1e6);
    scene.validate().unwrap();
    let before = scene.clone();
    assert!(
        scene
            .apply_object_transforms(&BTreeSet::from([good, too_large]), ApplyTransform::Scale)
            .is_err()
    );
    assert_eq!(scene, before);
    offset_mesh(&mut scene, good, Vec3::new(2., 0., 0.));
    let before = scene.clone();
    assert!(
        scene
            .center_object_origins(&BTreeSet::from([good, too_large]))
            .is_err()
    );
    assert_eq!(scene, before);
}

#[test]
fn unsupported_child_compensation_is_rejected_without_moving_the_hierarchy() {
    let mut scene = Scene::empty();
    let parent = scene.add(Primitive::Cube);
    let child = scene.add(Primitive::Cube);
    scene.object_mut(parent).unwrap().transform.scale = Vec3::splat(1e-5);
    scene.set_parent(child, Some(parent), false).unwrap();
    scene.validate().unwrap();
    let before = scene.clone();
    assert!(
        scene
            .apply_object_transforms(&BTreeSet::from([parent]), ApplyTransform::Scale)
            .is_err()
    );
    assert_eq!(scene, before);
}

#[test]
fn object_utilities_round_trip_through_history_and_scene_files() {
    let (mut scene, _, parent, child, _, _) = hierarchy();
    offset_mesh(&mut scene, parent, Vec3::new(2., -3., 1.));
    let before = scene.clone();
    let vertices = world_vertices(&scene);
    let mut history = History::default();
    history.checkpoint(&scene);
    scene
        .apply_object_transforms(
            &BTreeSet::from([parent, child]),
            ApplyTransform::RotationScale,
        )
        .unwrap();
    scene
        .center_object_origins(&BTreeSet::from([parent, child]))
        .unwrap();
    let after = scene.clone();
    assert_world_geometry(&scene, &vertices);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("applied-origins.forma");
    scene.save(&path).unwrap();
    assert_eq!(Scene::load(&path).unwrap(), after);
    assert!(history.undo(&mut scene));
    assert_eq!(scene, before);
    assert!(history.redo(&mut scene));
    assert_eq!(scene, after);
}

fn world_face_normals(scene: &Scene) -> BTreeMap<u64, Vec<Vec3>> {
    scene
        .mesh_instances()
        .map(|instance| {
            let transform = instance.world_transform.inverse().transpose();
            (
                instance.id,
                (0..instance.mesh.faces.len())
                    .map(|face| {
                        transform
                            .transform_vector3(instance.mesh.face_normal(face))
                            .normalize()
                    })
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn applying_reflections_preserves_renderer_normals_and_export_winding() {
    for mode in [ApplyTransform::Scale, ApplyTransform::RotationScale] {
        let (mut scene, _, parent, child, _, _) = hierarchy();
        let normals = world_face_normals(&scene);
        let directory = tempfile::tempdir().unwrap();
        let before_path = directory.path().join("before.stl");
        let after_path = directory.path().join("after.stl");
        scene.export_stl(&before_path, 1., None).unwrap();
        scene
            .apply_object_transforms(&BTreeSet::from([parent, child]), mode)
            .unwrap();
        for (id, actual) in world_face_normals(&scene) {
            for (actual, expected) in actual.into_iter().zip(&normals[&id]) {
                assert!(
                    actual.abs_diff_eq(*expected, 2e-6),
                    "Visible normal flipped on {id}: {expected:?} -> {actual:?}"
                );
            }
        }
        scene.export_stl(&after_path, 1., None).unwrap();
        let signed_volume = |path: &std::path::Path| {
            let mut imported = Scene::empty();
            let id = imported.import_stl(path, 1.).unwrap();
            let mesh = imported.object_mesh(id).unwrap();
            let origin = mesh.positions[0].as_dvec3();
            mesh.triangles()
                .iter()
                .map(|&[a, b, c]| {
                    let [a, b, c] =
                        [a, b, c].map(|i| mesh.positions[i as usize].as_dvec3() - origin);
                    a.dot(b.cross(c)) / 6.
                })
                .sum::<f64>()
        };
        let before = signed_volume(&before_path);
        let after = signed_volume(&after_path);
        assert!(
            before > 0. && after > 0.,
            "Applying a reflection inverted exported solids"
        );
        assert!((before - after).abs() < before.abs() * 1e-5);
    }
}

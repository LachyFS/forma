use forma_core::*;
use std::{collections::BTreeMap, fs};

fn empty_scene() -> Scene {
    Scene::empty()
}

fn volume(mesh: &Mesh) -> f32 {
    mesh.triangles()
        .into_iter()
        .map(|[a, b, c]| {
            mesh.positions[a as usize]
                .dot(mesh.positions[b as usize].cross(mesh.positions[c as usize]))
                / 6.0
        })
        .sum()
}

fn assert_closed(mesh: &Mesh) {
    let mut directions: BTreeMap<[u32; 2], (usize, i32)> = BTreeMap::new();
    for face in &mesh.faces {
        for (&a, &b) in face
            .iter()
            .zip(face.iter().cycle().skip(1))
            .take(face.len())
        {
            let key = [a.min(b), a.max(b)];
            let entry = directions.entry(key).or_default();
            entry.0 += 1;
            entry.1 += if a < b { 1 } else { -1 };
        }
    }
    assert!(
        directions
            .values()
            .all(|&(count, balance)| count == 2 && balance == 0),
        "Every closed-mesh edge needs two oppositely wound faces"
    );
    assert!(volume(mesh) > 0.0, "Winding must be outward");
}

#[test]
fn primitives_are_valid_closed_and_outward() {
    for primitive in [
        Primitive::Cube,
        Primitive::Sphere,
        Primitive::Cylinder,
        Primitive::Torus,
    ] {
        let mesh = Mesh::primitive(primitive);
        mesh.validate().unwrap();
        assert_closed(&mesh);
        for [a, b, c] in mesh.triangles() {
            let [a, b, c] = [a, b, c].map(|i| mesh.positions[i as usize]);
            let normal = (b - a).cross(c - a);
            assert!(normal.length_squared() > 1e-10);
            let center = (a + b + c) / 3.0;
            let expected = if primitive == Primitive::Torus {
                center - Vec3::new(center.x, 0.0, center.z).normalize() * 0.85
            } else {
                center
            };
            assert!(normal.dot(expected) > 0.0, "{primitive:?} inward triangle");
        }
    }
    let plane = Mesh::primitive(Primitive::Plane);
    plane.validate().unwrap();
    assert_eq!(plane.face_normal(0), Vec3::Y);
    assert_eq!(plane.edges().len(), 4);
}

#[test]
fn cube_has_polygon_edges_without_triangle_diagonals() {
    let mesh = Mesh::primitive(Primitive::Cube);
    assert_eq!(mesh.positions.len(), 8);
    assert_eq!(mesh.faces.len(), 6);
    assert_eq!(mesh.triangles().len(), 12);
    assert_eq!(mesh.edges().len(), 12);
    assert!((volume(&mesh) - 8.0).abs() < 1e-5);
}

#[test]
fn concave_polygon_triangulation_preserves_area_and_winding() {
    let mut mesh = Mesh {
        positions: vec![
            Vec3::new(0., 0., 0.),
            Vec3::new(3., 0., 0.),
            Vec3::new(3., 3., 0.),
            Vec3::new(1.5, 1., 0.),
            Vec3::new(0., 3., 0.),
        ],
        faces: vec![vec![0, 1, 2, 3, 4]],
    };
    for sign in [1.0, -1.0] {
        mesh.validate().unwrap();
        let triangles = mesh.triangles();
        assert_eq!(triangles.len(), 3);
        let area: f32 = triangles
            .iter()
            .map(|&[a, b, c]| {
                let cross = (mesh.positions[b as usize] - mesh.positions[a as usize])
                    .cross(mesh.positions[c as usize] - mesh.positions[a as usize]);
                assert!(cross.z * sign > 0.0);
                cross.z * 0.5
            })
            .sum();
        assert!((area - sign * 6.0).abs() < 1e-5);
        mesh.faces[0].reverse();
    }
}

#[test]
fn subdividing_cube_is_catmull_clark_and_keeps_closed_topology() {
    let mut mesh = Mesh::primitive(Primitive::Cube);
    mesh.subdivide();
    mesh.validate().unwrap();
    assert_eq!(mesh.positions.len(), 26);
    assert_eq!(mesh.faces.len(), 24);
    assert_eq!(mesh.edges().len(), 48);
    assert!((mesh.positions[0] - Vec3::splat(-5.0 / 9.0)).length() < 1e-5);
    assert!(mesh.faces.iter().all(|f| f.len() == 4));
    assert_closed(&mesh);
    mesh.subdivide();
    mesh.validate().unwrap();
    assert_closed(&mesh);
}

#[test]
fn subdivision_respects_open_boundary_rule() {
    let mut plane = Mesh::primitive(Primitive::Plane);
    plane.subdivide();
    plane.validate().unwrap();
    assert_eq!(plane.faces.len(), 4);
    assert_eq!(plane.positions.len(), 9);
    assert_eq!(plane.positions[0], Vec3::new(-0.75, 0.0, -0.75));
    assert!(
        plane
            .faces
            .iter()
            .enumerate()
            .all(|(i, _)| plane.face_normal(i).dot(Vec3::Y) > 0.99)
    );
}

#[test]
fn extrusion_replaces_cap_and_keeps_solid_closed() {
    let mut mesh = Mesh::primitive(Primitive::Cube);
    mesh.extrude_face(3, 1.5).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.positions.len(), 12);
    assert_eq!(mesh.faces.len(), 10);
    assert_closed(&mesh);
    assert!((volume(&mesh) - 14.0).abs() < 1e-5);
    assert!(
        mesh.faces[3]
            .iter()
            .all(|&i| (mesh.positions[i as usize].y - 2.5).abs() < 1e-6)
    );
    let saved = mesh.clone();
    assert!(mesh.extrude_face(999, 1.0).is_err());
    assert!(mesh.extrude_face(3, f32::NAN).is_err());
    assert!(mesh.extrude_face(3, 0.0).is_err());
    assert!(mesh.extrude_face(3, f32::MAX).is_err());
    assert_eq!(mesh, saved);
}

#[test]
fn mesh_validation_rejects_missing_repeated_and_degenerate_vertices() {
    let mut mesh = Mesh::primitive(Primitive::Cube);
    mesh.faces[0][0] = 100;
    assert!(mesh.validate().is_err());
    mesh.faces[0] = vec![0, 0, 1];
    assert!(mesh.validate().is_err());
    mesh.faces[0] = vec![0, 1, 2];
    mesh.positions[2] = mesh.positions[1];
    assert!(mesh.validate().is_err());
}

#[test]
fn camera_ray_agrees_with_perspective_and_orthographic_projection() {
    for orthographic in [false, true] {
        let camera = Camera {
            orthographic,
            yaw: 0.8,
            pitch: 0.4,
            ..Camera::default()
        };
        for uv in [Vec2::splat(0.5), Vec2::new(0.1, 0.3), Vec2::new(0.85, 0.95)] {
            let ray = camera.ray(uv, 1.7);
            assert!((ray.direction.length() - 1.0).abs() < 1e-6);
            let projected = camera.projection_matrix(1.7)
                * camera.view_matrix()
                * (ray.origin + ray.direction * 20.0).extend(1.0);
            let ndc = projected.truncate() / projected.w;
            assert!((ndc.x - (uv.x * 2.0 - 1.0)).abs() < 1e-5);
            assert!((ndc.y - (1.0 - uv.y * 2.0)).abs() < 1e-5);
            assert!((0.0..1.0).contains(&ndc.z));
        }
    }
}

#[test]
fn camera_navigation_stays_finite_and_frame_contains_subject() {
    let mut camera = Camera::default();
    camera.orbit(Vec2::new(1e6, -1e6));
    camera.zoom(1e6);
    camera.pan(Vec2::new(50.0, 50.0));
    assert!(camera.view_matrix().is_finite());
    assert!(camera.distance >= 0.02 && camera.pitch.abs() < 1.56);
    camera.frame(Vec3::new(7., 2., -1.), 3.0);
    assert_eq!(camera.target, Vec3::new(7., 2., -1.));
    assert!(camera.distance * (camera.fov_y * 0.5).sin() > 3.0);
    let saved = camera;
    camera.orbit(Vec2::splat(f32::NAN));
    camera.pan(Vec2::splat(f32::INFINITY));
    camera.zoom(f32::NAN);
    assert_eq!(saved, camera);
}

#[test]
fn picking_chooses_nearest_visible_polygon_with_world_distance() {
    let mut scene = empty_scene();
    let far = scene.add(Primitive::Cube);
    scene.object_mut(far).unwrap().transform.translation.z = -5.0;
    let near = scene.add(Primitive::Cube);
    scene.object_mut(near).unwrap().transform.scale = Vec3::new(2., 0.5, 2.);
    let ray = Ray {
        origin: Vec3::new(0., 0., 10.),
        direction: Vec3::new(0., 0., -10.),
    };
    let hit = scene.pick(ray).unwrap();
    assert_eq!(hit.object_id, near);
    assert_eq!(hit.face, 1);
    assert!((hit.distance - 8.0).abs() < 1e-5);
    assert!((hit.position - Vec3::new(0., 0., 2.)).length() < 1e-5);
    scene.object_mut(near).unwrap().visible = false;
    assert_eq!(scene.pick(ray).unwrap().object_id, far);
    assert!(
        scene
            .pick(Ray {
                origin: ray.origin,
                direction: Vec3::ZERO
            })
            .is_none()
    );
}

#[test]
fn bounds_include_rotated_scaled_geometry() {
    let mut scene = empty_scene();
    let id = scene.add(Primitive::Cube);
    let object = scene.object_mut(id).unwrap();
    object.transform.translation = Vec3::new(3., 4., 5.);
    object.transform.scale = Vec3::new(2., 3., 4.);
    object.transform.rotation = Vec3::new(0.3, 0.7, 1.0);
    let (center, radius) = scene.bounds(id).unwrap();
    assert!((center - Vec3::new(3., 4., 5.)).length() < 1e-5);
    assert!((radius - 29.0_f32.sqrt()).abs() < 1e-5);
}

#[test]
fn history_supports_undo_redo_branching_and_bounded_snapshots() {
    let mut scene = empty_scene();
    let mut history = History::default();
    assert!(!history.undo(&mut scene));
    history.checkpoint(&scene);
    let id = scene.add(Primitive::Cube);
    assert!(history.undo(&mut scene));
    assert!(scene.objects.is_empty());
    assert!(history.redo(&mut scene));
    assert!(scene.object(id).is_some());
    history.undo(&mut scene);
    history.checkpoint(&scene);
    scene.add(Primitive::Plane);
    assert!(!history.can_redo());
    for _ in 0..100 {
        history.checkpoint(&scene);
    }
    let mut count = 0;
    while history.undo(&mut scene) {
        count += 1;
    }
    assert_eq!(count, 64);
}

#[test]
fn scene_round_trip_is_versioned_atomic_and_validated() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Studio.forma");
    let scene = Scene::default();
    scene.validate().unwrap();
    scene.save(&path).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert_eq!(Scene::load(&path).unwrap(), scene);
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["version"], 3);
    assert_eq!(json["format"], "forma");
    let mut invalid = scene.clone();
    let id = invalid.objects[0].id;
    invalid.object_material_mut(id).unwrap().roughness = f32::NAN;
    assert!(invalid.save(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    let mut json = json;
    json["version"] = 99.into();
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(
        Scene::load(&path)
            .unwrap_err()
            .to_string()
            .contains("version")
    );
}

#[test]
fn scene_validation_rejects_duplicate_ids_invalid_camera_and_singular_transform() {
    let mut scene = Scene::default();
    let original_id = scene.objects[1].id;
    scene.objects[1].id = scene.objects[0].id;
    assert!(scene.validate().is_err());
    scene.objects[1].id = original_id;
    scene.camera.pitch = std::f32::consts::PI;
    assert!(scene.validate().is_err());
    scene.camera = Camera::default();
    scene.objects[0].transform.scale.x = 0.0;
    assert!(scene.validate().is_err());
}

#[test]
fn obj_import_accepts_uv_normals_negative_indices_comments_and_continuations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("quad.obj");
    fs::write(&path, "# a quad\nv 0 0 0\nv 2 0 0\nv 2 2 0\nv 0 2 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\ng Surface\ns 1\nf -4/1/1 -3/2/1 \\\n -2/3/1 -1/4/1 # comment\n").unwrap();
    let mut scene = empty_scene();
    let id = scene.import_obj(&path).unwrap();
    let mesh = scene.object_mesh(id).unwrap();
    assert_eq!(mesh.faces, vec![vec![0, 1, 2, 3]]);
    assert_eq!(mesh.face_normal(0), Vec3::Z);
    scene.validate().unwrap();
}

#[test]
fn invalid_obj_import_never_changes_scene() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("broken.obj");
    let mut scene = empty_scene();
    let before = scene.clone();
    for source in [
        "v NaN 0 0\nf 1 1 1",
        "v 0 0 0\nf 1 2 3",
        "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2",
        "v 0 0 0\nf 1//2 1//2 1//2",
        "nonsense 7",
    ] {
        fs::write(&path, source).unwrap();
        assert!(scene.import_obj(&path).is_err());
        assert_eq!(scene, before);
    }
}

#[test]
fn obj_export_bakes_transform_and_preserves_mirrored_winding() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cube.obj");
    let mut scene = empty_scene();
    let id = scene.add(Primitive::Cube);
    scene.object_mut(id).unwrap().transform.scale = Vec3::new(-2.0, 1.0, 1.0);
    scene.object_mut(id).unwrap().transform.translation.x = 4.0;
    scene.export_obj(&path).unwrap();
    let mut imported = empty_scene();
    let id = imported.import_obj(&path).unwrap();
    let mesh = imported.object_mesh(id).unwrap();
    assert_closed(mesh);
    assert!((volume(mesh) - 16.0).abs() < 1e-5);
    assert!((imported.bounds(id).unwrap().0.x - 4.0).abs() < 1e-5);
}

#[test]
fn duplicate_and_remove_keep_unique_names_and_monotonic_ids() {
    let mut scene = empty_scene();
    let a = scene.add(Primitive::Cube);
    let b = scene.duplicate(a).unwrap();
    assert_ne!(a, b);
    assert_eq!(scene.object(b).unwrap().name, "Cube.001");
    scene.remove(a);
    let c = scene.add(Primitive::Sphere);
    assert!(c > b);
    scene.validate().unwrap();
}

#[test]
fn crossing_polygon_with_nonzero_signed_area_is_rejected() {
    let mesh = Mesh {
        positions: vec![
            Vec3::new(0., 0., 0.),
            Vec3::new(3., 3., 0.),
            Vec3::new(0., 2., 0.),
            Vec3::new(2., 0., 0.),
        ],
        faces: vec![vec![0, 1, 2, 3]],
    };
    assert!(mesh.validate().is_err());
}

#[test]
fn collinear_polygon_corner_can_be_triangulated_without_zero_area_triangles() {
    let mesh = Mesh {
        positions: vec![
            Vec3::new(0., 0., 0.),
            Vec3::new(1., 0., 0.),
            Vec3::new(2., 0., 0.),
            Vec3::new(2., 2., 0.),
            Vec3::new(0., 2., 0.),
        ],
        faces: vec![vec![0, 1, 2, 3, 4]],
    };
    mesh.validate().unwrap();
    assert_eq!(mesh.triangles().len(), 3);
    for [a, b, c] in mesh.triangles() {
        assert!(
            (mesh.positions[b as usize] - mesh.positions[a as usize])
                .cross(mesh.positions[c as usize] - mesh.positions[a as usize])
                .z
                > 0.0
        );
    }
}

#[test]
fn tiny_and_huge_scaled_objects_remain_pickable() {
    for scale in [1.0e-5, 1.0e5] {
        let mut scene = empty_scene();
        let id = scene.add(Primitive::Cube);
        scene.object_mut(id).unwrap().transform.scale = Vec3::splat(scale);
        scene.validate().unwrap();
        let hit = scene
            .pick(Ray {
                origin: Vec3::Z * scale * 5.0,
                direction: -Vec3::Z,
            })
            .unwrap();
        assert_eq!(hit.object_id, id);
        assert!((hit.distance / scale - 4.0).abs() < 1.0e-4);
    }
}

#[test]
fn failed_checked_subdivision_leaves_source_unchanged() {
    let mut mesh = Mesh::primitive(Primitive::Cube);
    mesh.faces[0][0] = 1000;
    let before = mesh.clone();
    assert!(mesh.subdivide_checked().is_err());
    assert_eq!(mesh, before);
}

#[test]
fn exact_top_and_bottom_camera_views_have_stable_basis_and_can_be_saved() {
    for pitch in [std::f32::consts::FRAC_PI_2, -std::f32::consts::FRAC_PI_2] {
        let mut scene = empty_scene();
        scene.camera.pitch = pitch;
        scene.camera.yaw = 0.0;
        scene.camera.orthographic = true;
        scene.validate().unwrap();
        let camera = &mut scene.camera;
        assert!(camera.view_matrix().is_finite());
        let center = camera.ray(Vec2::splat(0.5), 1.5);
        assert!(center.direction.dot(-Vec3::Y * pitch.signum()) > 0.9999);
        let right = camera.ray(Vec2::new(1.0, 0.5), 1.5);
        assert!((right.origin - center.origin).normalize().dot(Vec3::X) > 0.9999);
        camera.pan(Vec2::new(50., 30.));
        assert!(camera.target.is_finite());
    }
}

#[test]
fn render_preferences_round_trip_and_follow_undo_redo() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Render.forma");
    let mut scene = empty_scene();
    let mut history = History::default();
    history.checkpoint(&scene);
    let preferences = RenderPreferences {
        exposure: 1.25,
        max_samples: 512,
        max_bounces: 16,
        ..Default::default()
    };
    scene.render = preferences;
    scene.save(&path).unwrap();
    assert_eq!(Scene::load(&path).unwrap().render, preferences);
    assert!(history.undo(&mut scene));
    assert_eq!(scene.render, RenderPreferences::default());
    assert!(history.redo(&mut scene));
    assert_eq!(scene.render, preferences);
}

#[test]
fn version_one_scene_without_render_preferences_loads_defaults() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Legacy.forma");
    let source = Scene::default();
    let objects: Vec<_> = source
        .objects
        .iter()
        .map(|object| {
            serde_json::json!({
                "id": object.id,
                "name": object.name,
                "mesh": source.object_mesh(object.id).unwrap(),
                "transform": object.transform,
                "material": source.object_material(object.id).unwrap(),
                "visible": object.visible,
            })
        })
        .collect();
    let document = serde_json::json!({
        "format": "forma",
        "version": 1,
        "scene": {
            "objects": objects,
            "camera": source.camera,
            "world": source.world,
            "next_id": source.next_id,
        }
    });
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let migrated = Scene::load(&path).unwrap();
    assert_eq!(migrated.render, RenderPreferences::default());
    assert_eq!(migrated.objects.len(), source.objects.len());
    assert_eq!(
        migrated.object_mesh(migrated.objects[0].id).unwrap(),
        source.object_mesh(source.objects[0].id).unwrap()
    );
}

#[test]
fn linked_instances_share_data_and_can_be_made_single_user() {
    let mut scene = empty_scene();
    let original = scene.add(Primitive::Cube);
    let linked = scene.duplicate_linked(original).unwrap();
    assert_eq!(
        scene.object(original).unwrap().mesh_id(),
        scene.object(linked).unwrap().mesh_id()
    );
    assert_eq!(
        scene.mesh_users(scene.object(original).unwrap().mesh_id().unwrap()),
        2
    );

    scene.object_mesh_mut(original).unwrap().positions[0].x = -2.0;
    assert_eq!(scene.object_mesh(linked).unwrap().positions[0].x, -2.0);

    let private_mesh = scene.make_mesh_single_user(linked).unwrap();
    assert_ne!(
        scene.object(original).unwrap().mesh_id(),
        Some(private_mesh)
    );
    scene.object_mesh_mut(linked).unwrap().positions[0].x = -3.0;
    assert_eq!(scene.object_mesh(original).unwrap().positions[0].x, -2.0);

    let material = scene.object_material(original).unwrap().clone();
    let shared_material = scene.create_material_data("Shared", material).unwrap();
    for id in [original, linked] {
        scene.assign_material(id, 0, shared_material).unwrap();
    }
    assert_eq!(scene.material_users(shared_material), 2);
    let private_material = scene.make_material_single_user(linked, 0).unwrap();
    assert_ne!(private_material, shared_material);
    scene.object_material_mut(linked).unwrap().metallic = 1.0;
    assert_ne!(
        scene.object_material(original).unwrap().metallic,
        scene.object_material(linked).unwrap().metallic
    );
    scene.validate().unwrap();
}

#[test]
fn hierarchy_composes_world_transforms_and_rejects_cycles() {
    let mut scene = empty_scene();
    let parent = scene.add_empty("Parent").unwrap();
    let child = scene.add(Primitive::Cube);
    scene.object_mut(parent).unwrap().transform.translation.x = 5.0;
    scene.object_mut(child).unwrap().transform.translation.x = 2.0;
    scene.set_parent(child, Some(parent), false).unwrap();
    assert_eq!(
        scene
            .world_transform(child)
            .unwrap()
            .transform_point3(Vec3::ZERO),
        Vec3::new(7.0, 0.0, 0.0)
    );
    assert!((scene.bounds(child).unwrap().0.x - 7.0).abs() < 1.0e-5);
    assert!(scene.set_parent(parent, Some(child), false).is_err());

    let world = scene.world_transform(child).unwrap();
    scene.set_parent(child, None, true).unwrap();
    assert!(
        (scene.world_transform(child).unwrap() - world)
            .to_cols_array()
            .into_iter()
            .all(|value| value.abs() < 1.0e-5)
    );
    scene.object_mut(parent).unwrap().transform.rotation.y = 0.7;
    scene.object_mut(parent).unwrap().transform.scale = Vec3::new(2.0, 0.5, 3.0);
    let world = scene.world_transform(child).unwrap();
    scene.set_parent(child, Some(parent), true).unwrap();
    assert!(
        (scene.world_transform(child).unwrap() - world)
            .to_cols_array()
            .into_iter()
            .all(|value| value.abs() < 1.0e-4)
    );
    scene.set_parent(child, None, true).unwrap();
    assert!(
        (scene.world_transform(child).unwrap() - world)
            .to_cols_array()
            .into_iter()
            .all(|value| value.abs() < 1.0e-4)
    );
    scene.validate().unwrap();
}

#[test]
fn collections_control_visibility_and_objects_support_multiple_membership() {
    let mut scene = empty_scene();
    let object = scene.add(Primitive::Cube);
    let collection = scene.add_collection("Furniture", None).unwrap();
    let child_collection = scene.add_collection("Chairs", Some(collection)).unwrap();
    assert!(
        scene
            .set_collection_parent(collection, child_collection)
            .is_err()
    );
    scene.link_object(object, collection).unwrap();
    scene.unlink_object(object, scene.root_collection).unwrap();
    assert_eq!(scene.object(object).unwrap().collections, vec![collection]);
    assert!(scene.is_effectively_visible(object));
    scene
        .collections
        .iter_mut()
        .find(|entry| entry.id == collection)
        .unwrap()
        .visible = false;
    assert!(!scene.is_effectively_visible(object));
    assert!(
        scene
            .pick(Ray {
                origin: Vec3::new(0.0, 0.0, 5.0),
                direction: Vec3::NEG_Z,
            })
            .is_none()
    );
    scene.remove_collection(collection).unwrap();
    assert_eq!(
        scene.collection(child_collection).unwrap().parent,
        Some(scene.root_collection)
    );
    assert!(
        scene
            .object(object)
            .unwrap()
            .collections
            .contains(&scene.root_collection)
    );
    scene.validate().unwrap();
}

#[test]
fn independent_data_blocks_compose_with_new_object_types() {
    let mut scene = empty_scene();
    let mesh = scene
        .create_mesh_data("Reusable cube", Mesh::primitive(Primitive::Cube))
        .unwrap();
    let material = scene
        .create_material_data("Reusable material", Material::default())
        .unwrap();
    let first = scene
        .instantiate_mesh("First", mesh, vec![material])
        .unwrap();
    let second = scene
        .instantiate_mesh("Second", mesh, vec![material])
        .unwrap();
    let empty = scene.add_empty("Control").unwrap();
    let light = scene.add_light("Key", Light::default()).unwrap();
    let camera = scene.add_camera("Camera", CameraData::default()).unwrap();
    let custom = scene
        .add_object(
            "Plugin object",
            ObjectData::Custom {
                kind: "com.example.volume".into(),
                properties: [("density".into(), serde_json::json!(0.5))]
                    .into_iter()
                    .collect(),
            },
        )
        .unwrap();

    assert_eq!(scene.mesh_users(mesh), 2);
    assert_eq!(scene.material_users(material), 2);
    assert_eq!(scene.mesh_instances().count(), 2);
    assert_eq!(scene.object(first).unwrap().kind(), "mesh");
    assert_eq!(scene.object(second).unwrap().kind(), "mesh");
    assert_eq!(scene.object(empty).unwrap().kind(), "empty");
    assert_eq!(scene.object(light).unwrap().kind(), "light");
    assert_eq!(scene.object(camera).unwrap().kind(), "camera");
    assert_eq!(scene.object(custom).unwrap().kind(), "com.example.volume");
    scene.validate().unwrap();
}

#[test]
fn render_preference_validation_rejects_nonfinite_and_out_of_range_values() {
    let mut scene = empty_scene();
    for exposure in [f32::NAN, f32::INFINITY, -10.01, 10.01] {
        scene.render = RenderPreferences {
            exposure,
            ..RenderPreferences::default()
        };
        assert!(scene.validate().is_err());
    }
    for max_samples in [0, 4097] {
        scene.render = RenderPreferences {
            max_samples,
            ..RenderPreferences::default()
        };
        assert!(scene.validate().is_err());
    }
    for max_bounces in [0, 33] {
        scene.render = RenderPreferences {
            max_bounces,
            ..RenderPreferences::default()
        };
        assert!(scene.validate().is_err());
    }
    for (exposure, max_samples, max_bounces) in [(-10.0, 1, 1), (10.0, 4096, 32)] {
        scene.render = RenderPreferences {
            exposure,
            max_samples,
            max_bounces,
            ..Default::default()
        };
        scene.validate().unwrap();
    }
}

#[test]
fn denoising_preferences_migrate_round_trip_and_validate() {
    use forma_core::{DenoiseQuality, DenoiseSettings};
    let mut scene = empty_scene();
    scene.render.denoise = DenoiseSettings {
        viewport: false,
        render: true,
        start_sample: 32,
        quality: DenoiseQuality::High,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("denoise.forma");
    scene.save(&path).unwrap();
    assert_eq!(
        Scene::load(&path).unwrap().render.denoise,
        scene.render.denoise
    );
    let mut document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    document["scene"]["render"]
        .as_object_mut()
        .unwrap()
        .remove("denoise");
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    assert_eq!(
        Scene::load(&path).unwrap().render.denoise,
        DenoiseSettings::default()
    );
    for start_sample in [0, 4097] {
        scene.render.denoise.start_sample = start_sample;
        assert!(scene.validate().is_err());
    }
}

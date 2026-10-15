use forma_core::{Material, Mesh, MeshData, Primitive, Scene, Vec3};
use std::{collections::BTreeSet, fs};

const ASCII_TRIANGLE: &str = "solid test part\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 10 0 0\nvertex 0 20 0\nendloop\nendfacet\nendsolid test part\n";

fn binary(triangles: &[[Vec3; 3]], solid_header: bool) -> Vec<u8> {
    let mut bytes = vec![0; 84];
    let header: &[u8] = if solid_header {
        b"solid this is still binary STL"
    } else {
        b"test binary STL"
    };
    bytes[..header.len()].copy_from_slice(header);
    bytes[80..84].copy_from_slice(&(triangles.len() as u32).to_le_bytes());
    for triangle in triangles {
        // Zero normals are common in STL; editable geometry supplies normals.
        for p in std::iter::once(&Vec3::ZERO).chain(triangle) {
            for coordinate in p.to_array() {
                bytes.extend_from_slice(&coordinate.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&0x8001u16.to_le_bytes());
    }
    bytes
}

fn volume(mesh: &Mesh) -> f64 {
    let origin = mesh.positions[0].as_dvec3();
    mesh.triangles()
        .into_iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| mesh.positions[i as usize].as_dvec3() - origin);
            a.dot(b.cross(c)) / 6.
        })
        .sum()
}

fn binary_count(path: &std::path::Path) -> u32 {
    let bytes = fs::read(path).unwrap();
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap());
    assert_eq!(bytes.len(), 84 + count as usize * 50);
    count
}

#[test]
fn binary_import_detects_solid_headers_and_deduplicates_exact_signed_zero_vertices() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared square.stl");
    let triangles = [
        [Vec3::ZERO, Vec3::X, Vec3::Y],
        [
            Vec3::new(-0., 1., -0.),
            Vec3::new(1., -0., 0.),
            Vec3::new(1., 1., 0.),
        ],
        [Vec3::new(0.00001, 0., 0.), Vec3::X, Vec3::Y],
    ];
    for solid_header in [false, true] {
        fs::write(&path, binary(&triangles, solid_header)).unwrap();
        let mut scene = Scene::empty();
        let id = scene.import_stl(&path, 1.).unwrap();
        let mesh = scene.object_mesh(id).unwrap();
        assert_eq!(
            mesh.positions.len(),
            5,
            "Only exact coordinates should weld"
        );
        assert_eq!(
            mesh.faces,
            vec![vec![0, 1, 2], vec![2, 1, 3], vec![4, 1, 2]]
        );
        assert!(
            mesh.positions
                .iter()
                .flat_map(|p| p.to_array())
                .filter(|&v| v == 0.)
                .all(|v| v.to_bits() == 0)
        );
        assert_eq!(scene.object(id).unwrap().name, "shared square");
        scene.validate().unwrap();
    }
}

#[test]
fn ascii_import_accepts_multiple_solids_whitespace_and_explicit_units() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assembly.stl");
    let source = format!(
        "\u{feff} \r\n{ASCII_TRIANGLE}\nSOLID second\r\n FACET normal 0 0 -5 outer loop\nvertex -0 20 0 vertex 1e1 0 0\n vertex 1e1 2e1 -0\nendloop endfacet\nENDSOLID second\n"
    );
    fs::write(&path, source).unwrap();
    for units in [0.001_f32, 0.01, 1., 0.0254] {
        let mut scene = Scene::empty();
        let id = scene.import_stl(&path, units).unwrap();
        let mesh = scene.object_mesh(id).unwrap();
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.faces.len(), 2);
        assert!((mesh.positions[1].x - 10. * units).abs() < 1e-6);
        assert!((mesh.positions[2].y - 20. * units).abs() < 1e-6);
        assert_eq!(mesh.face_normal(0), Vec3::Z);
        assert_eq!(
            mesh.face_normal(1),
            Vec3::Z,
            "Stored facet normal must not override winding"
        );
        scene.validate().unwrap();
    }
}

#[test]
fn malformed_binary_import_is_atomic_including_truncation_attributes_and_limits() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("broken.stl");
    let valid = binary(&[[Vec3::ZERO, Vec3::X, Vec3::Y]], false);
    let mut variants = vec![vec![], vec![0; 83], valid[..133].to_vec()];
    let mut trailing = valid.clone();
    trailing.push(0);
    variants.push(trailing);
    for (offset, value) in [
        (80, 2u32),
        (80, 2_000_001u32),
        (80, u32::MAX),
        (84, f32::NAN.to_bits()),
        (96, f32::INFINITY.to_bits()),
        (96, (1.0e20_f32).to_bits()),
    ] {
        let mut bad = valid.clone();
        bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        variants.push(bad);
    }
    variants.push(binary(&[[Vec3::ZERO, Vec3::X, Vec3::X]], true));
    variants.push(binary(&[], false));
    let mut scene = Scene::empty();
    scene.add(Primitive::Cube);
    let original = scene.clone();
    for bytes in variants {
        fs::write(&path, bytes).unwrap();
        assert!(scene.import_stl(&path, 1.).is_err());
        assert_eq!(scene, original);
    }
    let file = fs::File::create(&path).unwrap();
    file.set_len(256 * 1024 * 1024 + 1).unwrap();
    assert!(
        scene
            .import_stl(&path, 1.)
            .unwrap_err()
            .to_string()
            .contains("256 MiB")
    );
    assert_eq!(scene, original);
}

#[test]
fn malformed_ascii_import_never_publishes_partial_solids() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("broken.stl");
    let variants = [
        ASCII_TRIANGLE.replace("vertex 10 0 0", "vertex NaN 0 0"),
        ASCII_TRIANGLE.replace("normal 0 0 1", "normal 0 inf 1"),
        ASCII_TRIANGLE.replace("vertex 0 20 0", "vertex 20 0 0"),
        ASCII_TRIANGLE.replace("endfacet", ""),
        ASCII_TRIANGLE.replace("endloop", "vertex 0 0 20\nendloop"),
        ASCII_TRIANGLE.replace("outer loop", "loop"),
        ASCII_TRIANGLE.replace("endsolid test part", ""),
        format!("{ASCII_TRIANGLE}unexpected trailing data"),
        format!("{ASCII_TRIANGLE}solid second\nfacet normal 0 0 1\n"),
        "solid empty\nendsolid empty\n".to_owned(),
    ];
    let mut scene = Scene::empty();
    scene.add(Primitive::Cube);
    let original = scene.clone();
    for source in variants {
        fs::write(&path, source).unwrap();
        assert!(scene.import_stl(&path, 1.).is_err());
        assert_eq!(scene, original);
    }
}

#[test]
fn invalid_unit_scales_reject_import_and_export_without_touching_either_target() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.stl");
    let output = directory.path().join("output.stl");
    fs::write(&input, ASCII_TRIANGLE).unwrap();
    fs::write(&output, b"existing document").unwrap();
    let mut scene = Scene::empty();
    scene.add(Primitive::Cube);
    let original = scene.clone();
    for scale in [0., -1., f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(scene.import_stl(&input, scale).is_err());
        assert_eq!(scene, original);
        assert!(scene.export_stl(&output, scale, None).is_err());
        assert_eq!(fs::read(&output).unwrap(), b"existing document");
    }
    assert!(scene.import_stl(&input, f32::from_bits(1)).is_err());
    assert_eq!(scene, original);
    assert!(scene.export_stl(&output, f32::from_bits(1), None).is_err());
    assert_eq!(fs::read(&output).unwrap(), b"existing document");
}

#[test]
fn import_capacity_failures_preserve_ids_and_data_blocks() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("triangle.stl");
    fs::write(&path, ASCII_TRIANGLE).unwrap();
    let mut exhausted = Scene::empty();
    exhausted.next_id = u64::MAX - 3;
    let original = exhausted.clone();
    assert!(exhausted.import_stl(&path, 1.).is_err());
    assert_eq!(exhausted, original);

    let mut meshes = Scene::empty();
    meshes.meshes = (2..10_002)
        .map(|id| MeshData {
            id,
            name: format!("Mesh {id}"),
            mesh: Mesh::primitive(Primitive::Plane),
        })
        .collect();
    meshes.next_id = 10_002;
    meshes.validate().unwrap();
    let original = meshes.clone();
    assert!(
        meshes
            .import_stl(&path, 1.)
            .unwrap_err()
            .to_string()
            .contains("mesh limit")
    );
    assert_eq!(meshes, original);

    let mut objects = Scene::empty();
    objects.add(Primitive::Plane);
    let prototype = objects.objects[0].clone();
    objects.objects = (4..10_004)
        .map(|id| {
            let mut object = prototype.clone();
            object.id = id;
            object.name = format!("Plane {id}");
            object
        })
        .collect();
    objects.next_id = 10_004;
    objects.validate().unwrap();
    let original = objects.clone();
    assert!(
        objects
            .import_stl(&path, 1.)
            .unwrap_err()
            .to_string()
            .contains("object limit")
    );
    assert_eq!(objects, original);
}

#[test]
fn import_total_geometry_limit_is_checked_before_scene_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("triangle.stl");
    fs::write(&path, ASCII_TRIANGLE).unwrap();
    let mut scene = Scene::empty();
    for (index, count) in [2_000_000, 2_000_000, 1_000_000].into_iter().enumerate() {
        let mut mesh = Mesh::primitive(Primitive::Cube);
        mesh.positions.resize(count, Vec3::ZERO);
        scene.meshes.push(MeshData {
            id: index as u64 + 2,
            name: format!("Mesh {index}"),
            mesh,
        });
    }
    scene.next_id = 5;
    scene.validate().unwrap();
    assert!(
        scene
            .import_stl(&path, 1.)
            .unwrap_err()
            .to_string()
            .contains("five million")
    );
    assert_eq!(scene.next_id, 5);
    assert!(scene.objects.is_empty() && scene.materials.is_empty());
    assert_eq!(scene.meshes.len(), 3);
    assert_eq!(
        scene
            .meshes
            .iter()
            .map(|data| data.mesh.positions.len())
            .sum::<usize>(),
        5_000_000
    );
}

#[test]
fn binary_export_bakes_hierarchy_reflection_units_and_geometric_normals() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("part.stl");
    let mut scene = Scene::empty();
    let parent = scene.add_empty("Assembly").unwrap();
    let child = scene.add(Primitive::Cube);
    let transform = &mut scene.object_mut(parent).unwrap().transform;
    transform.translation = Vec3::new(5., 2., 1.);
    transform.rotation.y = 0.2;
    transform.scale = Vec3::new(2., 1.5, 0.75);
    scene.object_mut(child).unwrap().transform.translation.x = 1.;
    scene.object_mut(child).unwrap().transform.scale.x = -1.;
    scene.set_parent(child, Some(parent), false).unwrap();
    let world = scene.world_transform(child).unwrap();
    let expected: Vec<_> = scene
        .object_mesh(child)
        .unwrap()
        .positions
        .iter()
        .map(|&p| world.transform_point3(p))
        .collect();
    scene.export_stl(&path, 0.001, None).unwrap();
    assert_eq!(binary_count(&path), 12);
    let bytes = fs::read(&path).unwrap();
    assert!(!bytes.starts_with(b"solid"));
    for facet in bytes[84..].chunks_exact(50) {
        let values: [f32; 12] = std::array::from_fn(|i| {
            f32::from_le_bytes(facet[i * 4..i * 4 + 4].try_into().unwrap())
        });
        let normal = Vec3::new(values[0], values[1], values[2]);
        let [a, b, c] = std::array::from_fn(|i| {
            Vec3::new(values[3 + i * 3], values[4 + i * 3], values[5 + i * 3])
        });
        assert!(normal.dot((b - a).cross(c - a).normalize()) > 0.99999);
        assert_eq!(&facet[48..50], &[0, 0]);
    }
    let mut imported = Scene::empty();
    let id = imported.import_stl(&path, 0.001).unwrap();
    let mesh = imported.object_mesh(id).unwrap();
    assert_eq!(mesh.positions.len(), 8);
    for point in &mesh.positions {
        assert!(
            expected
                .iter()
                .any(|expected| point.distance(*expected) < 1e-5)
        );
    }
    assert!((volume(mesh) - 8. * f64::from(world.determinant().abs())).abs() < 1e-4);
    imported.validate().unwrap();
}

#[test]
fn export_respects_object_collection_parent_visibility_and_optional_selection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("visible.stl");
    let mut scene = Scene::empty();
    let visible = scene.add(Primitive::Cube);
    let locked = scene.add(Primitive::Cube);
    scene.object_mut(locked).unwrap().selectable = false;
    let hidden = scene.add(Primitive::Cube);
    scene.object_mut(hidden).unwrap().visible = false;
    let collection = scene.add_collection("Hidden collection", None).unwrap();
    scene.collection_mut(collection).unwrap().visible = false;
    let in_collection = scene.add(Primitive::Cube);
    scene.link_object(in_collection, collection).unwrap();
    scene
        .unlink_object(in_collection, scene.root_collection)
        .unwrap();
    let parent = scene.add_empty("Hidden parent").unwrap();
    scene.object_mut(parent).unwrap().visible = false;
    let child = scene.add(Primitive::Cube);
    scene.set_parent(child, Some(parent), false).unwrap();
    scene.export_stl(&path, 1., None).unwrap();
    assert_eq!(binary_count(&path), 24);
    scene
        .export_stl(&path, 1., Some(&BTreeSet::from([visible, hidden, parent])))
        .unwrap();
    assert_eq!(binary_count(&path), 12);
    let saved = fs::read(&path).unwrap();
    for selection in [
        BTreeSet::new(),
        BTreeSet::from([hidden]),
        BTreeSet::from([visible, 999_999]),
    ] {
        assert!(scene.export_stl(&path, 1., Some(&selection)).is_err());
        assert_eq!(fs::read(&path).unwrap(), saved);
    }
}

#[test]
fn export_ear_clips_concave_polygons_without_overlapping_fans() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("concave.stl");
    let mut scene = Scene::empty();
    let mesh = Mesh {
        positions: vec![
            Vec3::ZERO,
            Vec3::new(3., 0., 0.),
            Vec3::new(3., 3., 0.),
            Vec3::new(1.5, 1., 0.),
            Vec3::new(0., 3., 0.),
        ],
        faces: vec![vec![0, 1, 2, 3, 4]],
    };
    let data = scene.create_mesh_data("Concave", mesh).unwrap();
    let material = scene
        .create_material_data("Material", Material::default())
        .unwrap();
    scene
        .instantiate_mesh("Concave", data, vec![material])
        .unwrap();
    scene.export_stl(&path, 1., None).unwrap();
    assert_eq!(binary_count(&path), 3);
    let mut imported = Scene::empty();
    let id = imported.import_stl(&path, 1.).unwrap();
    let mesh = imported.object_mesh(id).unwrap();
    let area: f32 = mesh
        .triangles()
        .into_iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
            let cross = (b - a).cross(c - a);
            assert!(cross.z > 0.);
            cross.length() * 0.5
        })
        .sum();
    assert!((area - 6.).abs() < 1e-5);
}

#[test]
fn failed_export_preserves_existing_file_and_cleans_temporary_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preserved.stl");
    fs::write(&path, b"original content").unwrap();
    assert!(Scene::empty().export_stl(&path, 1., None).is_err());
    let mut scene = Scene::empty();
    let id = scene.add(Primitive::Cube);
    scene.object_mut(id).unwrap().transform.translation = Vec3::splat(1.0e8);
    scene.validate().unwrap();
    assert!(
        scene.export_stl(&path, 1., None).is_err(),
        "Float32 output coordinates collapse this cube"
    );
    assert_eq!(fs::read(&path).unwrap(), b"original content");
    scene.object_mut(id).unwrap().transform.translation = Vec3::ZERO;
    let folder_target = directory.path().join("folder.stl");
    fs::create_dir(&folder_target).unwrap();
    assert!(scene.export_stl(&folder_target, 1., None).is_err());
    assert!(folder_target.is_dir());
    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[test]
fn export_counts_all_linked_instances_before_allocating_output() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("instances.stl");
    fs::write(&path, b"original content").unwrap();
    let mut scene = Scene::empty();
    let first = scene.add(Primitive::Sphere);
    let prototype = scene.object(first).unwrap().clone();
    scene.objects = (first..first + 2200)
        .map(|id| {
            let mut object = prototype.clone();
            object.id = id;
            object.name = format!("Sphere {id}");
            object
        })
        .collect();
    scene.next_id = first + 2200;
    scene.validate().unwrap();
    let error = scene.export_stl(&path, 1., None).unwrap_err().to_string();
    assert!(error.contains("two million triangle limit"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), b"original content");
    scene
        .export_stl(&path, 1., Some(&BTreeSet::from([first])))
        .unwrap();
    assert_eq!(binary_count(&path), 960);
}

#[test]
#[cfg(unix)]
fn imported_names_are_valid_and_unique_even_with_control_characters() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("part\nmodel.stl");
    fs::write(&path, ASCII_TRIANGLE).unwrap();
    let mut scene = Scene::empty();
    let first = scene.import_stl(&path, 1.).unwrap();
    let second = scene.import_stl(&path, 1.).unwrap();
    assert_eq!(scene.object(first).unwrap().name, "partmodel");
    assert_eq!(scene.object(second).unwrap().name, "partmodel.001");
    scene.validate().unwrap();
}

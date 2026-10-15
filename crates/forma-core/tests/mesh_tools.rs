use forma_core::{Mesh, Primitive, Vec3};
use std::collections::{BTreeMap, BTreeSet};

fn volume(mesh: &Mesh) -> f64 {
    let origin = mesh.positions[0].as_dvec3();
    mesh.triangles()
        .into_iter()
        .map(|triangle| {
            let [a, b, c] = triangle.map(|i| mesh.positions[i as usize].as_dvec3() - origin);
            a.dot(b.cross(c)) / 6.
        })
        .sum()
}

fn assert_closed_outward(mesh: &Mesh) {
    mesh.validate().unwrap();
    let mut edges: BTreeMap<[u32; 2], Vec<[u32; 2]>> = BTreeMap::new();
    for face in &mesh.faces {
        for (i, &a) in face.iter().enumerate() {
            let b = face[(i + 1) % face.len()];
            edges.entry([a.min(b), a.max(b)]).or_default().push([a, b]);
        }
    }
    for uses in edges.values() {
        assert_eq!(uses.len(), 2, "Expected a closed edge: {uses:?}");
        assert_eq!(
            uses[0],
            [uses[1][1], uses[1][0]],
            "Expected consistent winding"
        );
    }
    assert!(volume(mesh) > 0.);
}

fn append_mesh(target: &mut Mesh, source: &Mesh, offset: Vec3) {
    let first = target.positions.len() as u32;
    target
        .positions
        .extend(source.positions.iter().map(|&p| p + offset));
    target.faces.extend(
        source
            .faces
            .iter()
            .map(|face| face.iter().map(|i| i + first).collect()),
    );
}

#[test]
fn selected_triangulation_preserves_concave_winding_area_and_selection() {
    for reversed in [false, true] {
        let mut mesh = Mesh {
            positions: vec![
                Vec3::new(0., 0., 0.),
                Vec3::new(3., 0., 0.),
                Vec3::new(3., 3., 0.),
                Vec3::new(1.5, 1., 0.),
                Vec3::new(0., 3., 0.),
                Vec3::new(5., 0., 0.),
                Vec3::new(6., 0., 0.),
                Vec3::new(6., 1., 0.),
                Vec3::new(5., 1., 0.),
            ],
            faces: vec![vec![5, 6, 7, 8], vec![0, 1, 2, 3, 4], vec![5, 6, 7]],
        };
        if reversed {
            mesh.faces[1].reverse();
        }
        let original = mesh.clone();
        let selected = mesh.triangulate_faces(&BTreeSet::from([1])).unwrap();
        assert_eq!(selected, BTreeSet::from([1, 2, 3]));
        assert_eq!(mesh.positions, original.positions);
        assert_eq!(mesh.faces[0], original.faces[0]);
        assert_eq!(mesh.faces[4], original.faces[2]);
        let mut area = 0.;
        for index in selected {
            let points: Vec<_> = mesh.faces[index]
                .iter()
                .map(|&i| mesh.positions[i as usize])
                .collect();
            let cross = (points[1] - points[0]).cross(points[2] - points[0]);
            assert!(cross.z * if reversed { -1. } else { 1. } > 0.);
            area += cross.length() * 0.5;
        }
        assert!((area - 6.).abs() < 1e-6);
    }
}

#[test]
fn face_tools_reject_invalid_selections_without_partial_edits() {
    let mut mesh = Mesh::primitive(Primitive::Cube);
    let original = mesh.clone();
    for selected in [BTreeSet::new(), BTreeSet::from([0, 999])] {
        assert!(mesh.triangulate_faces(&selected).is_err());
        assert_eq!(mesh, original);
        assert!(mesh.flip_faces(&selected).is_err());
        assert_eq!(mesh, original);
    }
    mesh.flip_faces(&BTreeSet::from([1, 3])).unwrap();
    assert_eq!(mesh.face_normal(1), -original.face_normal(1));
    assert_eq!(mesh.face_normal(3), -original.face_normal(3));
    assert_eq!(mesh.face_normal(0), original.face_normal(0));
    assert_eq!(mesh.positions, original.positions);
    mesh.flip_faces(&BTreeSet::from([1, 3])).unwrap();
    assert_eq!(mesh, original);
}

#[test]
fn normals_recover_every_cube_winding_combination_far_from_origin() {
    for mask in 0..64 {
        let mut mesh = Mesh::primitive(Primitive::Cube);
        for p in &mut mesh.positions {
            *p += Vec3::splat(100_000.);
        }
        for (face, vertices) in mesh.faces.iter_mut().enumerate() {
            if mask & (1 << face) != 0 {
                vertices.reverse();
            }
        }
        mesh.recalculate_normals().unwrap();
        assert_closed_outward(&mesh);
        assert!((volume(&mesh) - 8.).abs() < 1e-10);
        let repaired = mesh.clone();
        mesh.recalculate_normals().unwrap();
        assert_eq!(mesh, repaired, "Recalculation must be idempotent");
    }
}

#[test]
fn normals_orient_closed_components_independently_and_preserve_open_seed() {
    let mut mesh = Mesh::primitive(Primitive::Cube);
    for face in &mut mesh.faces {
        face.reverse();
    }
    append_mesh(&mut mesh, &Mesh::primitive(Primitive::Cube), Vec3::X * 10.);
    let mut plane = Mesh::primitive(Primitive::Plane);
    plane.triangulate_faces(&BTreeSet::from([0])).unwrap();
    plane.faces[0].reverse();
    append_mesh(&mut mesh, &plane, Vec3::Y * 10.);
    let seed_normal = mesh.face_normal(12);
    mesh.recalculate_normals().unwrap();
    for start in [0, 6] {
        assert!(
            volume(&Mesh {
                positions: mesh.positions.clone(),
                faces: mesh.faces[start..start + 6].to_vec()
            }) > 0.
        );
    }
    assert_eq!(mesh.face_normal(12), seed_normal);
    assert_eq!(mesh.face_normal(13), seed_normal);
}

#[test]
fn normals_preserve_thin_solids_at_arbitrary_orientations() {
    let rotations = [
        forma_core::Mat4::IDENTITY,
        forma_core::Mat4::from_rotation_z(0.7),
        forma_core::Mat4::from_rotation_x(0.4)
            * forma_core::Mat4::from_rotation_y(0.6)
            * forma_core::Mat4::from_rotation_z(0.7),
    ];
    for scale in [
        Vec3::new(10_000., 0.001, 0.001),
        Vec3::new(100., 0.0001, 2.),
    ] {
        for rotation in rotations {
            let mut mesh = Mesh::primitive(Primitive::Cube);
            for p in &mut mesh.positions {
                *p = rotation.transform_vector3(*p * scale);
            }
            mesh.validate().unwrap();
            let original_volume = volume(&mesh);
            assert!(original_volume > 0.);
            for index in [0, 1, 2, 4] {
                mesh.faces[index].reverse();
            }
            mesh.recalculate_normals().unwrap();
            assert_closed_outward(&mesh);
            assert!((volume(&mesh) - original_volume).abs() <= original_volume * 1.0e-5);
        }
    }
}

#[test]
fn bevel_checks_thin_dimensions_when_rejecting_elongated_concave_parts() {
    let mut mesh = Mesh::primitive(Primitive::Cube);
    for p in &mut mesh.positions {
        *p *= Vec3::new(10_000., 1., 0.001);
    }
    mesh.positions.push(Vec3::new(0., 0., 0.0005));
    // A small inward dent in the +Z face. Its depth must not be judged
    // against this part's much larger X dimension.
    mesh.faces[1] = vec![4, 5, 8];
    mesh.faces
        .extend([vec![5, 6, 8], vec![6, 7, 8], vec![7, 4, 8]]);
    mesh.validate().unwrap();
    let original = mesh.clone();
    let error = mesh.bevel_edges(0.0001).unwrap_err().to_string();
    assert!(error.contains("convex"), "{error}");
    assert_eq!(mesh, original);
}

#[test]
fn bevel_rejects_excessive_convexity_work_before_changing_geometry() {
    // A valid closed convex bipyramid, dense enough to exceed the interactive
    // supporting-plane budget without approaching the geometry size limit.
    let segments = 2100;
    let mut mesh = Mesh {
        positions: vec![Vec3::Y, -Vec3::Y],
        faces: Vec::new(),
    };
    for i in 0..segments {
        let angle = std::f32::consts::TAU * i as f32 / segments as f32;
        mesh.positions.push(Vec3::new(angle.cos(), 0., angle.sin()));
        let vertex = i + 2;
        let next = (i + 1) % segments + 2;
        mesh.faces.push(vec![0, next, vertex]);
        mesh.faces.push(vec![1, vertex, next]);
    }
    mesh.validate().unwrap();
    let original = mesh.clone();
    let error = mesh.bevel_edges(0.0001).unwrap_err().to_string();
    assert!(error.contains("convexity check is too large"), "{error}");
    assert_eq!(mesh, original);
}

#[test]
fn normals_reject_nonmanifold_edges_pinched_vertices_and_zero_volume_atomically() {
    let mut edge = Mesh::primitive(Primitive::Cube);
    edge.faces.push(edge.faces[0].clone());
    let mut pinched = Mesh::primitive(Primitive::Cube);
    append_mesh(
        &mut pinched,
        &Mesh::primitive(Primitive::Cube),
        Vec3::splat(2.),
    );
    // The first cube's vertex 6 and second cube's vertex 0 coincide; weld only
    // that vertex, creating two closed fans sharing one topological vertex.
    for face in &mut pinched.faces[6..] {
        for vertex in face {
            if *vertex == 8 {
                *vertex = 6;
            }
        }
    }
    let mut flat = Mesh::primitive(Primitive::Plane);
    flat.faces
        .push(flat.faces[0].iter().rev().copied().collect());
    for mut mesh in [edge, pinched, flat] {
        mesh.validate().unwrap();
        let original = mesh.clone();
        assert!(mesh.recalculate_normals().is_err());
        assert_eq!(mesh, original);
    }
}

#[test]
fn normals_reject_a_nonorientable_mobius_strip_atomically() {
    let segments = 16;
    let mut mesh = Mesh {
        positions: Vec::new(),
        faces: Vec::new(),
    };
    for i in 0..segments {
        let angle = std::f32::consts::TAU * i as f32 / segments as f32;
        for width in [-0.2, 0.2] {
            let radius = 2. + width * (angle * 0.5).cos();
            mesh.positions.push(Vec3::new(
                radius * angle.cos(),
                width * (angle * 0.5).sin(),
                radius * angle.sin(),
            ));
        }
        let [a, b] = if i + 1 == segments {
            [1, 0]
        } else {
            [2 * (i + 1), 2 * (i + 1) + 1]
        };
        mesh.faces.push(vec![2 * i, a, b, 2 * i + 1]);
    }
    mesh.validate().unwrap();
    let original = mesh.clone();
    assert!(
        mesh.recalculate_normals()
            .unwrap_err()
            .to_string()
            .contains("non-orientable")
    );
    assert_eq!(mesh, original);
}

#[test]
fn cube_bevel_has_exact_width_closed_topology_and_outward_caps() {
    let mut mesh = Mesh::primitive(Primitive::Cube);
    mesh.bevel_edges(0.2).unwrap();
    assert_closed_outward(&mesh);
    assert_eq!(mesh.positions.len(), 24);
    assert_eq!(mesh.faces.len(), 26);
    assert_eq!(mesh.edges().len(), 48);
    assert!(volume(&mesh) < 8. && volume(&mesh) > 7.);
    for p in &mesh.positions {
        let mut coordinates = p.abs().to_array();
        coordinates.sort_by(f32::total_cmp);
        assert!((coordinates[0] - 0.8).abs() < 1e-6);
        assert!((coordinates[1] - 0.8).abs() < 1e-6);
        assert!((coordinates[2] - 1.).abs() < 1e-6);
    }
    for (index, face) in mesh.faces.iter().enumerate() {
        let center = face
            .iter()
            .map(|&i| mesh.positions[i as usize])
            .sum::<Vec3>()
            / face.len() as f32;
        assert!(mesh.face_normal(index).dot(center) > 0.);
    }
}

#[test]
fn bevel_repairs_winding_and_handles_nonsquare_faces_and_multiple_solids() {
    let mut mesh = Mesh::primitive(Primitive::Cube);
    for p in &mut mesh.positions {
        p.x *= 3.;
        p.y *= 2.;
        p.x += 0.4 * p.z;
        *p += Vec3::splat(10_000.);
    }
    for index in [0, 2, 4] {
        mesh.faces[index].reverse();
    }
    append_mesh(
        &mut mesh,
        &Mesh::primitive(Primitive::Cube),
        Vec3::splat(-10.),
    );
    mesh.bevel_edges(0.1).unwrap();
    assert_closed_outward(&mesh);
    assert_eq!(mesh.positions.len(), 48);
    assert_eq!(mesh.faces.len(), 52);
}

#[test]
fn bevel_supports_cylinder_sphere_and_triangulated_convex_shells() {
    for primitive in [Primitive::Cylinder, Primitive::Sphere, Primitive::Cube] {
        let mut mesh = Mesh::primitive(primitive);
        if primitive == Primitive::Cube {
            let selected = (0..mesh.faces.len()).collect();
            mesh.triangulate_faces(&selected).unwrap();
        }
        let original_volume = volume(&mesh);
        mesh.bevel_edges(0.01)
            .unwrap_or_else(|error| panic!("{primitive:?}: {error:#}"));
        assert_closed_outward(&mesh);
        assert!(volume(&mesh) < original_volume);
    }
}

#[test]
fn bevel_rejects_oversized_nonfinite_open_concave_and_nonmanifold_inputs_atomically() {
    for width in [0., -0.1, f32::NAN, f32::INFINITY, 1., 10., 1e-10] {
        let mut mesh = Mesh::primitive(Primitive::Cube);
        let original = mesh.clone();
        assert!(
            mesh.bevel_edges(width).is_err(),
            "Invalid width {width} was accepted"
        );
        assert_eq!(mesh, original);
    }
    let mut nonmanifold = Mesh::primitive(Primitive::Cube);
    nonmanifold.faces.push(nonmanifold.faces[0].clone());
    for mut mesh in [
        Mesh::primitive(Primitive::Plane),
        Mesh::primitive(Primitive::Torus),
        nonmanifold,
    ] {
        let original = mesh.clone();
        assert!(mesh.bevel_edges(0.01).is_err());
        assert_eq!(mesh, original);
    }
}

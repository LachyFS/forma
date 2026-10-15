use forma_core::{Mesh, Vec3};

#[test]
fn mixed_faces_preserve_diagonals_winding_and_polygon_indices() {
    let mut mesh = Mesh {
        positions: vec![
            Vec3::new(0., 0., 0.),
            Vec3::new(1., 0., 0.),
            Vec3::new(1., 1., 0.),
            Vec3::new(0.5, 0.25, 0.),
            Vec3::new(0., 1., 0.),
        ],
        faces: vec![
            vec![0, 1, 2, 3, 4], // Concave polygon.
            vec![0, 1, 2],
            vec![0, 1, 2, 4],
            vec![4, 3, 2, 1, 0], // Reversed winding after smaller faces.
        ],
    };
    let expected = vec![
        ([1, 2, 3], 0),
        ([0, 1, 3], 0),
        ([0, 3, 4], 0),
        ([0, 1, 2], 1),
        ([4, 0, 1], 2),
        ([1, 2, 4], 2),
        ([0, 4, 3], 3),
        ([3, 2, 1], 3),
        ([3, 1, 0], 3),
    ];
    let original = mesh.positions.clone();
    for axis in 0..3 {
        // Exercise all projection axes, including translation away from origin.
        mesh.positions = original
            .iter()
            .map(|p| {
                let p = match axis {
                    0 => Vec3::new(0., p.x, p.y),
                    1 => Vec3::new(p.y, 0., p.x),
                    _ => *p,
                };
                p + Vec3::splat(10_000.)
            })
            .collect();
        mesh.validate().unwrap();
        assert_eq!(mesh.triangles_with_faces(), expected);
        assert_eq!(
            mesh.triangles(),
            expected
                .iter()
                .map(|&(triangle, _)| triangle)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn failed_faces_do_not_publish_partial_or_previous_triangles() {
    let mesh = Mesh {
        positions: vec![
            Vec3::new(0., 0., 0.),
            Vec3::new(2., 0., 0.),
            Vec3::new(2., 2., 0.),
            Vec3::new(0., 2., 0.),
            Vec3::new(0., 0., 0.),
        ],
        faces: vec![
            vec![0, 1, 2, 3],
            vec![],
            vec![0, 1],
            vec![0, 1, 99],
            vec![0, 1, 99, 3],
            vec![0, 1, 2, 3, 4], // Clips an ear, then fails at the repeated point.
            vec![0, 0, 0, 0],    // Zero projected area.
            vec![0, 2, 3],
        ],
    };
    assert_eq!(
        mesh.triangles_with_faces(),
        vec![([3, 0, 1], 0), ([1, 2, 3], 0), ([0, 2, 3], 7)]
    );
    assert_eq!(mesh.triangles(), vec![[3, 0, 1], [1, 2, 3], [0, 2, 3]]);
}

#[test]
fn validation_checks_each_face_after_reusing_larger_buffers() {
    let mut mesh = Mesh {
        positions: vec![
            Vec3::new(0., 0., 0.),
            Vec3::new(2., 0., 0.),
            Vec3::new(2., 2., 0.),
            Vec3::new(0., 2., 0.),
        ],
        faces: vec![vec![0, 1, 2, 3], vec![0, 1, 2]],
    };
    mesh.validate().unwrap();
    for (face, message) in [
        (vec![0, 1, 0], "repeats a vertex"),
        (vec![0, 1, 99], "references a missing vertex"),
        (vec![0, 2, 1, 3], "zero area"),
    ] {
        mesh.faces[1] = face;
        let error = mesh.validate().unwrap_err().to_string();
        assert!(
            error.contains("Face 1") && error.contains(message),
            "{error}"
        );
    }
}

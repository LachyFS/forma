use forma_core::{Mesh, Vec3};

#[test]
fn bevel_rejects_impossible_output_before_building_dense_topology() {
    // A closed convex bipyramid is legal input geometry, but its face corners
    // would require more than two million output vertices. It must fail on
    // that exact budget before allocating adjacency/fan maps or running the
    // quadratic convexity check (which has a different error).
    let ring = 333_334u32;
    let mut mesh = Mesh {
        positions: Vec::with_capacity(ring as usize + 2),
        faces: Vec::with_capacity(ring as usize * 2),
    };
    mesh.positions.extend([Vec3::Y, Vec3::NEG_Y]);
    for i in 0..ring {
        let angle = std::f64::consts::TAU * f64::from(i) / f64::from(ring);
        mesh.positions
            .push(Vec3::new(angle.cos() as f32, 0., angle.sin() as f32));
        let current = i + 2;
        let next = (i + 1) % ring + 2;
        mesh.faces.push(vec![0, next, current]);
        mesh.faces.push(vec![1, current, next]);
    }
    let first = mesh.faces[0].clone();
    let last = mesh.faces.last().unwrap().clone();
    let error = mesh.bevel_edges(0.01).unwrap_err().to_string();
    assert!(error.contains("two million vertex/face limit"), "{error}");
    assert_eq!(mesh.positions.len(), ring as usize + 2);
    assert_eq!(mesh.faces.len(), ring as usize * 2);
    assert_eq!(mesh.faces[0], first);
    assert_eq!(*mesh.faces.last().unwrap(), last);
}

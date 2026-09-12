//! Independent regression cases assembled during implementation review.
use forma_core::*;

fn empty_scene() -> Scene {
    Scene::empty()
}

#[test]
fn crossing_polygon_with_nonzero_signed_area_is_rejected() {
    // Edges 0→1 and 2→3 cross. Unlike a symmetric bow tie, this
    // polygon has nonzero signed area, so area alone is insufficient.
    let mesh = Mesh {
        positions: vec![
            Vec3::new(0., 0., 0.),
            Vec3::new(2., 2., 0.),
            Vec3::new(0., 2., 0.),
            Vec3::new(3., 0., 0.),
        ],
        faces: vec![vec![0, 1, 2, 3]],
    };
    assert!(
        mesh.validate().is_err(),
        "crossing polygon must not become overlapping render triangles"
    );
}

#[test]
fn valid_small_scale_object_can_be_picked() {
    let mut scene = empty_scene();
    let id = scene.add(Primitive::Cube);
    scene.object_mut(id).unwrap().transform.scale = Vec3::splat(0.00005);
    scene.validate().unwrap();
    let hit = scene.pick(Ray {
        origin: Vec3::new(0., 0., 0.02),
        direction: Vec3::NEG_Z,
    });
    assert_eq!(
        hit.map(|hit| hit.object_id),
        Some(id),
        "validated nonsingular small meshes must remain selectable"
    );
}

#[test]
fn inward_extrusion_keeps_edge_winding_and_reduces_cube_volume() {
    let mut mesh = Mesh::primitive(Primitive::Cube);
    mesh.extrude_face(3, -0.5).unwrap();
    mesh.validate().unwrap();
    let volume: f32 = mesh
        .triangles()
        .iter()
        .map(|&[a, b, c]| {
            mesh.positions[a as usize]
                .dot(mesh.positions[b as usize].cross(mesh.positions[c as usize]))
                / 6.
        })
        .sum();
    assert!((volume - 6.).abs() < 1e-5);
    let mut winding = std::collections::BTreeMap::<(u32, u32), (usize, i32)>::new();
    for face in &mesh.faces {
        for (&a, &b) in face
            .iter()
            .zip(face.iter().cycle().skip(1))
            .take(face.len())
        {
            let edge = winding.entry((a.min(b), a.max(b))).or_default();
            edge.0 += 1;
            edge.1 += if a < b { 1 } else { -1 };
        }
    }
    assert!(winding.values().all(|value| *value == (2, 0)));
}

#[test]
fn obj_import_at_object_limit_is_atomic() {
    let mut scene = empty_scene();
    scene.add(Primitive::Plane);
    let prototype = scene.objects[0].clone();
    scene.objects = (4..10_004)
        .map(|id| {
            let mut object = prototype.clone();
            object.id = id;
            object.name = format!("Plane {id}");
            object
        })
        .collect();
    scene.next_id = 10_004;
    scene.validate().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("additional.obj");
    std::fs::write(&path, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
    let next_id = scene.next_id;
    assert!(
        scene.import_obj(&path).is_err(),
        "import must reject edits that cannot subsequently be saved"
    );
    assert_eq!(scene.objects.len(), 10_000);
    assert_eq!(scene.next_id, next_id);
    scene.validate().unwrap();
}

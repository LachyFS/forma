//! CPU preparation benchmark and byte-for-byte oracle for mesh-space preparation.
//! Run: cargo test -p forma-render --release geometry_preparation_benchmark -- --ignored --nocapture
//! FORMA_GEOMETRY_BENCH selects baseline, optimized, or both (the default).
use super::*;
use forma_core::{Material, Mesh, Primitive};
use std::{hint::black_box, path::Path, time::Instant};

fn linked_scene(count: usize) -> Scene {
    let mut scene = Scene::default();
    scene.objects.clear();
    let id = scene.add(Primitive::Sphere);
    let object = scene.object(id).unwrap();
    let mesh = object.data.mesh_id().unwrap();
    let materials = object.data.material_ids().to_vec();
    for i in 1..count {
        let id = scene
            .instantiate_mesh(format!("Sphere {i}"), mesh, materials.clone())
            .unwrap();
        scene.object_mut(id).unwrap().transform.translation =
            Vec3::new((i % 16) as f32 * 2.5, (i / 16) as f32 * 2.5, 0.0);
    }
    scene
}

fn dense_scene(size: u32) -> Scene {
    let mut scene = Scene::default();
    scene.objects.clear();
    let mut mesh = Mesh {
        positions: Vec::new(),
        faces: Vec::new(),
    };
    for y in 0..=size {
        for x in 0..=size {
            let x = x as f32 / size as f32;
            let y = y as f32 / size as f32;
            mesh.positions
                .push(Vec3::new(x, y, (x * 12.0).sin() * (y * 8.0).cos() * 0.05));
        }
    }
    for y in 0..size {
        for x in 0..size {
            let a = y * (size + 1) + x;
            mesh.faces.push(vec![a, a + 1, a + size + 2, a + size + 1]);
        }
    }
    let mesh = scene.create_mesh_data("Dense grid", mesh).unwrap();
    let material = scene
        .create_material_data("Grid", Material::default())
        .unwrap();
    scene
        .instantiate_mesh("Dense grid", mesh, vec![material])
        .unwrap();
    scene
}

fn bundled_v8() -> Scene {
    Scene::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/scenes/v8-engine.forma"))
        .unwrap()
}

fn assert_geometry_matches(scene: &Scene) {
    let expected = reference_geometry(scene);
    let actual = Geometry::from_scene(scene);
    assert_eq!(actual.triangles.len(), expected.triangles.len());
    assert_eq!(
        bytemuck::cast_slice::<Triangle, u8>(&actual.triangles),
        bytemuck::cast_slice::<Triangle, u8>(&expected.triangles),
        "triangle attributes/order changed"
    );
    assert_eq!(
        bytemuck::cast_slice::<Node, u8>(&actual.nodes),
        bytemuck::cast_slice::<Node, u8>(&expected.nodes),
        "BVH nodes changed"
    );
    assert_eq!(actual.lights, expected.lights);
}

#[test]
fn mesh_preparation_matches_reference_scenes() {
    for scene in [
        Scene::default(),
        bundled_v8(),
        linked_scene(6),
        dense_scene(16),
    ] {
        assert_geometry_matches(&scene);
    }
}

#[test]
fn mesh_preparation_preserves_linked_materials_transforms_and_rebuilds() {
    let mut scene = linked_scene(4);
    let ids: Vec<_> = scene.objects.iter().map(|object| object.id).collect();
    let emissive = scene
        .create_material_data(
            "Instance material",
            Material {
                base_color: Vec3::new(0.8, 0.1, 0.3),
                emission: Vec3::new(1.0, 2.0, 3.0),
                metallic: 0.9,
                roughness: 0.06,
                ..Material::default()
            },
        )
        .unwrap();
    if let forma_core::ObjectData::Mesh { materials, .. } =
        &mut scene.object_mut(ids[1]).unwrap().data
    {
        materials[0] = emissive;
    }
    scene.object_mut(ids[0]).unwrap().transform.scale = Vec3::new(-1.0, 0.1, 2.0);
    scene.object_mut(ids[1]).unwrap().transform.rotation = Vec3::new(0.2, 1.1, -0.6);
    scene.object_mut(ids[2]).unwrap().visible = false;
    let parent = scene.add_empty("Parent").unwrap();
    scene.object_mut(parent).unwrap().transform.rotation = Vec3::new(0.1, 0.3, 0.8);
    scene.object_mut(parent).unwrap().transform.scale = Vec3::new(0.7, 2.0, 0.2);
    scene.set_parent(ids[3], Some(parent), false).unwrap();
    assert_geometry_matches(&scene);

    // Prepared mesh data belongs to one build: edits must affect every instance
    // on the next build, even when mesh IDs and object transforms do not change.
    scene.object_mesh_mut(ids[0]).unwrap().positions[0] += Vec3::new(0.02, 0.03, 0.01);
    scene.object_material_mut(ids[1]).unwrap().emission = Vec3::ZERO;
    scene.object_material_mut(ids[1]).unwrap().shader = forma_core::ShaderKind::Custom;
    scene.object_mut(ids[2]).unwrap().visible = true;
    assert_geometry_matches(&scene);
}

#[test]
fn mesh_preparation_preserves_creases_concavity_and_degenerate_faces() {
    let mut scene = Scene::default();
    scene.objects.clear();
    let id = scene.add(Primitive::Cube);
    let mesh = scene.object_mesh_mut(id).unwrap();
    mesh.positions = vec![Vec3::ZERO, Vec3::X];
    mesh.faces.clear();
    for angle in [0.0_f32, 44.999, 45.0, 45.001, 90.0, 180.0] {
        let angle = angle.to_radians();
        mesh.positions
            .push(Vec3::new(0.0, angle.cos(), angle.sin()));
        mesh.faces.push(vec![0, 1, mesh.positions.len() as u32 - 1]);
    }
    let start = mesh.positions.len() as u32;
    mesh.positions.extend([
        Vec3::new(3.0, 0.0, 0.0),
        Vec3::new(5.0, 0.0, 0.0),
        Vec3::new(5.0, 2.0, 0.0),
        Vec3::new(4.0, 0.4, 0.0),
        Vec3::new(3.0, 2.0, 0.0),
    ]);
    mesh.faces.push((start..start + 5).collect());
    mesh.faces.push(vec![0, 0, 1]);
    mesh.faces.push(vec![0, 1]);
    mesh.faces.push(Vec::new());
    mesh.positions.push(Vec3::new(30.0, 0.0, 0.0));
    assert_geometry_matches(&scene);

    let mesh = scene.object_mesh_mut(id).unwrap();
    mesh.positions.clear();
    mesh.faces.clear();
    assert_geometry_matches(&scene);
}

#[test]
#[ignore = "release-mode CPU measurement; no timing thresholds"]
fn geometry_preparation_benchmark() {
    let selected = std::env::var("FORMA_GEOMETRY_BENCH").unwrap_or_else(|_| "both".into());
    let samples = std::env::var("FORMA_GEOMETRY_SAMPLES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(31)
        .max(1);
    println!(
        "geometry preparation: samples={samples} warmup=5; includes BVH construction and output destruction"
    );
    for (name, scene) in [
        ("default", Scene::default()),
        ("v8", bundled_v8()),
        ("linked-128", linked_scene(128)),
        ("dense-128", dense_scene(128)),
    ] {
        assert_geometry_matches(&scene);
        println!(
            "{name} objects={} triangles={}",
            scene.objects.len(),
            Geometry::from_scene(&scene).triangles.len()
        );
        let operations: Vec<_> = [
            ("baseline", reference_geometry as fn(&Scene) -> Geometry),
            ("optimized", Geometry::from_scene),
        ]
        .into_iter()
        .filter(|(label, _)| selected == "both" || selected == *label)
        .collect();
        assert!(
            !operations.is_empty(),
            "FORMA_GEOMETRY_BENCH must be baseline, optimized, or both"
        );
        let mut timings = vec![Vec::with_capacity(samples); operations.len()];
        for _ in 0..5 {
            for (_, operation) in &operations {
                black_box(operation(black_box(&scene)));
            }
        }
        // Alternate order within each paired sample to reduce allocator and CPU
        // frequency drift when comparing implementations in the same process.
        for sample in 0..samples {
            for step in 0..operations.len() {
                let index = (sample + step) % operations.len();
                let start = Instant::now();
                black_box(operations[index].1(black_box(&scene)));
                timings[index].push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        for ((label, _), mut timings) in operations.into_iter().zip(timings) {
            timings.sort_by(f64::total_cmp);
            println!(
                "  {label} median_ms={:.3} p95_ms={:.3}",
                timings[samples / 2],
                timings[(samples * 95 / 100).min(samples - 1)]
            );
        }
    }
}

// Preserve the original per-object implementation as an independent correctness
// oracle and benchmark baseline. It intentionally does not share preparation helpers.
fn reference_geometry(scene: &Scene) -> Geometry {
    let mut triangles = Vec::new();
    for (object_index, object) in scene.objects.iter().enumerate() {
        if !scene.is_effectively_visible(object.id) {
            continue;
        }
        let Some(instance) = scene.mesh_instance(object.id) else {
            continue;
        };
        let transform = instance.world_transform;
        let normal_transform = transform.inverse().transpose();
        let mesh = instance.mesh;
        let min = mesh
            .positions
            .iter()
            .copied()
            .fold(Vec3::splat(f32::INFINITY), Vec3::min);
        let max = mesh
            .positions
            .iter()
            .copied()
            .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
        let span = (max - min).max(Vec3::splat(1e-6));
        let generated: Vec<_> = mesh.positions.iter().map(|p| (*p - min) / span).collect();
        let material_index = scene
            .materials
            .iter()
            .position(|m| Some(&m.id) == object.data.material_ids().first())
            .unwrap_or(0);
        let indices = mesh.triangles();
        let positions: Vec<_> = mesh
            .positions
            .iter()
            .map(|p| transform.transform_point3(*p))
            .collect();
        let edges: HashSet<_> = mesh
            .edges()
            .into_iter()
            .map(|[a, b]| edge_key(a, b))
            .collect();
        // Classify creases and angle-weight in mesh space, then transform
        // normals with the inverse transpose. Nonuniform object scaling must
        // not change smoothing groups or create seams on a flattened sphere.
        let mut adjacent = vec![Vec::<(Vec3, f32)>::new(); positions.len()];
        let face_normals: Vec<_> = indices
            .iter()
            .map(|face| {
                let [a, b, c] = face.map(|index| mesh.positions[index as usize]);
                let n = (b - a).cross(c - a).normalize_or_zero();
                for corner in 0..3 {
                    let p = mesh.positions[face[corner] as usize];
                    let e0 =
                        (mesh.positions[face[(corner + 1) % 3] as usize] - p).normalize_or_zero();
                    let e1 =
                        (mesh.positions[face[(corner + 2) % 3] as usize] - p).normalize_or_zero();
                    let angle = e0.dot(e1).clamp(-1.0, 1.0).acos();
                    adjacent[face[corner] as usize].push((n, angle));
                }
                n
            })
            .collect();
        for (face, geometric) in indices.iter().zip(face_normals) {
            if geometric.length_squared() < 0.5 {
                continue;
            }
            let normals = face.map(|index| {
                let normal = adjacent[index as usize]
                    .iter()
                    .filter(|(normal, _)| normal.dot(geometric) > std::f32::consts::FRAC_1_SQRT_2)
                    .map(|(normal, angle)| *normal * *angle)
                    .sum::<Vec3>()
                    .normalize_or_zero();
                normal_transform
                    .transform_vector3(normal)
                    .normalize_or_zero()
            });
            let mut edge_bits = 0u32;
            for corner in 0..3 {
                if edges.contains(&edge_key(face[(corner + 1) % 3], face[(corner + 2) % 3])) {
                    edge_bits |= 1 << corner;
                }
            }
            triangles.push(Triangle {
                generated0: generated[face[0] as usize]
                    .extend(material_index as f32)
                    .to_array(),
                generated1: vec4(generated[face[1] as usize]),
                generated2: vec4(generated[face[2] as usize]),
                v0: vec4(positions[face[0] as usize]),
                v1: vec4(positions[face[1] as usize]),
                v2: vec4(positions[face[2] as usize]),
                n0: vec4(normals[0]),
                n1: vec4(normals[1]),
                n2: vec4(normals[2]),
                base_color: vec4(instance.material.base_color.clamp(Vec3::ZERO, Vec3::ONE)),
                emission: instance
                    .material
                    .emission
                    .max(Vec3::ZERO)
                    .extend(f32::from(
                        instance.material.shader == forma_core::ShaderKind::Custom,
                    ))
                    .to_array(),
                params: [
                    instance.material.metallic.clamp(0.0, 1.0),
                    instance.material.roughness.clamp(0.02, 1.0),
                    (object_index + 1) as f32,
                    edge_bits as f32,
                ],
            });
        }
    }
    Geometry::build(triangles)
}

//! Booleans on dense converted meshes stay fast: result validation must cost
//! far less than truck's own boolean (review finding: it was quadratic).
use forma_cad::{RegenCache, RegenRequest, regenerate};
use forma_core::*;
use std::time::Instant;

/// A closed UV sphere with about 2·n² triangles.
fn sphere_mesh(n: u32) -> Mesh {
    let mut positions = vec![Vec3::new(0.0, 1.0, 0.0)];
    for i in 1..n {
        let t = std::f32::consts::PI * i as f32 / n as f32;
        for j in 0..n {
            let p = std::f32::consts::TAU * j as f32 / n as f32;
            positions.push(Vec3::new(t.sin() * p.cos(), t.cos(), t.sin() * p.sin()));
        }
    }
    positions.push(Vec3::new(0.0, -1.0, 0.0));
    let south = positions.len() as u32 - 1;
    let ring = |i: u32, j: u32| 1 + (i - 1) * n + (j % n);
    let mut faces = Vec::new();
    for j in 0..n {
        faces.push(vec![0, ring(1, j + 1), ring(1, j)]);
    }
    for i in 1..n - 1 {
        for j in 0..n {
            faces.push(vec![ring(i, j), ring(i, j + 1), ring(i + 1, j + 1)]);
            faces.push(vec![ring(i, j), ring(i + 1, j + 1), ring(i + 1, j)]);
        }
    }
    for j in 0..n {
        faces.push(vec![south, ring(n - 1, j), ring(n - 1, j + 1)]);
    }
    Mesh::new(positions, faces)
}

fn cut_sphere(n: u32) -> (usize, f64, f64, Vec<String>) {
    let mesh = sphere_mesh(n);
    let mut part = forma_cad::part_from_mesh("Ball", &mesh, None);
    part.id = 7;
    part.push_feature(
        "Cut",
        FeatureKind::Primitive(PrimitiveFeature {
            shape: PrimitiveShape::Box {
                size: Vec3::splat(1.0),
            },
            placement: Placement {
                origin: Vec3::new(0.8, 0.0, 0.0),
                rotation: Vec3::ZERO,
            },
            operation: FeatureOperation::Cut,
        }),
    );
    let mut cache = RegenCache::default();
    // Evaluate the base body first so the timing isolates the cut.
    let mut base = part.clone();
    base.rollback = Some(1);
    base.restamp();
    regenerate(
        &RegenRequest {
            part_id: 7,
            part: base,
            skip: Vec::new(),
        },
        &mut cache,
        &mut |_| {},
    );
    let started = Instant::now();
    let result = regenerate(
        &RegenRequest {
            part_id: 7,
            part,
            skip: Vec::new(),
        },
        &mut cache,
        &mut |_| {},
    );
    (
        mesh.faces.len(),
        started.elapsed().as_secs_f64(),
        result.stats.evaluate_ms / 1.0e3,
        result
            .issues
            .into_iter()
            .map(|issue| issue.message)
            .collect(),
    )
}

#[test]
fn cutting_a_dense_converted_mesh_stays_fast() {
    // 19,800 triangles took 22 s, almost all of it in result validation.
    let (triangles, total, evaluate, issues) = cut_sphere(100);
    println!("{triangles} triangles: {total:.2} s total, {evaluate:.2} s evaluating");
    assert!(issues.is_empty(), "{issues:?}");
    assert!(
        evaluate < 6.0,
        "cutting {triangles} triangles took {evaluate:.1} s; the per-feature budget is 20 s"
    );
}

#[test]
#[ignore = "timing report for a 100k-triangle mesh"]
fn cutting_a_hundred_thousand_triangles() {
    let (triangles, total, evaluate, issues) = cut_sphere(224);
    println!("{triangles} triangles: {total:.2} s total, {evaluate:.2} s evaluating, {issues:?}");
}

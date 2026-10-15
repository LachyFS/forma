//! CPU picking benchmark. Run with --release; scene loading and ray preparation
//! are outside the timed region. Timings are observations, not pass/fail limits.
use forma_core::{Primitive, Ray, Scene, Vec2, Vec3};
use std::{hint::black_box, path::Path, time::Instant};

fn main() {
    println!("samples=21 warmup=3; picking includes triangulation and all allocations");
    let default = Scene::default();
    benchmark_scene("default", &default);
    let engine = Scene::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/scenes/v8-engine.forma"),
    )
    .expect("load bundled engine scene");
    benchmark_scene("v8-engine", &engine);

    let mut cube = Scene::empty();
    cube.add(Primitive::Cube);
    let hit = Ray {
        origin: Vec3::new(0.1, 0.2, 5.0),
        direction: Vec3::NEG_Z,
    };
    measure("cube-hit", &cube, &vec![hit; 512]);
    cube.meshes[0]
        .mesh
        .positions
        .extend(std::iter::repeat_n(Vec3::splat(100.0), 100_000));
    measure("cube-unused-vertices-hit", &cube, &vec![hit; 512]);
    let mut dense = Scene::empty();
    let id = dense.add(Primitive::Sphere);
    dense.object_mesh_mut(id).unwrap().subdivide();
    dense.object_mesh_mut(id).unwrap().subdivide();
    measure("dense-hit", &dense, &[hit; 16]);
    measure(
        "dense-miss",
        &dense,
        &[Ray {
            origin: Vec3::new(5.0, 5.0, 5.0),
            direction: Vec3::NEG_Z,
        }; 16],
    );
}

fn benchmark_scene(name: &str, scene: &Scene) {
    let rays: Vec<_> = (0..8)
        .flat_map(|y| {
            (0..12).map(move |x| {
                scene.camera.ray(
                    Vec2::new((x as f32 + 0.5) / 12.0, (y as f32 + 0.5) / 8.0),
                    1.5,
                )
            })
        })
        .collect();
    let (hits, misses): (Vec<_>, Vec<_>) = rays
        .iter()
        .copied()
        .partition(|&ray| scene.pick(ray).is_some());
    println!(
        "{name} objects={} vertices={} faces={} rays={} hits={} misses={}",
        scene.objects.len(),
        scene
            .meshes
            .iter()
            .map(|m| m.mesh.positions.len())
            .sum::<usize>(),
        scene
            .meshes
            .iter()
            .map(|m| m.mesh.faces.len())
            .sum::<usize>(),
        rays.len(),
        hits.len(),
        misses.len(),
    );
    measure(&format!("{name}/viewport"), scene, &rays);
    measure(&format!("{name}/hits"), scene, &hits);
    measure(&format!("{name}/misses"), scene, &misses);
}

fn measure(name: &str, scene: &Scene, rays: &[Ray]) {
    assert!(!rays.is_empty());
    let mut timings = Vec::with_capacity(21);
    for sample in 0..24 {
        let start = Instant::now();
        for &ray in rays {
            black_box(black_box(scene).pick(black_box(ray)));
        }
        if sample >= 3 {
            timings.push(start.elapsed().as_secs_f64() * 1.0e6 / rays.len() as f64);
        }
    }
    timings.sort_by(f64::total_cmp);
    println!(
        "  {name} median_us_per_ray={:.3} p95_us_per_ray={:.3}",
        timings[10], timings[19]
    );
}

//! CPU visibility benchmark; run with --release from any working directory.
//! Scene construction/loading and ray generation are outside timed sections.
use forma_core::{Primitive, Ray, Scene, SurfaceQuery, Vec2, Vec3};
use std::{hint::black_box, path::Path, time::Instant};

fn main() {
    println!("samples=31 warmup=5; build includes destruction; queries reuse one index");
    let v8 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/scenes/v8-engine.forma");
    for (name, scene) in [
        ("default", Scene::default()),
        ("v8", Scene::load(&v8).expect("bundled V8 scene")),
        ("dense", dense_scene()),
    ] {
        let triangles: usize = scene
            .mesh_instances()
            .map(|i| i.mesh.triangles().len())
            .sum();
        let rays = rays(&scene);
        let query = SurfaceQuery::new(&scene);
        let hits = rays
            .iter()
            .filter(|&&(ray, distance)| query.occludes(ray, distance))
            .count();
        println!(
            "{name} objects={} triangles={triangles} queries={} hits={hits}",
            scene.objects.len(),
            rays.len()
        );
        measure("build", || {
            black_box(SurfaceQuery::new(black_box(&scene)));
        });
        measure("queries", || {
            let mut hits = 0;
            for &(ray, distance) in &rays {
                hits += black_box(&query).occludes(black_box(ray), black_box(distance)) as usize;
            }
            black_box(hits);
        });
    }
}

fn measure(name: &str, mut operation: impl FnMut()) {
    for _ in 0..5 {
        operation();
    }
    let mut timings = Vec::with_capacity(31);
    for _ in 0..31 {
        let start = Instant::now();
        operation();
        timings.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    timings.sort_by(f64::total_cmp);
    println!(
        "  {name} median_ms={:.3} p95_ms={:.3}",
        timings[15], timings[29]
    );
}

fn rays(scene: &Scene) -> Vec<(Ray, f32)> {
    let mut rays = Vec::new();
    for opposite in [false, true] {
        let mut camera = scene.camera;
        if opposite {
            camera.yaw += std::f32::consts::PI;
        }
        for y in 0..64 {
            for x in 0..64 {
                let ray = camera.ray(
                    Vec2::new((x as f32 + 0.5) / 64., (y as f32 + 0.5) / 64.),
                    1.,
                );
                // Include both fully occluded rays and rays clipped near the scene.
                let distance = camera.distance * [0.75, 1.0, 2.0, 10.0][(x + y) % 4];
                rays.push((ray, distance));
            }
        }
    }
    rays
}

fn dense_scene() -> Scene {
    let mut scene = Scene::empty();
    for y in 0..16 {
        for x in 0..16 {
            let id = scene.add(Primitive::Sphere);
            let object = scene.object_mut(id).unwrap();
            object.transform.translation = Vec3::new(
                (x as f32 - 7.5) * 2.5,
                (y as f32 - 7.5) * 2.5,
                ((x * 7 + y * 3) % 5) as f32 * 1.5,
            );
        }
    }
    scene.camera.frame(Vec3::new(0., 0., 3.), 28.);
    scene
}

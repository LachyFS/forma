//! CPU geometry benchmark; run with --release. Timings are observations, not
//! pass/fail thresholds. No GPU or window system is required.
use forma_core::{Mesh, Vec3};
use std::{hint::black_box, time::Instant};

fn main() {
    println!("samples=31 warmup=5; timings include output allocation and destruction");
    for (name, corners) in [("triangles", 3), ("quads", 4), ("concave", 5)] {
        let mesh = tiled_mesh(256, corners);
        mesh.validate().unwrap();
        println!(
            "{name} vertices={} faces={}",
            mesh.positions.len(),
            mesh.faces.len()
        );
        measure("triangulate", || {
            black_box(black_box(&mesh).triangles());
        });
        measure("triangulate-with-faces", || {
            black_box(black_box(&mesh).triangles_with_faces());
        });
        measure("validate", || {
            black_box(black_box(&mesh).validate()).unwrap();
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

fn tiled_mesh(size: u32, corners: usize) -> Mesh {
    let shape = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(0.5, 0.25, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ];
    let shape: Vec<_> = match corners {
        3 => shape[..3].to_vec(),
        4 => vec![shape[0], shape[1], shape[2], shape[4]],
        _ => shape.to_vec(),
    };
    let face_count = (size * size) as usize;
    let mut mesh = Mesh {
        positions: Vec::with_capacity(face_count * corners),
        faces: Vec::with_capacity(face_count),
    };
    for y in 0..size {
        for x in 0..size {
            let first = mesh.positions.len() as u32;
            let offset = Vec3::new(x as f32 * 2.0, y as f32 * 2.0, 0.0);
            mesh.positions.extend(shape.iter().map(|p| *p + offset));
            mesh.faces.push((first..first + corners as u32).collect());
        }
    }
    mesh
}

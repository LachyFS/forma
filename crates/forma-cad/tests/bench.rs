//! Regeneration timings (no thresholds). Run:
//! cargo test --release -p forma-cad --test bench -- --ignored --nocapture
use forma_cad::{RegenCache, RegenRequest, regenerate};
use forma_core::*;
use glam::{Vec2, Vec3};
use std::time::Instant;

fn primitive(shape: PrimitiveShape, operation: FeatureOperation) -> FeatureKind {
    FeatureKind::Primitive(PrimitiveFeature {
        shape,
        placement: Placement::default(),
        operation,
    })
}

fn regen(label: &str, part: &PartData, cache: &mut RegenCache) {
    let request = RegenRequest {
        part_id: part.id,
        part: part.clone(),
        skip: Vec::new(),
    };
    let result = regenerate(&request, cache, &mut |_| {});
    assert!(result.issues.is_empty(), "{label}: {:?}", result.issues);
    println!(
        "{label:<44} total {:>7.1} ms · evaluate {:>7.1} · tessellate {:>6.1} · reused {} · faces {:>3} · triangles {:>6}",
        result.stats.total_ms,
        result.stats.evaluate_ms,
        result.stats.tessellate_ms,
        result.stats.reused_features,
        result.stats.brep_faces,
        result.stats.triangles
    );
}

#[test]
#[ignore = "timing report"]
fn regeneration_timings() {
    let mut cache = RegenCache::default();
    let mut part = PartData::new(1, "Box");
    part.push_feature(
        "Box",
        primitive(
            PrimitiveShape::Box {
                size: Vec3::splat(2.0),
            },
            FeatureOperation::Join,
        ),
    );
    regen("box", &part, &mut cache);

    let mut plate = PartData::new(2, "Plate");
    plate.push_feature(
        "Plate",
        primitive(
            PrimitiveShape::Box {
                size: Vec3::new(10.0, 1.0, 6.0),
            },
            FeatureOperation::Join,
        ),
    );
    let mut sketch = Sketch::new(SketchPlane {
        origin: Vec3::new(0.0, 0.6, 0.0),
        ..SketchPlane::ground()
    });
    for i in 0..4 {
        for j in 0..3 {
            sketch.entities.push(
                SketchCurve::Circle {
                    center: Vec2::new(-3.75 + 2.5 * i as f32, -2.0 + 2.0 * j as f32),
                    radius: 0.4,
                }
                .into(),
            );
        }
    }
    plate.push_feature(
        "12 holes",
        FeatureKind::Extrude(ExtrudeFeature {
            sketch,
            profiles: Vec::new(),
            distance: 2.0,
            extent: ExtrudeExtent::OneSide,
            flip: true,
            operation: FeatureOperation::Cut,
        }),
    );
    regen(
        "plate 10x1x6 minus 12 holes (one feature)",
        &plate,
        &mut cache,
    );
    let pull = plate.push_feature(
        "Pull",
        FeatureKind::PushPull(PushPullFeature {
            face: FaceRef {
                point: Vec3::new(4.9, 0.5, 2.9),
                normal: Vec3::Y,
            },
            distance: 0.5,
        }),
    );
    regen("+ push/pull top face (prefix cached)", &plate, &mut cache);
    plate.edit_feature(pull, |feature| {
        if let FeatureKind::PushPull(p) = &mut feature.kind {
            p.distance = 0.75;
        }
    });
    regen(
        "edit push/pull distance (prefix cached)",
        &plate,
        &mut cache,
    );
    plate.quality = TessellationQuality::Fine;
    plate.restamp();
    regen(
        "same plate, fine tessellation",
        &plate,
        &mut RegenCache::default(),
    );

    let mut cylinder = PartData::new(3, "Cylinder");
    cylinder.push_feature(
        "Cylinder",
        primitive(
            PrimitiveShape::Cylinder {
                radius: 1.0,
                height: 2.0,
            },
            FeatureOperation::Join,
        ),
    );
    regen("cylinder (medium)", &cylinder, &mut cache);

    let mut mesh = Mesh::primitive(Primitive::Cube);
    mesh.subdivide();
    mesh.subdivide();
    mesh.subdivide();
    let converted = forma_cad::part_from_mesh("Subdivided cube", &mesh, None);
    regen(
        &format!("mesh to part ({} non-planar quads)", mesh.faces.len()),
        &converted,
        &mut cache,
    );

    let started = Instant::now();
    let text = forma_cad::export_step(&[forma_cad::StepPart {
        name: "Plate".into(),
        part: plate.clone(),
        transform: Mat4::IDENTITY,
    }])
    .unwrap();
    println!(
        "{:<44} {:>7.1} ms ({} KiB)",
        "STEP export of the plate",
        started.elapsed().as_secs_f64() * 1e3,
        text.len() / 1024
    );
    let started = Instant::now();
    let imported = forma_cad::import_step("Plate", &text).unwrap();
    println!(
        "{:<44} {:>7.1} ms",
        "STEP parse and check",
        started.elapsed().as_secs_f64() * 1e3
    );
    regen("imported STEP body", &imported, &mut cache);
}

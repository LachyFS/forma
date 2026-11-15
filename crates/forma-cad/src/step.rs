//! STEP (ISO 10303-21) import and export, and part → mesh baking.
//!
//! Forma works in metres; truck writes STEP in millimetres and reads
//! coordinates without units, so lengths are scaled at this boundary.
//! Exports bake intersection curves first (truck-stepio 0.3 writes raw
//! boolean results that do not read back) and use a fixed header timestamp
//! so identical parts give identical files.
use crate::KernelError;
use crate::body::{Body, Geometry, StepShell, guard};
use crate::boolean::bake_intersection_curves;
use crate::features::{RegenCache, evaluate_part};
use crate::geom::matrix_d;
use crate::protocol::{RegenStats, StepPart};
use crate::tessellate::editable_mesh;
use forma_core::{BaseBody, BaseBodyFeature, FeatureKind, FeatureOperation, Mesh, PartData};
use glam::DMat4;
use truck_modeling::{Solid, Transformed, builder};

/// Header timestamp for every export, so output is reproducible.
const STEP_TIMESTAMP: &str = "2000-01-01T00:00:00";

/// Metres per STEP length unit, from the file's unit declaration.
/// Files without a recognizable declaration are assumed to be millimetres.
fn metres_per_unit(text: &str) -> f64 {
    let compact: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect();
    if compact.contains("'INCH'") {
        0.0254
    } else if compact.contains("'FOOT'") {
        0.3048
    } else if compact.contains("SI_UNIT(.MILLI.,.METRE.)") {
        0.001
    } else if compact.contains("SI_UNIT(.CENTI.,.METRE.)") {
        0.01
    } else if compact.contains("SI_UNIT(.KILO.,.METRE.)") {
        1000.0
    } else if compact.contains("SI_UNIT($,.METRE.)") {
        1.0
    } else {
        0.001
    }
}

fn parse_shells(text: &str) -> Result<Vec<StepShell>, KernelError> {
    use truck_stepio::r#in::{Table, ruststep};
    let invalid = |detail: String| {
        KernelError::new(
            format!("This STEP file could not be read ({detail})"),
            "Export it again as STEP AP203 or AP214 with solid bodies",
        )
    };
    let exchange = ruststep::parser::parse(text).map_err(|error| invalid(format!("{error:?}")))?;
    let data = exchange
        .data
        .first()
        .ok_or_else(|| invalid("no DATA section".into()))?;
    let table = Table::from_data_section(data);
    let mut ids: Vec<_> = table.shell.keys().copied().collect();
    ids.sort_unstable();
    if ids.is_empty() {
        return Err(invalid("it contains no shells".into()));
    }
    ids.into_iter()
        .map(|id| {
            let compressed = table
                .to_compressed_shell(&table.shell[&id])
                .map_err(|error| invalid(format!("shell #{id}: {error}")))?;
            truck_topology::Shell::extract(compressed)
                .map_err(|error| invalid(format!("shell #{id}: {error:?}")))
        })
        .collect()
}

/// The body of an embedded STEP document, scaled to metres.
pub(crate) fn step_body(text: &str) -> Result<Body, KernelError> {
    guard(|| {
        let scale = metres_per_unit(text);
        let shells = parse_shells(text)?;
        let matrix = truck_modeling::Matrix4::from_scale(scale);
        let shells = shells
            .into_iter()
            .map(|shell| {
                shell.mapped(
                    move |p: &truck_modeling::Point3| {
                        truck_modeling::Point3::new(p.x * scale, p.y * scale, p.z * scale)
                    },
                    move |curve: &truck_stepio::r#in::alias::Curve3D| curve.transformed(matrix),
                    move |surface: &truck_stepio::r#in::alias::Surface| surface.transformed(matrix),
                )
            })
            .collect();
        Ok(Body {
            geometry: Geometry::Step(shells),
            min_radius: f64::INFINITY,
        })
    })
}

/// Read a STEP document into a new part with one STEP base body.
pub fn import_step(name: &str, text: &str) -> Result<PartData, KernelError> {
    if text.len() > forma_core::MAX_PART_STEP_BYTES {
        return Err(KernelError::new(
            "The STEP file is larger than 32 MiB",
            "Simplify the model or split it into several files",
        ));
    }
    let body = step_body(text)?;
    if body.face_count() == 0 {
        return Err(KernelError::new("The STEP file contains no faces", ""));
    }
    let mut part = PartData::new(0, name);
    part.push_feature(
        "STEP body",
        FeatureKind::BaseBody(BaseBodyFeature {
            body: BaseBody::Step {
                text: text.to_owned(),
            },
            operation: FeatureOperation::Join,
        }),
    );
    Ok(part)
}

#[cfg(test)]
fn evaluated_body(part: &PartData) -> Result<Body, KernelError> {
    evaluated_body_cached(part, &mut RegenCache::default())
}

fn evaluated_body_cached(part: &PartData, cache: &mut RegenCache) -> Result<Body, KernelError> {
    let mut stats = RegenStats::default();
    let (body, issues) = evaluate_part(part.id, part, &[], cache, &mut |_| {}, &mut stats);
    match body {
        Some(body) => Ok(body),
        None => Err(issues
            .into_iter()
            .next()
            .map(|issue| KernelError::new(issue.message, issue.suggestion))
            .unwrap_or_else(|| {
                KernelError::new("The part has no body", "Add or unsuppress a feature")
            })),
    }
}

/// Export placed parts as one STEP document (millimetres).
pub fn export_step(parts: &[StepPart]) -> Result<String, KernelError> {
    export_step_cached(parts, &mut RegenCache::default())
}

/// [`export_step`] reusing evaluated feature prefixes from `cache`.
pub(crate) fn export_step_cached(
    parts: &[StepPart],
    cache: &mut RegenCache,
) -> Result<String, KernelError> {
    if parts.is_empty() {
        return Err(KernelError::new("Select a part to export", ""));
    }
    let mut solids: Vec<Solid> = Vec::new();
    for placed in parts {
        let body = evaluated_body_cached(&placed.part, cache)?;
        let Geometry::Solid(solid) = &body.geometry else {
            return Err(KernelError::new(
                format!("{} is an imported STEP body", placed.name),
                "Export the original STEP file, or convert the part to a mesh",
            ));
        };
        let solid = guard(|| {
            let copy = builder::clone(solid);
            let (_, failed) = bake_intersection_curves(&copy, body.tolerance(2.0e-4, 0.05))?;
            if failed > 0 {
                return Err(KernelError::new(
                    format!("{} has curves that could not be exported", placed.name),
                    "Try a slightly different size for the last boolean",
                ));
            }
            let to_millimetres =
                DMat4::from_scale(glam::DVec3::splat(1000.0)) * placed.transform.as_dmat4();
            let mut moved = builder::transformed(&copy, matrix_d(to_millimetres));
            if to_millimetres.determinant() < 0.0 {
                moved.not();
            }
            Ok(moved)
        })?;
        solids.push(solid);
    }
    guard(|| {
        use truck_stepio::out;
        let compressed: Vec<_> = solids.iter().map(|solid| solid.compress()).collect();
        let models: out::StepModels<_, _, _> = compressed.iter().collect();
        let name = parts
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Ok(out::CompleteStepDisplay::new(
            models,
            out::StepHeaderDescriptor {
                file_name: name,
                time_stamp: STEP_TIMESTAMP.into(),
                authors: Vec::new(),
                organization: Vec::new(),
                organization_system: "Forma".into(),
                authorization: String::new(),
            },
        )
        .to_string())
    })
}

/// Bake a part into an editable polygon mesh in part space.
pub fn part_to_mesh(part: &PartData) -> Result<Mesh, KernelError> {
    part_to_mesh_cached(part, &mut RegenCache::default())
}

pub(crate) fn part_to_mesh_cached(
    part: &PartData,
    cache: &mut RegenCache,
) -> Result<Mesh, KernelError> {
    let body = evaluated_body_cached(part, cache)?;
    editable_mesh(&body, part.quality)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::{Placement, PrimitiveFeature, PrimitiveShape};
    use glam::{Mat4, Vec3};

    fn cylinder_part() -> PartData {
        let mut part = PartData::new(1, "Pin");
        part.push_feature(
            "Cylinder",
            FeatureKind::Primitive(PrimitiveFeature {
                shape: PrimitiveShape::Cylinder {
                    radius: 0.5,
                    height: 2.0,
                },
                placement: Placement::default(),
                operation: FeatureOperation::Join,
            }),
        );
        part
    }

    #[test]
    fn step_round_trips_in_metres_with_a_fixed_header() {
        let placed = StepPart {
            name: "Pin".into(),
            part: cylinder_part(),
            transform: Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)),
        };
        let text = export_step(std::slice::from_ref(&placed)).unwrap();
        assert!(text.contains(STEP_TIMESTAMP));
        assert_eq!(text, export_step(&[placed]).unwrap(), "deterministic");
        let part = import_step("Pin", &text).unwrap();
        let body = evaluated_body(&part).unwrap();
        let mesh = body.trimesh(0.005).unwrap();
        let bounds = mesh.bounds();
        // Placed one metre up, read back from millimetres into metres.
        assert!(
            (bounds.min.y - 0.0).abs() < 1.0e-3 && (bounds.max.y - 2.0).abs() < 1.0e-3,
            "{bounds:?}"
        );
        let expected = std::f64::consts::PI * 0.25 * 2.0;
        assert!((mesh.volume() - expected).abs() < 0.02, "{}", mesh.volume());
    }

    #[test]
    fn units_are_detected_from_the_header() {
        assert_eq!(
            metres_per_unit("( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) )"),
            0.001
        );
        assert_eq!(metres_per_unit("SI_UNIT ( $ , .METRE. )"), 1.0);
        assert_eq!(metres_per_unit("CONVERSION_BASED_UNIT('INCH',#5)"), 0.0254);
        assert!(import_step("Bad", "not a step file").is_err());
    }

    #[test]
    fn booleans_export_after_baking() {
        let mut part = cylinder_part();
        part.push_feature(
            "Box",
            FeatureKind::Primitive(PrimitiveFeature {
                shape: PrimitiveShape::Box {
                    size: Vec3::new(2.0, 1.0, 2.0),
                },
                placement: Placement::default(),
                operation: FeatureOperation::Cut,
            }),
        );
        let text = export_step(&[StepPart {
            name: "Cut".into(),
            part,
            transform: Mat4::IDENTITY,
        }])
        .unwrap();
        let back = import_step("Cut", &text).unwrap();
        let body = evaluated_body(&back).unwrap();
        let volume = body.trimesh(0.005).unwrap().volume();
        // Two cylinder stubs of 0.5 m each remain.
        let expected = std::f64::consts::PI * 0.25 * 1.0;
        assert!((volume - expected).abs() < 0.03, "{volume}");
    }
}

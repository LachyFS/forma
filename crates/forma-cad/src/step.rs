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

use truck_stepio::r#in::ruststep::ast::{DataSection, EntityInstance, Name, Parameter, Record};

/// Metres per STEP length unit, read from the parsed unit entities: the
/// length unit assigned in a `GLOBAL_UNIT_ASSIGNED_CONTEXT` if there is one,
/// otherwise the first length unit in the file. `SI_UNIT` prefixes and
/// `CONVERSION_BASED_UNIT` factors are evaluated. Text elsewhere in the file
/// (names, descriptions, the header) never affects the result. Files that
/// declare no length unit are assumed to be millimetres.
fn declared_metres_per_unit(data: &DataSection) -> f64 {
    let by_id: std::collections::HashMap<u64, &EntityInstance> = data
        .entities
        .iter()
        .map(|instance| match instance {
            EntityInstance::Simple { id, .. } | EntityInstance::Complex { id, .. } => {
                (*id, instance)
            }
        })
        .collect();
    let lookup = |parameter: &Parameter| match parameter {
        Parameter::Ref(Name::Entity(id)) => by_id.get(id).copied(),
        _ => None,
    };
    // Units the representation context assigns, in order.
    let assigned: Vec<&EntityInstance> = data
        .entities
        .iter()
        .flat_map(records)
        .filter(|record| {
            record
                .name
                .eq_ignore_ascii_case("GLOBAL_UNIT_ASSIGNED_CONTEXT")
        })
        .flat_map(|record| parameters(&record.parameter))
        .flat_map(|parameter| match parameter {
            Parameter::List(units) => units.iter().filter_map(&lookup).collect::<Vec<_>>(),
            other => lookup(other).into_iter().collect(),
        })
        .collect();
    let candidates: Vec<&EntityInstance> = if assigned.is_empty() {
        data.entities.iter().collect()
    } else {
        assigned
    };
    candidates
        .into_iter()
        .filter(|instance| {
            records(instance)
                .iter()
                .any(|record| record.name.eq_ignore_ascii_case("LENGTH_UNIT"))
                || is_metre_si_unit(instance)
        })
        .find_map(|instance| unit_factor(instance, &by_id, 0))
        .unwrap_or(0.001)
}

fn records(instance: &EntityInstance) -> Vec<&Record> {
    match instance {
        EntityInstance::Simple { record, .. } => vec![record],
        EntityInstance::Complex { subsuper, .. } => subsuper.0.iter().collect(),
    }
}

fn parameters(parameter: &Parameter) -> Vec<&Parameter> {
    match parameter {
        Parameter::List(list) => list.iter().collect(),
        other => vec![other],
    }
}

fn enumeration(parameter: &Parameter) -> Option<String> {
    match parameter {
        Parameter::Enumeration(value) => Some(value.trim_matches('.').to_ascii_uppercase()),
        _ => None,
    }
}

/// A simple `SI_UNIT(*, prefix, .METRE.)` instance (no LENGTH_UNIT record).
fn is_metre_si_unit(instance: &EntityInstance) -> bool {
    match instance {
        EntityInstance::Simple { record, .. } => {
            record.name.eq_ignore_ascii_case("SI_UNIT")
                && parameters(&record.parameter)
                    .last()
                    .and_then(|p| enumeration(p))
                    .is_some_and(|name| name == "METRE")
        }
        EntityInstance::Complex { .. } => false,
    }
}

/// Metres per unit for an SI or conversion-based length unit entity.
fn unit_factor(
    instance: &EntityInstance,
    by_id: &std::collections::HashMap<u64, &EntityInstance>,
    depth: usize,
) -> Option<f64> {
    if depth > 8 {
        return None;
    }
    for record in records(instance) {
        let params = parameters(&record.parameter);
        if record.name.eq_ignore_ascii_case("SI_UNIT") {
            // (prefix, name) in complex instances; (dimensions, prefix, name) simple.
            let name = params.last().and_then(|p| enumeration(p))?;
            if name != "METRE" {
                return None;
            }
            let prefix = params
                .len()
                .checked_sub(2)
                .and_then(|i| enumeration(params[i]));
            return Some(match prefix.as_deref() {
                None => 1.0,
                Some("EXA") => 1.0e18,
                Some("PETA") => 1.0e15,
                Some("TERA") => 1.0e12,
                Some("GIGA") => 1.0e9,
                Some("MEGA") => 1.0e6,
                Some("KILO") => 1.0e3,
                Some("HECTO") => 1.0e2,
                Some("DECA") => 1.0e1,
                Some("DECI") => 1.0e-1,
                Some("CENTI") => 1.0e-2,
                Some("MILLI") => 1.0e-3,
                Some("MICRO") => 1.0e-6,
                Some("NANO") => 1.0e-9,
                Some("PICO") => 1.0e-12,
                Some(_) => return None,
            });
        }
        if record.name.eq_ignore_ascii_case("CONVERSION_BASED_UNIT") {
            // (name, conversion_factor -> LENGTH_MEASURE_WITH_UNIT(value, unit)).
            let measured = params.get(1).and_then(|p| match p {
                Parameter::Ref(Name::Entity(id)) => by_id.get(id).copied(),
                _ => None,
            });
            let evaluated = measured.and_then(|measure| {
                let EntityInstance::Simple { record, .. } = measure else {
                    return None;
                };
                let inner = parameters(&record.parameter);
                let value = match inner.first()? {
                    Parameter::Typed { parameter, .. } => match parameter.as_ref() {
                        Parameter::Real(value) => *value,
                        Parameter::Integer(value) => *value as f64,
                        _ => return None,
                    },
                    Parameter::Real(value) => *value,
                    Parameter::Integer(value) => *value as f64,
                    _ => return None,
                };
                let base = match inner.get(1)? {
                    Parameter::Ref(Name::Entity(id)) => {
                        unit_factor(by_id.get(id)?, by_id, depth + 1)?
                    }
                    _ => return None,
                };
                let factor = value * base;
                (factor.is_finite() && factor > 0.0).then_some(factor)
            });
            if evaluated.is_some() {
                return evaluated;
            }
            // Fall back to well-known unit names.
            let name = match params.first() {
                Some(Parameter::String(name)) => name.to_ascii_uppercase(),
                _ => return None,
            };
            return match name.trim() {
                "INCH" | "INCHES" => Some(0.0254),
                "FOOT" | "FEET" => Some(0.3048),
                "YARD" => Some(0.9144),
                "MILE" => Some(1609.344),
                "MIL" | "THOU" => Some(0.0000254),
                _ => None,
            };
        }
    }
    None
}

fn parse_data(text: &str) -> Result<DataSection, KernelError> {
    use truck_stepio::r#in::ruststep;
    let exchange =
        ruststep::parser::parse(text).map_err(|error| step_error(format!("{error:?}")))?;
    exchange
        .data
        .into_iter()
        .next()
        .ok_or_else(|| step_error("no DATA section".into()))
}

fn step_error(detail: String) -> KernelError {
    KernelError::new(
        format!("This STEP file could not be read ({detail})"),
        "Export it again as STEP AP203 or AP214 with solid bodies",
    )
}

fn parse_shells(data: &DataSection) -> Result<Vec<StepShell>, KernelError> {
    use truck_stepio::r#in::Table;
    let table = Table::from_data_section(data);
    let mut ids: Vec<_> = table.shell.keys().copied().collect();
    ids.sort_unstable();
    if ids.is_empty() {
        return Err(step_error("it contains no shells".into()));
    }
    ids.into_iter()
        .map(|id| {
            let compressed = table
                .to_compressed_shell(&table.shell[&id])
                .map_err(|error| step_error(format!("shell #{id}: {error}")))?;
            truck_topology::Shell::extract(compressed)
                .map_err(|error| step_error(format!("shell #{id}: {error:?}")))
        })
        .collect()
}

/// The body of an embedded STEP document, scaled to metres.
pub(crate) fn step_body(text: &str) -> Result<Body, KernelError> {
    guard(|| {
        let data = parse_data(text)?;
        let scale = declared_metres_per_unit(&data);
        let shells = parse_shells(&data)?;
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
    fn units_come_from_unit_entities_not_from_names() {
        // Review case: Forma writes the object name into FILE_NAME, so names
        // like "Foot" or "inch" must not change the import scale.
        for name in ["Bracket", "Foot", "inch", "MILLI METRE"] {
            let mut part = cylinder_part();
            part.name = name.into();
            let text = export_step(&[StepPart {
                name: name.into(),
                part,
                transform: Mat4::IDENTITY,
            }])
            .unwrap();
            assert!(text.contains(&format!("FILE_NAME('{name}'")));
            let back = import_step(name, &text).unwrap();
            let bounds = evaluated_body(&back)
                .unwrap()
                .trimesh(0.01)
                .unwrap()
                .bounds();
            let height = bounds.max.y - bounds.min.y;
            assert!((height - 2.0).abs() < 1.0e-3, "{name}: height {height}");
        }
        assert!(import_step("Bad", "not a step file").is_err());
    }

    #[test]
    fn declared_units_scale_imports() {
        let exported = export_step(&[StepPart {
            name: "Pin".into(),
            part: cylinder_part(),
            transform: Mat4::IDENTITY,
        }])
        .unwrap();
        let millimetres = "( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) )";
        assert!(exported.contains(millimetres));
        let height = |text: &str| {
            let body = step_body(text).unwrap();
            let bounds = body.trimesh(0.05).unwrap().bounds();
            bounds.max.y - bounds.min.y
        };
        // Conversion-based inches: 2000 file units of 25.4 mm each.
        let inches = exported
            .replace(
                millimetres,
                "( CONVERSION_BASED_UNIT('INCH',#9001) LENGTH_UNIT() NAMED_UNIT(#9002) )",
            )
            .replace(
                "ENDSEC;\nEND-ISO",
                "#9001 = LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#9003);\n\
                 #9002 = DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.);\n\
                 #9003 = ( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );\n\
                 ENDSEC;\nEND-ISO",
            );
        assert!(
            (height(&inches) - 2000.0 * 0.0254).abs() < 1.0e-2,
            "{}",
            height(&inches)
        );
        // Plain metres and centimetres.
        let metres = exported.replace(
            millimetres,
            "( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.) )",
        );
        assert!((height(&metres) - 2000.0).abs() < 1.0e-3);
        let centimetres = exported.replace(
            millimetres,
            "( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.CENTI.,.METRE.) )",
        );
        assert!((height(&centimetres) - 20.0).abs() < 1.0e-3);
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

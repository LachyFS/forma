//! Booleans with the robustness layer truck-shapeops 0.4 needs in practice.
//!
//! Measured hazards and the guard for each:
//! * coplanar or flush faces make truck return `None` → retry with a finer
//!   tracing tolerance, then with the tool inflated by a tiny overshoot so
//!   no faces coincide;
//! * disjoint operands give an empty subtraction → bounding-box pre-check;
//! * a tool strictly inside a solid panics ("shell is not oriented and
//!   closed") → mesh-level containment test, building the void directly;
//! * tolerances below 1e-6 panic → scaled to part size and clamped;
//! * silently wrong results → volume sanity checks on every result;
//! * transforms of boolean results can loop forever → intersection curves
//!   are baked into B-splines on a deep copy before any transform.
use crate::KernelError;
use crate::body::{Body, Geometry, guard};
use crate::geom::{TriMesh, matrix_d};
use glam::{DMat4, DVec3};
use truck_modeling::{
    BSplineCurve, BoundedCurve, Curve, KnotVec, ParameterDivision1D, Solid, builder,
};
use truck_topology::shell::ShellCondition;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoolOp {
    Union,
    Subtract,
    Intersect,
}

impl BoolOp {
    fn verb(self) -> &'static str {
        match self {
            Self::Union => "Join",
            Self::Subtract => "Cut",
            Self::Intersect => "Intersect",
        }
    }
}

/// Combine `a` with tool `b`.
pub(crate) fn boolean(a: &Body, b: &Body, op: BoolOp) -> Result<Body, KernelError> {
    let (Some(solid_a), Some(solid_b)) = (a.as_solid(), b.as_solid()) else {
        return Err(KernelError::new(
            format!(
                "{} is not available for imported STEP bodies yet",
                op.verb()
            ),
            "Convert the STEP part to a mesh and back to a part to combine it",
        ));
    };
    let min_radius = a.min_radius.min(b.min_radius);
    let check_tolerance = a.tolerance(3.0e-3, 0.35).min(b.tolerance(3.0e-3, 0.35));
    let mesh_a = a.trimesh(check_tolerance)?;
    let mesh_b = b.trimesh(check_tolerance)?;
    let (bounds_a, bounds_b) = (mesh_a.bounds(), mesh_b.bounds());
    let size = bounds_a.union(&bounds_b).diagonal().max(1.0e-6);
    let (volume_a, volume_b) = (mesh_a.volume(), mesh_b.volume());
    if bounds_a.disjoint(&bounds_b, check_tolerance) {
        return separated(a, b, solid_a, solid_b, op, min_radius);
    }
    // Intersection tracing tolerance: fine enough for the smallest curvature,
    // never below truck's hard 1e-6 limit.
    let mut base = size * 0.01;
    if min_radius.is_finite() {
        base = base.min(min_radius * 0.2);
    }
    let base = base.clamp((size * 1.0e-4).max(2.0e-6), size * 0.05);
    let probes = Probes::new(&mesh_a, &mesh_b, op, size);
    let check = |result: &Solid| {
        sane(
            result,
            op,
            volume_a,
            volume_b,
            check_tolerance,
            min_radius,
            &probes,
        )
    };

    // Nested bounds: truck panics or silently returns an operand unchanged
    // when one surface never touches the other, so classify those first.
    let nested = bounds_a.contains_box(&bounds_b, 0.0) || bounds_b.contains_box(&bounds_a, 0.0);
    if nested
        && let Some(body) = containment(a, b, solid_a, solid_b, &mesh_a, &mesh_b, op, min_radius)?
    {
        return Ok(body);
    }
    let mut last_error = None;
    for (attempt, tolerance) in [base, base * 0.3].into_iter().enumerate() {
        match run(solid_a, solid_b, op, tolerance) {
            Ok(Some(result)) => match check(&result) {
                Ok(()) => return Ok(Body::solid(result, min_radius)),
                Err(error) => last_error = Some(error),
            },
            Ok(None) => {}
            Err(error) => last_error = Some(error),
        }
        if attempt == 0 && !nested {
            // A tool that never touches the other surface makes truck panic or
            // return nothing; classify it from the meshes instead.
            if let Some(body) =
                containment(a, b, solid_a, solid_b, &mesh_a, &mesh_b, op, min_radius)?
            {
                return Ok(body);
            }
        }
    }
    // Coplanar contact: inflate the tool by a tiny overshoot about its center
    // so coincident faces separate, then trace again.
    let center = bounds_b.center();
    for (overshoot, scale) in [(2.0e-4, 1.0), (1.5e-3, 0.3)] {
        let factor = 1.0 + overshoot * size / bounds_b.diagonal().max(1.0e-9);
        let inflate = DMat4::from_translation(center)
            * DMat4::from_scale(DVec3::splat(factor))
            * DMat4::from_translation(-center);
        let Ok(tool) = safe_transform(solid_b, inflate, check_tolerance) else {
            continue;
        };
        match run(solid_a, &tool, op, base * scale) {
            Ok(Some(result)) => match check(&result) {
                Ok(()) => return Ok(Body::solid(result, min_radius)),
                Err(error) => last_error = Some(error),
            },
            Ok(None) => {}
            Err(error) => last_error = Some(error),
        }
    }
    let detail = last_error
        .map(|error| format!(" ({})", error.message))
        .unwrap_or_default();
    Err(KernelError::new(
        format!(
            "{} failed: the kernel could not intersect these bodies{detail}",
            op.verb()
        ),
        "Faces that touch exactly or meet tangentially are hard for the kernel · \
         Move one body slightly so faces clearly overlap or clearly separate",
    ))
}

fn run(a: &Solid, b: &Solid, op: BoolOp, tolerance: f64) -> Result<Option<Solid>, KernelError> {
    let tolerance = tolerance.max(2.0e-6);
    guard(|| {
        Ok(match op {
            BoolOp::Union => truck_shapeops::or(a, b, tolerance),
            BoolOp::Intersect => truck_shapeops::and(a, b, tolerance),
            BoolOp::Subtract => {
                // A − B = A ∩ ¬B. `not` flips orientation flags on the clone's
                // face handles only; the caller's solid is untouched.
                let mut inverse = b.clone();
                inverse.not();
                truck_shapeops::and(a, &inverse, tolerance)
            }
        })
    })
}

/// Reject results that are empty, open or have implausible volumes.
/// Points whose membership in a correct result is known in advance: just
/// inside the tool's surface (and inside the target), and for intersections
/// just inside the target but outside the tool. They catch results truck
/// returns with plausible volume but the wrong regions, such as a hole that
/// is imprinted on a face but never cut.
struct Probes {
    /// Points that must be inside the result.
    inside: Vec<DVec3>,
    /// Points that must be outside the result.
    outside: Vec<DVec3>,
}

impl Probes {
    fn new(a: &TriMesh, b: &TriMesh, op: BoolOp, size: f64) -> Self {
        let inset = size * 2.0e-3;
        let sample = |mesh: &TriMesh| -> Vec<DVec3> {
            let count = mesh.triangles.len();
            let step = (count / 48).max(1);
            (0..count)
                .step_by(step)
                .filter_map(|i| {
                    let [p, q, r] = mesh.triangle(i);
                    let normal = (q - p).cross(r - p);
                    // Skip slivers whose centroid is too close to an edge.
                    (normal.length() > inset * inset * 4.0)
                        .then(|| (p + q + r) / 3.0 - normal.normalize() * inset)
                })
                .collect()
        };
        let in_b: Vec<DVec3> = sample(b).into_iter().filter(|p| b.contains(*p)).collect();
        let mut probes = Self {
            inside: Vec::new(),
            outside: Vec::new(),
        };
        match op {
            BoolOp::Union => probes.inside = in_b,
            BoolOp::Subtract => {
                probes.outside = in_b.into_iter().filter(|p| a.contains(*p)).collect()
            }
            BoolOp::Intersect => {
                probes.inside = in_b.into_iter().filter(|p| a.contains(*p)).collect();
                probes.outside = sample(a)
                    .into_iter()
                    .filter(|p| a.contains(*p) && !b.contains(*p))
                    .collect();
            }
        }
        probes
    }

    /// The share of probes the result misclassifies.
    fn errors(&self, result: &TriMesh) -> (usize, usize) {
        let wrong = self.inside.iter().filter(|p| !result.contains(**p)).count()
            + self.outside.iter().filter(|p| result.contains(**p)).count();
        (wrong, self.inside.len() + self.outside.len())
    }
}

/// A face whose largest triangle lacks material behind it or has material in
/// front of it, if any.
fn hollow_face(mesh: &TriMesh) -> Option<u32> {
    let size = mesh.bounds().diagonal();
    let mut largest: std::collections::BTreeMap<u32, (f64, usize)> = Default::default();
    for (index, &face) in mesh.faces.iter().enumerate() {
        let [p, q, r] = mesh.triangle(index);
        let area = (q - p).cross(r - p).length();
        let entry = largest.entry(face).or_insert((0.0, index));
        if area > entry.0 {
            *entry = (area, index);
        }
    }
    largest.into_iter().find_map(|(face, (area, index))| {
        let [p, q, r] = mesh.triangle(index);
        let normal = (q - p).cross(r - p).normalize_or_zero();
        // Stay well inside small triangles so the probe sees this face.
        let inset = (size * 2.0e-4).min(area.sqrt() * 0.05);
        if normal == DVec3::ZERO || inset <= 0.0 {
            return None;
        }
        let center = (p + q + r) / 3.0;
        let behind = mesh.contains(center - normal * inset);
        let ahead = mesh.contains(center + normal * inset);
        (!behind || ahead).then_some(face)
    })
}

fn sane(
    result: &Solid,
    op: BoolOp,
    volume_a: f64,
    volume_b: f64,
    tolerance: f64,
    min_radius: f64,
    probes: &Probes,
) -> Result<(), KernelError> {
    let shells = result.boundaries();
    if shells.iter().all(|shell| shell.is_empty()) {
        return Err(KernelError::new("the result was empty", ""));
    }
    if shells
        .iter()
        .any(|shell| shell.shell_condition() != ShellCondition::Closed)
    {
        return Err(KernelError::new("the result was not a closed solid", ""));
    }
    let body = Body::solid(result.clone(), min_radius);
    let mesh = body.trimesh(tolerance)?;
    let volume = mesh.volume();
    // Every face must have material just behind it and none just in front:
    // truck sometimes keeps a zero-thickness lid where a tool's cap was flush.
    if let Some(face) = hollow_face(&mesh) {
        return Err(KernelError::new(
            format!("face {face} of the result does not bound material"),
            "",
        ));
    }
    // Allow a few probes sitting on tessellation error near tangent surfaces.
    let (wrong, total) = probes.errors(&mesh);
    if total > 0 && wrong * 10 > total {
        return Err(KernelError::new(
            format!("the result missed part of the operation ({wrong} of {total} checks)"),
            "",
        ));
    }
    let slack = 0.03 * volume_a.max(volume_b) + 1.0e-12;
    let plausible = match op {
        BoolOp::Union => {
            volume >= volume_a.max(volume_b) - slack && volume <= volume_a + volume_b + slack
        }
        BoolOp::Subtract => {
            volume <= volume_a + slack && volume >= volume_a - volume_b - slack && volume > 0.0
        }
        BoolOp::Intersect => volume <= volume_a.min(volume_b) + slack && volume > 0.0,
    };
    if !plausible || !volume.is_finite() {
        return Err(KernelError::new(
            format!("the result volume {volume:.6} was inconsistent"),
            "",
        ));
    }
    Ok(())
}

/// Operands whose bounds do not overlap.
fn separated(
    a: &Body,
    b: &Body,
    solid_a: &Solid,
    solid_b: &Solid,
    op: BoolOp,
    min_radius: f64,
) -> Result<Body, KernelError> {
    match op {
        BoolOp::Subtract => Ok(a.clone()),
        BoolOp::Union => Ok(Body::solid(
            Solid::new(
                solid_a
                    .boundaries()
                    .iter()
                    .chain(solid_b.boundaries())
                    .cloned()
                    .collect(),
            ),
            min_radius,
        )),
        BoolOp::Intersect => {
            let _ = b;
            Err(KernelError::new(
                "Intersect left nothing: the bodies do not overlap",
                "Move the bodies so they overlap, or use Join",
            ))
        }
    }
}

/// Handle tools that are fully inside, fully outside or fully containing the
/// target without their surfaces crossing. `None` means the surfaces do cross.
#[allow(clippy::too_many_arguments)]
fn containment(
    a: &Body,
    b: &Body,
    solid_a: &Solid,
    solid_b: &Solid,
    mesh_a: &TriMesh,
    mesh_b: &TriMesh,
    op: BoolOp,
    min_radius: f64,
) -> Result<Option<Body>, KernelError> {
    if mesh_a.edges_cross(mesh_b) || mesh_b.edges_cross(mesh_a) {
        return Ok(None);
    }
    let sample = |mesh: &TriMesh| {
        let [p, q, r] = mesh.triangle(0);
        (p + q + r) / 3.0
    };
    let b_in_a = mesh_a.contains(sample(mesh_b));
    let a_in_b = !b_in_a && mesh_b.contains(sample(mesh_a));
    Ok(Some(match (b_in_a, a_in_b) {
        (true, _) => match op {
            BoolOp::Union => a.clone(),
            BoolOp::Intersect => b.clone(),
            BoolOp::Subtract => {
                // A closed void: the tool's shells, inverted, inside the target.
                let mut shells = solid_a.boundaries().clone();
                let mut inverse = solid_b.clone();
                inverse.not();
                shells.extend(inverse.into_boundaries());
                Body::solid(Solid::new(shells), min_radius)
            }
        },
        (false, true) => match op {
            BoolOp::Union => b.clone(),
            BoolOp::Intersect => a.clone(),
            BoolOp::Subtract => {
                return Err(KernelError::new(
                    "Cut removes the whole body: the tool encloses it",
                    "Make the tool smaller or move it so it only overlaps part of the body",
                ));
            }
        },
        (false, false) => return separated(a, b, solid_a, solid_b, op, min_radius).map(Some),
    }))
}

/// Transform any solid safely: deep-copy it, replace intersection curves by
/// B-splines through exact points, then transform the copy.
pub(crate) fn safe_transform(
    solid: &Solid,
    matrix: DMat4,
    tolerance: f64,
) -> Result<Solid, KernelError> {
    let copy = builder::clone(solid);
    let (_, failed) = bake_intersection_curves(&copy, tolerance)?;
    if failed > 0 {
        return Err(KernelError::new(
            "Could not approximate intersection curves before transforming",
            "",
        ));
    }
    guard(|| Ok(builder::transformed(&copy, matrix_d(matrix))))
}

/// Replace `Curve::IntersectionCurve` edges with degree-1 B-splines through
/// points sampled on the exact intersection within `tolerance`. Several
/// truck paths (sweeps, transforms, STEP export) mishandle intersection
/// curves. Edges are shared, so faces on both sides update together; call
/// this on a deep copy. Returns (converted, failed).
pub(crate) fn bake_intersection_curves(
    solid: &Solid,
    tolerance: f64,
) -> Result<(usize, usize), KernelError> {
    guard(|| {
        let mut seen = std::collections::HashSet::new();
        let (mut converted, mut failed) = (0, 0);
        for edge in solid.edge_iter() {
            if !seen.insert(edge.id()) {
                continue;
            }
            let curve = edge.curve();
            if !matches!(curve, Curve::IntersectionCurve(_)) {
                continue;
            }
            let baked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let range = curve.range_tuple();
                let (parameters, points) = curve.parameter_division(range, tolerance.max(2.0e-6));
                if points.len() < 2 {
                    return None;
                }
                let mut knots = Vec::with_capacity(parameters.len() + 2);
                knots.push(parameters[0]);
                knots.extend_from_slice(&parameters);
                knots.push(*parameters.last().unwrap());
                BSplineCurve::try_new(KnotVec::from(knots), points).ok()
            }));
            match baked {
                Ok(Some(curve)) => {
                    edge.set_curve(Curve::BSplineCurve(curve));
                    converted += 1;
                }
                _ => failed += 1,
            }
        }
        Ok((converted, failed))
    })
}

/// Whether a solid contains intersection curves (results of booleans).
pub(crate) fn has_intersection_curves(solid: &Solid) -> bool {
    solid
        .edge_iter()
        .any(|edge| matches!(edge.curve(), Curve::IntersectionCurve(_)))
}

/// Transform a body for use as a tool: freshly built solids transform
/// directly, boolean results are baked first.
pub(crate) fn transform_body(body: &Body, matrix: DMat4) -> Result<Body, KernelError> {
    let Geometry::Solid(solid) = &body.geometry else {
        return Err(KernelError::new(
            "Imported STEP bodies cannot be used as tools yet",
            "Convert the STEP part to a mesh and back to a part first",
        ));
    };
    if matrix == DMat4::IDENTITY {
        return Ok(body.clone());
    }
    // Scale changes curvature; keep the tightest radius conservative.
    let (scale, _, _) = matrix.to_scale_rotation_translation();
    let min_scale = scale.abs().min_element();
    let min_radius = body.min_radius * min_scale;
    let mut transformed = if has_intersection_curves(solid) {
        safe_transform(solid, matrix, body.tolerance(5.0e-4, 0.1))?
    } else {
        guard(|| Ok(builder::transformed(solid, matrix_d(matrix))))?
    };
    if matrix.determinant() < 0.0 {
        // A mirroring transform turns the solid inside out.
        transformed.not();
    }
    Ok(Body::solid(transformed, min_radius))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::primitive;
    use forma_core::PrimitiveShape;
    use glam::Vec3;
    use std::f64::consts::PI;

    fn cube(size: f32) -> Body {
        primitive(&PrimitiveShape::Box {
            size: Vec3::splat(size),
        })
        .unwrap()
    }

    fn moved(body: &Body, offset: DVec3) -> Body {
        transform_body(body, DMat4::from_translation(offset)).unwrap()
    }

    fn volume(body: &Body) -> f64 {
        body.volume(body.tolerance(5.0e-4, 0.1)).unwrap()
    }

    fn cylinder(radius: f32, height: f32) -> Body {
        primitive(&PrimitiveShape::Cylinder { radius, height }).unwrap()
    }

    #[test]
    fn booleans_match_expected_volumes() {
        let base = primitive(&PrimitiveShape::Box {
            size: Vec3::new(2.0, 1.0, 2.0),
        })
        .unwrap();
        let tool = cylinder(0.5, 2.0);
        let cut = boolean(&base, &tool, BoolOp::Subtract).unwrap();
        let expected = 4.0 - PI * 0.25;
        assert!(
            (volume(&cut) - expected).abs() < 0.02,
            "cut {}",
            volume(&cut)
        );
        let joined = boolean(&base, &tool, BoolOp::Union).unwrap();
        let expected = 4.0 + PI * 0.25;
        assert!((volume(&joined) - expected).abs() < 0.03);
        let both = boolean(&base, &tool, BoolOp::Intersect).unwrap();
        assert!((volume(&both) - PI * 0.25).abs() < 0.02);
    }

    #[test]
    fn known_hazards_resolve_or_fail_gracefully() {
        let base = cube(2.0);
        // Disjoint subtraction keeps the target.
        let far = moved(&cube(1.0), DVec3::new(10.0, 0.0, 0.0));
        let kept = boolean(&base, &far, BoolOp::Subtract).unwrap();
        assert!((volume(&kept) - 8.0).abs() < 1.0e-6);
        // Disjoint union keeps both lumps; intersection is a clear error.
        let lumps = boolean(&base, &far, BoolOp::Union).unwrap();
        assert!((volume(&lumps) - 9.0).abs() < 1.0e-6);
        assert!(boolean(&base, &far, BoolOp::Intersect).is_err());
        // A tool strictly inside leaves a void instead of panicking.
        let inner = cylinder(0.3, 0.5);
        let hollow = boolean(&base, &inner, BoolOp::Subtract).unwrap();
        let expected = 8.0 - PI * 0.09 * 0.5;
        assert!(
            (volume(&hollow) - expected).abs() < 0.02,
            "{}",
            volume(&hollow)
        );
        let union = boolean(&base, &inner, BoolOp::Union).unwrap();
        assert!((volume(&union) - 8.0).abs() < 1.0e-6);
        // A tool enclosing the target removes everything: a clear error.
        let error = boolean(&inner, &base, BoolOp::Subtract).unwrap_err();
        assert!(error.message.contains("whole body"), "{error}");
        // Flush caps: a through-hole exactly as tall as the plate.
        let plate = primitive(&PrimitiveShape::Box {
            size: Vec3::new(2.0, 0.5, 2.0),
        })
        .unwrap();
        let flush = cylinder(0.4, 0.5);
        let drilled = boolean(&plate, &flush, BoolOp::Subtract).unwrap();
        let expected = 2.0 - PI * 0.16 * 0.5;
        assert!(
            (volume(&drilled) - expected).abs() < 0.02,
            "{}",
            volume(&drilled)
        );
        // Coplanar faces: two boxes sharing their top and bottom planes.
        let shifted = moved(&cube(2.0), DVec3::new(1.0, 0.0, 0.0));
        match boolean(&base, &shifted, BoolOp::Union) {
            Ok(body) => assert!((volume(&body) - 12.0).abs() < 0.05, "{}", volume(&body)),
            Err(error) => assert!(error.message.contains("Join failed"), "{error}"),
        }
        // A boss sitting exactly on a face.
        let boss = moved(&cube(1.0), DVec3::new(0.0, 1.5, 0.0));
        match boolean(&base, &boss, BoolOp::Union) {
            Ok(body) => assert!((volume(&body) - 9.0).abs() < 0.05, "{}", volume(&body)),
            Err(error) => assert!(error.message.contains("Join failed"), "{error}"),
        }
    }

    #[test]
    fn transformed_boolean_results_stay_valid() {
        let cut = boolean(&cube(2.0), &cylinder(0.5, 3.0), BoolOp::Subtract).unwrap();
        assert!(has_intersection_curves(cut.as_solid().unwrap()));
        let moved =
            transform_body(&cut, DMat4::from_translation(DVec3::new(0.0, 5.0, 0.0))).unwrap();
        assert!(!has_intersection_curves(moved.as_solid().unwrap()));
        assert!((volume(&moved) - volume(&cut)).abs() < 0.02);
        let bounds = moved.trimesh(0.01).unwrap().bounds();
        assert!((bounds.min.y - 4.0).abs() < 1.0e-6);
        // Mirroring keeps the solid outward facing.
        let mirrored =
            transform_body(&cube(1.0), DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0))).unwrap();
        assert!(volume(&mirrored) > 0.9);
    }
}

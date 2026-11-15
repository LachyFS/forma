//! Feature evaluation and regeneration.
//!
//! Features run in history order against the body built so far. A failing
//! feature is reported as a [`FeatureIssue`] and skipped, so the rest of the
//! history still regenerates (as in a CAD timeline). Bodies after each
//! feature are memoized by a hash of the feature prefix, so editing the last
//! feature re-evaluates only that feature.
use crate::KernelError;
use crate::body::{
    Body, Geometry, extrude_face, extrude_profiles, guard, orient_outward, primitive,
    revolve_profiles, sketch_profiles,
};
use crate::boolean::{BoolOp, bake_intersection_curves, boolean, transform_body};
use crate::geom::{TriMesh, closest_on_triangle, dvec, point, vector};
use crate::polyhedron;
use crate::protocol::{RegenRequest, RegenResult, RegenStats};
use crate::tessellate::tessellate;
use forma_core::{
    BaseBody, ChamferFeature, EdgeRef, ExtrudeExtent, FaceRef, Feature, FeatureIssue, FeatureKind,
    FeatureOperation, PartData, PushPullFeature,
};
use glam::DVec3;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use truck_modeling::{Solid, Wire, builder};

/// Most parts whose evaluated histories one worker keeps.
const CACHED_PARTS: usize = 24;

#[derive(Clone)]
struct State {
    hash: u64,
    body: Option<Body>,
    issues: Vec<FeatureIssue>,
}

/// Memoized feature-prefix states per part.
#[derive(Default)]
pub struct RegenCache {
    parts: HashMap<u64, Vec<State>>,
    order: VecDeque<u64>,
}

impl RegenCache {
    fn states(&mut self, part: u64) -> &mut Vec<State> {
        if let Some(position) = self.order.iter().position(|id| *id == part) {
            self.order.remove(position);
        }
        self.order.push_back(part);
        while self.order.len() > CACHED_PARTS {
            if let Some(evicted) = self.order.pop_front() {
                self.parts.remove(&evicted);
            }
        }
        self.parts.entry(part).or_default()
    }

    pub fn clear(&mut self) {
        self.parts.clear();
        self.order.clear();
    }
}

fn hash_step(previous: u64, feature: &Feature, skipped: bool) -> u64 {
    #[derive(serde::Serialize)]
    struct Key<'a> {
        previous: u64,
        kind: &'a FeatureKind,
        skipped: bool,
    }
    let json = serde_json::to_vec(&Key {
        previous,
        kind: &feature.kind,
        skipped,
    })
    .unwrap_or_default();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in json {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Evaluate a part's active history, reusing cached prefixes.
pub(crate) fn evaluate_part(
    part_id: u64,
    part: &PartData,
    skip: &[FeatureIssue],
    cache: &mut RegenCache,
    progress: &mut dyn FnMut(u64),
    stats: &mut RegenStats,
) -> (Option<Body>, Vec<FeatureIssue>) {
    let active: Vec<&Feature> = part.active_features().collect();
    let states = cache.states(part_id);
    let mut hash = 0x5eed_u64;
    let mut body: Option<Body> = None;
    let mut issues: Vec<FeatureIssue> = Vec::new();
    for (index, feature) in active.iter().enumerate() {
        let skipped = skip.iter().find(|issue| issue.feature == feature.id);
        hash = hash_step(hash, feature, skipped.is_some());
        if let Some(state) = states.get(index)
            && state.hash == hash
        {
            body = state.body.clone();
            issues = state.issues.clone();
            stats.reused_features += 1;
            continue;
        }
        states.truncate(index);
        if let Some(issue) = skipped {
            issues.push(issue.clone());
        } else {
            progress(feature.id);
            stats.evaluated_features += 1;
            match guard(|| evaluate(feature, body.as_ref())) {
                Ok(next) => body = Some(next),
                Err(error) => issues.push(FeatureIssue {
                    feature: feature.id,
                    message: format!("{}: {}", feature.name, error.message),
                    suggestion: error.suggestion,
                }),
            }
        }
        states.push(State {
            hash,
            body: body.clone(),
            issues: issues.clone(),
        });
    }
    (body, issues)
}

/// Regenerate a part: evaluate, then tessellate for display.
pub fn regenerate(
    request: &RegenRequest,
    cache: &mut RegenCache,
    progress: &mut dyn FnMut(u64),
) -> RegenResult {
    let start = Instant::now();
    let mut stats = RegenStats::default();
    let (body, mut issues) = evaluate_part(
        request.part_id,
        &request.part,
        &request.skip,
        cache,
        progress,
        &mut stats,
    );
    stats.evaluate_ms = start.elapsed().as_secs_f64() * 1.0e3;
    let tessellate_start = Instant::now();
    let mut mesh = None;
    let mut faces = Vec::new();
    if let Some(body) = &body {
        stats.brep_faces = body.face_count();
        match guard(|| tessellate(body, request.part.quality)) {
            Ok(result) => {
                if result.missing_faces > 0 {
                    issues.push(FeatureIssue {
                        feature: 0,
                        message: format!(
                            "{} faces could not be tessellated and are not shown",
                            result.missing_faces
                        ),
                        suggestion: "Try another tessellation quality".into(),
                    });
                }
                stats.triangles = result.mesh.faces.len();
                mesh = Some(result.mesh);
                faces = result.faces;
            }
            Err(error) => issues.push(FeatureIssue {
                feature: 0,
                message: error.message,
                suggestion: error.suggestion,
            }),
        }
    }
    stats.tessellate_ms = tessellate_start.elapsed().as_secs_f64() * 1.0e3;
    stats.total_ms = start.elapsed().as_secs_f64() * 1.0e3;
    RegenResult {
        part_id: request.part_id,
        stamp: request.part.stamp,
        mesh,
        faces,
        issues,
        stats,
    }
}

/// The body of a history evaluated from scratch, for nested tools.
fn evaluate_history(features: &[Feature]) -> Result<Body, KernelError> {
    let mut body: Option<Body> = None;
    for feature in features.iter().filter(|f| !f.suppressed) {
        body = Some(evaluate(feature, body.as_ref()).map_err(|error| {
            KernelError::new(
                format!("tool feature {}: {}", feature.name, error.message),
                error.suggestion,
            )
        })?);
    }
    body.ok_or_else(|| KernelError::new("the tool body is empty", "Unsuppress a tool feature"))
}

/// Evaluate one feature against the current body.
fn evaluate(feature: &Feature, body: Option<&Body>) -> Result<Body, KernelError> {
    match &feature.kind {
        FeatureKind::Primitive(primitive_feature) => {
            let shape = primitive(&primitive_feature.shape)?;
            let placed = transform_body(&shape, primitive_feature.placement.matrix().as_dmat4())?;
            apply(body, placed, primitive_feature.operation)
        }
        FeatureKind::Extrude(extrude) => {
            let profiles = sketch_profiles(&extrude.sketch, &extrude.profiles)?;
            let mut distance = f64::from(extrude.distance);
            if extrude.flip {
                distance = -distance;
            }
            let (start, end) = match extrude.extent {
                ExtrudeExtent::OneSide => (0.0, distance),
                ExtrudeExtent::Symmetric => (-distance.abs() * 0.5, distance.abs() * 0.5),
                ExtrudeExtent::TwoSides { back_distance } => {
                    let back = f64::from(back_distance);
                    let sign = if extrude.flip { -1.0 } else { 1.0 };
                    (-back * sign, distance)
                }
            };
            let (start, end) = (start.min(end), start.max(end));
            let tool = extrude_profiles(&extrude.sketch.plane, &profiles, start, end)?;
            apply(body, tool, extrude.operation)
        }
        FeatureKind::Revolve(revolve) => {
            let profiles = sketch_profiles(&revolve.sketch, &revolve.profiles)?;
            let tool = revolve_profiles(
                &revolve.sketch.plane,
                &profiles,
                revolve.axis_origin,
                revolve.axis_direction,
                f64::from(revolve.angle),
            )?;
            apply(body, tool, revolve.operation)
        }
        FeatureKind::Combine(combine) => {
            let mut current = body.cloned();
            for tool in &combine.tools {
                let tool_body = evaluate_history(&tool.features)?;
                let placed = transform_body(&tool_body, tool.transform.as_dmat4())?;
                current = Some(apply(current.as_ref(), placed, combine.operation)?);
            }
            current.ok_or_else(|| KernelError::new("Combine produced nothing", ""))
        }
        FeatureKind::PushPull(push_pull) => {
            let body = body.ok_or_else(nothing_to_edit)?;
            push_pull_face(body, push_pull)
        }
        FeatureKind::Chamfer(chamfer) => {
            let body = body.ok_or_else(nothing_to_edit)?;
            chamfer_edges(body, chamfer)
        }
        FeatureKind::BaseBody(base) => {
            let tool = match &base.body {
                BaseBody::Mesh { mesh } => {
                    let poly = polyhedron::from_mesh(mesh)?;
                    polyhedron::build_solid(&poly)?
                }
                BaseBody::Step { text } => crate::step::step_body(text)?,
            };
            apply(body, tool, base.operation)
        }
    }
}

fn nothing_to_edit() -> KernelError {
    KernelError::new(
        "there is no body to edit yet",
        "Move this feature after one that creates a body",
    )
}

/// Combine a feature's new solid with the current body.
fn apply(
    body: Option<&Body>,
    tool: Body,
    operation: FeatureOperation,
) -> Result<Body, KernelError> {
    let Some(body) = body else {
        return match operation {
            FeatureOperation::Join | FeatureOperation::NewBody => Ok(tool),
            FeatureOperation::Cut | FeatureOperation::Intersect => Err(KernelError::new(
                format!("{} needs an existing body", operation.label()),
                "Use Join for the first feature of a part",
            )),
        };
    };
    match operation {
        FeatureOperation::Join => boolean(body, &tool, BoolOp::Union),
        FeatureOperation::Cut => boolean(body, &tool, BoolOp::Subtract),
        FeatureOperation::Intersect => boolean(body, &tool, BoolOp::Intersect),
        FeatureOperation::NewBody => match (&body.geometry, tool.geometry) {
            (Geometry::Solid(a), Geometry::Solid(b)) => Ok(Body::solid(
                Solid::new(
                    a.boundaries()
                        .iter()
                        .cloned()
                        .chain(b.into_boundaries())
                        .collect(),
                ),
                body.min_radius.min(tool.min_radius),
            )),
            (Geometry::Step(a), Geometry::Step(b)) => Ok(Body {
                geometry: Geometry::Step(a.iter().cloned().chain(b).collect()),
                min_radius: body.min_radius.min(tool.min_radius),
            }),
            _ => Err(KernelError::new(
                "STEP bodies and modelled bodies cannot share a part yet",
                "Keep the STEP import in its own part",
            )),
        },
    }
}

// ---------------------------------------------------------------------------
// Face references and push/pull
// ---------------------------------------------------------------------------

/// The B-rep face a reference points at: matching normal, nearest by lateral
/// distance within the face's plane (offsets along the normal count less, so
/// a face moved by an earlier edit is still found).
pub(crate) fn resolve_face(
    mesh: &TriMesh,
    reference: &FaceRef,
    size: f64,
) -> Result<u32, KernelError> {
    let target = reference.point.as_dvec3();
    let normal = reference.normal.as_dvec3().normalize_or_zero();
    let mut best: Option<(f64, u32)> = None;
    for (index, &face) in mesh.faces.iter().enumerate() {
        let [a, b, c] = mesh.triangle(index);
        let n = (b - a).cross(c - a).normalize_or_zero();
        if n.dot(normal) < 0.98 {
            continue;
        }
        let offset = n.dot(target - a);
        let projected = target - n * offset;
        let lateral = projected.distance(closest_on_triangle(projected, a, b, c));
        let score = lateral + offset.abs() * 0.25;
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, face));
        }
    }
    match best {
        Some((score, face)) if score <= size * 0.05 => Ok(face),
        _ => Err(KernelError::new(
            "the referenced face no longer exists",
            "An earlier edit removed or moved it · Pick the face again",
        )),
    }
}

fn face_is_planar(mesh: &TriMesh, face: u32, normal: DVec3) -> bool {
    mesh.faces
        .iter()
        .enumerate()
        .filter(|(_, f)| **f == face)
        .all(|(i, _)| {
            let [a, b, c] = mesh.triangle(i);
            (b - a).cross(c - a).normalize_or_zero().dot(normal) > 1.0 - 1.0e-6
        })
}

fn push_pull_face(body: &Body, feature: &PushPullFeature) -> Result<Body, KernelError> {
    let Some(solid) = body.as_solid() else {
        return Err(KernelError::new(
            "push/pull is not available on imported STEP bodies yet",
            "Convert the STEP part to a mesh and back to a part first",
        ));
    };
    let distance = f64::from(feature.distance);
    let size = body.size();
    let tolerance = body.tolerance(1.0e-3, 0.2);
    let mesh = body.trimesh(tolerance)?;
    let face = resolve_face(&mesh, &feature.face, size)?;
    let normal = mesh
        .face_distance(face, feature.face.point.as_dvec3())
        .map(|(_, n)| n)
        .unwrap_or(feature.face.normal.as_dvec3());
    if !face_is_planar(&mesh, face, normal) {
        return Err(KernelError::new(
            "push/pull works on flat faces",
            "Pick a planar face, or offset curved faces with a new feature",
        ));
    }
    // 1. Polyhedral bodies: slide the face's vertices along neighbouring planes.
    let mut polyhedral_error = None;
    if let Some(poly) = polyhedron::extract(solid, size * 1.0e-7) {
        match polyhedron::move_face(&poly, face as usize, distance) {
            Ok(moved) => return polyhedron::build_solid(&moved),
            Err(error) => polyhedral_error = Some(error),
        }
    }
    // 2. Prisms along the face normal (plates with holes): re-extrude.
    if let Some(result) = prism_push_pull(body, solid, face, normal, distance, size) {
        return result;
    }
    if let Some(error) = polyhedral_error {
        return Err(error);
    }
    // 3. General case: extrude the face and join or cut with overlap.
    let copy = builder::clone(solid);
    bake_intersection_curves(&copy, tolerance)?;
    let source = copy
        .face_iter()
        .nth(face as usize)
        .cloned()
        .ok_or_else(|| KernelError::new("the face disappeared during push/pull", ""))?;
    let overlap = size * 1.0e-4;
    let start = builder::translated(
        &source,
        vector(normal * if distance > 0.0 { -overlap } else { overlap }),
    );
    let sweep = normal * (distance + overlap * distance.signum());
    let tool = orient_outward(extrude_face(&start, vector(sweep)), body.min_radius)?;
    let op = if distance > 0.0 {
        BoolOp::Union
    } else {
        BoolOp::Subtract
    };
    boolean(body, &tool, op).map_err(|error| {
        KernelError::new(
            format!(
                "push/pull could not merge the moved face ({})",
                error.message
            ),
            "Faces next to this one may be coplanar with the move · Try a sketch extrusion instead",
        )
    })
}

/// Exact unit normals of a face's surface on a parameter grid, or `None`
/// for unbounded parameter ranges.
fn surface_normals(surface: &truck_modeling::Surface) -> Option<Vec<DVec3>> {
    use std::ops::Bound;
    use truck_modeling::{ParametricSurface, ParametricSurface3D};
    let ((u0, u1), (v0, v1)) = surface.parameter_range();
    let bounded = |a: Bound<f64>, b: Bound<f64>| match (a, b) {
        (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) => {
            Some((a, b))
        }
        _ => None,
    };
    let ((ua, ub), (va, vb)) = (bounded(u0, u1)?, bounded(v0, v1)?);
    let mut normals = Vec::with_capacity(36);
    for i in 0..6 {
        for j in 0..6 {
            let u = ua + (ub - ua) * (i as f64 + 0.5) / 6.0;
            let v = va + (vb - va) * (j as f64 + 0.5) / 6.0;
            let n = crate::geom::dvec_v(surface.normal(u, v));
            if n.is_finite() && n.length_squared() > 0.0 {
                normals.push(n.normalize());
            }
        }
    }
    (!normals.is_empty()).then_some(normals)
}

/// Push/pull on a body that is a prism along the face normal: every face is
/// a cap (normal ±n at the top or bottom height) or a side (parallel to n).
/// Classified with exact surface normals: tessellations of boolean results
/// are too irregular to tell a wall from a slight draft.
fn prism_push_pull(
    body: &Body,
    solid: &Solid,
    face: u32,
    normal: DVec3,
    distance: f64,
    size: f64,
) -> Option<Result<Body, KernelError>> {
    let mut top = None;
    let mut bottoms = Vec::new();
    for (index, brep_face) in solid.face_iter().enumerate() {
        let surface = brep_face.oriented_surface();
        let heights: Vec<f64> = brep_face
            .vertex_iter()
            .map(|v| normal.dot(dvec(v.point())))
            .collect();
        let level = *heights.first()?;
        let flat = heights.iter().all(|h| (h - level).abs() <= size * 1.0e-7);
        match &surface {
            truck_modeling::Surface::Plane(plane) => {
                let n = crate::geom::dvec_v(plane.normal()).normalize_or_zero();
                let along = n.dot(normal);
                if along > 1.0 - 1.0e-9 && flat {
                    if index as u32 != face {
                        return None;
                    }
                    top = Some(level);
                } else if along < -1.0 + 1.0e-9 && flat {
                    bottoms.push((index, level));
                } else if along.abs() > 1.0e-9 {
                    return None;
                }
            }
            other => {
                let normals = surface_normals(other)?;
                if !normals.iter().all(|n| n.dot(normal).abs() < 1.0e-7) {
                    return None;
                }
            }
        }
    }
    let (top, [(bottom, base)]) = (top?, bottoms.as_slice()) else {
        return None;
    };
    let new_height = top - base + distance;
    if new_height <= size * 1.0e-6 {
        return Some(Err(KernelError::new(
            format!("push/pull of {distance:.4} m would remove the whole body"),
            "Use a smaller distance",
        )));
    }
    let rebuilt = guard(|| {
        let copy = builder::clone(solid);
        bake_intersection_curves(&copy, body.tolerance(3.0e-4, 0.1))?;
        let cap = copy
            .face_iter()
            .nth(*bottom)
            .cloned()
            .ok_or_else(|| KernelError::new("the base face disappeared", ""))?;
        let solid = extrude_face(&cap, vector(normal * new_height));
        let rebuilt = orient_outward(solid, body.min_radius)?;
        // Swept approximations of intersection curves occasionally defeat
        // truck's tessellator; prefer the general path over a hole in the part.
        let faces = rebuilt.face_meshes(rebuilt.tolerance(4.0e-3, 0.4))?;
        if faces.iter().any(Option::is_none) {
            return Err(KernelError::new(
                "the re-extruded body did not tessellate",
                "",
            ));
        }
        Ok(rebuilt)
    });
    rebuilt.ok().map(Ok)
}

// ---------------------------------------------------------------------------
// Chamfer
// ---------------------------------------------------------------------------

fn chamfer_edges(body: &Body, feature: &ChamferFeature) -> Result<Body, KernelError> {
    let Some(solid) = body.as_solid() else {
        return Err(KernelError::new(
            "chamfer is not available on imported STEP bodies yet",
            "",
        ));
    };
    let size = body.size();
    let tolerance = body.tolerance(1.0e-3, 0.2);
    let mesh = body.trimesh(tolerance)?;
    let distance = f64::from(feature.distance);
    // Build every wedge from the original body first; cutting one edge
    // changes the neighbours of the next.
    let wedges = feature
        .edges
        .iter()
        .map(|edge| chamfer_wedge(solid, &mesh, edge, distance, size))
        .collect::<Result<Vec<_>, _>>()?;
    let mut current = body.clone();
    for wedge in wedges {
        current = boolean(&current, &wedge, BoolOp::Subtract).map_err(|error| {
            KernelError::new(
                format!("the chamfer cut failed ({})", error.message),
                "Use a smaller chamfer distance",
            )
        })?;
    }
    Ok(current)
}

/// Faces of `solid` (by index) whose boundary uses `edge` or touches `vertex`.
fn incident_faces(solid: &Solid, matches: impl Fn(&truck_modeling::Edge) -> bool) -> Vec<usize> {
    solid
        .face_iter()
        .enumerate()
        .filter(|(_, face)| {
            face.boundaries()
                .iter()
                .any(|wire: &Wire| wire.edge_iter().any(&matches))
        })
        .map(|(index, _)| index)
        .collect()
}

fn face_normal_from_mesh(mesh: &TriMesh, face: usize) -> Option<DVec3> {
    let mut sum = DVec3::ZERO;
    let mut normals = Vec::new();
    for i in (0..mesh.faces.len()).filter(|&i| mesh.faces[i] == face as u32) {
        let [a, b, c] = mesh.triangle(i);
        let n = (b - a).cross(c - a);
        sum += n;
        normals.push(n.normalize_or_zero());
    }
    let average = sum.normalize_or_zero();
    (average != DVec3::ZERO && normals.iter().all(|n| n.dot(average) > 1.0 - 1.0e-6))
        .then_some(average)
}

fn chamfer_wedge(
    solid: &Solid,
    mesh: &TriMesh,
    reference: &EdgeRef,
    distance: f64,
    size: f64,
) -> Result<Body, KernelError> {
    let (ra, rb) = (reference.start.as_dvec3(), reference.end.as_dvec3());
    let near = size * 1.0e-5;
    let edge = solid
        .edge_iter()
        .find(|edge| {
            let (a, b) = (dvec(edge.front().point()), dvec(edge.back().point()));
            (a.distance(ra) <= near && b.distance(rb) <= near)
                || (a.distance(rb) <= near && b.distance(ra) <= near)
        })
        .ok_or_else(|| {
            KernelError::new(
                "a chamfered edge no longer exists",
                "An earlier edit changed it · Pick the edge again",
            )
        })?;
    let id = edge.id();
    let faces = incident_faces(solid, |e| e.id() == id);
    let [f1, f2] = faces.as_slice() else {
        return Err(KernelError::new(
            "a chamfered edge is not shared by two faces",
            "",
        ));
    };
    let (Some(n1), Some(n2)) = (
        face_normal_from_mesh(mesh, *f1),
        face_normal_from_mesh(mesh, *f2),
    ) else {
        return Err(KernelError::new(
            "chamfer works on edges between flat faces",
            "Pick a straight edge between two planar faces",
        ));
    };
    let (a, b) = (dvec(edge.front().point()), dvec(edge.back().point()));
    let length = a.distance(b);
    let e = (b - a) / length;
    if (n1.dot(e)).abs() > 1.0e-6 || (n2.dot(e)).abs() > 1.0e-6 || n1.dot(n2) > 0.999 {
        return Err(KernelError::new(
            "chamfer works on straight edges between faces meeting at an angle",
            "",
        ));
    }
    let midpoint = (a + b) * 0.5;
    let inward = -(n1 + n2).normalize();
    if !mesh.contains(midpoint + inward * size * 1.0e-4) {
        return Err(KernelError::new(
            "chamfer supports convex edges only",
            "Pick an outside edge",
        ));
    }
    if distance >= length || distance * 2.0 >= size {
        return Err(KernelError::new(
            "the chamfer distance is larger than the edge",
            "Use a smaller distance",
        ));
    }
    let mut t1 = n1.cross(e);
    if t1.dot(n2) > 0.0 {
        t1 = -t1;
    }
    let mut t2 = n2.cross(e);
    if t2.dot(n1) > 0.0 {
        t2 = -t2;
    }
    // The wedge must leave the body at both ends of the edge.
    for (vertex, outward) in [(edge.front().clone(), -e), (edge.back().clone(), e)] {
        let vid = vertex.id();
        for f in incident_faces(solid, |edge| {
            edge.front().id() == vid || edge.back().id() == vid
        }) {
            if f == *f1 || f == *f2 {
                continue;
            }
            match face_normal_from_mesh(mesh, f) {
                Some(n) if n.dot(outward) > 0.2 => {}
                _ => {
                    return Err(KernelError::new(
                        "chamfer supports edges ending at convex corners",
                        "Pick an edge whose ends meet outside faces",
                    ));
                }
            }
        }
    }
    let margin = distance * 2.0;
    let start = a - e * margin;
    let corner_a = start + t1 * distance;
    let corner_b = start + t2 * distance;
    let along = (corner_a - corner_b).normalize();
    let p1 = corner_a + along * distance;
    let p2 = corner_b - along * distance;
    let p3 = start - (t1 + t2).normalize() * distance * 2.0;
    let vertices: Vec<_> = [p1, p2, p3]
        .iter()
        .map(|p| builder::vertex(point(*p)))
        .collect();
    let wire = Wire::from(vec![
        builder::line(&vertices[0], &vertices[1]),
        builder::line(&vertices[1], &vertices[2]),
        builder::line(&vertices[2], &vertices[0]),
    ]);
    let face = builder::try_attach_plane(&[wire])
        .map_err(|_| KernelError::new("could not build the chamfer wedge", ""))?;
    let solid = extrude_face(&face, vector(e * (length + margin * 2.0)));
    orient_outward(solid, f64::INFINITY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::{
        CombineFeature, ExtrudeFeature, Placement, PrimitiveFeature, PrimitiveShape, Sketch,
        SketchCurve, SketchPlane, ToolBody,
    };
    use glam::{Mat4, Vec2, Vec3};
    use std::f64::consts::PI;

    fn box_kind(size: Vec3, origin: Vec3, operation: FeatureOperation) -> FeatureKind {
        FeatureKind::Primitive(PrimitiveFeature {
            shape: PrimitiveShape::Box { size },
            placement: Placement {
                origin,
                rotation: Vec3::ZERO,
            },
            operation,
        })
    }

    fn regen(part: &PartData, cache: &mut RegenCache) -> RegenResult {
        regenerate(
            &RegenRequest {
                part_id: part.id,
                part: part.clone(),
                skip: Vec::new(),
            },
            cache,
            &mut |_| {},
        )
    }

    fn volume_of(part: &PartData) -> f64 {
        let mut cache = RegenCache::default();
        let mut stats = RegenStats::default();
        let (body, issues) = evaluate_part(part.id, part, &[], &mut cache, &mut |_| {}, &mut stats);
        assert!(issues.is_empty(), "{issues:?}");
        let body = body.unwrap();
        body.volume(body.tolerance(3.0e-4, 0.05)).unwrap()
    }

    #[test]
    fn plates_with_holes_extrude_cut_and_push_pull() {
        let mut part = PartData::new(1, "Plate");
        part.push_feature(
            "Base",
            box_kind(Vec3::new(4.0, 0.5, 3.0), Vec3::ZERO, FeatureOperation::Join),
        );
        let mut sketch = Sketch::new(SketchPlane::ground());
        sketch.plane.origin = Vec3::new(0.0, 0.25, 0.0);
        sketch.entities.push(
            SketchCurve::Circle {
                center: Vec2::new(1.0, 0.5),
                radius: 0.4,
            }
            .into(),
        );
        let cut = part.push_feature(
            "Hole",
            FeatureKind::Extrude(ExtrudeFeature {
                sketch,
                profiles: Vec::new(),
                distance: 1.0,
                extent: ExtrudeExtent::OneSide,
                flip: true,
                operation: FeatureOperation::Cut,
            }),
        );
        let expected = 6.0 - PI * 0.16 * 0.5;
        assert!(
            (volume_of(&part) - expected).abs() < 0.02,
            "{}",
            volume_of(&part)
        );
        // Pull the top face up by 0.25 (prism path: the plate has a curved hole).
        part.push_feature(
            "Pull",
            FeatureKind::PushPull(PushPullFeature {
                face: FaceRef {
                    point: Vec3::new(-1.5, 0.25, -1.0),
                    normal: Vec3::Y,
                },
                distance: 0.25,
            }),
        );
        let pulled = (12.0 - PI * 0.16) * 0.75;
        assert!(
            (volume_of(&part) - pulled).abs() < 0.03,
            "{}",
            volume_of(&part)
        );
        // Cache: editing the last feature reuses the first two.
        let mut cache = RegenCache::default();
        let first = regen(&part, &mut cache);
        assert!(first.issues.is_empty(), "{:?}", first.issues);
        assert_eq!(first.stats.evaluated_features, 3);
        let last = part.features.last().unwrap().id;
        part.edit_feature(last, |feature| {
            if let FeatureKind::PushPull(p) = &mut feature.kind {
                p.distance = 0.5;
            }
        });
        let second = regen(&part, &mut cache);
        assert_eq!(second.stats.reused_features, 2);
        assert_eq!(second.stats.evaluated_features, 1);
        assert!(second.mesh.as_ref().unwrap().has_groups());
        let _ = cut;
    }

    #[test]
    fn failing_features_are_reported_and_skipped() {
        let mut part = PartData::new(2, "Part");
        part.push_feature(
            "Cut first",
            box_kind(Vec3::ONE, Vec3::ZERO, FeatureOperation::Cut),
        );
        part.push_feature(
            "Base",
            box_kind(Vec3::splat(2.0), Vec3::ZERO, FeatureOperation::Join),
        );
        part.push_feature(
            "Lost face",
            FeatureKind::PushPull(PushPullFeature {
                face: FaceRef {
                    point: Vec3::new(50.0, 50.0, 50.0),
                    normal: Vec3::Y,
                },
                distance: 0.1,
            }),
        );
        let result = regen(&part, &mut RegenCache::default());
        assert_eq!(result.issues.len(), 2, "{:?}", result.issues);
        assert_eq!(result.issues[0].feature, part.features[0].id);
        assert!(result.issues[0].message.starts_with("Cut first:"));
        assert!(!result.issues[0].suggestion.is_empty());
        assert!(result.mesh.is_some(), "the base box still renders");
    }

    #[test]
    fn combine_tools_keep_their_history_and_transform() {
        let mut part = PartData::new(3, "Bracket");
        part.push_feature(
            "Base",
            box_kind(Vec3::splat(2.0), Vec3::ZERO, FeatureOperation::Join),
        );
        part.push_feature(
            "Cut",
            FeatureKind::Combine(CombineFeature {
                operation: FeatureOperation::Cut,
                tools: vec![ToolBody {
                    name: "Corner".into(),
                    transform: Mat4::from_translation(Vec3::new(1.0, 1.0, 1.0)),
                    features: vec![Feature::new(
                        90,
                        "Box",
                        box_kind(Vec3::splat(1.0), Vec3::ZERO, FeatureOperation::Join),
                    )],
                }],
            }),
        );
        part.next_feature_id = 100;
        let v = volume_of(&part);
        assert!((v - (8.0 - 0.125)).abs() < 0.01, "{v}");
    }

    #[test]
    fn chamfers_cut_box_edges() {
        let mut part = PartData::new(4, "Block");
        part.push_feature(
            "Base",
            box_kind(Vec3::splat(2.0), Vec3::ZERO, FeatureOperation::Join),
        );
        part.push_feature(
            "Chamfer",
            FeatureKind::Chamfer(ChamferFeature {
                edges: vec![EdgeRef {
                    start: Vec3::new(-1.0, 1.0, 1.0),
                    end: Vec3::new(1.0, 1.0, 1.0),
                }],
                distance: 0.25,
            }),
        );
        let v = volume_of(&part);
        let expected = 8.0 - 0.25 * 0.25 * 0.5 * 2.0;
        assert!((v - expected).abs() < 0.01, "{v} vs {expected}");
    }

    #[test]
    fn mesh_base_bodies_push_pull_through_the_polyhedral_path() {
        let mesh = forma_core::Mesh::primitive(forma_core::Primitive::Cube);
        let mut part = crate::convert::part_from_mesh("Cube", &mesh, None);
        part.push_feature(
            "Pull",
            FeatureKind::PushPull(PushPullFeature {
                face: FaceRef {
                    point: Vec3::new(1.0, 0.2, 0.3),
                    normal: Vec3::X,
                },
                distance: 1.0,
            }),
        );
        assert!((volume_of(&part) - 12.0).abs() < 1.0e-6);
    }
}

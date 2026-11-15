//! Solids and the constructive operations that do not need booleans:
//! primitives, sketch profiles, extrusion and revolution.
//!
//! Every function here runs inside `catch_unwind` at the feature level, but
//! the hazards documented in the truck audit are avoided up front: faces are
//! oriented before sweeping, hole winding is normalized, rotation axes are
//! unit length, and transforms are applied only to freshly built solids that
//! contain no intersection curves.
use crate::KernelError;
use crate::geom::{Bounds, TriMesh, dvec, point, vector};
use forma_core::{PrimitiveShape, ProfileLoop, Sketch, SketchCurve, SketchPlane};
use glam::{DVec2, DVec3, Vec2};
use std::f64::consts::{PI, TAU};
use truck_meshalgo::tessellation::{MeshableShape, RobustMeshableShape};
use truck_modeling::{
    BoundedCurve, Edge, Face, InnerSpace, Invertible, ParametricCurve, ParametricSurface3D, Point3,
    Rad, Shell, Solid, Surface, Vector3, Vertex, Wire, builder,
};
use truck_polymesh::PolygonMesh;

/// Shells read from STEP files use truck-stepio's own curve and surface types.
pub(crate) type StepShell = truck_topology::Shell<
    Point3,
    truck_stepio::r#in::alias::Curve3D,
    truck_stepio::r#in::alias::Surface,
>;

#[derive(Clone, Debug)]
pub(crate) enum Geometry {
    Solid(Solid),
    /// Imported STEP shells: they tessellate and export, but truck 0.4 cannot
    /// run booleans on them.
    Step(Vec<StepShell>),
}

/// A part body plus construction metadata that guides tolerances.
#[derive(Clone, Debug)]
pub(crate) struct Body {
    pub geometry: Geometry,
    /// Smallest radius of curvature introduced by construction (arcs,
    /// cylinders, revolved profiles), or infinity for straight geometry.
    pub min_radius: f64,
}

impl Body {
    pub fn solid(solid: Solid, min_radius: f64) -> Self {
        Self {
            geometry: Geometry::Solid(solid),
            min_radius,
        }
    }

    pub fn as_solid(&self) -> Option<&Solid> {
        match &self.geometry {
            Geometry::Solid(solid) => Some(solid),
            Geometry::Step(_) => None,
        }
    }

    pub fn face_count(&self) -> usize {
        match &self.geometry {
            Geometry::Solid(solid) => solid.face_iter().count(),
            Geometry::Step(shells) => shells.iter().map(|shell| shell.len()).sum(),
        }
    }

    /// Bounds of the vertices, which may miss curved bulges between them.
    pub fn vertex_bounds(&self) -> Bounds {
        let mut bounds = Bounds::empty();
        match &self.geometry {
            Geometry::Solid(solid) => {
                for vertex in solid.vertex_iter() {
                    bounds.add(dvec(vertex.point()));
                }
            }
            Geometry::Step(shells) => {
                for shell in shells {
                    for vertex in shell.vertex_iter() {
                        bounds.add(dvec(vertex.point()));
                    }
                }
            }
        }
        bounds
    }

    /// Characteristic size: the vertex bounds diagonal, padded by curvature.
    pub fn size(&self) -> f64 {
        let bounds = self.vertex_bounds();
        let radius = if self.min_radius.is_finite() {
            self.min_radius
        } else {
            0.0
        };
        bounds.diagonal().max(radius * 2.0).max(1.0e-6)
    }

    /// A chord tolerance for tessellation and boolean intersection tracing.
    pub fn tolerance(&self, chord_fraction: f64, max_angle: f64) -> f64 {
        let size = self.size();
        let mut tolerance = size * chord_fraction;
        if self.min_radius.is_finite() {
            // Sagitta of the largest permitted facet angle on the tightest curve.
            tolerance = tolerance.min(self.min_radius * (1.0 - (max_angle * 0.5).cos()));
        }
        tolerance.clamp((size * 2.0e-5).max(2.0e-6), (size * 0.02).max(2.0e-6))
    }

    /// Oriented triangles of every face, with the face index per triangle.
    pub fn trimesh(&self, tolerance: f64) -> Result<TriMesh, KernelError> {
        let faces = self.face_meshes(tolerance)?;
        let mut mesh = TriMesh::default();
        for (index, poly) in faces.into_iter().enumerate() {
            let Some(poly) = poly else { continue };
            let offset = mesh.positions.len() as u32;
            mesh.positions
                .extend(poly.positions().iter().map(|p| dvec(*p)));
            for triangle in poly.faces().triangle_iter() {
                mesh.triangles.push(triangle.map(|v| offset + v.pos as u32));
                mesh.faces.push(index as u32);
            }
        }
        if mesh.triangles.is_empty() {
            return Err(KernelError::new(
                "The body could not be tessellated",
                "Try a coarser tessellation quality or simplify the last feature",
            ));
        }
        Ok(mesh)
    }

    /// One outward-oriented polygon mesh per B-rep face, in face order.
    /// Faces whose tessellation failed are `None`.
    pub fn face_meshes(&self, tolerance: f64) -> Result<Vec<Option<PolygonMesh>>, KernelError> {
        let tolerance = tolerance.max(2.0e-6);
        let attempt = |robust: bool| -> Result<Vec<Option<PolygonMesh>>, KernelError> {
            guard(|| match &self.geometry {
                Geometry::Solid(solid) => {
                    let meshed = if robust {
                        solid.robust_triangulation(tolerance)
                    } else {
                        solid.triangulation(tolerance)
                    };
                    Ok(meshed
                        .face_iter()
                        .map(|face| oriented(face.surface(), face.orientation()))
                        .collect())
                }
                Geometry::Step(shells) => Ok(shells
                    .iter()
                    .flat_map(|shell| {
                        let meshed = shell.robust_triangulation(tolerance);
                        meshed
                            .face_iter()
                            .map(|face| oriented(face.surface(), face.orientation()))
                            .collect::<Vec<_>>()
                    })
                    .collect()),
            })
        };
        let first = attempt(false);
        match first {
            Ok(faces) if faces.iter().all(Option::is_some) => Ok(faces),
            _ => {
                // Edges approximating intersection curves (after baking) and
                // imported geometry need the slower nearest-point search.
                let robust = attempt(true)?;
                Ok(robust)
            }
        }
    }

    /// Signed volume from a tessellation; positive for an outward solid.
    pub fn volume(&self, tolerance: f64) -> Result<f64, KernelError> {
        Ok(self.trimesh(tolerance)?.volume())
    }
}

fn oriented(surface: Option<PolygonMesh>, orientation: bool) -> Option<PolygonMesh> {
    let mut poly = surface?;
    if !orientation {
        poly.invert();
    }
    // Tessellating surfaces that touch a rotation axis yields NaN normals;
    // positions are what Forma consumes, so drop broken normals instead.
    if poly
        .normals()
        .iter()
        .any(|n| !n.x.is_finite() || !n.y.is_finite() || !n.z.is_finite())
    {
        let positions = poly.positions().clone();
        let faces: Vec<Vec<usize>> = poly
            .faces()
            .face_iter()
            .map(|face| face.iter().map(|v| v.pos).collect())
            .collect();
        poly = PolygonMesh::new(
            truck_polymesh::StandardAttributes {
                positions,
                ..Default::default()
            },
            truck_polymesh::Faces::from_iter(faces),
        );
    }
    (poly
        .positions()
        .iter()
        .all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite()))
    .then_some(poly)
}

/// Run kernel code, turning panics into errors. truck reports many failures
/// by panicking; the worker process additionally isolates aborts and hangs.
pub(crate) fn guard<T>(f: impl FnOnce() -> Result<T, KernelError>) -> Result<T, KernelError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown failure".into());
            Err(KernelError::new(
                format!("The geometry kernel failed ({detail})"),
                "Adjust the feature's dimensions slightly or simplify the geometry",
            ))
        }
    }
}

/// Make a solid outward facing by its tessellated volume.
pub(crate) fn orient_outward(mut solid: Solid, min_radius: f64) -> Result<Body, KernelError> {
    let body = Body::solid(solid.clone(), min_radius);
    let tolerance = body.tolerance(4.0e-3, 0.4);
    let volume = body.volume(tolerance)?;
    if !volume.is_finite() || volume.abs() <= 1.0e-18 {
        return Err(KernelError::new(
            "The feature produced an empty solid",
            "Check that the profile is closed and the distances are not zero",
        ));
    }
    if volume < 0.0 {
        solid.not();
    }
    Ok(Body::solid(solid, min_radius))
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

/// A parametric primitive centered on the origin with its axis along +Y.
pub(crate) fn primitive(shape: &PrimitiveShape) -> Result<Body, KernelError> {
    match *shape {
        PrimitiveShape::Box { size } => {
            let s = size.as_dvec3();
            let face = rectangle_face(-s * 0.5, s)?;
            let solid = extrude_face(&face, Vector3::new(0.0, s.y, 0.0));
            orient_outward(solid, f64::INFINITY)
        }
        PrimitiveShape::Cylinder { radius, height } => {
            let (r, h) = (f64::from(radius), f64::from(height));
            let circle = circle_wire(DVec3::new(0.0, -h * 0.5, 0.0), DVec3::Y, DVec3::X, r);
            let face = attach(&[circle])?;
            let solid = extrude_face(&face, Vector3::new(0.0, h, 0.0));
            orient_outward(solid, r)
        }
        PrimitiveShape::Sphere { radius } => {
            let r = f64::from(radius);
            let north = builder::vertex(Point3::new(0.0, r, 0.0));
            let south = builder::vertex(Point3::new(0.0, -r, 0.0));
            let arc = builder::circle_arc(&north, &south, Point3::new(r, 0.0, 0.0));
            let shell = builder::cone(&Wire::from(vec![arc]), Vector3::unit_y(), Rad(TAU));
            orient_outward(Solid::new(vec![shell]), r)
        }
        PrimitiveShape::Cone {
            bottom_radius,
            top_radius,
            height,
        } => {
            let (rb, rt, h) = (
                f64::from(bottom_radius),
                f64::from(top_radius),
                f64::from(height),
            );
            let apex = builder::vertex(Point3::new(0.0, h * 0.5, 0.0));
            let bottom_center = builder::vertex(Point3::new(0.0, -h * 0.5, 0.0));
            let bottom_rim = builder::vertex(Point3::new(rb, -h * 0.5, 0.0));
            let mut edges = Vec::new();
            if rt > 1.0e-9 {
                let top_rim = builder::vertex(Point3::new(rt, h * 0.5, 0.0));
                edges.push(builder::line(&apex, &top_rim));
                edges.push(builder::line(&top_rim, &bottom_rim));
            } else {
                edges.push(builder::line(&apex, &bottom_rim));
            }
            edges.push(builder::line(&bottom_rim, &bottom_center));
            let shell = builder::cone(&Wire::from(edges), Vector3::unit_y(), Rad(TAU));
            let tightest = if rt > 1.0e-9 { rt.min(rb) } else { rb * 0.25 };
            orient_outward(Solid::new(vec![shell]), tightest)
        }
        PrimitiveShape::Torus {
            major_radius,
            minor_radius,
        } => {
            let (big, small) = (f64::from(major_radius), f64::from(minor_radius));
            let circle = circle_wire(DVec3::new(big, 0.0, 0.0), DVec3::Z, DVec3::X, small);
            let face = attach(&[circle])?;
            let solid = builder::rsweep(
                &face,
                Point3::new(0.0, 0.0, 0.0),
                Vector3::unit_y(),
                Rad(TAU),
            );
            orient_outward(solid, small)
        }
    }
}

/// Axis-aligned box face at `min.y`, spanning `size.x` by `size.z`.
fn rectangle_face(min: DVec3, size: DVec3) -> Result<Face, KernelError> {
    let y = min.y;
    let corners = [
        DVec3::new(min.x, y, min.z),
        DVec3::new(min.x, y, min.z + size.z),
        DVec3::new(min.x + size.x, y, min.z + size.z),
        DVec3::new(min.x + size.x, y, min.z),
    ];
    let vertices: Vec<Vertex> = corners.iter().map(|c| builder::vertex(point(*c))).collect();
    let wire = Wire::from(
        (0..4)
            .map(|i| builder::line(&vertices[i], &vertices[(i + 1) % 4]))
            .collect::<Vec<_>>(),
    );
    attach(&[wire])
}

fn attach(wires: &[Wire]) -> Result<Face, KernelError> {
    builder::try_attach_plane(wires).map_err(|error| {
        KernelError::new(
            format!("Could not build a planar face ({error:?})"),
            "Make sure the profile is closed, flat and does not cross itself",
        )
    })
}

/// A full circle of two half arcs around `center` in the plane normal to
/// `axis`, starting along `reference`.
pub(crate) fn circle_wire(center: DVec3, axis: DVec3, reference: DVec3, radius: f64) -> Wire {
    let axis = axis.normalize();
    let x = (reference - axis * reference.dot(axis)).normalize();
    let y = axis.cross(x);
    let at = |angle: f64| center + (x * angle.cos() + y * angle.sin()) * radius;
    let v0 = builder::vertex(point(at(0.0)));
    let v1 = builder::vertex(point(at(PI)));
    Wire::from(vec![
        builder::circle_arc(&v0, &v1, point(at(PI * 0.5))),
        builder::circle_arc(&v1, &v0, point(at(PI * 1.5))),
    ])
}

/// Sweep a planar face along `direction`, orienting it first: truck keeps the
/// input face as the start cap, so a face pointing against the sweep would
/// give an inside-out solid.
pub(crate) fn extrude_face(face: &Face, direction: Vector3) -> Solid {
    if face_normal(face).dot(direction) < 0.0 {
        builder::tsweep(&face.inverse(), direction)
    } else {
        builder::tsweep(face, direction)
    }
}

/// Outward normal of a planar face, from its oriented surface or, for
/// non-planar faces, from its boundary winding.
pub(crate) fn face_normal(face: &Face) -> Vector3 {
    match face.oriented_surface() {
        Surface::Plane(plane) => plane.normal(),
        surface => {
            let (u, v) = surface_middle(&surface);
            let n = surface.normal(u, v);
            if crate::geom::finite(n) {
                n
            } else {
                wire_winding_normal(&face.boundaries()[0])
            }
        }
    }
}

fn surface_middle(surface: &Surface) -> (f64, f64) {
    use truck_modeling::ParametricSurface;
    let ((u0, u1), (v0, v1)) = surface.parameter_range();
    let mid = |a: std::ops::Bound<f64>, b: std::ops::Bound<f64>| match (a, b) {
        (std::ops::Bound::Included(a), std::ops::Bound::Included(b))
        | (std::ops::Bound::Excluded(a), std::ops::Bound::Excluded(b))
        | (std::ops::Bound::Included(a), std::ops::Bound::Excluded(b))
        | (std::ops::Bound::Excluded(a), std::ops::Bound::Included(b)) => (a + b) * 0.5,
        _ => 0.0,
    };
    (mid(u0, u1), mid(v0, v1))
}

/// Winding normal of a closed wire from points sampled along its curves.
/// Works for loops of fewer than three vertices, such as a two-arc circle.
pub(crate) fn wire_winding_normal(wire: &Wire) -> Vector3 {
    let mut points = Vec::new();
    for edge in wire.edge_iter() {
        let curve = edge.oriented_curve();
        let (t0, t1) = curve.range_tuple();
        for k in 0..8 {
            points.push(dvec(curve.subs(t0 + (t1 - t0) * k as f64 / 8.0)));
        }
    }
    vector(crate::geom::newell(&points))
}

// ---------------------------------------------------------------------------
// Sketch profiles
// ---------------------------------------------------------------------------

/// A sketch region as truck wires on the sketch plane (in part space).
pub(crate) struct ProfileWires {
    pub outer: Wire,
    pub holes: Vec<Wire>,
    /// Tightest arc radius in the profile, or infinity.
    pub min_radius: f64,
}

/// Regions picked by interior points, or the outermost regions (each with
/// its holes) when `selection` is empty. Adjacent picks are merged so they
/// extrude as one solid.
pub(crate) fn sketch_profiles(
    sketch: &Sketch,
    selection: &[Vec2],
) -> Result<Vec<forma_core::Profile>, KernelError> {
    sketch.validate().map_err(|error| {
        KernelError::new(
            format!("The sketch is invalid: {error}"),
            "Fix the sketch in Draw mode",
        )
    })?;
    let profiles = sketch.profiles(forma_core::SKETCH_TOLERANCE * 10.0);
    if profiles.is_empty() {
        return Err(KernelError::new(
            "The sketch has no closed regions",
            "Close the outline so its curves meet end to end",
        ));
    }
    let mut chosen: Vec<usize> = if selection.is_empty() {
        crate::regions::outermost_regions(&profiles)
    } else {
        // Regions never overlap, so each point picks at most one.
        selection
            .iter()
            .filter_map(|point| profiles.iter().position(|profile| profile.contains(*point)))
            .collect()
    };
    chosen.sort_unstable();
    chosen.dedup();
    if chosen.is_empty() {
        return Err(KernelError::new(
            "None of the selected regions exist any more",
            "Reselect the regions to use after editing the sketch",
        ));
    }
    let picked = chosen.into_iter().map(|i| profiles[i].clone()).collect();
    Ok(crate::regions::merge_adjacent(picked))
}

pub(crate) fn profile_wires(
    plane: &SketchPlane,
    profile: &forma_core::Profile,
) -> Result<ProfileWires, KernelError> {
    let mut min_radius = f64::INFINITY;
    let outer = loop_wire(plane, &profile.outer, &mut min_radius)?;
    let holes = profile
        .holes
        .iter()
        .map(|hole| loop_wire(plane, hole, &mut min_radius))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ProfileWires {
        outer,
        holes,
        min_radius,
    })
}

/// A planar face from profile wires. `try_attach_plane` accepts holes wound
/// the same way as the outer loop and silently builds a wrong solid, so hole
/// winding is normalized against the outer loop first.
pub(crate) fn profile_face(wires: &ProfileWires) -> Result<Face, KernelError> {
    let normal = wire_winding_normal(&wires.outer);
    let mut all = vec![wires.outer.clone()];
    for hole in &wires.holes {
        all.push(if wire_winding_normal(hole).dot(normal) > 0.0 {
            hole.inverse()
        } else {
            hole.clone()
        });
    }
    attach(&all)
}

fn loop_wire(
    plane: &SketchPlane,
    profile_loop: &ProfileLoop,
    min_radius: &mut f64,
) -> Result<Wire, KernelError> {
    let origin = plane.origin.as_dvec3();
    let (ax, ay) = (plane.x_axis.as_dvec3(), plane.y_axis.as_dvec3());
    let to_world = |p: DVec2| origin + ax * p.x + ay * p.y;
    // Split curves into pieces of at most ~half a turn, as (start, transit, end)
    // in sketch space, where `transit` is None for straight lines.
    let mut pieces: Vec<(DVec2, Option<DVec2>, DVec2)> = Vec::new();
    for segment in &profile_loop.segments {
        match *segment {
            SketchCurve::Line { start, end } => {
                pieces.push((start.as_dvec2(), None, end.as_dvec2()))
            }
            SketchCurve::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => {
                *min_radius = min_radius.min(f64::from(radius));
                arc_pieces(
                    center.as_dvec2(),
                    f64::from(radius),
                    f64::from(start_angle),
                    f64::from(sweep),
                    &mut pieces,
                );
            }
            SketchCurve::Circle { center, radius } => {
                *min_radius = min_radius.min(f64::from(radius));
                arc_pieces(center.as_dvec2(), f64::from(radius), 0.0, TAU, &mut pieces);
            }
        }
    }
    if pieces.is_empty() {
        return Err(KernelError::new("A profile loop is empty", ""));
    }
    // Consecutive pieces share vertices; the loop closes on the first one.
    let vertices: Vec<Vertex> = pieces
        .iter()
        .map(|(start, _, _)| builder::vertex(point(to_world(*start))))
        .collect();
    let mut edges = Vec::with_capacity(pieces.len());
    for (i, (_, transit, _)) in pieces.iter().enumerate() {
        let (a, b) = (&vertices[i], &vertices[(i + 1) % vertices.len()]);
        if a.point() == b.point() {
            continue;
        }
        edges.push(match transit {
            None => builder::line(a, b),
            Some(transit) => builder::circle_arc(a, b, point(to_world(*transit))),
        });
    }
    if edges.is_empty() {
        return Err(KernelError::new("A profile loop is degenerate", ""));
    }
    Ok(Wire::from(edges))
}

fn arc_pieces(
    center: DVec2,
    radius: f64,
    start: f64,
    sweep: f64,
    pieces: &mut Vec<(DVec2, Option<DVec2>, DVec2)>,
) {
    let count = ((sweep.abs() / (PI * 0.95)).ceil() as usize).max(1);
    let at = |angle: f64| center + DVec2::new(angle.cos(), angle.sin()) * radius;
    for k in 0..count {
        let a0 = start + sweep * k as f64 / count as f64;
        let a1 = start + sweep * (k + 1) as f64 / count as f64;
        pieces.push((at(a0), Some(at((a0 + a1) * 0.5)), at(a1)));
    }
}

// ---------------------------------------------------------------------------
// Extrude and revolve
// ---------------------------------------------------------------------------

/// Extrude every profile between offsets `start` and `end` along the plane
/// normal. Profiles never overlap, so their solids are combined as lumps.
pub(crate) fn extrude_profiles(
    plane: &SketchPlane,
    profiles: &[forma_core::Profile],
    start: f64,
    end: f64,
) -> Result<Body, KernelError> {
    if (end - start).abs() < f64::from(forma_core::MIN_PART_LENGTH) {
        return Err(KernelError::new(
            "The extrusion has zero length",
            "Enter a nonzero distance",
        ));
    }
    let normal = plane.normal().as_dvec3().normalize();
    let mut shells = Vec::new();
    let mut min_radius = f64::INFINITY;
    for profile in profiles {
        let wires = profile_wires(plane, profile)?;
        min_radius = min_radius.min(wires.min_radius);
        let mut face = profile_face(&wires)?;
        if start.abs() > 0.0 {
            // A fresh planar face has no intersection curves; moving it is safe.
            face = builder::translated(&face, vector(normal * start));
        }
        let solid = extrude_face(&face, vector(normal * (end - start)));
        let body = orient_outward(solid, wires.min_radius)?;
        if let Geometry::Solid(solid) = body.geometry {
            shells.extend(solid.into_boundaries());
        }
    }
    Ok(Body::solid(Solid::new(shells), min_radius))
}

/// Revolve profiles around an axis in the sketch plane.
pub(crate) fn revolve_profiles(
    plane: &SketchPlane,
    profiles: &[forma_core::Profile],
    axis_origin: Vec2,
    axis_direction: Vec2,
    angle: f64,
) -> Result<Body, KernelError> {
    let origin2 = axis_origin.as_dvec2();
    let direction2 = axis_direction.as_dvec2().normalize();
    let side = |p: DVec2| direction2.perp_dot(p - origin2);
    let world = |p: DVec2| {
        plane.origin.as_dvec3() + plane.x_axis.as_dvec3() * p.x + plane.y_axis.as_dvec3() * p.y
    };
    let axis = (plane.x_axis.as_dvec3() * direction2.x + plane.y_axis.as_dvec3() * direction2.y)
        .normalize();
    let origin = world(origin2);
    let full = angle.abs() >= TAU - 1.0e-6;
    let mut shells = Vec::new();
    let mut min_radius = f64::INFINITY;
    for profile in profiles {
        let polygon: Vec<DVec2> = profile
            .outer
            .polygon(profile_tolerance(profile))
            .iter()
            .map(|p| p.as_dvec2())
            .collect();
        let size = polygon
            .iter()
            .map(|p| p.distance(polygon[0]))
            .fold(0.0, f64::max)
            .max(1.0e-9);
        let sides: Vec<f64> = polygon.iter().map(|p| side(*p)).collect();
        let eps = size * 1.0e-6;
        let (low, high) = sides
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), s| {
                (lo.min(*s), hi.max(*s))
            });
        if low < -eps && high > eps {
            return Err(KernelError::new(
                "The profile crosses the revolve axis",
                "Move the axis to the profile's edge or split the profile at the axis",
            ));
        }
        let farthest = low.abs().max(high.abs());
        // Distance of the profile's nearest point from the axis bounds the
        // tightest curvature of the revolved surfaces.
        let nearest = sides.iter().map(|s| s.abs()).fold(f64::INFINITY, f64::min);
        let wires = profile_wires(plane, profile)?;
        min_radius = min_radius.min(wires.min_radius).min(if nearest > eps {
            nearest
        } else {
            farthest * 0.25
        });
        let touches = nearest <= eps;
        let solid = if touches && full && profile.holes.is_empty() {
            revolve_touching_axis(
                &wires.outer,
                origin,
                axis,
                side_points(&wires.outer, origin, axis, size),
            )?
        } else {
            let face = profile_face(&wires)?;
            builder::rsweep(&face, point(origin), vector(axis), Rad(angle))
        };
        let body = orient_outward(solid, min_radius)?;
        if let Geometry::Solid(solid) = body.geometry {
            shells.extend(solid.into_boundaries());
        }
    }
    Ok(Body::solid(Solid::new(shells), min_radius))
}

fn profile_tolerance(profile: &forma_core::Profile) -> f32 {
    let (min, max) = profile.outer.bounds();
    ((max - min).max_element() * 1.0e-3).max(1.0e-6)
}

/// Indices of wire vertices lying on the axis.
fn side_points(wire: &Wire, origin: DVec3, axis: DVec3, size: f64) -> Vec<bool> {
    wire.vertex_iter()
        .map(|v| {
            let p = dvec(v.point()) - origin;
            (p - axis * p.dot(axis)).length() <= size * 1.0e-6
        })
        .collect()
}

/// A full revolution of a profile with one run of edges on the axis. Those
/// edges would sweep into degenerate faces, so the rest of the loop is swept
/// with `builder::cone`, which closes the poles properly.
fn revolve_touching_axis(
    outer: &Wire,
    origin: DVec3,
    axis: DVec3,
    on_axis: Vec<bool>,
) -> Result<Solid, KernelError> {
    let edges: Vec<Edge> = outer.edge_iter().cloned().collect();
    let n = edges.len();
    // An edge lies on the axis when both of its vertices do.
    let axial: Vec<bool> = (0..n).map(|i| on_axis[i] && on_axis[(i + 1) % n]).collect();
    let runs = (0..n)
        .filter(|&i| axial[i] && !axial[(i + n - 1) % n])
        .count();
    if runs != 1 || axial.iter().all(|a| *a) {
        // Touching at isolated points, or several runs: fall back to a face sweep.
        let face = attach(std::slice::from_ref(outer))?;
        return Ok(builder::rsweep(
            &face,
            point(origin),
            vector(axis),
            Rad(TAU),
        ));
    }
    // Start right after the axial run and collect the off-axis edges.
    let first = (0..n)
        .find(|&i| !axial[i] && axial[(i + n - 1) % n])
        .unwrap();
    let open: Vec<Edge> = (0..n)
        .map(|k| (first + k) % n)
        .take_while(|&i| !axial[i])
        .map(|i| edges[i].clone())
        .collect();
    let wire = Wire::from(open);
    // `cone` sweeps around the axis through the wire's first vertex.
    let start = dvec(wire.front_vertex().unwrap().point());
    let along = (start - origin).dot(axis);
    debug_assert!((start - origin - axis * along).length() < 1.0e-6 * (1.0 + along.abs()));
    let shell: Shell = builder::cone(&wire, vector(axis), Rad(TAU));
    Ok(Solid::new(vec![shell]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::{SketchCurve, SketchEntity};

    fn volume(body: &Body) -> f64 {
        body.volume(body.tolerance(3.0e-4, 0.05)).unwrap()
    }

    #[test]
    fn primitives_have_expected_volumes_and_outward_orientation() {
        let cases = [
            (
                PrimitiveShape::Box {
                    size: glam::Vec3::new(1.0, 2.0, 3.0),
                },
                6.0,
            ),
            (
                PrimitiveShape::Cylinder {
                    radius: 0.5,
                    height: 2.0,
                },
                PI * 0.25 * 2.0,
            ),
            (PrimitiveShape::Sphere { radius: 1.0 }, 4.0 / 3.0 * PI),
            (
                PrimitiveShape::Cone {
                    bottom_radius: 1.0,
                    top_radius: 0.0,
                    height: 3.0,
                },
                PI,
            ),
            (
                PrimitiveShape::Cone {
                    bottom_radius: 1.0,
                    top_radius: 0.5,
                    height: 1.0,
                },
                PI / 3.0 * (1.0 + 0.5 + 0.25),
            ),
            (
                PrimitiveShape::Torus {
                    major_radius: 1.0,
                    minor_radius: 0.25,
                },
                2.0 * PI * PI * 1.0 * 0.0625,
            ),
        ];
        for (shape, expected) in cases {
            let body = primitive(&shape).unwrap();
            let v = volume(&body);
            assert!(
                (v - expected).abs() < expected * 0.01,
                "{shape:?}: volume {v} expected {expected}"
            );
        }
    }

    fn rectangle(min: (f32, f32), max: (f32, f32)) -> Vec<SketchEntity> {
        let p = |x: f32, y: f32| Vec2::new(x, y);
        let line = |a: Vec2, b: Vec2| SketchEntity::from(SketchCurve::Line { start: a, end: b });
        vec![
            line(p(min.0, min.1), p(max.0, min.1)),
            line(p(max.0, min.1), p(max.0, max.1)),
            line(p(max.0, max.1), p(min.0, max.1)),
            line(p(min.0, max.1), p(min.0, min.1)),
        ]
    }

    #[test]
    fn extruded_profiles_keep_holes_and_follow_the_plane_normal() {
        let mut sketch = Sketch::new(SketchPlane::ground());
        sketch.entities = rectangle((0.0, 0.0), (4.0, 3.0));
        sketch.entities.push(
            SketchCurve::Circle {
                center: Vec2::new(1.0, 1.5),
                radius: 0.5,
            }
            .into(),
        );
        // A clockwise hole drawn by hand must still be a hole.
        sketch.entities.push(
            SketchCurve::Arc {
                center: Vec2::new(3.0, 1.5),
                radius: 0.4,
                start_angle: 0.0,
                sweep: -PI as f32,
            }
            .into(),
        );
        sketch.entities.push(
            SketchCurve::Arc {
                center: Vec2::new(3.0, 1.5),
                radius: 0.4,
                start_angle: -PI as f32,
                sweep: -PI as f32,
            }
            .into(),
        );
        let profiles = sketch_profiles(&sketch, &[]).unwrap();
        assert_eq!(profiles.len(), 1);
        let body = extrude_profiles(&sketch.plane, &profiles, 0.0, 0.5).unwrap();
        let expected = (12.0 - PI * 0.25 - PI * 0.16) * 0.5;
        let v = volume(&body);
        assert!(
            (v - expected).abs() < 0.01,
            "volume {v} expected {expected}"
        );
        // Ground plane normal is +Y: the solid sits above y = 0.
        let bounds = body.trimesh(0.01).unwrap().bounds();
        assert!(bounds.min.y > -1.0e-6 && (bounds.max.y - 0.5).abs() < 1.0e-6);
        // Symmetric extents straddle the plane.
        let body = extrude_profiles(&sketch.plane, &profiles, -0.25, 0.25).unwrap();
        let bounds = body.trimesh(0.01).unwrap().bounds();
        assert!((bounds.min.y + 0.25).abs() < 1.0e-6 && (bounds.max.y - 0.25).abs() < 1.0e-6);
    }

    #[test]
    fn revolved_profiles_touching_the_axis_close_their_poles() {
        let mut sketch = Sketch::new(SketchPlane::front());
        sketch.entities = rectangle((0.0, 0.0), (1.0, 2.0));
        let profiles = sketch_profiles(&sketch, &[]).unwrap();
        let body = revolve_profiles(
            &sketch.plane,
            &profiles,
            Vec2::ZERO,
            Vec2::Y,
            std::f32::consts::TAU as f64,
        )
        .unwrap();
        let v = volume(&body);
        assert!((v - PI * 2.0).abs() < 0.05, "cylinder by revolution: {v}");
        // An offset profile makes a ring; half a turn halves it.
        sketch.entities = rectangle((0.5, 0.0), (1.0, 0.5));
        let profiles = sketch_profiles(&sketch, &[]).unwrap();
        let half = revolve_profiles(&sketch.plane, &profiles, Vec2::ZERO, Vec2::Y, PI).unwrap();
        let v = volume(&half);
        let expected = PI * (1.0 - 0.25) * 0.5 * 0.5;
        assert!(
            (v - expected).abs() < expected * 0.02,
            "half ring {v} vs {expected}"
        );
        // Crossing the axis is refused.
        sketch.entities = rectangle((-0.5, 0.0), (1.0, 0.5));
        let profiles = sketch_profiles(&sketch, &[]).unwrap();
        let error =
            revolve_profiles(&sketch.plane, &profiles, Vec2::ZERO, Vec2::Y, TAU).unwrap_err();
        assert!(error.message.contains("crosses"));
    }
}

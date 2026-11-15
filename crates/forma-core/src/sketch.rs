//! Planar 2D sketches: the shared drawing data behind Draw mode and CAD features.
//!
//! A sketch is a set of curves in the local 2D coordinates of a [`SketchPlane`].
//! Closed chains of curves form [`Profile`]s, which CAD features extrude or
//! revolve. Sketch data is plain, serializable geometry with no kernel types,
//! so scenes stay independent of the modelling kernel that consumes them.
use crate::Ray;
use anyhow::{Result, ensure};

mod arrangement;
use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

/// Largest coordinate accepted in sketch space, matching object translation limits.
pub const MAX_SKETCH_COORDINATE: f32 = 1.0e8;
/// Maximum number of curves in one sketch. Profiles and snapping are
/// rebuilt after every edit, so the limit keeps that interactive (about
/// 35 ms of arrangement work at the limit).
pub const MAX_SKETCH_ENTITIES: usize = 25_000;
/// Default distance under which sketch endpoints are considered coincident.
pub const SKETCH_TOLERANCE: f32 = 1.0e-5;

/// A plane in its owner's local space. `x_axis` and `y_axis` are orthonormal;
/// the plane normal is `x_axis × y_axis`, so sketch loops that wind counter
/// clockwise in 2D face along the normal.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchPlane {
    pub origin: Vec3,
    pub x_axis: Vec3,
    pub y_axis: Vec3,
}

impl Default for SketchPlane {
    fn default() -> Self {
        Self::ground()
    }
}

impl SketchPlane {
    /// The ground plane (world XZ, normal +Y). 2D +x is world +X and 2D +y is
    /// world −Z, so the view from above matches the 2D drawing orientation.
    pub fn ground() -> Self {
        Self {
            origin: Vec3::ZERO,
            x_axis: Vec3::X,
            y_axis: Vec3::NEG_Z,
        }
    }

    /// The front plane (world XY, normal +Z).
    pub fn front() -> Self {
        Self {
            origin: Vec3::ZERO,
            x_axis: Vec3::X,
            y_axis: Vec3::Y,
        }
    }

    /// The right plane (world ZY seen from +X, normal +X).
    pub fn right() -> Self {
        Self {
            origin: Vec3::ZERO,
            x_axis: Vec3::NEG_Z,
            y_axis: Vec3::Y,
        }
    }

    /// A plane through `origin` facing `normal`. The in-plane axes are chosen
    /// deterministically: x follows world X projected into the plane, falling
    /// back to world Z for planes that face along X.
    pub fn from_point_normal(origin: Vec3, normal: Vec3) -> Option<Self> {
        let normal = normal.try_normalize()?;
        let reference = if normal.x.abs() > 0.9 {
            Vec3::NEG_Z * normal.x.signum()
        } else {
            Vec3::X
        };
        let x_axis = (reference - normal * reference.dot(normal)).try_normalize()?;
        let y_axis = normal.cross(x_axis).normalize();
        let plane = Self {
            origin,
            x_axis,
            y_axis,
        };
        plane.is_valid().then_some(plane)
    }

    pub fn normal(&self) -> Vec3 {
        self.x_axis.cross(self.y_axis)
    }

    pub fn to_world(&self, point: Vec2) -> Vec3 {
        self.origin + self.x_axis * point.x + self.y_axis * point.y
    }

    /// Projects a 3D point orthogonally onto the plane.
    pub fn to_local(&self, point: Vec3) -> Vec2 {
        let offset = point - self.origin;
        Vec2::new(offset.dot(self.x_axis), offset.dot(self.y_axis))
    }

    /// Plane-to-space matrix: columns are x, y, normal and origin.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_cols(
            self.x_axis.extend(0.0),
            self.y_axis.extend(0.0),
            self.normal().extend(0.0),
            self.origin.extend(1.0),
        )
    }

    /// The plane transformed by an affine matrix. Returns `None` when the
    /// transform collapses the plane.
    pub fn transformed(&self, matrix: Mat4) -> Option<Self> {
        let origin = matrix.transform_point3(self.origin);
        let x = matrix.transform_vector3(self.x_axis);
        let y = matrix.transform_vector3(self.y_axis);
        let normal = x.cross(y).try_normalize()?;
        let x_axis = x.try_normalize()?;
        let y_axis = normal.cross(x_axis).normalize();
        let plane = Self {
            origin,
            x_axis,
            y_axis,
        };
        plane.is_valid().then_some(plane)
    }

    /// The 2D point where `ray` meets the plane, in front of the ray origin.
    pub fn intersect(&self, ray: Ray) -> Option<Vec2> {
        let normal = self.normal();
        let denominator = ray.direction.dot(normal);
        if denominator.abs() < 1.0e-8 {
            return None;
        }
        let t = (self.origin - ray.origin).dot(normal) / denominator;
        (t.is_finite() && t >= 0.0).then(|| self.to_local(ray.origin + ray.direction * t))
    }

    pub fn is_valid(&self) -> bool {
        self.origin.is_finite()
            && self.x_axis.is_finite()
            && self.y_axis.is_finite()
            && self.origin.abs().max_element() <= MAX_SKETCH_COORDINATE
            && (self.x_axis.length() - 1.0).abs() < 1.0e-3
            && (self.y_axis.length() - 1.0).abs() < 1.0e-3
            && self.x_axis.dot(self.y_axis).abs() < 1.0e-3
    }
}

/// One curve in sketch coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SketchCurve {
    Line {
        start: Vec2,
        end: Vec2,
    },
    /// A circular arc starting at `start_angle` and sweeping `sweep` radians;
    /// positive sweeps run counter clockwise. `|sweep|` is below one turn.
    Arc {
        center: Vec2,
        radius: f32,
        start_angle: f32,
        sweep: f32,
    },
    Circle {
        center: Vec2,
        radius: f32,
    },
}

impl SketchCurve {
    /// An arc through three points, or `None` when they are (nearly) collinear.
    pub fn arc_through(start: Vec2, through: Vec2, end: Vec2) -> Option<Self> {
        // Work in double precision relative to `start`: absolute coordinates
        // far from the origin cancel catastrophically in f32.
        let a = start.as_dvec2();
        let (b, c) = (through.as_dvec2() - a, end.as_dvec2() - a);
        let d = 2.0 * b.perp_dot(c);
        let scale = b.length().max(c.length()).max(1.0e-12);
        if d.abs() < 1.0e-9 * scale * scale {
            return None;
        }
        let (b2, c2) = (b.length_squared(), c.length_squared());
        let relative = glam::DVec2::new(c.y * b2 - b.y * c2, b.x * c2 - c.x * b2) / d;
        let center = (a + relative).as_vec2();
        if !center.is_finite() {
            return None;
        }
        // Measure from the stored (rounded) centre so the arc reproduces its
        // start exactly and its end within the centre's rounding.
        let origin = center.as_dvec2();
        let offset = |p: Vec2| p.as_dvec2() - origin;
        let radius = offset(start).length();
        let angle = |p: Vec2| {
            let v = offset(p);
            v.y.atan2(v.x)
        };
        let start_angle = angle(start);
        let ccw = |angle: f64| (angle - start_angle).rem_euclid(std::f64::consts::TAU);
        let to_end = ccw(angle(end));
        let to_through = ccw(angle(through));
        let sweep = if to_through <= to_end {
            to_end
        } else {
            to_end - std::f64::consts::TAU
        };
        let curve = Self::Arc {
            center,
            radius: radius as f32,
            start_angle: start_angle as f32,
            sweep: sweep as f32,
        };
        curve.is_valid().then_some(curve)
    }

    pub fn start(&self) -> Vec2 {
        self.point_at(0.0)
    }

    pub fn end(&self) -> Vec2 {
        self.point_at(1.0)
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, Self::Circle { .. })
    }

    /// The point at normalized parameter `t` in 0..=1.
    pub fn point_at(&self, t: f32) -> Vec2 {
        match *self {
            Self::Line { start, end } => start.lerp(end, t),
            Self::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => center + Vec2::from_angle(start_angle + sweep * t) * radius,
            Self::Circle { center, radius } => center + Vec2::from_angle(TAU * t) * radius,
        }
    }

    /// Unit tangent at `t`, following the curve direction.
    pub fn tangent_at(&self, t: f32) -> Vec2 {
        match *self {
            Self::Line { start, end } => (end - start).normalize_or_zero(),
            Self::Arc {
                start_angle, sweep, ..
            } => Vec2::from_angle(start_angle + sweep * t).perp() * sweep.signum(),
            Self::Circle { .. } => Vec2::from_angle(TAU * t).perp(),
        }
    }

    pub fn midpoint(&self) -> Vec2 {
        self.point_at(0.5)
    }

    /// The circle or arc center.
    pub fn center(&self) -> Option<Vec2> {
        match *self {
            Self::Line { .. } => None,
            Self::Arc { center, .. } | Self::Circle { center, .. } => Some(center),
        }
    }

    pub fn length(&self) -> f32 {
        match *self {
            Self::Line { start, end } => start.distance(end),
            Self::Arc { radius, sweep, .. } => radius * sweep.abs(),
            Self::Circle { radius, .. } => radius * TAU,
        }
    }

    pub fn reversed(&self) -> Self {
        match *self {
            Self::Line { start, end } => Self::Line {
                start: end,
                end: start,
            },
            Self::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => Self::Arc {
                center,
                radius,
                start_angle: start_angle + sweep,
                sweep: -sweep,
            },
            circle @ Self::Circle { .. } => circle,
        }
    }

    /// Points along the curve whose chords deviate from it by at most
    /// `tolerance`. Includes both endpoints; closed curves repeat the start.
    pub fn polyline(&self, tolerance: f32) -> Vec<Vec2> {
        match *self {
            Self::Line { start, end } => vec![start, end],
            Self::Arc { radius, sweep, .. } => {
                let segments = arc_segments(radius, sweep.abs(), tolerance);
                (0..=segments)
                    .map(|i| self.point_at(i as f32 / segments as f32))
                    .collect()
            }
            Self::Circle { radius, .. } => {
                let segments = arc_segments(radius, TAU, tolerance);
                (0..=segments)
                    .map(|i| self.point_at(i as f32 / segments as f32))
                    .collect()
            }
        }
    }

    /// The closest parameter on the curve and the distance to it.
    pub fn closest(&self, point: Vec2) -> (f32, f32) {
        match *self {
            Self::Line { start, end } => {
                let direction = end - start;
                let t = if direction.length_squared() > 0.0 {
                    ((point - start).dot(direction) / direction.length_squared()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (t, point.distance(start + direction * t))
            }
            Self::Circle { center, radius } => {
                let offset = point - center;
                let t = offset.to_angle().rem_euclid(TAU) / TAU;
                (t, (offset.length() - radius).abs())
            }
            Self::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => {
                let offset = point - center;
                let relative = (offset.to_angle() - start_angle).rem_euclid(TAU);
                let along = if sweep >= 0.0 {
                    relative
                } else {
                    TAU - relative
                };
                if along <= sweep.abs() {
                    let t = along / sweep.abs().max(f32::EPSILON);
                    (t, (offset.length() - radius).abs())
                } else {
                    let to_start = point.distance(self.start());
                    let to_end = point.distance(self.end());
                    if to_start <= to_end {
                        (0.0, to_start)
                    } else {
                        (1.0, to_end)
                    }
                }
            }
        }
    }

    /// The portion between normalized parameters `t0` and `t1`. A circle's
    /// portion is an arc; `t1` may exceed one to wrap past angle zero.
    pub fn sub_curve(&self, t0: f32, t1: f32) -> Self {
        match *self {
            Self::Line { .. } => Self::Line {
                start: self.point_at(t0),
                end: self.point_at(t1),
            },
            Self::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => Self::Arc {
                center,
                radius,
                start_angle: start_angle + sweep * t0,
                sweep: sweep * (t1 - t0),
            },
            Self::Circle { center, radius } => Self::Arc {
                center,
                radius,
                start_angle: TAU * t0,
                sweep: TAU * (t1 - t0),
            },
        }
    }

    /// The same curve translated by `offset`.
    pub fn translated(&self, offset: Vec2) -> Self {
        match *self {
            Self::Line { start, end } => Self::Line {
                start: start + offset,
                end: end + offset,
            },
            Self::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => Self::Arc {
                center: center + offset,
                radius,
                start_angle,
                sweep,
            },
            Self::Circle { center, radius } => Self::Circle {
                center: center + offset,
                radius,
            },
        }
    }

    /// Points where a circle or arc reaches its extreme x or y.
    pub fn quadrant_points(&self) -> Vec<Vec2> {
        match *self {
            Self::Line { .. } => Vec::new(),
            Self::Circle { center, radius } => (0..4)
                .map(|i| center + Vec2::from_angle(i as f32 * std::f32::consts::FRAC_PI_2) * radius)
                .collect(),
            Self::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => (0..4)
                .filter_map(|i| {
                    let angle = i as f32 * std::f32::consts::FRAC_PI_2;
                    let relative = (angle - start_angle).rem_euclid(TAU);
                    let along = if sweep >= 0.0 {
                        relative
                    } else {
                        (TAU - relative).rem_euclid(TAU)
                    };
                    (along <= sweep.abs()).then(|| center + Vec2::from_angle(angle) * radius)
                })
                .collect(),
        }
    }

    /// Axis-aligned bounds as (min, max).
    pub fn bounds(&self) -> (Vec2, Vec2) {
        match *self {
            Self::Line { start, end } => (start.min(end), start.max(end)),
            Self::Circle { center, radius } => (center - radius, center + radius),
            Self::Arc { .. } => {
                let points = self.polyline(self.length() * 1.0e-3 + 1.0e-6);
                points.iter().fold(
                    (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
                    |(min, max), p| (min.min(*p), max.max(*p)),
                )
            }
        }
    }

    pub fn is_valid(&self) -> bool {
        let finite = |p: Vec2| p.is_finite() && p.abs().max_element() <= MAX_SKETCH_COORDINATE;
        match *self {
            Self::Line { start, end } => {
                finite(start) && finite(end) && start.distance(end) > SKETCH_TOLERANCE
            }
            Self::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => {
                finite(center)
                    && radius.is_finite()
                    && radius > SKETCH_TOLERANCE
                    && radius <= MAX_SKETCH_COORDINATE
                    && start_angle.is_finite()
                    && sweep.is_finite()
                    && sweep.abs() > 1.0e-5
                    && sweep.abs() < TAU
                    // Like lines, arcs must be longer than the join tolerance.
                    && radius * sweep.abs() > SKETCH_TOLERANCE
            }
            Self::Circle { center, radius } => {
                finite(center)
                    && radius.is_finite()
                    && radius > SKETCH_TOLERANCE
                    && radius <= MAX_SKETCH_COORDINATE
            }
        }
    }
}

fn arc_segments(radius: f32, sweep: f32, tolerance: f32) -> usize {
    let tolerance = tolerance.max(radius * 1.0e-5).max(1.0e-7);
    // Sagitta s = r(1 − cos(θ/2)) ≤ tolerance gives the largest chord angle.
    let step = 2.0 * (1.0 - (tolerance / radius).min(1.0)).acos();
    let segments = if step > 1.0e-6 {
        (sweep / step).ceil() as usize
    } else {
        512
    };
    segments.clamp(if sweep >= TAU - 1.0e-4 { 8 } else { 1 }, 1024)
}

/// A sketch curve with drawing attributes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchEntity {
    pub curve: SketchCurve,
    /// Construction geometry guides drawing but never forms profiles.
    #[serde(default)]
    pub construction: bool,
}

impl From<SketchCurve> for SketchEntity {
    fn from(curve: SketchCurve) -> Self {
        Self {
            curve,
            construction: false,
        }
    }
}

/// A closed, consistently oriented chain of curves. Each segment's end meets
/// the next segment's start; the last segment returns to the first start.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileLoop {
    pub segments: Vec<SketchCurve>,
    /// Index of the source entity for each segment.
    pub entities: Vec<usize>,
}

impl ProfileLoop {
    /// Positive for counter-clockwise loops. Exact for lines and arcs.
    pub fn signed_area(&self) -> f32 {
        let mut area = 0.0_f64;
        for segment in &self.segments {
            match *segment {
                SketchCurve::Line { start, end } => {
                    area += start.as_dvec2().perp_dot(end.as_dvec2());
                }
                SketchCurve::Arc { radius, sweep, .. } => {
                    // Chord triangle from the origin plus the circular segment.
                    let (start, end) = (segment.start().as_dvec2(), segment.end().as_dvec2());
                    let (r, w) = (radius as f64, sweep as f64);
                    area += start.perp_dot(end) + r * r * (w - w.sin());
                }
                SketchCurve::Circle { radius, .. } => {
                    area += std::f64::consts::TAU * (radius as f64).powi(2);
                }
            }
        }
        (area * 0.5) as f32
    }

    /// The loop as a closed polygon (first point not repeated).
    pub fn polygon(&self, tolerance: f32) -> Vec<Vec2> {
        let mut points = Vec::new();
        for segment in &self.segments {
            let mut polyline = segment.polyline(tolerance);
            polyline.pop();
            points.extend(polyline);
        }
        points
    }

    pub fn contains(&self, point: Vec2) -> bool {
        point_in_polygon(point, &self.polygon(self.tolerance()))
    }

    fn tolerance(&self) -> f32 {
        let (min, max) = self.bounds();
        ((max - min).max_element() * 1.0e-3).max(1.0e-6)
    }

    pub fn bounds(&self) -> (Vec2, Vec2) {
        self.segments.iter().map(SketchCurve::bounds).fold(
            (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
            |(min, max), (a, b)| (min.min(a), max.max(b)),
        )
    }
}

/// A closed region: a counter-clockwise outer loop with clockwise holes.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub outer: ProfileLoop,
    pub holes: Vec<ProfileLoop>,
}

impl Profile {
    pub fn area(&self) -> f32 {
        self.outer.signed_area() + self.holes.iter().map(ProfileLoop::signed_area).sum::<f32>()
    }

    /// Whether `point` lies inside the outer loop and outside every hole.
    pub fn contains(&self, point: Vec2) -> bool {
        self.outer.contains(point) && !self.holes.iter().any(|hole| hole.contains(point))
    }
}

/// A planar drawing in the local space of its plane.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    pub plane: SketchPlane,
    #[serde(default)]
    pub entities: Vec<SketchEntity>,
}

impl Sketch {
    pub fn new(plane: SketchPlane) -> Self {
        Self {
            plane,
            entities: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.plane.is_valid(), "Invalid sketch plane");
        ensure!(
            self.entities.len() <= MAX_SKETCH_ENTITIES,
            "Sketch exceeds {MAX_SKETCH_ENTITIES} curves"
        );
        for (index, entity) in self.entities.iter().enumerate() {
            ensure!(entity.curve.is_valid(), "Invalid sketch curve {index}");
        }
        Ok(())
    }

    /// Closed regions formed by non-construction curves: the bounded faces
    /// of their planar arrangement.
    ///
    /// Curves are split wherever they cross, touch or overlap, and endpoints
    /// closer than `tolerance` are joined (or closer than single precision
    /// can resolve at the sketch's coordinates, when that is coarser, so
    /// arcs far from the origin still close), so overlapping rectangles yield one
    /// profile per region and lines overshooting a corner still close it.
    /// Dangling curves never bound a region. Each profile's outer loop runs
    /// counter clockwise; connected groups of curves lying wholly inside it
    /// are clockwise holes, and the regions inside those groups are profiles
    /// of their own. Consecutive pieces of one curve are merged again, so a
    /// lone circle stays a single circle segment. Profiles are ordered from
    /// largest to smallest area.
    pub fn profiles(&self, tolerance: f32) -> Vec<Profile> {
        arrangement::profiles(&self.entities, tolerance.max(SKETCH_TOLERANCE))
    }

    /// Points where two curves cross or touch away from shared endpoints,
    /// including construction curves. Useful as snapping targets.
    pub fn intersection_points(&self, tolerance: f32) -> Vec<Vec2> {
        arrangement::crossing_points(&self.entities, tolerance.max(SKETCH_TOLERANCE) as f64)
    }

    /// The piece of curve `entity` between the nearest curve contacts on
    /// either side of `at`, and what remains of the curve without it.
    /// Contacts include construction curves and T-junctions. A curve that
    /// meets nothing is removed whole.
    pub fn trim_piece(&self, entity: usize, at: Vec2, tolerance: f32) -> Option<TrimPiece> {
        let tol = tolerance.max(SKETCH_TOLERANCE) as f64;
        let (geo, params) = arrangement::split_parameters(&self.entities, entity, tol)?;
        let (t, _) = geo.closest(at.as_dvec2());
        let piece = |a: f64, b: f64| geo.sub(a, b).to_curve();
        let valid = |curve: &SketchCurve| curve.is_valid();
        if geo.is_closed() {
            if params.len() < 2 {
                return Some(TrimPiece {
                    entity,
                    removed: self.entities[entity].curve,
                    kept: Vec::new(),
                });
            }
            let after = params.iter().position(|&p| p > t).unwrap_or(params.len());
            let (start, end) = if after == 0 || after == params.len() {
                (params[params.len() - 1], params[0] + 1.0)
            } else {
                (params[after - 1], params[after])
            };
            let kept = piece(end, start + 1.0);
            return Some(TrimPiece {
                entity,
                removed: piece(start, end),
                kept: [kept].into_iter().filter(valid).collect(),
            });
        }
        if params.len() < 2 {
            return Some(TrimPiece {
                entity,
                removed: self.entities[entity].curve,
                kept: Vec::new(),
            });
        }
        let after = params
            .iter()
            .position(|&p| p > t)
            .unwrap_or(params.len() - 1)
            .clamp(1, params.len() - 1);
        let (start, end) = (params[after - 1], params[after]);
        let mut kept = Vec::new();
        if start > 0.0 {
            kept.push(piece(0.0, start));
        }
        if end < 1.0 {
            kept.push(piece(end, 1.0));
        }
        kept.retain(valid);
        Some(TrimPiece {
            entity,
            removed: piece(start, end),
            kept,
        })
    }

    /// Removes the piece found by [`Sketch::trim_piece`]. The remaining
    /// parts keep the curve's construction flag; a second remaining part is
    /// appended. Returns whether anything changed.
    pub fn trim(&mut self, entity: usize, at: Vec2, tolerance: f32) -> bool {
        let Some(trim) = self.trim_piece(entity, at, tolerance) else {
            return false;
        };
        let construction = self.entities[entity].construction;
        let mut kept = trim.kept.into_iter();
        match kept.next() {
            Some(first) => self.entities[entity].curve = first,
            None => {
                self.entities.remove(entity);
            }
        }
        self.entities.extend(kept.map(|curve| SketchEntity {
            curve,
            construction,
        }));
        true
    }
}

/// Result of [`Sketch::trim_piece`].
#[derive(Clone, Debug, PartialEq)]
pub struct TrimPiece {
    pub entity: usize,
    pub removed: SketchCurve,
    /// Zero, one or two curves that replace the trimmed entity.
    pub kept: Vec<SketchCurve>,
}

pub(crate) fn point_in_polygon(point: Vec2, polygon: &[Vec2]) -> bool {
    let mut inside = false;
    let mut j = polygon.len().wrapping_sub(1);
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[j]);
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(a: (f32, f32), b: (f32, f32)) -> SketchEntity {
        SketchCurve::Line {
            start: Vec2::new(a.0, a.1),
            end: Vec2::new(b.0, b.1),
        }
        .into()
    }

    fn rectangle(min: (f32, f32), max: (f32, f32)) -> Vec<SketchEntity> {
        vec![
            line(min, (max.0, min.1)),
            line((max.0, min.1), max),
            line(max, (min.0, max.1)),
            line((min.0, max.1), min),
        ]
    }

    #[test]
    fn planes_round_trip_points_and_face_their_normals() {
        for plane in [
            SketchPlane::ground(),
            SketchPlane::front(),
            SketchPlane::right(),
            SketchPlane::from_point_normal(Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 1.0, 0.0))
                .unwrap(),
            SketchPlane::from_point_normal(Vec3::ZERO, Vec3::X).unwrap(),
        ] {
            assert!(plane.is_valid());
            let p = Vec2::new(0.25, -1.5);
            assert!(plane.to_local(plane.to_world(p)).distance(p) < 1.0e-5);
        }
        assert!((SketchPlane::ground().normal() - Vec3::Y).length() < 1.0e-6);
        assert!((SketchPlane::front().normal() - Vec3::Z).length() < 1.0e-6);
        assert!((SketchPlane::right().normal() - Vec3::X).length() < 1.0e-6);
        let normal = Vec3::new(1.0, 1.0, 0.0).normalize();
        let plane = SketchPlane::from_point_normal(Vec3::ZERO, normal).unwrap();
        assert!(plane.normal().distance(normal) < 1.0e-5);
    }

    #[test]
    fn rays_hit_planes_only_in_front() {
        let plane = SketchPlane::ground();
        let hit = plane.intersect(Ray {
            origin: Vec3::new(2.0, 5.0, -3.0),
            direction: Vec3::NEG_Y,
        });
        assert_eq!(hit, Some(Vec2::new(2.0, 3.0)));
        assert!(
            plane
                .intersect(Ray {
                    origin: Vec3::new(0.0, 5.0, 0.0),
                    direction: Vec3::Y,
                })
                .is_none()
        );
    }

    #[test]
    fn arcs_through_three_points_pass_through_them() {
        let (a, b, c) = (
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
        );
        let arc = SketchCurve::arc_through(a, b, c).unwrap();
        assert!(arc.start().distance(a) < 1.0e-5);
        assert!(arc.end().distance(c) < 1.0e-5);
        assert!(arc.midpoint().distance(b) < 1.0e-5);
        let clockwise = SketchCurve::arc_through(c, b, a).unwrap();
        assert!(clockwise.midpoint().distance(b) < 1.0e-5);
        assert!(SketchCurve::arc_through(a, Vec2::ZERO, c).is_none());
        assert!((arc.length() - std::f32::consts::PI).abs() < 1.0e-4);
    }

    #[test]
    fn polylines_respect_tolerance() {
        let circle = SketchCurve::Circle {
            center: Vec2::ZERO,
            radius: 2.0,
        };
        let points = circle.polyline(1.0e-3);
        assert!(points.first().unwrap().distance(*points.last().unwrap()) < 1.0e-5);
        for pair in points.windows(2) {
            let mid = (pair[0] + pair[1]) * 0.5;
            assert!(2.0 - mid.length() <= 1.0e-3 + 1.0e-6);
        }
    }

    #[test]
    fn rectangles_and_circles_form_profiles_with_holes() {
        let mut sketch = Sketch::new(SketchPlane::ground());
        sketch.entities = rectangle((0.0, 0.0), (4.0, 3.0));
        // Shuffle direction of one edge; chaining must still close the loop.
        sketch.entities[2].curve = sketch.entities[2].curve.reversed();
        sketch.entities.push(
            SketchCurve::Circle {
                center: Vec2::new(2.0, 1.5),
                radius: 0.5,
            }
            .into(),
        );
        let profiles = sketch.profiles(SKETCH_TOLERANCE);
        // The ring around the circle and the disc inside it are both regions.
        assert_eq!(profiles.len(), 2);
        let profile = &profiles[0];
        assert_eq!(profile.holes.len(), 1);
        assert!(profile.outer.signed_area() > 0.0);
        assert!(profile.holes[0].signed_area() < 0.0);
        let expected = 12.0 - std::f32::consts::PI * 0.25;
        assert!((profile.area() - expected).abs() < 0.01);
        assert!(profile.contains(Vec2::new(0.5, 0.5)));
        assert!(!profile.contains(Vec2::new(2.0, 1.5)));
        let disc = &profiles[1];
        assert!(disc.holes.is_empty());
        assert_eq!(disc.outer.segments.len(), 1, "a lone circle stays whole");
        assert!(matches!(disc.outer.segments[0], SketchCurve::Circle { .. }));
        assert!(disc.contains(Vec2::new(2.0, 1.5)));
        // The hole is the same circle, traversed clockwise in two halves.
        assert_eq!(profile.holes[0].entities, vec![4, 4]);
        assert_loops_are_closed(&profiles);
    }

    #[test]
    fn nested_loops_are_separate_regions_and_construction_is_ignored() {
        let mut sketch = Sketch::new(SketchPlane::front());
        sketch.entities = rectangle((0.0, 0.0), (10.0, 10.0));
        sketch.entities.extend(rectangle((2.0, 2.0), (8.0, 8.0)));
        sketch.entities.extend(rectangle((4.0, 4.0), (6.0, 6.0)));
        let mut construction: SketchEntity = SketchCurve::Circle {
            center: Vec2::new(5.0, 5.0),
            radius: 20.0,
        }
        .into();
        construction.construction = true;
        sketch.entities.push(construction);
        let profiles = sketch.profiles(SKETCH_TOLERANCE);
        assert_eq!(profiles.len(), 3);
        let areas: Vec<f32> = profiles.iter().map(Profile::area).collect();
        for (area, expected) in areas.iter().zip([64.0, 32.0, 4.0]) {
            assert!((area - expected).abs() < 1.0e-3, "{areas:?}");
        }
        assert_eq!(profiles[0].holes.len(), 1);
        assert_eq!(profiles[1].holes.len(), 1);
        assert!(profiles[2].holes.is_empty());
        // Every region is reported once: the point sets do not overlap.
        for point in [
            Vec2::new(1.0, 1.0),
            Vec2::new(3.0, 3.0),
            Vec2::new(5.0, 5.0),
        ] {
            assert_eq!(profiles.iter().filter(|p| p.contains(point)).count(), 1);
        }
        assert_loops_are_closed(&profiles);
    }

    #[test]
    fn open_chains_and_branches_do_not_form_profiles() {
        let mut sketch = Sketch {
            entities: vec![line((0.0, 0.0), (1.0, 0.0)), line((1.0, 0.0), (1.0, 1.0))],
            ..Sketch::default()
        };
        assert!(sketch.profiles(SKETCH_TOLERANCE).is_empty());
        // An arc closing a line forms a D-shaped profile.
        sketch.entities = vec![
            line((-1.0, 0.0), (1.0, 0.0)),
            SketchCurve::arc_through(
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 1.0),
                Vec2::new(-1.0, 0.0),
            )
            .unwrap()
            .into(),
        ];
        let profiles = sketch.profiles(SKETCH_TOLERANCE);
        assert_eq!(profiles.len(), 1);
        assert!((profiles[0].area() - std::f32::consts::FRAC_PI_2).abs() < 0.01);
    }

    #[test]
    fn closest_points_follow_arcs_and_clamp_to_endpoints() {
        let arc = SketchCurve::arc_through(
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
        )
        .unwrap();
        let (t, d) = arc.closest(Vec2::new(0.0, 2.0));
        assert!((t - 0.5).abs() < 1.0e-4 && (d - 1.0).abs() < 1.0e-4);
        let (t, _) = arc.closest(Vec2::new(0.5, -3.0));
        assert!(t == 0.0 || t == 1.0);
    }

    #[test]
    fn validation_rejects_degenerate_curves() {
        let mut sketch = Sketch::default();
        sketch.entities.push(line((0.0, 0.0), (0.0, 0.0)));
        assert!(sketch.validate().is_err());
        sketch.entities[0] = line((0.0, 0.0), (1.0, 0.0));
        assert!(sketch.validate().is_ok());
        sketch.plane.x_axis = Vec3::new(2.0, 0.0, 0.0);
        assert!(sketch.validate().is_err());
    }

    fn circle(center: (f32, f32), radius: f32) -> SketchEntity {
        SketchCurve::Circle {
            center: Vec2::new(center.0, center.1),
            radius,
        }
        .into()
    }

    fn sketch_of(entities: Vec<SketchEntity>) -> Sketch {
        Sketch {
            entities,
            ..Sketch::default()
        }
    }

    fn sorted_areas(profiles: &[Profile]) -> Vec<f32> {
        let mut areas: Vec<f32> = profiles.iter().map(Profile::area).collect();
        areas.sort_by(f32::total_cmp);
        areas
    }

    fn assert_close(actual: &[f32], expected: &[f32], tolerance: f32) {
        assert_eq!(actual.len(), expected.len(), "{actual:?} vs {expected:?}");
        for (a, e) in actual.iter().zip(expected) {
            assert!((a - e).abs() <= tolerance, "{actual:?} vs {expected:?}");
        }
    }

    /// Every loop is a closed chain and outer/hole orientations are correct.
    fn assert_loops_are_closed(profiles: &[Profile]) {
        for profile in profiles {
            assert!(profile.outer.signed_area() > 0.0);
            for hole in &profile.holes {
                assert!(hole.signed_area() < 0.0);
            }
            for profile_loop in std::iter::once(&profile.outer).chain(&profile.holes) {
                assert_eq!(profile_loop.segments.len(), profile_loop.entities.len());
                let count = profile_loop.segments.len();
                for (i, segment) in profile_loop.segments.iter().enumerate() {
                    assert!(segment.is_valid(), "{segment:?}");
                    if count == 1 {
                        assert!(
                            segment.is_closed() || segment.start().distance(segment.end()) < 1.0e-4
                        );
                        continue;
                    }
                    let next = profile_loop.segments[(i + 1) % count];
                    assert!(
                        segment.end().distance(next.start()) < 1.0e-4,
                        "gap between {segment:?} and {next:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn overlapping_rectangles_split_into_three_regions() {
        let mut entities = rectangle((0.0, 0.0), (4.0, 2.0));
        entities.extend(rectangle((2.0, 1.0), (6.0, 3.0)));
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        assert_close(&sorted_areas(&profiles), &[2.0, 6.0, 6.0], 1.0e-4);
        let overlap = profiles
            .iter()
            .find(|p| p.contains(Vec2::new(3.0, 1.5)))
            .unwrap();
        assert!((overlap.area() - 2.0).abs() < 1.0e-4);
        // The overlap is bounded by pieces of both rectangles.
        assert!(overlap.outer.entities.iter().any(|&e| e < 4));
        assert!(overlap.outer.entities.iter().any(|&e| e >= 4));
        assert_loops_are_closed(&profiles);
    }

    #[test]
    fn overlapping_circles_form_a_lens_and_two_moons() {
        let sketch = sketch_of(vec![circle((0.0, 0.0), 1.0), circle((1.0, 0.0), 1.0)]);
        let profiles = sketch.profiles(SKETCH_TOLERANCE);
        let lens = 2.0 * std::f32::consts::PI / 3.0 - 3.0_f32.sqrt() / 2.0;
        let moon = std::f32::consts::PI - lens;
        assert_close(&sorted_areas(&profiles), &[lens, moon, moon], 2.0e-3);
        for profile in &profiles {
            assert!(
                profile
                    .outer
                    .segments
                    .iter()
                    .all(|s| matches!(s, SketchCurve::Arc { .. }))
            );
        }
        assert_loops_are_closed(&profiles);
    }

    #[test]
    fn rectangle_crossed_by_circle_and_line_splits_regions() {
        let mut entities = rectangle((0.0, 0.0), (4.0, 4.0));
        entities.push(circle((4.0, 2.0), 1.0));
        let profiles = sketch_of(entities.clone()).profiles(SKETCH_TOLERANCE);
        let half = std::f32::consts::PI * 0.5;
        assert_close(&sorted_areas(&profiles), &[half, half, 16.0 - half], 1.0e-3);
        // A chord line across the square adds one more split.
        entities.push(line((0.0, 3.0), (4.0, 3.0)));
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        assert_eq!(profiles.len(), 4);
        assert_loops_are_closed(&profiles);
    }

    #[test]
    fn t_junctions_and_overshoots_split_regions() {
        let mut entities = rectangle((0.0, 0.0), (4.0, 2.0));
        // Ends exactly on the top and bottom edges.
        entities.push(line((1.0, 0.0), (1.0, 2.0)));
        // Overshoots both edges; the dangling ends are ignored.
        entities.push(line((3.0, -1.0), (3.0, 3.0)));
        // A dangling spur inside a region does not change it.
        entities.push(line((2.0, 1.0), (2.5, 1.5)));
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        assert_close(&sorted_areas(&profiles), &[2.0, 2.0, 4.0], 1.0e-4);
        assert_loops_are_closed(&profiles);
        // Corners where four lines overshoot each other still close.
        let profiles = sketch_of(vec![
            line((-0.5, 0.0), (2.5, 0.0)),
            line((2.0, -0.5), (2.0, 2.5)),
            line((2.5, 2.0), (-0.5, 2.0)),
            line((0.0, 2.5), (0.0, -0.5)),
        ])
        .profiles(SKETCH_TOLERANCE);
        assert_close(&sorted_areas(&profiles), &[4.0], 1.0e-4);
        assert_eq!(profiles[0].outer.segments.len(), 4);
    }

    #[test]
    fn tangent_circles_touch_without_merging_regions() {
        // Internal tangency: a circle touching the inside of a square.
        let mut entities = rectangle((0.0, 0.0), (2.0, 2.0));
        entities.push(circle((1.0, 0.5), 0.5));
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        let disc = std::f32::consts::PI * 0.25;
        assert_close(&sorted_areas(&profiles), &[disc, 4.0 - disc], 1.0e-3);
        assert_loops_are_closed(&profiles);
        // External tangency: two discs that only share one point.
        let profiles = sketch_of(vec![circle((0.0, 0.0), 1.0), circle((2.0, 0.0), 1.0)])
            .profiles(SKETCH_TOLERANCE);
        let disc = std::f32::consts::PI;
        assert_close(&sorted_areas(&profiles), &[disc, disc], 2.0e-3);
        // A line tangent to a circle does not cut it.
        let mut entities = rectangle((-2.0, -1.0), (2.0, 3.0));
        entities.push(circle((0.0, 0.0), 1.0));
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        let disc = std::f32::consts::PI;
        assert_close(&sorted_areas(&profiles), &[disc, 16.0 - disc], 2.0e-3);
    }

    #[test]
    fn gaps_close_only_within_tolerance() {
        let tolerance = 1.0e-3;
        let gap = |size: f32| {
            sketch_of(vec![
                line((0.0, 0.0), (1.0, 0.0)),
                line((1.0, 0.0), (1.0, 1.0)),
                line((1.0, 1.0), (0.0, 1.0)),
                line((0.0, 1.0), (0.0, size)),
            ])
            .profiles(tolerance)
        };
        assert_eq!(gap(tolerance * 0.5).len(), 1);
        assert!(gap(tolerance * 3.0).is_empty());
        // A line stopping just short of another closes when within tolerance.
        let mut entities = rectangle((0.0, 0.0), (2.0, 1.0));
        entities.push(line((1.0, tolerance * 0.4), (1.0, 1.0 - tolerance * 0.4)));
        assert_eq!(sketch_of(entities).profiles(tolerance).len(), 2);
    }

    #[test]
    fn duplicate_and_collinear_overlapping_curves_are_merged() {
        let mut entities = rectangle((0.0, 0.0), (2.0, 2.0));
        entities.extend(rectangle((0.0, 0.0), (2.0, 2.0)));
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        assert_close(&sorted_areas(&profiles), &[4.0], 1.0e-4);
        // Bottom edge drawn as two overlapping lines.
        let profiles = sketch_of(vec![
            line((0.0, 0.0), (1.5, 0.0)),
            line((0.5, 0.0), (2.0, 0.0)),
            line((2.0, 0.0), (2.0, 2.0)),
            line((2.0, 2.0), (0.0, 2.0)),
            line((0.0, 2.0), (0.0, 0.0)),
        ])
        .profiles(SKETCH_TOLERANCE);
        assert_close(&sorted_areas(&profiles), &[4.0], 1.0e-4);
        assert_loops_are_closed(&profiles);
        // Overlapping concentric arcs forming a full circle.
        let arc = |start: f32, sweep: f32| -> SketchEntity {
            SketchCurve::Arc {
                center: Vec2::ZERO,
                radius: 1.0,
                start_angle: start,
                sweep,
            }
            .into()
        };
        let profiles = sketch_of(vec![arc(0.0, 4.0), arc(3.0, 4.0)]).profiles(SKETCH_TOLERANCE);
        assert_close(&sorted_areas(&profiles), &[std::f32::consts::PI], 2.0e-3);
    }

    #[test]
    fn bridges_and_islands_become_holes() {
        // A square island joined to its surrounding square by a line.
        let mut entities = rectangle((0.0, 0.0), (10.0, 10.0));
        entities.extend(rectangle((4.0, 4.0), (6.0, 6.0)));
        entities.push(line((0.0, 5.0), (4.0, 5.0)));
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        assert_close(&sorted_areas(&profiles), &[4.0, 96.0], 1.0e-3);
        let ring = &profiles[0];
        assert_eq!(ring.holes.len(), 1);
        assert_eq!(
            ring.outer.segments.len(),
            4,
            "the bridge is not part of the loop"
        );
        assert_loops_are_closed(&profiles);
    }

    #[test]
    fn many_curves_build_regions_quickly() {
        let started = std::time::Instant::now();
        let mut entities = Vec::new();
        for i in 0..=20 {
            let x = i as f32;
            entities.push(line((x, 0.0), (x, 20.0)));
            entities.push(line((0.0, x), (20.0, x)));
        }
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        assert_eq!(profiles.len(), 400);
        assert!(profiles.iter().all(|p| (p.area() - 1.0).abs() < 1.0e-3));
        let mut entities = Vec::new();
        for i in 0..600 {
            let (x, y) = ((i % 30) as f32 * 3.0, (i / 30) as f32 * 3.0);
            entities.push(circle((x, y), 1.0));
            entities.extend(rectangle((x + 1.2, y), (x + 1.8, y + 0.6)));
        }
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        assert_eq!(profiles.len(), 1200);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "arrangement took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn intersection_points_skip_shared_endpoints() {
        let mut entities = rectangle((0.0, 0.0), (2.0, 2.0));
        entities.push(line((1.0, -1.0), (1.0, 3.0)));
        entities.push(circle((2.0, 2.0), 0.5));
        let points = sketch_of(entities).intersection_points(SKETCH_TOLERANCE);
        // The circle around the corner meets both edges that end there.
        let expect = [
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 2.0),
            Vec2::new(1.5, 2.0),
            Vec2::new(2.0, 1.5),
        ];
        assert_eq!(points.len(), expect.len(), "{points:?}");
        for p in expect {
            assert!(
                points.iter().any(|q| q.distance(p) < 1.0e-4),
                "{p} in {points:?}"
            );
        }
    }

    #[test]
    fn trimming_removes_the_piece_between_neighbouring_contacts() {
        // A horizontal line crossed by two verticals: trim its middle.
        let mut sketch = sketch_of(vec![
            line((0.0, 0.0), (4.0, 0.0)),
            line((1.0, -1.0), (1.0, 1.0)),
            line((3.0, -1.0), (3.0, 1.0)),
        ]);
        let trim = sketch
            .trim_piece(0, Vec2::new(2.0, 0.1), SKETCH_TOLERANCE)
            .unwrap();
        assert_eq!(trim.kept.len(), 2);
        assert!(trim.removed.start().distance(Vec2::new(1.0, 0.0)) < 1.0e-5);
        assert!(trim.removed.end().distance(Vec2::new(3.0, 0.0)) < 1.0e-5);
        assert!(sketch.trim(0, Vec2::new(2.0, 0.1), SKETCH_TOLERANCE));
        assert_eq!(sketch.entities.len(), 4);
        // The end piece has one contact; the outer end disappears.
        let index = sketch
            .entities
            .iter()
            .position(|e| e.curve.start().x < 0.5 && e.curve.start().y == 0.0)
            .unwrap();
        assert!(sketch.trim(index, Vec2::new(0.2, 0.0), SKETCH_TOLERANCE));
        assert_eq!(sketch.entities.len(), 3);
        // A curve that meets nothing is removed whole.
        let mut lonely = sketch_of(vec![line((0.0, 0.0), (1.0, 0.0))]);
        assert!(lonely.trim(0, Vec2::new(0.5, 0.0), SKETCH_TOLERANCE));
        assert!(lonely.entities.is_empty());
    }

    #[test]
    fn trimming_circles_keeps_the_remaining_arc() {
        let mut entities = rectangle((0.0, 0.0), (2.0, 2.0));
        entities.push(circle((2.0, 1.0), 0.5));
        let mut sketch = sketch_of(entities);
        // Trim the half of the circle outside the square.
        let trim = sketch
            .trim_piece(4, Vec2::new(2.6, 1.0), SKETCH_TOLERANCE)
            .unwrap();
        assert_eq!(trim.kept.len(), 1);
        assert!((trim.removed.length() - std::f32::consts::PI * 0.5).abs() < 1.0e-4);
        let kept = trim.kept[0];
        assert!(kept.midpoint().distance(Vec2::new(1.5, 1.0)) < 1.0e-4);
        assert!(sketch.trim(4, Vec2::new(2.6, 1.0), SKETCH_TOLERANCE));
        // The square's right edge between the arc ends is a separate piece.
        let profiles = sketch.profiles(SKETCH_TOLERANCE);
        let half = std::f32::consts::PI * 0.125;
        assert_close(&sorted_areas(&profiles), &[half, 4.0 - half], 1.0e-3);
        // A lone circle is removed whole; construction curves count as contacts.
        let mut lone = sketch_of(vec![circle((0.0, 0.0), 1.0)]);
        assert!(lone.trim(0, Vec2::new(1.0, 0.0), SKETCH_TOLERANCE));
        assert!(lone.entities.is_empty());
        let mut guide: SketchEntity = line((0.0, -2.0), (0.0, 2.0));
        guide.construction = true;
        let mut cut = sketch_of(vec![circle((0.0, 0.0), 1.0), guide]);
        assert!(cut.trim(0, Vec2::new(1.0, 0.0), SKETCH_TOLERANCE));
        assert!(matches!(cut.entities[0].curve, SketchCurve::Arc { .. }));
        assert!(cut.entities[1].construction);
    }

    #[test]
    fn sub_curves_quadrants_and_translation() {
        let arc = SketchCurve::arc_through(
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
        )
        .unwrap();
        let half = arc.sub_curve(0.0, 0.5);
        assert!(half.end().distance(Vec2::new(0.0, 1.0)) < 1.0e-5);
        assert_eq!(arc.quadrant_points().len(), 3);
        let circle = SketchCurve::Circle {
            center: Vec2::ZERO,
            radius: 2.0,
        };
        assert_eq!(circle.quadrant_points().len(), 4);
        let wrapped = circle.sub_curve(0.75, 1.25);
        assert!(wrapped.midpoint().distance(Vec2::new(2.0, 0.0)) < 1.0e-5);
        let moved = circle.translated(Vec2::new(1.0, 1.0));
        assert_eq!(moved.center(), Some(Vec2::new(1.0, 1.0)));
    }

    #[test]
    fn curves_shorter_than_tolerance_are_invalid_and_never_panic_trim() {
        // Review case "tinyarc": passed validation, then panicked in trim.
        let tiny = SketchCurve::Arc {
            center: Vec2::ZERO,
            radius: 0.5,
            start_angle: 0.0,
            sweep: 1.5e-5,
        };
        assert!(!tiny.is_valid());
        let sketch = sketch_of(vec![tiny.into()]);
        assert!(sketch.validate().is_err());
        assert_eq!(
            sketch.trim_piece(0, Vec2::new(0.5, 0.0), SKETCH_TOLERANCE),
            None
        );
        // Review case "tangenttiny": a typed tangent arc of 0.001 degrees.
        let start = Vec2::new(1.0, 0.0);
        let center = start + Vec2::Y * 0.5;
        let typed = SketchCurve::Arc {
            center,
            radius: 0.5,
            start_angle: (start - center).to_angle(),
            sweep: 0.001_f32.to_radians(),
        };
        assert!(!typed.is_valid());
        // A valid line barely longer than the tolerance, touched near both
        // ends and in its middle, trims without collapsing its own ends.
        let short = 2.5e-5;
        let mut sketch = sketch_of(vec![
            line((0.0, 0.0), (short, 0.0)),
            line((0.0, -1.0), (0.0, 1.0)),
            line((short * 0.5, -1.0), (short * 0.5, 1.0)),
            line((short, -1.0), (short, 1.0)),
        ]);
        for at in [0.0, short * 0.25, short * 0.5, short] {
            let trim = sketch.trim_piece(0, Vec2::new(at, 0.0), SKETCH_TOLERANCE);
            assert!(trim.is_some());
        }
        assert!(sketch.trim(0, Vec2::new(short * 0.25, 0.0), SKETCH_TOLERANCE));
        assert!(sketch.validate().is_ok());
    }

    /// The review's randomized check: profiles never panic, loops are closed
    /// and oriented, regions are disjoint, and trimming any curve leaves a
    /// valid sketch.
    #[test]
    fn random_sketches_keep_profile_and_trim_invariants() {
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> u64 {
                let mut x = self.0;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                self.0 = x;
                x
            }
            fn grid(&mut self, n: u32) -> f32 {
                (self.next() % n as u64) as f32
            }
        }
        let mut rng = Rng(0x9e3779b97f4a7c15);
        for _ in 0..400 {
            let count = 2 + (rng.next() % 8) as usize;
            let mut entities = Vec::new();
            for _ in 0..count {
                let point = |r: &mut Rng| Vec2::new(r.grid(6), r.grid(6));
                let curve = match rng.next() % 3 {
                    0 => SketchCurve::Line {
                        start: point(&mut rng),
                        end: point(&mut rng),
                    },
                    1 => SketchCurve::Circle {
                        center: point(&mut rng),
                        radius: 0.5 + rng.grid(4) * 0.5,
                    },
                    _ => SketchCurve::Arc {
                        center: point(&mut rng),
                        radius: 0.5 + rng.grid(4) * 0.5,
                        start_angle: rng.grid(8) * std::f32::consts::FRAC_PI_4,
                        sweep: (rng.grid(15) + 1.0)
                            * std::f32::consts::FRAC_PI_4
                            * if rng.next() % 2 == 0 { 1.0 } else { -1.0 },
                    },
                };
                if curve.is_valid() {
                    entities.push(SketchEntity::from(curve));
                }
            }
            let sketch = sketch_of(entities.clone());
            let profiles = sketch.profiles(SKETCH_TOLERANCE);
            assert_loops_are_closed(&profiles);
            for gx in 0..30 {
                for gy in 0..30 {
                    let point = Vec2::new(
                        -3.0 + gx as f32 * 0.4 + 0.0123,
                        -3.0 + gy as f32 * 0.4 + 0.0371,
                    );
                    if entities.iter().any(|e| e.curve.closest(point).1 < 0.03) {
                        continue;
                    }
                    let inside = profiles.iter().filter(|p| p.contains(point)).count();
                    assert!(inside <= 1, "{point} in {inside} profiles of {entities:?}");
                }
            }
            for index in 0..entities.len() {
                let mut trimmed = sketch.clone();
                trimmed.trim(index, entities[index].curve.midpoint(), SKETCH_TOLERANCE);
                assert!(trimmed.validate().is_ok(), "trim {index} of {entities:?}");
            }
        }
    }

    #[test]
    fn coincident_circles_collapse_to_one_carrier() {
        // Review case "coincident": a circle drawn twice inside a square.
        let mut entities = rectangle((-2.0, -2.0), (2.0, 2.0));
        entities.push(circle((0.0, 0.0), 1.0));
        entities.push(circle((0.0, 0.0), 1.0));
        let profiles = sketch_of(entities.clone()).profiles(SKETCH_TOLERANCE);
        let disc = std::f32::consts::PI;
        assert_close(&sorted_areas(&profiles), &[disc, 16.0 - disc], 1.0e-3);
        assert_eq!(profiles[0].holes.len(), 1);
        assert_loops_are_closed(&profiles);
        // Two identical circles alone, and a radius differing within tolerance.
        let profiles = sketch_of(vec![circle((0.0, 0.0), 1.0), circle((0.0, 0.0), 1.0)])
            .profiles(SKETCH_TOLERANCE);
        assert_close(&sorted_areas(&profiles), &[disc], 1.0e-3);
        let profiles = sketch_of(vec![
            circle((0.0, 0.0), 1.0),
            circle((0.0, 0.0), 1.0 + 5.0e-6),
        ])
        .profiles(SKETCH_TOLERANCE);
        assert_close(&sorted_areas(&profiles), &[disc], 1.0e-3);
        // Review case "uicircle": a circle redrawn by snapping to a quadrant.
        let center = Vec2::new(0.3, 0.7);
        let first = SketchCurve::Circle {
            center,
            radius: 1.234,
        };
        let redrawn = SketchCurve::Circle {
            center,
            radius: center.distance(first.quadrant_points()[1]),
        };
        let mut entities = rectangle((-2.0, -2.0), (3.0, 3.0));
        entities.extend([SketchEntity::from(first), SketchEntity::from(redrawn)]);
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].holes.len(), 1);
        // A duplicate circle crossed by a diameter, and an arc lying on a circle.
        let profiles = sketch_of(vec![
            circle((0.0, 0.0), 1.0),
            circle((0.0, 0.0), 1.0),
            line((-2.0, 0.0), (2.0, 0.0)),
        ])
        .profiles(SKETCH_TOLERANCE);
        assert_close(&sorted_areas(&profiles), &[disc * 0.5, disc * 0.5], 1.0e-3);
        let mut entities = rectangle((-2.0, -2.0), (2.0, 2.0));
        entities.push(circle((0.0, 0.0), 1.0));
        entities.push(
            SketchCurve::Arc {
                center: Vec2::ZERO,
                radius: 1.0,
                start_angle: 0.0,
                sweep: 3.0,
            }
            .into(),
        );
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        assert_close(&sorted_areas(&profiles), &[disc, 16.0 - disc], 1.0e-3);
    }

    #[test]
    fn arcs_stay_precise_and_close_loops_far_from_the_origin() {
        // Review case "arcprec": three point arcs away from the origin.
        for offset in [0.0_f32, 10.0, 100.0, 1000.0] {
            for chord in [10.0_f32, 1.0, 0.1] {
                let a = Vec2::splat(offset);
                let c = Vec2::new(offset + chord, offset);
                let b = Vec2::new(offset + chord * 0.5, offset + chord * 0.25);
                let arc = SketchCurve::arc_through(a, b, c).unwrap();
                let limit = 2.0e-6 + offset * 4.0e-7;
                assert!(arc.start().distance(a) < limit, "start {offset} {chord}");
                assert!(arc.end().distance(c) < limit, "end {offset} {chord}");
                assert!(arc.closest(b).1 < limit, "through {offset} {chord}");
            }
        }
        // Review case "dshape": a line closed by a three point arc, and a U
        // closed by a tangent arc, at every offset and size.
        let tangent_arc = |start: Vec2, tangent: Vec2, end: Vec2| {
            let normal = tangent.perp();
            let chord = end - start;
            let k = chord.length_squared() / (2.0 * normal.dot(chord));
            let center = start + normal * k;
            let from = (start - center).to_angle();
            let to = (end - center).to_angle();
            let sweep = if k > 0.0 {
                (to - from).rem_euclid(TAU)
            } else {
                -(from - to).rem_euclid(TAU)
            };
            SketchCurve::Arc {
                center,
                radius: k.abs(),
                start_angle: from,
                sweep,
            }
        };
        for offset in [0.0_f32, 0.5, 1.0, 2.0, 10.0, 50.0, 100.0, 300.0, 1000.0] {
            for width in [2.0_f32, 0.2, 0.02, 0.005] {
                let a = Vec2::splat(offset);
                let b = a + Vec2::new(width, 0.0);
                let top = a + Vec2::new(width * 0.5, width * 0.2);
                let arc = SketchCurve::arc_through(b, top, a).unwrap();
                let d = sketch_of(vec![
                    SketchCurve::Line { start: a, end: b }.into(),
                    arc.into(),
                ]);
                assert_eq!(
                    d.profiles(SKETCH_TOLERANCE).len(),
                    1,
                    "D at {offset} width {width}"
                );
                let c = b + Vec2::new(0.0, width);
                let cap = tangent_arc(c, Vec2::Y, a + Vec2::new(0.0, width));
                let u = sketch_of(vec![
                    SketchCurve::Line { start: a, end: b }.into(),
                    SketchCurve::Line { start: b, end: c }.into(),
                    cap.into(),
                    SketchCurve::Line {
                        start: cap.end(),
                        end: a,
                    }
                    .into(),
                ]);
                assert_eq!(
                    u.profiles(SKETCH_TOLERANCE).len(),
                    1,
                    "U at {offset} width {width}"
                );
            }
        }
    }

    #[test]
    fn small_arcs_keep_their_regions_in_large_sketches() {
        // Review case "smallcircle": holes far smaller than the sketch.
        for (extent, radius) in [
            (1.0_f32, 0.0005_f32),
            (100.0, 0.0005),
            (100.0, 0.002),
            (1000.0, 0.01),
        ] {
            let mut entities = rectangle((0.0, 0.0), (extent, extent));
            entities.push(circle((extent * 0.5, extent * 0.5), radius));
            let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
            assert_eq!(profiles.len(), 2, "square {extent} + circle {radius}");
            assert_eq!(
                profiles[0].holes.len(),
                1,
                "square {extent} + circle {radius}"
            );
        }
        // A small slot made of two semicircles inside a 100 m square.
        let mut entities = rectangle((0.0, 0.0), (100.0, 100.0));
        for start_angle in [std::f32::consts::FRAC_PI_2, -std::f32::consts::FRAC_PI_2] {
            entities.push(
                SketchCurve::Arc {
                    center: Vec2::splat(50.0),
                    radius: 0.003,
                    start_angle,
                    sweep: std::f32::consts::PI,
                }
                .into(),
            );
        }
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].holes.len(), 1);
    }

    #[test]
    fn dense_grids_find_intersections_quickly() {
        // Review case "grid": 402 lines and 40 000 crossings took 1.2 s.
        let mut entities = Vec::new();
        for i in 0..=200 {
            let x = i as f32;
            entities.push(line((x, 0.0), (x, 200.0)));
            entities.push(line((0.0, x), (200.0, x)));
        }
        let sketch = sketch_of(entities);
        let started = std::time::Instant::now();
        let points = sketch.intersection_points(SKETCH_TOLERANCE);
        // Interior crossings plus T-junctions on the border (corners are
        // shared endpoints, not crossings).
        assert_eq!(points.len(), 199 * 199 + 4 * 199);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(600),
            "intersection points took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn many_islands_find_their_parent_region_quickly() {
        // 2500 squares inside one outline: every island is a hole of the
        // outline, found without testing each island against every region.
        let mut entities = rectangle((-1.0, -1.0), (151.0, 151.0));
        for i in 0..2500 {
            let (x, y) = ((i % 50) as f32 * 3.0, (i / 50) as f32 * 3.0);
            entities.extend(rectangle((x, y), (x + 1.0, y + 1.0)));
        }
        let started = std::time::Instant::now();
        let profiles = sketch_of(entities).profiles(SKETCH_TOLERANCE);
        assert_eq!(profiles.len(), 2501);
        assert_eq!(profiles[0].holes.len(), 2500);
        assert!((profiles[0].area() - (152.0 * 152.0 - 2500.0)).abs() < 1.0);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "profiles took {:?}",
            started.elapsed()
        );
    }
}

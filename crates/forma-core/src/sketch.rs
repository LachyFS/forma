//! Planar 2D sketches: the shared drawing data behind Draw mode and CAD features.
//!
//! A sketch is a set of curves in the local 2D coordinates of a [`SketchPlane`].
//! Closed chains of curves form [`Profile`]s, which CAD features extrude or
//! revolve. Sketch data is plain, serializable geometry with no kernel types,
//! so scenes stay independent of the modelling kernel that consumes them.
use crate::Ray;
use anyhow::{Result, ensure};
use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

/// Largest coordinate accepted in sketch space, matching object translation limits.
pub const MAX_SKETCH_COORDINATE: f32 = 1.0e8;
/// Maximum number of curves in one sketch.
pub const MAX_SKETCH_ENTITIES: usize = 100_000;
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
        let center = circumcenter(start, through, end)?;
        let radius = start.distance(center);
        let angle = |p: Vec2| (p - center).to_angle();
        let start_angle = angle(start);
        let ccw = |a: f32| (a - start_angle).rem_euclid(TAU);
        let to_end = ccw(angle(end));
        let to_through = ccw(angle(through));
        let sweep = if to_through <= to_end {
            to_end
        } else {
            to_end - TAU
        };
        let curve = Self::Arc {
            center,
            radius,
            start_angle,
            sweep,
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

fn circumcenter(a: Vec2, b: Vec2, c: Vec2) -> Option<Vec2> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    let scale = (b - a).length().max((c - a).length()).max(1.0e-12);
    if d.abs() < 1.0e-9 * scale * scale {
        return None;
    }
    let (a2, b2, c2) = (a.length_squared(), b.length_squared(), c.length_squared());
    let center = Vec2::new(
        (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d,
        (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d,
    );
    center.is_finite().then_some(center)
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
    /// Positive for counter-clockwise loops.
    pub fn signed_area(&self) -> f32 {
        signed_area(&self.polygon(self.tolerance()))
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

    /// The same loop traversed the other way. A circle has no direction, so
    /// a reversed circle becomes two clockwise half arcs of the same entity.
    fn reversed(&self) -> Self {
        let mut segments = Vec::with_capacity(self.segments.len() + 1);
        let mut entities = Vec::with_capacity(self.segments.len() + 1);
        for (segment, entity) in self.segments.iter().zip(&self.entities).rev() {
            match *segment {
                SketchCurve::Circle { center, radius } => {
                    for start_angle in [0.0, -std::f32::consts::PI] {
                        segments.push(SketchCurve::Arc {
                            center,
                            radius,
                            start_angle,
                            sweep: -std::f32::consts::PI,
                        });
                        entities.push(*entity);
                    }
                }
                curve => {
                    segments.push(curve.reversed());
                    entities.push(*entity);
                }
            }
        }
        Self { segments, entities }
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

    /// Closed regions formed by non-construction curves.
    ///
    /// Curves are joined end to end where endpoints coincide within
    /// `tolerance`; circles are closed loops on their own. Loops nested inside
    /// another loop become holes, alternating with depth so islands inside
    /// holes are profiles again. Curves that cross without sharing endpoints
    /// are not split.
    pub fn profiles(&self, tolerance: f32) -> Vec<Profile> {
        let tolerance = tolerance.max(SKETCH_TOLERANCE);
        let mut loops = closed_loops(&self.entities, tolerance);
        // Normalize every loop to counter-clockwise before nesting.
        for profile_loop in &mut loops {
            if profile_loop.signed_area() < 0.0 {
                *profile_loop = profile_loop.reversed();
            }
        }
        loops.retain(|l| l.signed_area().abs() > tolerance * tolerance);
        let areas: Vec<f32> = loops.iter().map(ProfileLoop::signed_area).collect();
        let samples: Vec<Vec2> = loops.iter().map(interior_sample).collect();
        // parent = smallest loop strictly containing this one.
        let parents: Vec<Option<usize>> = (0..loops.len())
            .map(|i| {
                (0..loops.len())
                    .filter(|&j| j != i && areas[j] > areas[i] && loops[j].contains(samples[i]))
                    .min_by(|&a, &b| areas[a].total_cmp(&areas[b]))
            })
            .collect();
        let depth = |mut i: usize| {
            let mut depth = 0;
            while let Some(parent) = parents[i] {
                depth += 1;
                i = parent;
                if depth > loops.len() {
                    break;
                }
            }
            depth
        };
        let depths: Vec<usize> = (0..loops.len()).map(depth).collect();
        let mut profiles = Vec::new();
        for i in (0..loops.len()).filter(|&i| depths[i].is_multiple_of(2)) {
            let holes = (0..loops.len())
                .filter(|&j| parents[j] == Some(i))
                .map(|j| loops[j].reversed())
                .collect();
            profiles.push(Profile {
                outer: loops[i].clone(),
                holes,
            });
        }
        profiles
    }
}

/// A point strictly inside a counter-clockwise or clockwise loop, used to test
/// nesting without being confused by shared boundaries.
fn interior_sample(profile_loop: &ProfileLoop) -> Vec2 {
    let polygon = profile_loop.polygon(profile_loop.tolerance());
    let (min, max) = profile_loop.bounds();
    let size = (max - min).max_element().max(1.0e-6);
    for segment in &profile_loop.segments {
        let point = segment.midpoint();
        let normal = segment.tangent_at(0.5).perp();
        for offset in [1.0e-4, 1.0e-3, 1.0e-2] {
            for side in [1.0, -1.0] {
                let candidate = point + normal * side * offset * size;
                if point_in_polygon(candidate, &polygon) {
                    return candidate;
                }
            }
        }
    }
    polygon.iter().copied().sum::<Vec2>() / polygon.len().max(1) as f32
}

fn closed_loops(entities: &[SketchEntity], tolerance: f32) -> Vec<ProfileLoop> {
    let mut loops = Vec::new();
    let mut open = Vec::new();
    for (index, entity) in entities.iter().enumerate() {
        if entity.construction || !entity.curve.is_valid() {
            continue;
        }
        if entity.curve.is_closed() {
            loops.push(ProfileLoop {
                segments: vec![entity.curve],
                entities: vec![index],
            });
        } else {
            open.push(index);
        }
    }
    // Greedy chaining: start from each unused curve and follow the unique
    // continuation at each end; branch points stop a chain.
    let mut used = vec![false; entities.len()];
    let near = |a: Vec2, b: Vec2| a.distance(b) <= tolerance;
    let degree = |point: Vec2| {
        open.iter()
            .filter(|&&i| {
                let curve = entities[i].curve;
                near(curve.start(), point) || near(curve.end(), point)
            })
            .count()
    };
    for &first in &open {
        if used[first] {
            continue;
        }
        let mut segments = vec![entities[first].curve];
        let mut indices = vec![first];
        let mut chain_used = vec![first];
        let start = entities[first].curve.start();
        let mut closed = false;
        loop {
            let end = segments.last().unwrap().end();
            if segments.len() > 1 && near(end, start) {
                closed = true;
                break;
            }
            if degree(end) != 2 {
                break;
            }
            let next = open.iter().copied().find(|&i| {
                !used[i] && !chain_used.contains(&i) && {
                    let curve = entities[i].curve;
                    near(curve.start(), end) || near(curve.end(), end)
                }
            });
            let Some(next) = next else { break };
            let curve = entities[next].curve;
            let oriented = if near(curve.start(), end) {
                curve
            } else {
                curve.reversed()
            };
            segments.push(oriented);
            indices.push(next);
            chain_used.push(next);
            if segments.len() > open.len() {
                break;
            }
        }
        if closed {
            for &i in &chain_used {
                used[i] = true;
            }
            loops.push(ProfileLoop {
                segments,
                entities: indices,
            });
        }
    }
    loops
}

pub(crate) fn signed_area(points: &[Vec2]) -> f32 {
    let mut area = 0.0;
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        area += a.perp_dot(b);
    }
    area * 0.5
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
        assert_eq!(profiles.len(), 1);
        let profile = &profiles[0];
        assert_eq!(profile.holes.len(), 1);
        assert!(profile.outer.signed_area() > 0.0);
        assert!(profile.holes[0].signed_area() < 0.0);
        let expected = 12.0 - std::f32::consts::PI * 0.25;
        assert!((profile.area() - expected).abs() < 0.01);
        assert!(profile.contains(Vec2::new(0.5, 0.5)));
        assert!(!profile.contains(Vec2::new(2.0, 1.5)));
    }

    #[test]
    fn islands_inside_holes_are_profiles_and_construction_is_ignored() {
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
        assert_eq!(profiles.len(), 2);
        let ring = profiles.iter().find(|p| !p.holes.is_empty()).unwrap();
        assert!((ring.area() - 64.0).abs() < 1.0e-3);
        let island = profiles.iter().find(|p| p.holes.is_empty()).unwrap();
        assert!((island.area() - 4.0).abs() < 1.0e-3);
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
}

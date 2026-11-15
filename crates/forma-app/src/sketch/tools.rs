//! Drawing tools as pure geometry: the in-progress step of each tool, the
//! dimensions it accepts from numeric entry, and the curves it would create
//! for a pointer position.
use forma_core::{SketchCurve, Vec2};
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DrawTool {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    Polygon,
    Trim,
}

impl DrawTool {
    pub(crate) const ALL: [Self; 7] = [
        Self::Select,
        Self::Line,
        Self::Rectangle,
        Self::Circle,
        Self::Arc,
        Self::Polygon,
        Self::Trim,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Line => "Line",
            Self::Rectangle => "Rectangle",
            Self::Circle => "Circle",
            Self::Arc => "Arc",
            Self::Polygon => "Polygon",
            Self::Trim => "Trim",
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Select => "Q",
            Self::Line => "L",
            Self::Rectangle => "R",
            Self::Circle => "C",
            Self::Arc => "A",
            Self::Polygon => "P",
            Self::Trim => "T",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|tool| tool.key().eq_ignore_ascii_case(key))
    }
}

/// The in-progress part of a tool, between clicks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Step {
    Idle,
    /// A polyline chain. `tangent` is the direction the chain left its last
    /// point, for tangent arcs; `arc` draws the next piece as a tangent arc.
    Line {
        start: Vec2,
        chain_start: Vec2,
        segments: usize,
        tangent: Option<Vec2>,
        arc: bool,
    },
    Rectangle {
        first: Vec2,
    },
    Circle {
        center: Vec2,
    },
    Arc {
        start: Vec2,
        end: Option<Vec2>,
    },
    Polygon {
        center: Vec2,
    },
}

impl Step {
    pub(crate) fn is_active(&self) -> bool {
        *self != Self::Idle
    }

    /// The point inference lines start from.
    pub(crate) fn anchor(&self) -> Option<Vec2> {
        match *self {
            Self::Idle => None,
            Self::Line { start, .. } => Some(start),
            Self::Rectangle { first } => Some(first),
            Self::Circle { center } | Self::Polygon { center } => Some(center),
            Self::Arc { start, end } => Some(end.unwrap_or(start)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dimension {
    Length,
    Angle,
    Width,
    Height,
    Radius,
    Sweep,
}

impl Dimension {
    pub(crate) fn is_angle(self) -> bool {
        matches!(self, Self::Angle | Self::Sweep)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Length => "Length",
            Self::Angle => "Angle",
            Self::Width => "Width",
            Self::Height => "Height",
            Self::Radius => "Radius",
            Self::Sweep => "Sweep",
        }
    }
}

/// Tool settings that shape the geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Options {
    pub rect_center: bool,
    pub polygon_sides: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            rect_center: false,
            polygon_sides: 6,
        }
    }
}

/// Values typed for a step's dimensions; `None` follows the pointer.
pub(crate) type Locks = [Option<f32>; 2];

/// The dimensions numeric entry can set during `step`.
pub(crate) fn dimensions(step: &Step) -> &'static [Dimension] {
    match step {
        Step::Idle => &[],
        Step::Line { arc: false, .. } => &[Dimension::Length, Dimension::Angle],
        Step::Line { arc: true, .. } => &[Dimension::Radius, Dimension::Sweep],
        Step::Rectangle { .. } => &[Dimension::Width, Dimension::Height],
        Step::Circle { .. } => &[Dimension::Radius],
        Step::Arc { end: None, .. } => &[Dimension::Length, Dimension::Angle],
        Step::Arc { end: Some(_), .. } => &[Dimension::Radius],
        Step::Polygon { .. } => &[Dimension::Radius, Dimension::Angle],
    }
}

fn signum_or_one(value: f32) -> f32 {
    if value < 0.0 { -1.0 } else { 1.0 }
}

/// A point at `length` and `angle` from `start`, where unlocked values
/// follow `cursor`. A locked angle with a free length projects the pointer
/// onto the locked direction.
fn polar(start: Vec2, cursor: Vec2, locks: Locks) -> Vec2 {
    let offset = cursor - start;
    let angle = locks[1].unwrap_or_else(|| {
        if offset.length_squared() > 0.0 {
            offset.y.atan2(offset.x)
        } else {
            0.0
        }
    });
    let direction = Vec2::from_angle(angle);
    let length = match (locks[0], locks[1]) {
        (Some(length), _) => length,
        (None, Some(_)) => offset.dot(direction),
        (None, None) => offset.length(),
    };
    start + direction * length
}

/// The tangent arc leaving `start` along `tangent` and ending at `end`.
pub(crate) fn tangent_arc(start: Vec2, tangent: Vec2, end: Vec2) -> Option<SketchCurve> {
    let normal = tangent.normalize_or_zero().perp();
    let chord = end - start;
    let denominator = 2.0 * normal.dot(chord);
    if denominator.abs() < chord.length() * 1.0e-4 {
        return None;
    }
    let k = chord.length_squared() / denominator;
    let center = start + normal * k;
    let from = (start - center).to_angle();
    let to = (end - center).to_angle();
    let sweep = if k > 0.0 {
        (to - from).rem_euclid(TAU)
    } else {
        -(from - to).rem_euclid(TAU)
    };
    let arc = SketchCurve::Arc {
        center,
        radius: k.abs(),
        start_angle: from,
        sweep,
    };
    arc.is_valid().then_some(arc)
}

/// Tangent arc with optional locked radius and sweep; the turning side
/// follows the pointer.
fn locked_tangent_arc(
    start: Vec2,
    tangent: Vec2,
    cursor: Vec2,
    locks: Locks,
) -> Option<SketchCurve> {
    if locks == [None, None] {
        return tangent_arc(start, tangent, cursor);
    }
    let tangent = tangent.normalize_or_zero();
    let normal = tangent.perp();
    let side = signum_or_one(normal.dot(cursor - start));
    let free = tangent_arc(start, tangent, cursor);
    let radius = locks[0]
        .or_else(|| {
            free.and_then(|arc| match arc {
                SketchCurve::Arc { radius, .. } => Some(radius),
                _ => None,
            })
        })?
        .abs();
    let center = start + normal * radius * side;
    let sweep = match locks[1] {
        Some(sweep) => sweep.abs().min(TAU - 1.0e-3),
        None => {
            let from = (start - center).to_angle();
            let to = (cursor - center).to_angle();
            if side > 0.0 {
                (to - from).rem_euclid(TAU)
            } else {
                (from - to).rem_euclid(TAU)
            }
        }
    };
    let arc = SketchCurve::Arc {
        center,
        radius,
        start_angle: (start - center).to_angle(),
        sweep: sweep * side,
    };
    arc.is_valid().then_some(arc)
}

/// Where a click would place the step's next point, after numeric locks.
pub(crate) fn resolve(step: &Step, cursor: Vec2, locks: Locks) -> Vec2 {
    match *step {
        Step::Line {
            start, arc: false, ..
        } => polar(start, cursor, locks),
        Step::Arc { start, end: None } => polar(start, cursor, locks),
        Step::Line {
            start,
            tangent: Some(tangent),
            arc: true,
            ..
        } => locked_tangent_arc(start, tangent, cursor, locks)
            .map(|arc| arc.end())
            .unwrap_or(cursor),
        Step::Rectangle { first } => {
            let offset = cursor - first;
            let mut size = offset;
            if let Some(width) = locks[0] {
                size.x = width.abs() * signum_or_one(offset.x);
            }
            if let Some(height) = locks[1] {
                size.y = height.abs() * signum_or_one(offset.y);
            }
            first + size
        }
        Step::Circle { center } => match locks[0] {
            Some(radius) => center + (cursor - center).normalize_or(Vec2::X) * radius.abs(),
            None => cursor,
        },
        Step::Polygon { center } => polar(center, cursor, locks),
        _ => cursor,
    }
}

fn rectangle_corners(first: Vec2, point: Vec2, center: bool) -> Option<[Vec2; 4]> {
    let (low, high) = if center {
        let half = (point - first).abs();
        (first - half, first + half)
    } else {
        (first.min(point), first.max(point))
    };
    let size = high - low;
    if size.x <= forma_core::SKETCH_TOLERANCE * 10.0
        || size.y <= forma_core::SKETCH_TOLERANCE * 10.0
    {
        return None;
    }
    Some([
        low,
        Vec2::new(high.x, low.y),
        high,
        Vec2::new(low.x, high.y),
    ])
}

fn closed_polyline(points: &[Vec2]) -> Vec<SketchCurve> {
    (0..points.len())
        .map(|i| SketchCurve::Line {
            start: points[i],
            end: points[(i + 1) % points.len()],
        })
        .filter(SketchCurve::is_valid)
        .collect()
}

/// The arc through `start` and `end` whose bulge follows `cursor`, with an
/// optional locked radius.
fn three_point_arc(
    start: Vec2,
    end: Vec2,
    cursor: Vec2,
    radius: Option<f32>,
) -> Option<SketchCurve> {
    let Some(radius) = radius else {
        return SketchCurve::arc_through(start, cursor, end);
    };
    let chord = end - start;
    let half = chord.length() * 0.5;
    if half <= 0.0 {
        return None;
    }
    let radius = radius.abs().max(half);
    let middle = (start + end) * 0.5;
    let normal = chord.normalize().perp();
    let side = signum_or_one(normal.dot(cursor - middle));
    let height = (radius * radius - half * half).max(0.0).sqrt();
    // The bulge passes the pointer's side; a pointer further out than the
    // radius asks for the major arc.
    let major = normal.dot(cursor - middle).abs() > radius;
    let center = middle - normal * side * height * if major { -1.0 } else { 1.0 };
    let through = center + normal * side * radius;
    SketchCurve::arc_through(start, through, end)
}

pub(crate) fn polygon_points(center: Vec2, vertex: Vec2, sides: u32) -> Vec<Vec2> {
    let offset = vertex - center;
    let (radius, angle) = (offset.length(), offset.y.atan2(offset.x));
    let sides = sides.clamp(3, 64);
    (0..sides)
        .map(|k| center + Vec2::from_angle(angle + k as f32 * TAU / sides as f32) * radius)
        .collect()
}

/// A centre rectangle's typed width and height span both sides of its
/// centre, so the corner it resolves sits at half those values.
fn shape_locks(step: &Step, locks: Locks, options: Options) -> Locks {
    match step {
        Step::Rectangle { .. } if options.rect_center => locks.map(|v| v.map(|v| v * 0.5)),
        _ => locks,
    }
}

/// The curves a click at `cursor` would create, after numeric locks.
pub(crate) fn preview(
    step: &Step,
    cursor: Vec2,
    locks: Locks,
    options: Options,
) -> Vec<SketchCurve> {
    let point = resolve(step, cursor, shape_locks(step, locks, options));
    let curves = match *step {
        Step::Idle => Vec::new(),
        Step::Line {
            start, arc: false, ..
        } => vec![SketchCurve::Line { start, end: point }],
        Step::Line {
            start,
            tangent,
            arc: true,
            ..
        } => match tangent.and_then(|t| locked_tangent_arc(start, t, cursor, locks)) {
            Some(arc) => vec![arc],
            None => vec![SketchCurve::Line { start, end: point }],
        },
        Step::Rectangle { first } => rectangle_corners(first, point, options.rect_center)
            .map(|corners| closed_polyline(&corners))
            .unwrap_or_default(),
        Step::Circle { center } => vec![SketchCurve::Circle {
            center,
            radius: center.distance(point),
        }],
        Step::Arc { start, end: None } => vec![SketchCurve::Line { start, end: point }],
        Step::Arc {
            start,
            end: Some(end),
        } => three_point_arc(start, end, cursor, locks[0])
            .into_iter()
            .collect(),
        Step::Polygon { center } => {
            if center.distance(point) <= forma_core::SKETCH_TOLERANCE * 10.0 {
                Vec::new()
            } else {
                closed_polyline(&polygon_points(center, point, options.polygon_sides))
            }
        }
    };
    curves.into_iter().filter(SketchCurve::is_valid).collect()
}

/// The current value of each of the step's dimensions, for live labels.
pub(crate) fn measure(
    step: &Step,
    cursor: Vec2,
    locks: Locks,
    options: Options,
) -> [Option<f32>; 2] {
    let point = resolve(step, cursor, shape_locks(step, locks, options));
    match *step {
        Step::Line {
            start, arc: false, ..
        }
        | Step::Arc { start, end: None } => {
            let offset = point - start;
            [Some(offset.length()), Some(offset.y.atan2(offset.x))]
        }
        Step::Line { arc: true, .. } => match preview(step, cursor, locks, options).first() {
            Some(SketchCurve::Arc { radius, sweep, .. }) => [Some(*radius), Some(sweep.abs())],
            _ => [None, None],
        },
        Step::Rectangle { first } => {
            let size = (point - first).abs() * if options.rect_center { 2.0 } else { 1.0 };
            [Some(size.x), Some(size.y)]
        }
        Step::Circle { center } => [Some(center.distance(point)), None],
        Step::Arc { end: Some(_), .. } => match preview(step, cursor, locks, options).first() {
            Some(SketchCurve::Arc { radius, .. }) => [Some(*radius), None],
            _ => [None, None],
        },
        Step::Polygon { center } => {
            let offset = point - center;
            [Some(offset.length()), Some(offset.y.atan2(offset.x))]
        }
        Step::Idle => [None, None],
    }
}

/// Normalizes an angle for display in degrees within (-180, 180].
pub(crate) fn display_degrees(angle: f32) -> f32 {
    let degrees = angle.to_degrees();
    let wrapped = (degrees + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped <= -180.0 + 1.0e-4 {
        180.0
    } else {
        wrapped
    }
}

/// True when two points coincide for chaining and closing loops.
pub(crate) fn same_point(a: Vec2, b: Vec2) -> bool {
    a.distance(b) <= forma_core::SKETCH_TOLERANCE * 4.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn line_step(start: Vec2) -> Step {
        Step::Line {
            start,
            chain_start: start,
            segments: 0,
            tangent: None,
            arc: false,
        }
    }

    #[test]
    fn numeric_locks_shape_lines() {
        let step = line_step(Vec2::ZERO);
        let cursor = Vec2::new(3.0, 3.0);
        let end = resolve(&step, cursor, [Some(2.0), None]);
        assert!(end.distance(Vec2::new(2.0_f32.sqrt(), 2.0_f32.sqrt())) < 1.0e-5);
        let end = resolve(&step, cursor, [Some(2.0), Some(0.0)]);
        assert!(end.distance(Vec2::new(2.0, 0.0)) < 1.0e-5);
        // A locked angle with free length follows the pointer along the ray.
        let end = resolve(&step, Vec2::new(4.0, 1.0), [None, Some(0.0)]);
        assert!(end.distance(Vec2::new(4.0, 0.0)) < 1.0e-5);
    }

    #[test]
    fn rectangles_follow_the_pointer_quadrant() {
        let step = Step::Rectangle { first: Vec2::ZERO };
        let options = Options::default();
        let curves = preview(&step, Vec2::new(-1.0, 2.0), [Some(3.0), None], options);
        assert_eq!(curves.len(), 4);
        let (min, max) = curves
            .iter()
            .fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(lo, hi), c| {
                let (a, b) = c.bounds();
                (lo.min(a), hi.max(b))
            });
        assert_eq!((min, max), (Vec2::new(-3.0, 0.0), Vec2::new(0.0, 2.0)));
        let centered = preview(
            &step,
            Vec2::new(1.0, 1.0),
            [Some(4.0), Some(2.0)],
            Options {
                rect_center: true,
                ..options
            },
        );
        let (min, max) =
            centered
                .iter()
                .fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(lo, hi), c| {
                    let (a, b) = c.bounds();
                    (lo.min(a), hi.max(b))
                });
        assert_eq!((min, max), (Vec2::new(-2.0, -1.0), Vec2::new(2.0, 1.0)));
        assert!(preview(&step, Vec2::new(1.0, 0.0), [None, None], options).is_empty());
    }

    #[test]
    fn tangent_arcs_leave_along_the_tangent() {
        let arc = tangent_arc(Vec2::ZERO, Vec2::X, Vec2::new(1.0, 1.0)).unwrap();
        assert!(arc.start().distance(Vec2::ZERO) < 1.0e-5);
        assert!(arc.end().distance(Vec2::new(1.0, 1.0)) < 1.0e-5);
        assert!(arc.tangent_at(0.0).distance(Vec2::X) < 1.0e-4);
        let clockwise = tangent_arc(Vec2::ZERO, Vec2::X, Vec2::new(1.0, -1.0)).unwrap();
        assert!(clockwise.tangent_at(0.0).distance(Vec2::X) < 1.0e-4);
        assert!(tangent_arc(Vec2::ZERO, Vec2::X, Vec2::new(2.0, 0.0)).is_none());
        let step = Step::Line {
            start: Vec2::ZERO,
            chain_start: Vec2::ZERO,
            segments: 1,
            tangent: Some(Vec2::X),
            arc: true,
        };
        let locked = preview(
            &step,
            Vec2::new(0.5, 2.0),
            [Some(1.0), Some(PI)],
            Options::default(),
        );
        assert!(locked[0].end().distance(Vec2::new(0.0, 2.0)) < 1.0e-4);
    }

    #[test]
    fn three_point_arcs_and_polygons() {
        let step = Step::Arc {
            start: Vec2::new(-1.0, 0.0),
            end: Some(Vec2::new(1.0, 0.0)),
        };
        let arc = preview(&step, Vec2::new(0.0, 1.0), [None, None], Options::default())[0];
        assert!(arc.midpoint().distance(Vec2::new(0.0, 1.0)) < 1.0e-4);
        let locked = preview(
            &step,
            Vec2::new(0.0, 0.5),
            [Some(2.0), None],
            Options::default(),
        )[0];
        if let SketchCurve::Arc { radius, .. } = locked {
            assert!((radius - 2.0).abs() < 1.0e-4);
        } else {
            panic!("expected an arc");
        }
        assert!(locked.midpoint().y > 0.0);
        let hexagon = preview(
            &Step::Polygon { center: Vec2::ZERO },
            Vec2::new(1.0, 0.0),
            [None, None],
            Options::default(),
        );
        assert_eq!(hexagon.len(), 6);
        assert!(hexagon.iter().all(|c| (c.length() - 1.0).abs() < 1.0e-4));
    }

    #[test]
    fn angles_display_in_degrees() {
        assert!((display_degrees(PI) - 180.0).abs() < 1.0e-3);
        assert!((display_degrees(-PI * 0.5) + 90.0).abs() < 1.0e-3);
        assert!((display_degrees(TAU + 0.1) - 0.1_f32.to_degrees()).abs() < 1.0e-3);
    }
}

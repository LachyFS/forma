//! Snapping and inference for Draw mode, in sketch-plane coordinates.
//!
//! Point snaps (endpoints, intersections, centers, midpoints, quadrants and
//! the origin) win when the pointer is near one. Otherwise inference lines
//! are considered: horizontal and vertical through the tool's anchor,
//! parallel and perpendicular to a remembered reference line, and x/y
//! alignment with recently hovered points. Two inference lines meeting near
//! the pointer snap to their crossing; one line combines with a nearby curve
//! or with the grid. With nothing nearby the pointer snaps to the grid.
use forma_core::{SketchCurve, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapKind {
    Free,
    Grid,
    Endpoint,
    Midpoint,
    Center,
    Quadrant,
    Intersection,
    OnCurve,
    Origin,
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Aligned,
}

impl SnapKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Free | Self::Grid => "",
            Self::Endpoint => "Endpoint",
            Self::Midpoint => "Midpoint",
            Self::Center => "Center",
            Self::Quadrant => "Quadrant",
            Self::Intersection => "Intersection",
            Self::OnCurve => "On curve",
            Self::Origin => "Origin",
            Self::Horizontal => "Horizontal",
            Self::Vertical => "Vertical",
            Self::Parallel => "Parallel",
            Self::Perpendicular => "Perpendicular",
            Self::Aligned => "Aligned",
        }
    }

    /// Point snaps lock onto existing geometry rather than a direction.
    pub(crate) fn is_point(self) -> bool {
        matches!(
            self,
            Self::Endpoint
                | Self::Midpoint
                | Self::Center
                | Self::Quadrant
                | Self::Intersection
                | Self::Origin
        )
    }

    /// Distance bonus, in snap radii, that lets important targets win over
    /// slightly closer but less meaningful ones.
    fn priority(self) -> f32 {
        match self {
            Self::Endpoint | Self::Intersection => 0.35,
            Self::Center | Self::Origin => 0.25,
            Self::Midpoint => 0.15,
            Self::Quadrant => 0.1,
            _ => 0.0,
        }
    }
}

/// A key point of the sketch that the pointer can lock onto.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SnapPoint {
    pub point: Vec2,
    pub kind: SnapKind,
    pub entity: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuideKind {
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Aligned,
}

/// A dashed inference line from the point that produced it to the snap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Guide {
    pub from: Vec2,
    pub to: Vec2,
    pub kind: GuideKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Snap {
    pub point: Vec2,
    pub kind: SnapKind,
    /// Entity the snap belongs to, for on-curve and point snaps.
    pub entity: Option<usize>,
    pub guides: Vec<Guide>,
}

impl Snap {
    pub(crate) fn free(point: Vec2) -> Self {
        Self {
            point,
            kind: SnapKind::Free,
            entity: None,
            guides: Vec::new(),
        }
    }
}

/// Everything the snapper may lock onto, borrowed from the session.
pub(crate) struct SnapContext<'a> {
    pub points: &'a [SnapPoint],
    /// Curves for on-curve snapping, with their entity index.
    pub curves: &'a [(usize, SketchCurve)],
    /// Entities excluded from snapping (for example, the ones being dragged).
    pub exclude: &'a [usize],
    /// The tool's current point: horizontal/vertical inference runs through it.
    pub anchor: Option<Vec2>,
    /// Directions for parallel and perpendicular inference through the anchor.
    pub references: &'a [Vec2],
    /// Recently hovered points for x/y alignment.
    pub tracked: &'a [Vec2],
    pub grid: f32,
    /// Snap radius in plane units.
    pub radius: f32,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug)]
struct Inference {
    origin: Vec2,
    direction: Vec2,
    kind: GuideKind,
}

impl Inference {
    fn project(&self, point: Vec2) -> Vec2 {
        self.origin + self.direction * (point - self.origin).dot(self.direction)
    }

    fn distance(&self, point: Vec2) -> f32 {
        (point - self.origin).perp_dot(self.direction).abs()
    }

    fn snap_kind(&self) -> SnapKind {
        match self.kind {
            GuideKind::Horizontal => SnapKind::Horizontal,
            GuideKind::Vertical => SnapKind::Vertical,
            GuideKind::Parallel => SnapKind::Parallel,
            GuideKind::Perpendicular => SnapKind::Perpendicular,
            GuideKind::Aligned => SnapKind::Aligned,
        }
    }

    fn guide(&self, to: Vec2) -> Guide {
        Guide {
            from: self.origin,
            to,
            kind: self.kind,
        }
    }

    fn crossing(&self, other: &Inference) -> Option<Vec2> {
        let denominator = self.direction.perp_dot(other.direction);
        if denominator.abs() < 1.0e-4 {
            return None;
        }
        let t = (other.origin - self.origin).perp_dot(other.direction) / denominator;
        Some(self.origin + self.direction * t)
    }
}

pub(crate) fn round_to(value: f32, step: f32) -> f32 {
    if step > 0.0 && step.is_finite() {
        (value / step).round() * step
    } else {
        value
    }
}

fn grid_point(point: Vec2, grid: f32) -> Vec2 {
    Vec2::new(round_to(point.x, grid), round_to(point.y, grid))
}

/// Snaps `cursor` (in plane units) according to `cx`.
pub(crate) fn snap(cursor: Vec2, cx: &SnapContext) -> Snap {
    if !cx.enabled || !cursor.is_finite() {
        return Snap::free(cursor);
    }
    let radius = cx.radius.max(1.0e-9);
    let excluded = |entity: Option<usize>| entity.is_some_and(|e| cx.exclude.contains(&e));
    // 1. Point snaps.
    let mut best: Option<(f32, SnapPoint)> = None;
    let origin = SnapPoint {
        point: Vec2::ZERO,
        kind: SnapKind::Origin,
        entity: None,
    };
    for candidate in cx.points.iter().chain(std::iter::once(&origin)) {
        if excluded(candidate.entity) {
            continue;
        }
        let distance = candidate.point.distance(cursor);
        if distance > radius {
            continue;
        }
        let score = distance - candidate.kind.priority() * radius;
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, *candidate));
        }
    }
    if let Some((_, target)) = best {
        let mut snap = Snap {
            point: target.point,
            kind: target.kind,
            entity: target.entity,
            guides: Vec::new(),
        };
        // Show axis guides when the target happens to line up with the anchor.
        if let Some(anchor) = cx.anchor {
            let offset = target.point - anchor;
            let exact = radius * 1.0e-3;
            if offset.length() > exact {
                if offset.y.abs() <= exact {
                    snap.guides.push(Guide {
                        from: anchor,
                        to: target.point,
                        kind: GuideKind::Horizontal,
                    });
                } else if offset.x.abs() <= exact {
                    snap.guides.push(Guide {
                        from: anchor,
                        to: target.point,
                        kind: GuideKind::Vertical,
                    });
                }
            }
        }
        return snap;
    }
    // 2. Inference lines near the pointer.
    let mut lines = Vec::new();
    if let Some(anchor) = cx.anchor {
        lines.push(Inference {
            origin: anchor,
            direction: Vec2::X,
            kind: GuideKind::Horizontal,
        });
        lines.push(Inference {
            origin: anchor,
            direction: Vec2::Y,
            kind: GuideKind::Vertical,
        });
        for reference in cx.references {
            let Some(direction) = reference.try_normalize() else {
                continue;
            };
            // Directions that coincide with the axes are already covered.
            let axis_aligned = |d: Vec2| d.x.abs() > 0.9998 || d.y.abs() > 0.9998;
            if !axis_aligned(direction) {
                lines.push(Inference {
                    origin: anchor,
                    direction,
                    kind: GuideKind::Parallel,
                });
            }
            let perpendicular = direction.perp();
            if !axis_aligned(perpendicular) {
                lines.push(Inference {
                    origin: anchor,
                    direction: perpendicular,
                    kind: GuideKind::Perpendicular,
                });
            }
        }
    }
    for tracked in cx.tracked {
        if cx
            .anchor
            .is_some_and(|a| a.distance(*tracked) < radius * 1.0e-3)
        {
            continue;
        }
        for direction in [Vec2::X, Vec2::Y] {
            lines.push(Inference {
                origin: *tracked,
                direction,
                kind: GuideKind::Aligned,
            });
        }
    }
    let near: Vec<Inference> = lines
        .into_iter()
        .filter(|line| line.distance(cursor) <= radius * 0.8)
        .collect();
    // Two inferences crossing near the pointer, e.g. horizontal from the
    // anchor and vertical from a tracked corner.
    let mut crossing: Option<(f32, Vec2, Inference, Inference)> = None;
    for (i, a) in near.iter().enumerate() {
        for b in &near[i + 1..] {
            if let Some(point) = a.crossing(b) {
                let distance = point.distance(cursor);
                if distance <= radius * 1.2 && crossing.is_none_or(|(d, ..)| distance < d) {
                    crossing = Some((distance, point, *a, *b));
                }
            }
        }
    }
    if let Some((_, point, a, b)) = crossing {
        // Anchor inferences describe the result better than alignments.
        let primary = if a.kind == GuideKind::Aligned { b } else { a };
        return Snap {
            point,
            kind: primary.snap_kind(),
            entity: None,
            guides: vec![a.guide(point), b.guide(point)],
        };
    }
    let nearest_curve = cx
        .curves
        .iter()
        .filter(|(entity, _)| !cx.exclude.contains(entity))
        .map(|(entity, curve)| (*entity, *curve, curve.closest(cursor).1))
        .filter(|(_, _, distance)| *distance <= radius * 0.8)
        .min_by(|a, b| a.2.total_cmp(&b.2));
    if let Some(line) = near
        .iter()
        .min_by(|a, b| a.distance(cursor).total_cmp(&b.distance(cursor)))
    {
        // An inference line meeting a nearby curve.
        if let Some((entity, curve, _)) = nearest_curve
            && let Some(point) = line_curve_crossing(line, &curve, cursor, radius)
        {
            return Snap {
                point,
                kind: line.snap_kind(),
                entity: Some(entity),
                guides: vec![line.guide(point)],
            };
        }
        let projected = line.project(cursor);
        let point = match line.kind {
            GuideKind::Horizontal | GuideKind::Aligned if line.direction == Vec2::X => {
                Vec2::new(round_to(projected.x, cx.grid), line.origin.y)
            }
            GuideKind::Vertical | GuideKind::Aligned => {
                Vec2::new(line.origin.x, round_to(projected.y, cx.grid))
            }
            _ => {
                let along = round_to((projected - line.origin).dot(line.direction), cx.grid);
                line.origin + line.direction * along
            }
        };
        return Snap {
            point,
            kind: line.snap_kind(),
            entity: None,
            guides: vec![line.guide(point)],
        };
    }
    if let Some((entity, curve, _)) = nearest_curve {
        let (t, _) = curve.closest(cursor);
        return Snap {
            point: curve.point_at(t),
            kind: SnapKind::OnCurve,
            entity: Some(entity),
            guides: Vec::new(),
        };
    }
    Snap {
        point: grid_point(cursor, cx.grid),
        kind: SnapKind::Grid,
        entity: None,
        guides: Vec::new(),
    }
}

/// The crossing of an inference line with a curve closest to `near`.
fn line_curve_crossing(
    line: &Inference,
    curve: &SketchCurve,
    near: Vec2,
    radius: f32,
) -> Option<Vec2> {
    let far = line.direction * 1.0e6;
    let carrier = SketchCurve::Line {
        start: line.origin - far,
        end: line.origin + far,
    };
    let sketch = forma_core::Sketch {
        entities: vec![carrier.into(), (*curve).into()],
        ..Default::default()
    };
    sketch
        .intersection_points(1.0e-6)
        .into_iter()
        .filter(|p| p.distance(near) <= radius)
        .min_by(|a, b| a.distance(near).total_cmp(&b.distance(near)))
}

/// Adaptive grid spacing for a plane scale in pixels per unit: a 1-2-5 minor
/// step at least `MINOR_PIXELS` apart, and a decade major step.
pub(crate) fn grid_steps(pixels_per_unit: f32) -> (f32, f32) {
    const MINOR_PIXELS: f32 = 14.0;
    if !pixels_per_unit.is_finite() || pixels_per_unit <= 0.0 {
        return (1.0, 10.0);
    }
    let units = MINOR_PIXELS / pixels_per_unit;
    let decade = 10f32.powf(units.log10().floor());
    let minor = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * decade)
        .find(|step| *step >= units)
        .unwrap_or(decade * 10.0);
    let major = 10f32.powf(((minor * 5.0).log10() - 1.0e-4).ceil());
    (minor, major)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(points: &'a [SnapPoint], curves: &'a [(usize, SketchCurve)]) -> SnapContext<'a> {
        SnapContext {
            points,
            curves,
            exclude: &[],
            anchor: None,
            references: &[],
            tracked: &[],
            grid: 0.5,
            radius: 0.2,
            enabled: true,
        }
    }

    #[test]
    fn endpoints_beat_midpoints_and_grid() {
        let points = [
            SnapPoint {
                point: Vec2::new(1.0, 1.0),
                kind: SnapKind::Midpoint,
                entity: Some(0),
            },
            SnapPoint {
                point: Vec2::new(1.1, 1.0),
                kind: SnapKind::Endpoint,
                entity: Some(1),
            },
        ];
        let snap = snap(Vec2::new(1.04, 1.02), &context(&points, &[]));
        assert_eq!(snap.kind, SnapKind::Endpoint);
        assert_eq!(snap.point, Vec2::new(1.1, 1.0));
        let snap = super::snap(Vec2::new(3.13, 2.8), &context(&points, &[]));
        assert_eq!(
            (snap.kind, snap.point),
            (SnapKind::Grid, Vec2::new(3.0, 3.0))
        );
    }

    #[test]
    fn disabled_snapping_returns_the_raw_point() {
        let mut cx = context(&[], &[]);
        cx.enabled = false;
        let p = Vec2::new(0.123, 4.567);
        assert_eq!(snap(p, &cx), Snap::free(p));
    }

    #[test]
    fn horizontal_inference_rounds_along_the_line() {
        let mut cx = context(&[], &[]);
        cx.anchor = Some(Vec2::new(0.3, 0.7));
        let snap = snap(Vec2::new(2.12, 0.78), &cx);
        assert_eq!(snap.kind, SnapKind::Horizontal);
        assert_eq!(snap.point, Vec2::new(2.0, 0.7));
        assert_eq!(snap.guides.len(), 1);
    }

    #[test]
    fn crossing_inferences_close_a_rectangle() {
        let tracked = [Vec2::new(0.0, 2.0)];
        let mut cx = context(&[], &[]);
        cx.anchor = Some(Vec2::new(3.0, 0.0));
        cx.tracked = &tracked;
        // Vertical through the anchor, horizontal alignment with the tracked point.
        let snap = snap(Vec2::new(3.08, 1.93), &cx);
        assert_eq!(snap.point, Vec2::new(3.0, 2.0));
        assert_eq!(snap.kind, SnapKind::Vertical);
        assert_eq!(snap.guides.len(), 2);
    }

    #[test]
    fn perpendicular_reference_and_curve_crossing() {
        let references = [Vec2::new(1.0, 1.0)];
        let mut cx = context(&[], &[]);
        cx.anchor = Some(Vec2::ZERO);
        cx.references = &references;
        cx.grid = 0.0;
        let snap = snap(Vec2::new(-1.0, 1.05), &cx);
        assert_eq!(snap.kind, SnapKind::Perpendicular);
        assert!(snap.point.distance(Vec2::new(-1.025, 1.025)) < 1.0e-4);
        // Horizontal inference meets a circle.
        let curves = [(
            3,
            SketchCurve::Circle {
                center: Vec2::new(2.0, 0.0),
                radius: 1.0,
            },
        )];
        let mut cx = context(&[], &curves);
        cx.anchor = Some(Vec2::ZERO);
        let snap = super::snap(Vec2::new(1.05, 0.03), &cx);
        assert_eq!(snap.entity, Some(3));
        assert!(snap.point.distance(Vec2::new(1.0, 0.0)) < 1.0e-4);
    }

    #[test]
    fn excluded_entities_do_not_attract() {
        let points = [SnapPoint {
            point: Vec2::new(1.0, 1.0),
            kind: SnapKind::Endpoint,
            entity: Some(4),
        }];
        let mut cx = context(&points, &[]);
        cx.exclude = &[4];
        assert_eq!(snap(Vec2::new(1.02, 1.0), &cx).kind, SnapKind::Grid);
    }

    #[test]
    fn grid_steps_stay_readable_at_every_zoom() {
        for pixels_per_unit in [0.01, 0.3, 1.0, 7.0, 55.0, 140.0, 999.0, 12345.0] {
            let (minor, major) = grid_steps(pixels_per_unit);
            let pixels = minor * pixels_per_unit;
            assert!(
                (14.0..=14.0 * 2.5 + 0.01).contains(&pixels),
                "{pixels_per_unit}: {pixels}"
            );
            assert!(major > minor);
            let ratio = major / minor;
            assert!((ratio - ratio.round()).abs() < 1.0e-3 && (2.0..=20.0).contains(&ratio));
        }
    }
}

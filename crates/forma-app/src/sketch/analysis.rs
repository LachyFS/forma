//! Derived data for the edited sketch, rebuilt only when it changes:
//! profiles, snap targets and display polylines.
use super::REFERENCE_BASE;
use super::snap::{SnapKind, SnapPoint};
use forma_core::{Profile, SKETCH_TOLERANCE, SketchCurve, SketchEntity, Vec2};

pub(crate) struct Analysis {
    pub entities: Vec<SketchEntity>,
    pub profiles: Vec<Profile>,
    /// Outer loop and holes of each profile as polygons.
    pub profile_polygons: Vec<Vec<Vec<Vec2>>>,
    pub polylines: Vec<Vec<Vec2>>,
    pub snap_points: Vec<SnapPoint>,
    /// Curves for on-curve snapping and picking, including reference
    /// geometry at `REFERENCE_BASE + i`.
    pub curves: Vec<(usize, SketchCurve)>,
    /// Endpoints with the number of curves meeting there.
    pub endpoints: Vec<(Vec2, usize)>,
}

/// Chord tolerance that keeps arcs smooth on screen at any size.
pub(crate) fn display_tolerance(curve: &SketchCurve) -> f32 {
    match *curve {
        SketchCurve::Line { .. } => 1.0,
        SketchCurve::Arc { radius, .. } | SketchCurve::Circle { radius, .. } => {
            (radius * 1.5e-3).max(1.0e-7)
        }
    }
}

pub(crate) fn analyze(entities: &[SketchEntity], reference: &[SketchCurve]) -> Analysis {
    let profiles = forma_core::Sketch {
        entities: entities.to_vec(),
        ..Default::default()
    }
    .profiles(SKETCH_TOLERANCE);
    let profile_polygons = profiles
        .iter()
        .map(|profile| {
            std::iter::once(&profile.outer)
                .chain(&profile.holes)
                .map(|profile_loop| {
                    let (min, max) = profile_loop.bounds();
                    profile_loop.polygon(((max - min).max_element() * 1.0e-3).max(1.0e-7))
                })
                .collect()
        })
        .collect();
    let polylines = entities
        .iter()
        .map(|entity| entity.curve.polyline(display_tolerance(&entity.curve)))
        .collect();
    let mut snap_points = Vec::new();
    let mut endpoints: Vec<(Vec2, usize)> = Vec::new();
    let push_point = |points: &mut Vec<SnapPoint>, point: Vec2, kind: SnapKind, entity: usize| {
        if !points
            .iter()
            .any(|p| p.kind == kind && p.point.distance(point) <= SKETCH_TOLERANCE * 4.0)
        {
            points.push(SnapPoint {
                point,
                kind,
                entity: Some(entity),
            });
        }
    };
    let all_curves = entities.iter().map(|e| e.curve).enumerate().chain(
        reference
            .iter()
            .copied()
            .enumerate()
            .map(|(i, c)| (REFERENCE_BASE + i, c)),
    );
    for (index, curve) in all_curves.clone() {
        if !curve.is_valid() {
            continue;
        }
        match curve {
            SketchCurve::Line { start, end } => {
                for point in [start, end] {
                    push_point(&mut snap_points, point, SnapKind::Endpoint, index);
                }
                push_point(
                    &mut snap_points,
                    curve.midpoint(),
                    SnapKind::Midpoint,
                    index,
                );
            }
            SketchCurve::Arc { center, .. } => {
                push_point(&mut snap_points, curve.start(), SnapKind::Endpoint, index);
                push_point(&mut snap_points, curve.end(), SnapKind::Endpoint, index);
                push_point(
                    &mut snap_points,
                    curve.midpoint(),
                    SnapKind::Midpoint,
                    index,
                );
                push_point(&mut snap_points, center, SnapKind::Center, index);
                for point in curve.quadrant_points() {
                    push_point(&mut snap_points, point, SnapKind::Quadrant, index);
                }
            }
            SketchCurve::Circle { center, .. } => {
                push_point(&mut snap_points, center, SnapKind::Center, index);
                for point in curve.quadrant_points() {
                    push_point(&mut snap_points, point, SnapKind::Quadrant, index);
                }
            }
        }
        if index < REFERENCE_BASE && !curve.is_closed() {
            for point in [curve.start(), curve.end()] {
                match endpoints
                    .iter_mut()
                    .find(|(p, _)| p.distance(point) <= SKETCH_TOLERANCE * 4.0)
                {
                    Some((_, count)) => *count += 1,
                    None => endpoints.push((point, 1)),
                }
            }
        }
    }
    let crossing_sketch = forma_core::Sketch {
        entities: all_curves.clone().map(|(_, c)| c.into()).collect(),
        ..Default::default()
    };
    for point in crossing_sketch.intersection_points(SKETCH_TOLERANCE) {
        snap_points.push(SnapPoint {
            point,
            kind: SnapKind::Intersection,
            entity: None,
        });
    }
    Analysis {
        entities: entities.to_vec(),
        profiles,
        profile_polygons,
        polylines,
        snap_points,
        curves: all_curves.filter(|(_, c)| c.is_valid()).collect(),
        endpoints,
    }
}

impl Analysis {
    /// The profile region containing `point`. Regions never overlap, so the
    /// first hit is the only one.
    pub(crate) fn profile_at(&self, point: Vec2) -> Option<usize> {
        self.profile_polygons.iter().position(|loops| {
            let mut loops = loops.iter();
            loops.next().is_some_and(|outer| inside(point, outer))
                && !loops.any(|hole| inside(point, hole))
        })
    }

    /// The entity nearest `point` within `radius` plane units.
    pub(crate) fn entity_at(&self, point: Vec2, radius: f32) -> Option<usize> {
        self.entities
            .iter()
            .enumerate()
            .map(|(index, entity)| (index, entity.curve.closest(point).1))
            .filter(|(_, distance)| *distance <= radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }
}

fn inside(point: Vec2, polygon: &[Vec2]) -> bool {
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

    #[test]
    fn regions_are_found_from_cached_polygons() {
        let line = |a: (f32, f32), b: (f32, f32)| -> SketchEntity {
            SketchCurve::Line {
                start: Vec2::new(a.0, a.1),
                end: Vec2::new(b.0, b.1),
            }
            .into()
        };
        let mut entities = vec![
            line((0.0, 0.0), (4.0, 0.0)),
            line((4.0, 0.0), (4.0, 4.0)),
            line((4.0, 4.0), (0.0, 4.0)),
            line((0.0, 4.0), (0.0, 0.0)),
        ];
        entities.push(
            SketchCurve::Circle {
                center: Vec2::splat(2.0),
                radius: 1.0,
            }
            .into(),
        );
        let analysis = analyze(&entities, &[]);
        let ring = analysis.profile_at(Vec2::new(0.5, 0.5)).unwrap();
        let disc = analysis.profile_at(Vec2::splat(2.0)).unwrap();
        assert_ne!(ring, disc);
        assert!(analysis.profiles[disc].holes.is_empty());
        assert_eq!(analysis.profile_at(Vec2::splat(9.0)), None);
        assert_eq!(analysis.entity_at(Vec2::new(2.0, 0.05), 0.1), Some(0));
        // Endpoints shared by two lines count as connected.
        assert!(analysis.endpoints.iter().all(|(_, count)| *count == 2));
        assert!(
            analysis
                .snap_points
                .iter()
                .any(|p| p.kind == SnapKind::Center && p.point == Vec2::splat(2.0))
        );
    }
}

//! Derived data for the edited sketch, rebuilt only when it changes:
//! profiles, snap targets and display polylines.
use super::REFERENCE_BASE;
use super::snap::{SnapKind, SnapPoint};
use forma_core::{Profile, SKETCH_TOLERANCE, SketchCurve, SketchEntity, Vec2};
use std::{collections::HashMap, rc::Rc};

pub(crate) struct Analysis {
    pub entities: Vec<SketchEntity>,
    pub profiles: Vec<Profile>,
    /// Outer loop and holes of each profile as polygons.
    pub profile_polygons: Vec<Vec<Vec<Vec2>>>,
    pub polylines: Vec<Vec<Vec2>>,
    /// What the pointer snaps to; empty while dragging, when the drag
    /// brings its own targets.
    pub targets: Rc<SnapTargets>,
    /// Endpoints with the number of curves meeting there.
    pub endpoints: Vec<(Vec2, usize)>,
}

/// Snap points and curves for the pointer.
#[derive(Debug, Default)]
pub(crate) struct SnapTargets {
    pub points: Vec<SnapPoint>,
    /// Curves for on-curve snapping and picking, including reference
    /// geometry at `REFERENCE_BASE + i`.
    pub curves: Vec<(usize, SketchCurve)>,
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

/// Distance under which two derived points are the same point.
const SAME: f32 = SKETCH_TOLERANCE * 4.0;

/// Points bucketed on a `SAME`-sized grid, so finding a coincident point is
/// constant time however large the sketch.
struct PointHash<T> {
    cells: HashMap<(i64, i64), Vec<usize>>,
    items: Vec<(Vec2, T)>,
}

impl<T> PointHash<T> {
    fn new() -> Self {
        Self {
            cells: HashMap::new(),
            items: Vec::new(),
        }
    }

    fn cell(p: Vec2) -> (i64, i64) {
        ((p.x / SAME).floor() as i64, (p.y / SAME).floor() as i64)
    }

    /// The index of a stored point within `SAME` of `p` that `matches`.
    fn find(&self, p: Vec2, matches: impl Fn(&T) -> bool) -> Option<usize> {
        let (x, y) = Self::cell(p);
        (-1..=1)
            .flat_map(|dx| (-1..=1).map(move |dy| (x + dx, y + dy)))
            .filter_map(|key| self.cells.get(&key))
            .flatten()
            .copied()
            .find(|&i| self.items[i].0.distance(p) <= SAME && matches(&self.items[i].1))
    }

    fn push(&mut self, p: Vec2, value: T) {
        self.cells
            .entry(Self::cell(p))
            .or_default()
            .push(self.items.len());
        self.items.push((p, value));
    }
}

/// Snap targets of `entities` except those in `skip`, plus reference
/// curves. Coincident points of one kind are kept once.
pub(crate) fn snap_targets(
    entities: &[SketchEntity],
    skip: &[usize],
    reference: &[SketchCurve],
) -> SnapTargets {
    let curves: Vec<(usize, SketchCurve)> = entities
        .iter()
        .map(|e| e.curve)
        .enumerate()
        .filter(|(index, _)| !skip.contains(index))
        .chain(
            reference
                .iter()
                .copied()
                .enumerate()
                .map(|(i, c)| (REFERENCE_BASE + i, c)),
        )
        .filter(|(_, curve)| curve.is_valid())
        .collect();
    let mut points: PointHash<(SnapKind, usize)> = PointHash::new();
    let mut push = |point: Vec2, kind: SnapKind, entity: usize| {
        if points.find(point, |(k, _)| *k == kind).is_none() {
            points.push(point, (kind, entity));
        }
    };
    for &(index, curve) in &curves {
        match curve {
            SketchCurve::Line { start, end } => {
                push(start, SnapKind::Endpoint, index);
                push(end, SnapKind::Endpoint, index);
                push(curve.midpoint(), SnapKind::Midpoint, index);
            }
            SketchCurve::Arc { center, .. } => {
                push(curve.start(), SnapKind::Endpoint, index);
                push(curve.end(), SnapKind::Endpoint, index);
                push(curve.midpoint(), SnapKind::Midpoint, index);
                push(center, SnapKind::Center, index);
                for point in curve.quadrant_points() {
                    push(point, SnapKind::Quadrant, index);
                }
            }
            SketchCurve::Circle { center, .. } => {
                push(center, SnapKind::Center, index);
                for point in curve.quadrant_points() {
                    push(point, SnapKind::Quadrant, index);
                }
            }
        }
    }
    let mut snap_points: Vec<SnapPoint> = points
        .items
        .into_iter()
        .map(|(point, (kind, entity))| SnapPoint {
            point,
            kind,
            entity: Some(entity),
        })
        .collect();
    let crossings = forma_core::Sketch {
        entities: curves.iter().map(|(_, c)| (*c).into()).collect(),
        ..Default::default()
    };
    snap_points.extend(
        crossings
            .intersection_points(SKETCH_TOLERANCE)
            .into_iter()
            .map(|point| SnapPoint {
                point,
                kind: SnapKind::Intersection,
                entity: None,
            }),
    );
    SnapTargets {
        points: snap_points,
        curves,
    }
}

/// Endpoints of open curves with how many curves meet at each.
fn endpoints(entities: &[SketchEntity]) -> Vec<(Vec2, usize)> {
    let mut ends: PointHash<usize> = PointHash::new();
    for entity in entities
        .iter()
        .filter(|e| e.curve.is_valid() && !e.curve.is_closed())
    {
        for point in [entity.curve.start(), entity.curve.end()] {
            match ends.find(point, |_| true) {
                Some(index) => ends.items[index].1 += 1,
                None => ends.push(point, 1),
            }
        }
    }
    ends.items
}

fn profile_polygons(profiles: &[Profile]) -> Vec<Vec<Vec<Vec2>>> {
    profiles
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
        .collect()
}

/// Everything derived from the sketch, after an edit.
pub(crate) fn analyze(entities: &[SketchEntity], reference: &[SketchCurve]) -> Analysis {
    let profiles = forma_core::Sketch {
        entities: entities.to_vec(),
        ..Default::default()
    }
    .profiles(SKETCH_TOLERANCE);
    Analysis {
        entities: entities.to_vec(),
        profile_polygons: profile_polygons(&profiles),
        profiles,
        polylines: entities
            .iter()
            .map(|entity| entity.curve.polyline(display_tolerance(&entity.curve)))
            .collect(),
        targets: Rc::new(snap_targets(entities, &[], reference)),
        endpoints: endpoints(entities),
    }
}

/// Sketches up to this size keep live region shading while dragging.
const LIVE_PROFILES_DURING_DRAG: usize = 400;

/// What a drag needs every pointer move: curves and their ends. Snapping
/// uses the targets the drag built when it started; regions are kept live
/// only for small sketches and are rebuilt in full when the drag ends.
pub(crate) fn analyze_for_drag(entities: &[SketchEntity]) -> Analysis {
    let profiles = if entities.len() <= LIVE_PROFILES_DURING_DRAG {
        forma_core::Sketch {
            entities: entities.to_vec(),
            ..Default::default()
        }
        .profiles(SKETCH_TOLERANCE)
    } else {
        Vec::new()
    };
    Analysis {
        entities: entities.to_vec(),
        profile_polygons: profile_polygons(&profiles),
        profiles,
        polylines: entities
            .iter()
            .map(|entity| entity.curve.polyline(display_tolerance(&entity.curve)))
            .collect(),
        targets: Rc::new(SnapTargets::default()),
        endpoints: endpoints(entities),
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

    fn line(a: (f32, f32), b: (f32, f32)) -> SketchEntity {
        SketchCurve::Line {
            start: Vec2::new(a.0, a.1),
            end: Vec2::new(b.0, b.1),
        }
        .into()
    }

    #[test]
    fn regions_are_found_from_cached_polygons() {
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
                .targets
                .points
                .iter()
                .any(|p| p.kind == SnapKind::Center && p.point == Vec2::splat(2.0))
        );
    }

    #[test]
    fn dragged_endpoints_slide_along_curves_without_sticking() {
        // Review case "dragsnap": L's bottom end starts on M (a T-junction)
        // and is dragged along M two pixels at a time. Its own former
        // junction used to hold it for ~11 px at a time.
        use crate::sketch::snap::{SnapContext, snap};
        let before = vec![line((0.0, 0.0), (12.0, 0.0)), line((5.0, 0.0), (5.0, 5.0))];
        let targets = snap_targets(&before, &[1], &[]);
        let radius = 11.0 / 100.0;
        for step in 1..=12 {
            let cursor = Vec2::new(5.0 + step as f32 * 0.02, 0.0);
            let cx = SnapContext {
                points: &targets.points,
                curves: &targets.curves,
                exclude: &[],
                anchor: None,
                references: &[],
                tracked: &[],
                grid: 0.1,
                radius,
                enabled: true,
            };
            let snapped = snap(cursor, &cx);
            assert!(
                (snapped.point.x - cursor.x).abs() < 1.0e-4,
                "move {step}: cursor {} snapped to {} ({:?})",
                cursor.x,
                snapped.point.x,
                snapped.kind
            );
        }
    }

    #[test]
    fn dense_sketches_analyze_quickly() {
        // Review case "grid": 402 lines took 1.2 s per analysis.
        let mut entities = Vec::new();
        for i in 0..=200 {
            let x = i as f32;
            entities.push(line((x, 0.0), (x, 200.0)));
            entities.push(line((0.0, x), (200.0, x)));
        }
        let started = std::time::Instant::now();
        let analysis = analyze(&entities, &[]);
        assert_eq!(analysis.profiles.len(), 40_000);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "analysis took {:?}",
            started.elapsed()
        );
        // Drags rebuild only display data for large sketches.
        let started = std::time::Instant::now();
        let light = analyze_for_drag(&entities);
        assert!(light.profiles.is_empty() && light.targets.points.is_empty());
        assert_eq!(light.polylines.len(), entities.len());
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
    }
}

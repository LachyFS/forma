//! Choosing and merging sketch regions.
//!
//! `Sketch::profiles` returns every bounded region of the sketch's planar
//! arrangement as its own profile, largest first: a circle inside a rectangle
//! gives the ring (with the circle as a hole) and the disc. Features store
//! interior points to pick regions robustly across sketch edits. Adjacent
//! picked regions are merged in 2D before extrusion, so a rectangle split by
//! a line still becomes one solid instead of two lumps touching face to face.
use forma_core::{Profile, ProfileLoop, Sketch, SketchCurve};
use glam::{DVec2, Vec2};
use std::f64::consts::TAU;

/// A point strictly inside `profile` (inside its outer loop, outside holes).
pub fn interior_point(profile: &Profile) -> Option<Vec2> {
    let (min, max) = profile.outer.bounds();
    let size = (max - min).max_element().max(1.0e-9);
    let tolerance = (size * 1.0e-3).max(1.0e-6);
    let loops: Vec<Vec<Vec2>> = std::iter::once(&profile.outer)
        .chain(&profile.holes)
        .map(|l| l.polygon(tolerance))
        .collect();
    // Scan horizontal lines; the widest inside span's midpoint is well clear
    // of every boundary.
    let mut best: Option<(f32, Vec2)> = None;
    for k in 1..24 {
        let y = min.y + (max.y - min.y) * (k as f32 / 24.0 + 0.0137);
        let mut xs = Vec::new();
        for polygon in &loops {
            for i in 0..polygon.len() {
                let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
                if (a.y > y) != (b.y > y) {
                    xs.push(a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y));
                }
            }
        }
        xs.sort_by(f32::total_cmp);
        for pair in xs.windows(2) {
            let width = pair[1] - pair[0];
            let candidate = Vec2::new((pair[0] + pair[1]) * 0.5, y);
            if width > best.map_or(0.0, |(w, _)| w) && profile.contains(candidate) {
                best = Some((width, candidate));
            }
        }
    }
    best.map(|(_, point)| point)
}

/// Regions not lying inside another region's holes: the outermost regions,
/// each with its holes. This is the default selection for an extrusion.
pub fn outermost_regions(profiles: &[Profile]) -> Vec<usize> {
    let samples: Vec<Option<Vec2>> = profiles.iter().map(interior_point).collect();
    (0..profiles.len())
        .filter(|&i| {
            let Some(sample) = samples[i] else {
                return false;
            };
            !profiles
                .iter()
                .enumerate()
                .any(|(j, other)| j != i && other.holes.iter().any(|hole| hole.contains(sample)))
        })
        .collect()
}

/// Interior points of the default (outermost) regions of a sketch.
pub fn default_region_points(sketch: &Sketch) -> Vec<Vec2> {
    let profiles = sketch.profiles(forma_core::SKETCH_TOLERANCE);
    outermost_regions(&profiles)
        .into_iter()
        .filter_map(|i| interior_point(&profiles[i]))
        .collect()
}

/// A directed straight or circular piece of a region boundary.
#[derive(Clone, Copy, Debug)]
struct Piece {
    start: DVec2,
    end: DVec2,
    /// Circle center, radius, start angle and signed sweep for arcs.
    arc: Option<(DVec2, f64, f64, f64)>,
}

impl Piece {
    fn midpoint(&self) -> DVec2 {
        match self.arc {
            None => (self.start + self.end) * 0.5,
            Some((center, radius, a0, sweep)) => {
                center + DVec2::from_angle(a0 + sweep * 0.5) * radius
            }
        }
    }

    /// Split at the given points lying strictly inside the piece.
    fn split(&self, points: &[DVec2], tolerance: f64) -> Vec<Piece> {
        let mut cuts: Vec<f64> = Vec::new();
        for &p in points {
            if p.distance(self.start) <= tolerance || p.distance(self.end) <= tolerance {
                continue;
            }
            match self.arc {
                None => {
                    let d = self.end - self.start;
                    let t = (p - self.start).dot(d) / d.length_squared().max(1.0e-300);
                    if t > 0.0 && t < 1.0 && (self.start + d * t).distance(p) <= tolerance {
                        cuts.push(t);
                    }
                }
                Some((center, radius, a0, sweep)) => {
                    if (p.distance(center) - radius).abs() > tolerance {
                        continue;
                    }
                    let relative = ((p - center).to_angle() - a0).rem_euclid(TAU);
                    let along = if sweep >= 0.0 {
                        relative
                    } else {
                        TAU - relative
                    };
                    let t = along / sweep.abs();
                    if t > 0.0 && t < 1.0 {
                        cuts.push(t);
                    }
                }
            }
        }
        if cuts.is_empty() {
            return vec![*self];
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1.0e-9);
        let mut pieces = Vec::new();
        let mut previous = 0.0;
        for t in cuts.into_iter().chain(std::iter::once(1.0)) {
            pieces.push(self.sub(previous, t));
            previous = t;
        }
        pieces
    }

    fn sub(&self, t0: f64, t1: f64) -> Piece {
        match self.arc {
            None => Piece {
                start: self.start.lerp(self.end, t0),
                end: self.start.lerp(self.end, t1),
                arc: None,
            },
            Some((center, radius, a0, sweep)) => {
                let (b0, b1) = (a0 + sweep * t0, a0 + sweep * t1);
                Piece {
                    start: center + DVec2::from_angle(b0) * radius,
                    end: center + DVec2::from_angle(b1) * radius,
                    arc: Some((center, radius, b0, b1 - b0)),
                }
            }
        }
    }

    fn reverses(&self, other: &Piece, tolerance: f64) -> bool {
        self.start.distance(other.end) <= tolerance
            && self.end.distance(other.start) <= tolerance
            && self.midpoint().distance(other.midpoint()) <= tolerance
    }

    fn curve(&self) -> SketchCurve {
        match self.arc {
            None => SketchCurve::Line {
                start: self.start.as_vec2(),
                end: self.end.as_vec2(),
            },
            Some((center, radius, a0, sweep)) => SketchCurve::Arc {
                center: center.as_vec2(),
                radius: radius as f32,
                start_angle: a0 as f32,
                sweep: sweep as f32,
            },
        }
    }
}

fn loop_pieces(profile_loop: &ProfileLoop, counter_clockwise: bool, out: &mut Vec<Piece>) {
    for segment in &profile_loop.segments {
        match *segment {
            SketchCurve::Line { start, end } => out.push(Piece {
                start: start.as_dvec2(),
                end: end.as_dvec2(),
                arc: None,
            }),
            SketchCurve::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => {
                let (center, radius) = (center.as_dvec2(), f64::from(radius));
                let (a0, sweep) = (f64::from(start_angle), f64::from(sweep));
                out.push(Piece {
                    start: center + DVec2::from_angle(a0) * radius,
                    end: center + DVec2::from_angle(a0 + sweep) * radius,
                    arc: Some((center, radius, a0, sweep)),
                });
            }
            SketchCurve::Circle { center, radius } => {
                // A circle takes its direction from the loop it bounds.
                let (center, radius) = (center.as_dvec2(), f64::from(radius));
                let sweep = if counter_clockwise { TAU } else { -TAU };
                let whole = Piece {
                    start: center + DVec2::X * radius,
                    end: center + DVec2::X * radius,
                    arc: Some((center, radius, 0.0, sweep)),
                };
                out.push(whole.sub(0.0, 0.5));
                out.push(whole.sub(0.5, 1.0));
            }
        }
    }
}

/// Merge regions that share boundary curves into single profiles. Regions
/// that touch nothing pass through unchanged.
pub(crate) fn merge_adjacent(profiles: Vec<Profile>) -> Vec<Profile> {
    if profiles.len() < 2 {
        return profiles;
    }
    let size = profiles
        .iter()
        .map(|p| {
            let (min, max) = p.outer.bounds();
            (max - min).max_element()
        })
        .fold(0.0_f32, f32::max);
    let tolerance = (f64::from(size) * 1.0e-6).max(1.0e-7);
    let mut pieces = Vec::new();
    for profile in &profiles {
        loop_pieces(&profile.outer, true, &mut pieces);
        for hole in &profile.holes {
            loop_pieces(hole, false, &mut pieces);
        }
    }
    // Split every piece at every vertex so shared boundaries line up exactly.
    let vertices: Vec<DVec2> = pieces.iter().flat_map(|p| [p.start, p.end]).collect();
    let pieces: Vec<Piece> = pieces
        .iter()
        .flat_map(|piece| piece.split(&vertices, tolerance))
        .collect();
    let mut alive = vec![true; pieces.len()];
    let mut cancelled = false;
    for i in 0..pieces.len() {
        if !alive[i] {
            continue;
        }
        if let Some(j) =
            (i + 1..pieces.len()).find(|&j| alive[j] && pieces[i].reverses(&pieces[j], tolerance))
        {
            alive[i] = false;
            alive[j] = false;
            cancelled = true;
        }
    }
    if !cancelled {
        return profiles;
    }
    // Chain the remaining boundary into closed loops.
    let mut loops: Vec<Vec<Piece>> = Vec::new();
    let remaining: Vec<Piece> = pieces
        .iter()
        .zip(&alive)
        .filter(|(_, alive)| **alive)
        .map(|(p, _)| *p)
        .collect();
    let mut used = vec![false; remaining.len()];
    for first in 0..remaining.len() {
        if used[first] {
            continue;
        }
        used[first] = true;
        let mut chain = vec![remaining[first]];
        let mut closed = false;
        for _ in 0..remaining.len() {
            let end = chain.last().unwrap().end;
            if end.distance(chain[0].start) <= tolerance && chain.len() > 1 {
                closed = true;
                break;
            }
            let Some(next) = (0..remaining.len())
                .find(|&k| !used[k] && remaining[k].start.distance(end) <= tolerance)
            else {
                break;
            };
            used[next] = true;
            chain.push(remaining[next]);
        }
        if !closed {
            // Inconsistent boundary: fall back to the unmerged regions.
            return profiles;
        }
        loops.push(chain);
    }
    let to_loop = |chain: &[Piece]| ProfileLoop {
        segments: chain.iter().map(Piece::curve).collect(),
        entities: vec![0; chain.len()],
    };
    let mut outers = Vec::new();
    let mut holes = Vec::new();
    for chain in &loops {
        let profile_loop = to_loop(chain);
        if profile_loop.signed_area() >= 0.0 {
            outers.push(profile_loop);
        } else {
            holes.push(profile_loop);
        }
    }
    let mut merged: Vec<Profile> = outers
        .into_iter()
        .map(|outer| Profile {
            outer,
            holes: Vec::new(),
        })
        .collect();
    for hole in holes {
        let sample = hole.segments[0].midpoint();
        // The smallest outer loop around the hole owns it.
        let owner = (0..merged.len())
            .filter(|&i| {
                merged[i]
                    .outer
                    .contains(sample + (hole.segments[0].tangent_at(0.5).perp() * -1.0e-4))
                    || merged[i].outer.contains(sample)
            })
            .min_by(|&a, &b| {
                merged[a]
                    .outer
                    .signed_area()
                    .total_cmp(&merged[b].outer.signed_area())
            });
        match owner {
            Some(owner) => merged[owner].holes.push(hole),
            None => return profiles,
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::{SketchEntity, SketchPlane};

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
    fn nested_regions_default_to_the_outer_ring() {
        let mut sketch = Sketch::new(SketchPlane::ground());
        sketch.entities = rectangle((0.0, 0.0), (4.0, 3.0));
        sketch.entities.push(
            SketchCurve::Circle {
                center: Vec2::new(2.0, 1.5),
                radius: 0.5,
            }
            .into(),
        );
        let profiles = sketch.profiles(forma_core::SKETCH_TOLERANCE);
        assert_eq!(profiles.len(), 2, "ring and disc");
        let outer = outermost_regions(&profiles);
        assert_eq!(outer, vec![0]);
        let point = interior_point(&profiles[0]).unwrap();
        assert!(profiles[0].contains(point) && !profiles[1].contains(point));
        let disc = interior_point(&profiles[1]).unwrap();
        assert!(profiles[1].contains(disc) && !profiles[0].contains(disc));
        // Selecting ring and disc together merges them into the full rectangle.
        let merged = merge_adjacent(profiles);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].holes.is_empty());
        assert!((merged[0].area() - 12.0).abs() < 1.0e-3);
    }

    #[test]
    fn split_rectangles_merge_into_one_region() {
        let mut sketch = Sketch::new(SketchPlane::front());
        sketch.entities = rectangle((0.0, 0.0), (4.0, 2.0));
        sketch.entities.push(line((1.5, -0.5), (1.5, 2.5)));
        let profiles = sketch.profiles(forma_core::SKETCH_TOLERANCE);
        assert_eq!(profiles.len(), 2);
        assert_eq!(outermost_regions(&profiles).len(), 2);
        let merged = merge_adjacent(profiles);
        assert_eq!(merged.len(), 1);
        assert!((merged[0].area() - 8.0).abs() < 1.0e-3);
        // Overlapping rectangles: three regions merge into the union outline.
        let mut sketch = Sketch::new(SketchPlane::front());
        sketch.entities = rectangle((0.0, 0.0), (2.0, 2.0));
        sketch.entities.extend(rectangle((1.0, 1.0), (3.0, 3.0)));
        let profiles = sketch.profiles(forma_core::SKETCH_TOLERANCE);
        assert_eq!(profiles.len(), 3);
        let merged = merge_adjacent(profiles);
        assert_eq!(merged.len(), 1);
        assert!((merged[0].area() - 7.0).abs() < 1.0e-3);
    }
}

//! Planar arrangement of sketch curves.
//!
//! Curves are split wherever they meet (crossings, T-junctions, tangencies and
//! endpoints that touch within tolerance), coincident vertices are merged, and
//! the resulting planar graph is walked to find its minimal faces. Every
//! bounded face becomes a [`Profile`]; connected groups of curves that sit
//! inside a face become its holes. This matches how CAD sketchers present
//! regions: two overlapping rectangles give three regions, a circle inside a
//! rectangle gives a ring and a disc.
//!
//! All geometry is evaluated in `f64` and converted back to sketch precision
//! only for the output loops.
use super::{Profile, ProfileLoop, SketchCurve, SketchEntity};
use glam::{DVec2, Vec2};
use std::collections::HashMap;
use std::f64::consts::{PI, TAU};

/// Double precision view of a curve. Every kind is parameterized over 0..=1;
/// a circle's parameter is its angle divided by one turn.
#[derive(Clone, Copy, Debug)]
pub(super) enum Geo {
    Line {
        a: DVec2,
        b: DVec2,
    },
    Arc {
        c: DVec2,
        r: f64,
        start: f64,
        sweep: f64,
    },
    Circle {
        c: DVec2,
        r: f64,
    },
}

impl Geo {
    pub(super) fn new(curve: &SketchCurve) -> Self {
        match *curve {
            SketchCurve::Line { start, end } => Self::Line {
                a: start.as_dvec2(),
                b: end.as_dvec2(),
            },
            SketchCurve::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => Self::Arc {
                c: center.as_dvec2(),
                r: radius as f64,
                start: start_angle as f64,
                sweep: sweep as f64,
            },
            SketchCurve::Circle { center, radius } => Self::Circle {
                c: center.as_dvec2(),
                r: radius as f64,
            },
        }
    }

    pub(super) fn is_closed(&self) -> bool {
        matches!(self, Self::Circle { .. })
    }

    pub(super) fn point(&self, t: f64) -> DVec2 {
        match *self {
            Self::Line { a, b } => a.lerp(b, t),
            Self::Arc { c, r, start, sweep } => c + DVec2::from_angle(start + sweep * t) * r,
            Self::Circle { c, r } => c + DVec2::from_angle(TAU * t) * r,
        }
    }

    fn length(&self) -> f64 {
        match *self {
            Self::Line { a, b } => a.distance(b),
            Self::Arc { r, sweep, .. } => r * sweep.abs(),
            Self::Circle { r, .. } => r * TAU,
        }
    }

    /// Unit tangent along the parameter direction.
    fn tangent(&self, t: f64) -> DVec2 {
        match *self {
            Self::Line { a, b } => (b - a).normalize_or_zero(),
            Self::Arc { start, sweep, .. } => {
                DVec2::from_angle(start + sweep * t).perp() * sweep.signum()
            }
            Self::Circle { .. } => DVec2::from_angle(TAU * t).perp(),
        }
    }

    /// Signed curvature along the parameter direction; positive turns left.
    fn curvature(&self) -> f64 {
        match *self {
            Self::Line { .. } => 0.0,
            Self::Arc { r, sweep, .. } => sweep.signum() / r,
            Self::Circle { r, .. } => 1.0 / r,
        }
    }

    pub(super) fn bounds(&self) -> (DVec2, DVec2) {
        match *self {
            Self::Line { a, b } => (a.min(b), a.max(b)),
            Self::Circle { c, r } => (c - r, c + r),
            Self::Arc { c, r, start, sweep } => {
                let (a, b) = (self.point(0.0), self.point(1.0));
                let (mut min, mut max) = (a.min(b), a.max(b));
                // Include every axis extreme the arc sweeps past.
                for quadrant in 0..4 {
                    let angle = quadrant as f64 * PI * 0.5;
                    if arc_along(angle, start, sweep) <= sweep.abs() {
                        let p = c + DVec2::from_angle(angle) * r;
                        min = min.min(p);
                        max = max.max(p);
                    }
                }
                (min, max)
            }
        }
    }

    /// Closest parameter on the curve and the distance to it.
    pub(super) fn closest(&self, p: DVec2) -> (f64, f64) {
        match *self {
            Self::Line { a, b } => {
                let d = b - a;
                let t = if d.length_squared() > 0.0 {
                    ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (t, p.distance(a + d * t))
            }
            Self::Circle { c, r } => {
                let offset = p - c;
                (
                    offset.y.atan2(offset.x).rem_euclid(TAU) / TAU,
                    (offset.length() - r).abs(),
                )
            }
            Self::Arc { c, r, start, sweep } => {
                let offset = p - c;
                let along = arc_along(offset.y.atan2(offset.x), start, sweep);
                if along <= sweep.abs() {
                    (
                        along / sweep.abs().max(f64::MIN_POSITIVE),
                        (offset.length() - r).abs(),
                    )
                } else {
                    let (to_start, to_end) =
                        (p.distance(self.point(0.0)), p.distance(self.point(1.0)));
                    if to_start <= to_end {
                        (0.0, to_start)
                    } else {
                        (1.0, to_end)
                    }
                }
            }
        }
    }

    /// Parameter of a point that lies on this curve's carrier (line or circle),
    /// when it falls within the curve's extent allowing `tol` of overshoot.
    fn param_of(&self, p: DVec2, tol: f64) -> Option<f64> {
        match *self {
            Self::Line { a, b } => {
                let d = b - a;
                let length = d.length();
                if length <= 0.0 {
                    return None;
                }
                let t = (p - a).dot(d) / (length * length);
                let slack = tol / length;
                (-slack..=1.0 + slack)
                    .contains(&t)
                    .then_some(t.clamp(0.0, 1.0))
            }
            Self::Circle { c, .. } => {
                let o = p - c;
                Some(o.y.atan2(o.x).rem_euclid(TAU) / TAU)
            }
            Self::Arc { c, r, start, sweep } => {
                let o = p - c;
                let along = arc_along(o.y.atan2(o.x), start, sweep);
                let width = sweep.abs();
                let slack = tol / r;
                if along <= width + slack {
                    Some((along / width).min(1.0))
                } else if along >= TAU - slack {
                    Some(0.0)
                } else {
                    None
                }
            }
        }
    }

    /// The portion between two parameters. For circles `t1` may exceed one to
    /// wrap past the start angle.
    pub(super) fn sub(&self, t0: f64, t1: f64) -> Self {
        match *self {
            Self::Line { .. } => Self::Line {
                a: self.point(t0),
                b: self.point(t1),
            },
            Self::Arc { c, r, start, sweep } => Self::Arc {
                c,
                r,
                start: start + sweep * t0,
                sweep: sweep * (t1 - t0),
            },
            Self::Circle { c, r } => Self::Arc {
                c,
                r,
                start: TAU * t0,
                sweep: TAU * (t1 - t0),
            },
        }
    }

    pub(super) fn to_curve(self) -> SketchCurve {
        match self {
            Self::Line { a, b } => SketchCurve::Line {
                start: a.as_vec2(),
                end: b.as_vec2(),
            },
            Self::Arc { c, r, start, sweep } => SketchCurve::Arc {
                center: c.as_vec2(),
                radius: r as f32,
                start_angle: start.rem_euclid(TAU) as f32,
                sweep: sweep as f32,
            },
            Self::Circle { c, r } => SketchCurve::Circle {
                center: c.as_vec2(),
                radius: r as f32,
            },
        }
    }

    /// Points along the curve from `t = 0` towards `t = 1`, excluding the end.
    fn sample(&self, chord: f64, out: &mut Vec<DVec2>) {
        match *self {
            Self::Line { a, .. } => out.push(a),
            Self::Arc { r, sweep, .. } => {
                let steps = arc_steps(r, sweep.abs(), chord);
                out.extend((0..steps).map(|i| self.point(i as f64 / steps as f64)));
            }
            Self::Circle { r, .. } => {
                let steps = arc_steps(r, TAU, chord).max(8);
                out.extend((0..steps).map(|i| self.point(i as f64 / steps as f64)));
            }
        }
    }
}

/// Distance travelled along an arc's sweep direction from its start angle to
/// `angle`, in radians within 0..TAU.
fn arc_along(angle: f64, start: f64, sweep: f64) -> f64 {
    let relative = (angle - start).rem_euclid(TAU);
    if sweep >= 0.0 {
        relative
    } else {
        (TAU - relative).rem_euclid(TAU)
    }
}

fn arc_steps(radius: f64, sweep: f64, chord: f64) -> usize {
    let chord = chord.max(radius * 1.0e-6).max(1.0e-12);
    let step = 2.0 * (1.0 - (chord / radius).min(1.0)).acos();
    if step > 1.0e-9 {
        ((sweep / step).ceil() as usize).clamp(1, 2048)
    } else {
        2048
    }
}

/// Carrier intersections of two curves as parameter pairs, filtered to both
/// curves' extents. Coincident carriers (collinear lines, concentric circles)
/// report nothing; their overlaps are found through endpoint contacts.
pub(super) fn intersect(a: &Geo, b: &Geo, tol: f64) -> Vec<(f64, f64)> {
    let points: Vec<DVec2> = match (*a, *b) {
        (Geo::Line { a: a0, b: a1 }, Geo::Line { a: b0, b: b1 }) => {
            let (d1, d2) = (a1 - a0, b1 - b0);
            let denominator = d1.perp_dot(d2);
            if denominator.abs() <= 1.0e-12 * d1.length() * d2.length() {
                return Vec::new();
            }
            let t = (b0 - a0).perp_dot(d2) / denominator;
            vec![a0 + d1 * t]
        }
        (Geo::Line { a: p0, b: p1 }, Geo::Arc { c, r, .. } | Geo::Circle { c, r })
        | (Geo::Arc { c, r, .. } | Geo::Circle { c, r }, Geo::Line { a: p0, b: p1 }) => {
            line_circle(p0, p1, c, r, tol)
        }
        (
            Geo::Arc { c: c1, r: r1, .. } | Geo::Circle { c: c1, r: r1 },
            Geo::Arc { c: c2, r: r2, .. } | Geo::Circle { c: c2, r: r2 },
        ) => circle_circle(c1, r1, c2, r2, tol),
    };
    points
        .into_iter()
        .filter(|p| p.is_finite())
        .filter_map(|p| Some((a.param_of(p, tol)?, b.param_of(p, tol)?)))
        .collect()
}

fn line_circle(p0: DVec2, p1: DVec2, c: DVec2, r: f64, tol: f64) -> Vec<DVec2> {
    let d = p1 - p0;
    let length = d.length();
    if length <= 0.0 {
        return Vec::new();
    }
    let direction = d / length;
    let along = (c - p0).dot(direction);
    let foot = p0 + direction * along;
    let h = foot.distance(c);
    if h > r + tol {
        Vec::new()
    } else if (h - r).abs() <= tol {
        vec![foot]
    } else {
        let half = (r * r - h * h).max(0.0).sqrt();
        vec![foot - direction * half, foot + direction * half]
    }
}

fn circle_circle(c1: DVec2, r1: f64, c2: DVec2, r2: f64, tol: f64) -> Vec<DVec2> {
    let d = c1.distance(c2);
    if d <= tol || d > r1 + r2 + tol || d < (r1 - r2).abs() - tol {
        return Vec::new();
    }
    let u = (c2 - c1) / d;
    if (d - (r1 + r2)).abs() <= tol {
        return vec![c1 + u * r1];
    }
    if (d - (r1 - r2).abs()).abs() <= tol {
        return vec![c1 + u * r1 * if r1 >= r2 { 1.0 } else { -1.0 }];
    }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = (r1 * r1 - a * a).max(0.0).sqrt();
    let middle = c1 + u * a;
    vec![middle + u.perp() * h, middle - u.perp() * h]
}

/// Candidate pairs whose bounds overlap (within `tol`), by sweeping along x.
pub(super) fn overlapping_pairs(bounds: &[(DVec2, DVec2)], tol: f64) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..bounds.len()).collect();
    order.sort_by(|&a, &b| bounds[a].0.x.total_cmp(&bounds[b].0.x));
    let mut pairs = Vec::new();
    for (position, &i) in order.iter().enumerate() {
        let (min_i, max_i) = bounds[i];
        for &j in &order[position + 1..] {
            let (min_j, max_j) = bounds[j];
            if min_j.x > max_i.x + tol {
                break;
            }
            if min_j.y <= max_i.y + tol && max_j.y >= min_i.y - tol {
                pairs.push((i.min(j), i.max(j)));
            }
        }
    }
    pairs
}

/// Every contact between two curves: carrier intersections plus endpoints of
/// either curve lying on the other (T-junctions and collinear overlaps).
/// Returns `(t_a, t_b, point)` triples.
pub(super) fn contacts(a: &Geo, b: &Geo, tol: f64) -> Vec<(f64, f64, DVec2)> {
    let mut found: Vec<(f64, f64, DVec2)> = intersect(a, b, tol)
        .into_iter()
        .map(|(ta, tb)| (ta, tb, a.point(ta)))
        .collect();
    for (from, to, swap) in [(a, b, false), (b, a, true)] {
        if from.is_closed() {
            continue;
        }
        for t_from in [0.0, 1.0] {
            let p = from.point(t_from);
            let (t_to, distance) = to.closest(p);
            if distance <= tol {
                found.push(if swap {
                    (t_to, t_from, p)
                } else {
                    (t_from, t_to, p)
                });
            }
        }
    }
    found
}

/// Union-find over point indices.
struct Merge {
    parent: Vec<usize>,
}

impl Merge {
    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[a.max(b)] = a.min(b);
        }
    }
}

/// Merges points closer than `tol` (transitively) with a spatial hash.
fn merge_points(points: &[DVec2], tol: f64) -> Merge {
    let mut merge = Merge {
        parent: (0..points.len()).collect(),
    };
    let cell = tol.max(1.0e-12);
    let key = |p: DVec2| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (index, point) in points.iter().enumerate() {
        let (x, y) = key(*point);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(bucket) = grid.get(&(x + dx, y + dy)) {
                    for &other in bucket {
                        if points[other].distance(*point) <= tol {
                            merge.union(index, other);
                        }
                    }
                }
            }
        }
        grid.entry((x, y)).or_default().push(index);
    }
    merge
}

#[derive(Clone, Copy, Debug)]
struct Piece {
    u: usize,
    v: usize,
    geo: Geo,
    entity: usize,
}

/// The split, merged and deduplicated planar graph.
struct Graph {
    vertices: Vec<DVec2>,
    pieces: Vec<Piece>,
}

fn build_graph(entities: &[SketchEntity], tol: f64) -> Graph {
    let curves: Vec<(usize, Geo)> = entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| !entity.construction && entity.curve.is_valid())
        .map(|(index, entity)| (index, Geo::new(&entity.curve)))
        .collect();
    let mut points: Vec<DVec2> = Vec::new();
    let mut splits: Vec<Vec<(f64, usize)>> = vec![Vec::new(); curves.len()];
    for (index, (_, geo)) in curves.iter().enumerate() {
        if !geo.is_closed() {
            for t in [0.0, 1.0] {
                splits[index].push((t, points.len()));
                points.push(geo.point(t));
            }
        }
    }
    let bounds: Vec<_> = curves.iter().map(|(_, geo)| geo.bounds()).collect();
    for (i, j) in overlapping_pairs(&bounds, tol) {
        for (ti, tj, point) in contacts(&curves[i].1, &curves[j].1, tol) {
            let id = points.len();
            points.push(point);
            splits[i].push((ti, id));
            splits[j].push((tj, id));
        }
    }
    let mut merge = merge_points(&points, tol);
    let mut vertex_of_root: HashMap<usize, usize> = HashMap::new();
    let mut vertices: Vec<DVec2> = Vec::new();
    let mut vertex = |id: usize, merge: &mut Merge, vertices: &mut Vec<DVec2>| {
        let root = merge.find(id);
        *vertex_of_root.entry(root).or_insert_with(|| {
            vertices.push(points[root]);
            vertices.len() - 1
        })
    };
    let mut pieces: Vec<Piece> = Vec::new();
    for (index, (entity, geo)) in curves.iter().enumerate() {
        let mut stops: Vec<(f64, usize)> = splits[index]
            .iter()
            .map(|&(t, id)| (t, vertex(id, &mut merge, &mut vertices)))
            .collect();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        let length = geo.length();
        // Collapse neighbouring stops at the same vertex. A curve that leaves
        // and returns to one vertex keeps both stops; it is split below.
        stops.dedup_by(|next, previous| {
            next.1 == previous.1 && (next.0 - previous.0) * length <= tol * 4.0
        });
        if geo.is_closed() {
            if stops.len() > 1
                && stops[0].1 == stops[stops.len() - 1].1
                && (stops[0].0 + 1.0 - stops[stops.len() - 1].0) * length <= tol * 4.0
            {
                stops.pop();
            }
            let fresh = |t: f64, vertices: &mut Vec<DVec2>| {
                vertices.push(geo.point(t));
                (t, vertices.len() - 1)
            };
            match stops.len() {
                0 => {
                    stops.push(fresh(0.0, &mut vertices));
                    stops.push(fresh(0.5, &mut vertices));
                }
                1 => {
                    let t = (stops[0].0 + 0.5).rem_euclid(1.0);
                    stops.push(fresh(t, &mut vertices));
                    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
                }
                _ => {}
            }
            for k in 0..stops.len() {
                let (t0, u) = stops[k];
                let (t1, v) = if k + 1 < stops.len() {
                    stops[k + 1]
                } else {
                    (stops[0].0 + 1.0, stops[0].1)
                };
                push_piece(
                    &mut pieces,
                    &mut vertices,
                    *entity,
                    geo.sub(t0, t1),
                    u,
                    v,
                    (t1 - t0) * length,
                    tol,
                );
            }
        } else {
            for pair in stops.windows(2) {
                let ((t0, u), (t1, v)) = (pair[0], pair[1]);
                push_piece(
                    &mut pieces,
                    &mut vertices,
                    *entity,
                    geo.sub(t0, t1),
                    u,
                    v,
                    (t1 - t0) * length,
                    tol,
                );
            }
        }
    }
    // Lines are rebuilt from their vertices so loops join exactly.
    for piece in &mut pieces {
        if let Geo::Line { .. } = piece.geo {
            piece.geo = Geo::Line {
                a: vertices[piece.u],
                b: vertices[piece.v],
            };
        }
    }
    dedupe(&mut pieces, tol);
    Graph { vertices, pieces }
}

#[allow(clippy::too_many_arguments)]
fn push_piece(
    pieces: &mut Vec<Piece>,
    vertices: &mut Vec<DVec2>,
    entity: usize,
    geo: Geo,
    u: usize,
    v: usize,
    length: f64,
    tol: f64,
) {
    if length <= tol * 0.5 {
        return;
    }
    if u == v {
        if length <= tol * 4.0 {
            return;
        }
        // A curve that closes on itself: split it so no edge is a self loop.
        vertices.push(geo.point(0.5));
        let middle = vertices.len() - 1;
        pieces.push(Piece {
            u,
            v: middle,
            geo: geo.sub(0.0, 0.5),
            entity,
        });
        pieces.push(Piece {
            u: middle,
            v,
            geo: geo.sub(0.5, 1.0),
            entity,
        });
        return;
    }
    pieces.push(Piece { u, v, geo, entity });
}

/// Drops pieces that retrace an earlier piece between the same vertices.
fn dedupe(pieces: &mut Vec<Piece>, tol: f64) {
    let mut seen: HashMap<(usize, usize), Vec<DVec2>> = HashMap::new();
    pieces.retain(|piece| {
        let key = (piece.u.min(piece.v), piece.u.max(piece.v));
        let middle = piece.geo.point(0.5);
        let entry = seen.entry(key).or_default();
        if entry.iter().any(|m| m.distance(middle) <= tol * 4.0) {
            false
        } else {
            entry.push(middle);
            true
        }
    });
}

/// Removes edges that cannot bound a face: dangling chains, repeatedly.
fn prune(graph: &Graph, live: &mut [bool]) {
    let mut degree = vec![0usize; graph.vertices.len()];
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); graph.vertices.len()];
    for (index, piece) in graph.pieces.iter().enumerate() {
        if live[index] {
            degree[piece.u] += 1;
            degree[piece.v] += 1;
            incident[piece.u].push(index);
            incident[piece.v].push(index);
        }
    }
    let mut queue: Vec<usize> = (0..degree.len()).filter(|&v| degree[v] == 1).collect();
    while let Some(vertex) = queue.pop() {
        for &edge in &incident[vertex] {
            if !live[edge] {
                continue;
            }
            live[edge] = false;
            let piece = graph.pieces[edge];
            for end in [piece.u, piece.v] {
                degree[end] -= 1;
                if degree[end] == 1 {
                    queue.push(end);
                }
            }
        }
    }
}

/// Half-edge `2e` runs along piece `e`, `2e + 1` runs back.
struct Faces {
    /// Half-edges of each face, in boundary order.
    loops: Vec<Vec<usize>>,
    face_of: Vec<usize>,
}

fn half_edge_origin(graph: &Graph, half: usize) -> usize {
    let piece = graph.pieces[half / 2];
    if half.is_multiple_of(2) { piece.u } else { piece.v }
}

fn trace_faces(graph: &Graph, live: &[bool]) -> Faces {
    let half_count = graph.pieces.len() * 2;
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); graph.vertices.len()];
    let mut shortest = vec![f64::INFINITY; graph.vertices.len()];
    for (index, piece) in graph.pieces.iter().enumerate() {
        if live[index] {
            outgoing[piece.u].push(index * 2);
            outgoing[piece.v].push(index * 2 + 1);
            let length = piece.geo.length();
            shortest[piece.u] = shortest[piece.u].min(length);
            shortest[piece.v] = shortest[piece.v].min(length);
        }
    }
    // Order half-edges counter clockwise around each vertex. Tangent ties are
    // broken by curvature, as seen a short distance along each edge.
    let mut position = vec![usize::MAX; half_count];
    for (vertex, edges) in outgoing.iter_mut().enumerate() {
        let probe = shortest[vertex] * 0.1;
        let key = |half: usize| {
            let piece = graph.pieces[half / 2];
            let (direction, curvature) = if half.is_multiple_of(2) {
                (piece.geo.tangent(0.0), piece.geo.curvature())
            } else {
                (-piece.geo.tangent(1.0), -piece.geo.curvature())
            };
            let angle = direction.y.atan2(direction.x) + curvature * probe * 0.5;
            (angle + PI).rem_euclid(TAU) - PI
        };
        edges.sort_by(|&a, &b| key(a).total_cmp(&key(b)).then(a.cmp(&b)));
        for (index, &half) in edges.iter().enumerate() {
            position[half] = index;
        }
    }
    let mut face_of = vec![usize::MAX; half_count];
    let mut loops = Vec::new();
    for start in 0..half_count {
        if !live[start / 2] || face_of[start] != usize::MAX {
            continue;
        }
        let face = loops.len();
        let mut boundary = Vec::new();
        let mut half = start;
        loop {
            face_of[half] = face;
            boundary.push(half);
            let twin = half ^ 1;
            let vertex = half_edge_origin(graph, twin);
            let around = &outgoing[vertex];
            let next = around[(position[twin] + around.len() - 1) % around.len()];
            if next == start || face_of[next] != usize::MAX || boundary.len() > half_count {
                break;
            }
            half = next;
        }
        loops.push(boundary);
    }
    Faces { loops, face_of }
}

/// Minimal bounded faces of the arrangement, as profiles with holes.
pub(super) fn profiles(entities: &[SketchEntity], tolerance: f32) -> Vec<Profile> {
    let tol = tolerance as f64;
    let graph = build_graph(entities, tol);
    if graph.pieces.is_empty() {
        return Vec::new();
    }
    let mut live = vec![true; graph.pieces.len()];
    prune(&graph, &mut live);
    // Bridges have the same face on both sides. Removing them separates
    // islands from the face around them, so they can become holes instead.
    let faces = loop {
        let faces = trace_faces(&graph, &live);
        let mut removed = false;
        for (edge, alive) in live.iter_mut().enumerate() {
            if *alive && faces.face_of[edge * 2] == faces.face_of[edge * 2 + 1] {
                *alive = false;
                removed = true;
            }
        }
        if !removed {
            break faces;
        }
        prune(&graph, &mut live);
    };
    let (min, max) = graph
        .vertices
        .iter()
        .fold((DVec2::INFINITY, DVec2::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(*p), hi.max(*p))
        });
    let chord = ((max - min).max_element() * 2.0e-5).max(tol * 0.25);
    let polygons: Vec<Vec<DVec2>> = faces
        .loops
        .iter()
        .map(|boundary| {
            let mut polygon = Vec::new();
            for &half in boundary {
                let geo = graph.pieces[half / 2].geo;
                if half.is_multiple_of(2) {
                    geo.sample(chord, &mut polygon);
                } else {
                    geo.sub(1.0, 0.0).sample(chord, &mut polygon);
                }
            }
            polygon
        })
        .collect();
    let areas: Vec<f64> = polygons.iter().map(|p| signed_area(p)).collect();
    let minimum_area = tol * tol * 4.0;
    // Connected components, to find which outer boundary belongs to whom.
    let mut components = Merge {
        parent: (0..graph.vertices.len()).collect(),
    };
    for (index, piece) in graph.pieces.iter().enumerate() {
        if live[index] {
            components.union(piece.u, piece.v);
        }
    }
    let component_of_face: Vec<usize> = faces
        .loops
        .iter()
        .map(|boundary| components.find(half_edge_origin(&graph, boundary[0])))
        .collect();
    let bounded: Vec<usize> = (0..faces.loops.len())
        .filter(|&face| areas[face] > minimum_area)
        .collect();
    let face_bounds: Vec<(DVec2, DVec2)> = polygons
        .iter()
        .map(|polygon| {
            polygon
                .iter()
                .fold((DVec2::INFINITY, DVec2::NEG_INFINITY), |(lo, hi), p| {
                    (lo.min(*p), hi.max(*p))
                })
        })
        .collect();
    let mut holes: HashMap<usize, Vec<usize>> = HashMap::new();
    for outer in (0..faces.loops.len()).filter(|&face| areas[face] < -minimum_area) {
        let component = component_of_face[outer];
        let sample = graph.vertices[half_edge_origin(&graph, faces.loops[outer][0])];
        let parent = bounded
            .iter()
            .copied()
            .filter(|&face| component_of_face[face] != component)
            .filter(|&face| {
                let (lo, hi) = face_bounds[face];
                sample.cmpge(lo).all() && sample.cmple(hi).all()
            })
            .filter(|&face| point_in_polygon(sample, &polygons[face]))
            .min_by(|&a, &b| areas[a].total_cmp(&areas[b]));
        if let Some(parent) = parent {
            holes.entry(parent).or_default().push(outer);
        }
    }
    let to_loop = |face: usize| {
        let segments: Vec<(SketchCurve, usize)> = faces.loops[face]
            .iter()
            .map(|&half| {
                let piece = graph.pieces[half / 2];
                let geo = if half.is_multiple_of(2) {
                    piece.geo
                } else {
                    piece.geo.sub(1.0, 0.0)
                };
                (geo.to_curve(), piece.entity)
            })
            .collect();
        coalesce(segments)
    };
    let mut profiles: Vec<(f64, Profile)> = bounded
        .iter()
        .map(|&face| {
            let hole_faces = holes.remove(&face).unwrap_or_default();
            let area = areas[face] + hole_faces.iter().map(|&h| areas[h]).sum::<f64>();
            (
                area,
                Profile {
                    outer: to_loop(face),
                    holes: hole_faces.into_iter().map(to_loop).collect(),
                },
            )
        })
        .collect();
    profiles.sort_by(|a, b| b.0.total_cmp(&a.0));
    profiles.into_iter().map(|(_, profile)| profile).collect()
}

/// Merges consecutive pieces of the same source curve back into one segment,
/// so split points that are not corners of this face disappear.
fn coalesce(segments: Vec<(SketchCurve, usize)>) -> ProfileLoop {
    let count = segments.len();
    let mut result = ProfileLoop {
        segments: Vec::with_capacity(count),
        entities: Vec::with_capacity(count),
    };
    if count == 0 {
        return result;
    }
    // Start at a boundary between different curves so runs never wrap.
    let offset = (0..count)
        .find(|&i| segments[i].1 != segments[(i + count - 1) % count].1)
        .unwrap_or(0);
    let mut index = 0;
    while index < count {
        let (first, entity) = segments[(offset + index) % count];
        let mut run = vec![first];
        index += 1;
        while index < count && segments[(offset + index) % count].1 == entity {
            run.push(segments[(offset + index) % count].0);
            index += 1;
        }
        let whole_loop = run.len() == count;
        for curve in merge_run(&run, whole_loop) {
            result.segments.push(curve);
            result.entities.push(entity);
        }
    }
    result
}

fn merge_run(run: &[SketchCurve], whole_loop: bool) -> Vec<SketchCurve> {
    if run.len() == 1 {
        return run.to_vec();
    }
    match (run[0], run[run.len() - 1]) {
        (SketchCurve::Line { start, .. }, SketchCurve::Line { end, .. }) => {
            vec![SketchCurve::Line { start, end }]
        }
        (
            SketchCurve::Arc {
                center,
                radius,
                start_angle,
                ..
            },
            _,
        ) => {
            let sweep: f64 = run
                .iter()
                .map(|curve| match curve {
                    SketchCurve::Arc { sweep, .. } => *sweep as f64,
                    _ => 0.0,
                })
                .sum();
            if sweep.abs() >= TAU - 1.0e-4 {
                if whole_loop && sweep > 0.0 {
                    vec![SketchCurve::Circle { center, radius }]
                } else {
                    // A circle has no direction or start: a clockwise loop, or
                    // one that touches other curves, uses two halves that
                    // begin where the loop meets the circle.
                    let half = std::f32::consts::PI * sweep.signum() as f32;
                    [0.0, half]
                        .into_iter()
                        .map(|offset| SketchCurve::Arc {
                            center,
                            radius,
                            start_angle: (start_angle + offset).rem_euclid(std::f32::consts::TAU),
                            sweep: half,
                        })
                        .collect()
                }
            } else {
                vec![SketchCurve::Arc {
                    center,
                    radius,
                    start_angle,
                    sweep: sweep as f32,
                }]
            }
        }
        _ => run.to_vec(),
    }
}

fn signed_area(points: &[DVec2]) -> f64 {
    let mut area = 0.0;
    for (i, a) in points.iter().enumerate() {
        area += a.perp_dot(points[(i + 1) % points.len()]);
    }
    area * 0.5
}

fn point_in_polygon(point: DVec2, polygon: &[DVec2]) -> bool {
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

/// Parameters on `target` where any other curve meets it, including the
/// target's own endpoints, sorted and merged within `tol`.
pub(super) fn split_parameters(
    entities: &[SketchEntity],
    target: usize,
    tol: f64,
) -> Option<(Geo, Vec<f64>)> {
    let entity = entities.get(target)?;
    if !entity.curve.is_valid() {
        return None;
    }
    let geo = Geo::new(&entity.curve);
    let (lo, hi) = geo.bounds();
    let mut params: Vec<f64> = if geo.is_closed() {
        Vec::new()
    } else {
        vec![0.0, 1.0]
    };
    for (index, other) in entities.iter().enumerate() {
        if index == target || !other.curve.is_valid() {
            continue;
        }
        let other = Geo::new(&other.curve);
        let (olo, ohi) = other.bounds();
        if olo.x > hi.x + tol || ohi.x < lo.x - tol || olo.y > hi.y + tol || ohi.y < lo.y - tol {
            continue;
        }
        params.extend(contacts(&geo, &other, tol).into_iter().map(|(t, _, _)| t));
    }
    params.sort_by(f64::total_cmp);
    let length = geo.length().max(f64::MIN_POSITIVE);
    params.dedup_by(|next, previous| (*next - *previous) * length <= tol);
    if geo.is_closed()
        && params.len() > 1
        && (params[0] + 1.0 - params[params.len() - 1]) * length <= tol
    {
        params.pop();
    }
    Some((geo, params))
}

/// The intersection points between every pair of curves (crossings and
/// tangencies, not shared endpoints), merged within `tol`.
pub(super) fn crossing_points(entities: &[SketchEntity], tol: f64) -> Vec<Vec2> {
    let curves: Vec<Geo> = entities
        .iter()
        .filter(|entity| entity.curve.is_valid())
        .map(|entity| Geo::new(&entity.curve))
        .collect();
    let bounds: Vec<_> = curves.iter().map(Geo::bounds).collect();
    let mut points: Vec<DVec2> = Vec::new();
    for (i, j) in overlapping_pairs(&bounds, tol) {
        for (ti, tj) in intersect(&curves[i], &curves[j], tol) {
            let at_end = |geo: &Geo, t: f64| !geo.is_closed() && (t <= 0.0 || t >= 1.0);
            let p = curves[i].point(ti);
            // Shared endpoints are already endpoint snaps.
            if at_end(&curves[i], ti) && at_end(&curves[j], tj) {
                continue;
            }
            if !points.iter().any(|q| q.distance(p) <= tol) {
                points.push(p);
            }
        }
    }
    points.into_iter().map(|p| p.as_vec2()).collect()
}

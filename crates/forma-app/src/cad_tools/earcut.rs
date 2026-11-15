//! Polygon triangulation with holes: a port of mapbox/earcut's algorithm
//! (ISC licence) without its z-order hashing.
//!
//! Holes are joined to the outer ring left-most hole first through a bridge
//! from the hole's left-most vertex to a mutually visible ring vertex. When
//! earlier bridges have duplicated that vertex, the copy whose wedge
//! contains the bridge is chosen (`locally_inside`, `sector_contains_sector`),
//! so bridges never cross. Ears are then clipped; when none remain the ring
//! is cleaned, local self-intersections cured, and finally split along a
//! valid diagonal. Nothing is ever fanned blindly: a piece that cannot be
//! triangulated is dropped rather than covered with overlapping triangles.
use glam::{DVec2, Vec2};

struct Node {
    /// Index into the caller's point list.
    i: u32,
    p: DVec2,
    prev: usize,
    next: usize,
    steiner: bool,
}

struct Earcut {
    nodes: Vec<Node>,
    triangles: Vec<[u32; 3]>,
    /// Ear-clipping steps left before giving up on degenerate input.
    budget: usize,
}

/// Twice the signed area of triangle `p q r` in earcut's convention:
/// negative for a left (counter-clockwise) turn.
fn area(p: DVec2, q: DVec2, r: DVec2) -> f64 {
    (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)
}

fn point_in_triangle(a: DVec2, b: DVec2, c: DVec2, p: DVec2) -> bool {
    (c.x - p.x) * (a.y - p.y) >= (a.x - p.x) * (c.y - p.y)
        && (a.x - p.x) * (b.y - p.y) >= (b.x - p.x) * (a.y - p.y)
        && (b.x - p.x) * (c.y - p.y) >= (c.x - p.x) * (b.y - p.y)
}

/// [`point_in_triangle`] that does not count a point equal to `a`.
fn point_in_triangle_except_first(a: DVec2, b: DVec2, c: DVec2, p: DVec2) -> bool {
    a != p && point_in_triangle(a, b, c, p)
}

fn sign(value: f64) -> i32 {
    if value > 0.0 {
        1
    } else if value < 0.0 {
        -1
    } else {
        0
    }
}

fn on_segment(p: DVec2, q: DVec2, r: DVec2) -> bool {
    q.x <= p.x.max(r.x) && q.x >= p.x.min(r.x) && q.y <= p.y.max(r.y) && q.y >= p.y.min(r.y)
}

fn intersects(p1: DVec2, q1: DVec2, p2: DVec2, q2: DVec2) -> bool {
    let o1 = sign(area(p1, q1, p2));
    let o2 = sign(area(p1, q1, q2));
    let o3 = sign(area(p2, q2, p1));
    let o4 = sign(area(p2, q2, q1));
    (o1 != o2 && o3 != o4)
        || (o1 == 0 && on_segment(p1, p2, q1))
        || (o2 == 0 && on_segment(p1, q2, q1))
        || (o3 == 0 && on_segment(p2, p1, q2))
        || (o4 == 0 && on_segment(p2, q1, q2))
}

impl Earcut {
    fn p(&self, n: usize) -> DVec2 {
        self.nodes[n].p
    }
    fn next(&self, n: usize) -> usize {
        self.nodes[n].next
    }
    fn prev(&self, n: usize) -> usize {
        self.nodes[n].prev
    }

    fn insert(&mut self, i: u32, p: DVec2, last: Option<usize>) -> usize {
        let n = self.nodes.len();
        self.nodes.push(Node {
            i,
            p,
            prev: n,
            next: n,
            steiner: false,
        });
        if let Some(last) = last {
            let after = self.next(last);
            self.nodes[n].next = after;
            self.nodes[n].prev = last;
            self.nodes[after].prev = n;
            self.nodes[last].next = n;
        }
        n
    }

    fn remove(&mut self, n: usize) {
        let (prev, next) = (self.prev(n), self.next(n));
        self.nodes[next].prev = prev;
        self.nodes[prev].next = next;
    }

    /// A circular list of `points`, wound counter-clockwise (y up) for the
    /// outer ring and clockwise for holes.
    fn linked_list(&mut self, points: &[(u32, DVec2)], outer: bool) -> Option<usize> {
        let mut doubled = 0.0;
        for k in 0..points.len() {
            let (a, b) = (points[(k + points.len() - 1) % points.len()].1, points[k].1);
            doubled += (a.x - b.x) * (b.y + a.y);
        }
        let mut last = None;
        if outer == (doubled > 0.0) {
            for &(i, p) in points {
                last = Some(self.insert(i, p, last));
            }
        } else {
            for &(i, p) in points.iter().rev() {
                last = Some(self.insert(i, p, last));
            }
        }
        if let Some(l) = last
            && self.p(l) == self.p(self.next(l))
        {
            self.remove(l);
            last = Some(self.next(l));
        }
        last
    }

    /// Drops duplicate and collinear points between `start` and `end`.
    fn filter_points(&mut self, start: usize, end: Option<usize>) -> usize {
        let mut end = end.unwrap_or(start);
        let mut p = start;
        loop {
            let mut again = false;
            let (prev, next) = (self.prev(p), self.next(p));
            if !self.nodes[p].steiner
                && (self.p(p) == self.p(next) || area(self.p(prev), self.p(p), self.p(next)) == 0.0)
            {
                self.remove(p);
                p = self.prev(p);
                end = p;
                if p == self.next(p) {
                    break;
                }
                again = true;
            } else {
                p = next;
            }
            if !(again || p != end) {
                break;
            }
        }
        end
    }

    fn is_ear(&self, ear: usize) -> bool {
        let (a, c) = (self.prev(ear), self.next(ear));
        let (pa, pb, pc) = (self.p(a), self.p(ear), self.p(c));
        if area(pa, pb, pc) >= 0.0 {
            return false;
        }
        let low = pa.min(pb).min(pc);
        let high = pa.max(pb).max(pc);
        let mut p = self.next(c);
        while p != a {
            let q = self.p(p);
            if q.x >= low.x
                && q.x <= high.x
                && q.y >= low.y
                && q.y <= high.y
                && point_in_triangle_except_first(pa, pb, pc, q)
                && area(self.p(self.prev(p)), q, self.p(self.next(p))) >= 0.0
            {
                return false;
            }
            p = self.next(p);
        }
        true
    }

    fn emit(&mut self, a: usize, b: usize, c: usize) {
        self.triangles
            .push([self.nodes[a].i, self.nodes[b].i, self.nodes[c].i]);
    }

    fn earcut_linked(&mut self, ear: Option<usize>, pass: u8) {
        let Some(mut ear) = ear else {
            return;
        };
        let mut stop = ear;
        while self.prev(ear) != self.next(ear) {
            if self.budget == 0 {
                return;
            }
            self.budget -= 1;
            let (prev, next) = (self.prev(ear), self.next(ear));
            if self.is_ear(ear) {
                self.emit(prev, ear, next);
                self.remove(ear);
                ear = self.next(next);
                stop = ear;
                continue;
            }
            ear = next;
            if ear == stop {
                match pass {
                    0 => {
                        let filtered = self.filter_points(ear, None);
                        self.earcut_linked(Some(filtered), 1);
                    }
                    1 => {
                        let filtered = self.filter_points(ear, None);
                        let cured = self.cure_local_intersections(filtered);
                        self.earcut_linked(Some(cured), 2);
                    }
                    _ => self.split_earcut(ear),
                }
                return;
            }
        }
    }

    fn cure_local_intersections(&mut self, start: usize) -> usize {
        let mut start = start;
        let mut p = start;
        loop {
            let a = self.prev(p);
            let b = self.next(self.next(p));
            if self.p(a) != self.p(b)
                && intersects(self.p(a), self.p(p), self.p(self.next(p)), self.p(b))
                && self.locally_inside(a, b)
                && self.locally_inside(b, a)
            {
                self.emit(a, p, b);
                let after = self.next(p);
                self.remove(p);
                self.remove(after);
                p = b;
                start = b;
            }
            p = self.next(p);
            if p == start {
                break;
            }
        }
        self.filter_points(p, None)
    }

    fn split_earcut(&mut self, start: usize) {
        let mut a = start;
        loop {
            let mut b = self.next(self.next(a));
            while b != self.prev(a) {
                if self.nodes[a].i != self.nodes[b].i && self.is_valid_diagonal(a, b) {
                    let c = self.split_polygon(a, b);
                    let a = self.filter_points(a, Some(self.next(a)));
                    let c = self.filter_points(c, Some(self.next(c)));
                    self.earcut_linked(Some(a), 0);
                    self.earcut_linked(Some(c), 0);
                    return;
                }
                b = self.next(b);
            }
            a = self.next(a);
            if a == start {
                // No valid diagonal: leave this piece untriangulated.
                return;
            }
        }
    }

    fn eliminate_holes(&mut self, holes: Vec<usize>, mut outer: usize) -> usize {
        let mut queue: Vec<usize> = holes
            .into_iter()
            .map(|list| {
                if list == self.next(list) {
                    self.nodes[list].steiner = true;
                }
                self.leftmost(list)
            })
            .collect();
        queue.sort_by(|&a, &b| {
            let (pa, pb) = (self.p(a), self.p(b));
            pa.x.total_cmp(&pb.x)
                .then(pa.y.total_cmp(&pb.y))
                .then_with(|| {
                    // Same left-most point: the hole whose first edge turns
                    // less steeply goes first, as in earcut 3.
                    let sa = (self.p(self.next(a)).y - pa.y) / (self.p(self.next(a)).x - pa.x);
                    let sb = (self.p(self.next(b)).y - pb.y) / (self.p(self.next(b)).x - pb.x);
                    sa.total_cmp(&sb)
                })
        });
        for hole in queue {
            outer = self.eliminate_hole(hole, outer);
        }
        outer
    }

    fn eliminate_hole(&mut self, hole: usize, outer: usize) -> usize {
        let Some(bridge) = self.find_hole_bridge(hole, outer) else {
            return outer;
        };
        let reverse = self.split_polygon(bridge, hole);
        self.filter_points(reverse, Some(self.next(reverse)));
        self.filter_points(bridge, Some(self.next(bridge)))
    }

    /// The ring vertex a hole's left-most vertex can connect to.
    fn find_hole_bridge(&self, hole: usize, outer: usize) -> Option<usize> {
        let h = self.p(hole);
        let mut qx = f64::NEG_INFINITY;
        let mut m = None;
        let mut p = outer;
        loop {
            let (a, b) = (self.p(p), self.p(self.next(p)));
            if h.y <= a.y && h.y >= b.y && b.y != a.y {
                let x = a.x + (h.y - a.y) * (b.x - a.x) / (b.y - a.y);
                if x <= h.x && x > qx {
                    qx = x;
                    m = Some(if a.x < b.x { p } else { self.next(p) });
                    if x == h.x {
                        // The hole touches the ring: bridge to the touch point.
                        return m;
                    }
                }
            }
            p = self.next(p);
            if p == outer {
                break;
            }
        }
        let mut m = m?;
        // Ring vertices inside the triangle (hole point, ray hit, m) block
        // the view; the one at the smallest angle to the ray is visible.
        let stop = m;
        let pm = self.p(m);
        let mut tan_min = f64::INFINITY;
        let mut p = m;
        loop {
            let q = self.p(p);
            if h.x >= q.x
                && q.x >= pm.x
                && h.x != q.x
                && point_in_triangle(
                    DVec2::new(if h.y < pm.y { h.x } else { qx }, h.y),
                    pm,
                    DVec2::new(if h.y < pm.y { qx } else { h.x }, h.y),
                    q,
                )
            {
                let tan = (h.y - q.y).abs() / (h.x - q.x);
                let current = self.p(m);
                if self.locally_inside(p, hole)
                    && (tan < tan_min
                        || (tan == tan_min
                            && (q.x > current.x
                                || (q.x == current.x && self.sector_contains_sector(m, p)))))
                {
                    m = p;
                    tan_min = tan;
                }
            }
            p = self.next(p);
            if p == stop {
                break;
            }
        }
        Some(m)
    }

    /// Whether `m`'s wedge contains `p`'s (for choosing between duplicates
    /// of one vertex made by earlier bridges).
    fn sector_contains_sector(&self, m: usize, p: usize) -> bool {
        area(self.p(self.prev(m)), self.p(m), self.p(self.prev(p))) < 0.0
            && area(self.p(self.next(p)), self.p(m), self.p(self.next(m))) < 0.0
    }

    fn leftmost(&self, start: usize) -> usize {
        let mut p = start;
        let mut best = start;
        loop {
            let (q, b) = (self.p(p), self.p(best));
            if q.x < b.x || (q.x == b.x && q.y < b.y) {
                best = p;
            }
            p = self.next(p);
            if p == start {
                break;
            }
        }
        best
    }

    fn is_valid_diagonal(&self, a: usize, b: usize) -> bool {
        let (pa, pb) = (self.p(a), self.p(b));
        let (ai, bi) = (self.nodes[a].i, self.nodes[b].i);
        self.nodes[self.next(a)].i != bi
            && self.nodes[self.prev(a)].i != bi
            && !self.intersects_polygon(a, b)
            && ((self.locally_inside(a, b)
                && self.locally_inside(b, a)
                && self.middle_inside(a, b)
                && (area(self.p(self.prev(a)), pa, self.p(self.prev(b))) != 0.0
                    || area(pa, self.p(self.prev(b)), pb) != 0.0))
                || (pa == pb
                    && area(self.p(self.prev(a)), pa, self.p(self.next(a))) > 0.0
                    && area(self.p(self.prev(b)), pb, self.p(self.next(b))) > 0.0))
            && ai != bi
    }

    fn intersects_polygon(&self, a: usize, b: usize) -> bool {
        let (ai, bi) = (self.nodes[a].i, self.nodes[b].i);
        let mut p = a;
        loop {
            let next = self.next(p);
            if self.nodes[p].i != ai
                && self.nodes[next].i != ai
                && self.nodes[p].i != bi
                && self.nodes[next].i != bi
                && intersects(self.p(p), self.p(next), self.p(a), self.p(b))
            {
                return true;
            }
            p = next;
            if p == a {
                return false;
            }
        }
    }

    fn locally_inside(&self, a: usize, b: usize) -> bool {
        let (pa, pb) = (self.p(a), self.p(b));
        let (prev, next) = (self.p(self.prev(a)), self.p(self.next(a)));
        if area(prev, pa, next) < 0.0 {
            area(pa, pb, next) >= 0.0 && area(pa, prev, pb) >= 0.0
        } else {
            area(pa, pb, prev) < 0.0 || area(pa, next, pb) < 0.0
        }
    }

    fn middle_inside(&self, a: usize, b: usize) -> bool {
        let middle = (self.p(a) + self.p(b)) * 0.5;
        let mut inside = false;
        let mut p = a;
        loop {
            let (q, r) = (self.p(p), self.p(self.next(p)));
            if (q.y > middle.y) != (r.y > middle.y)
                && r.y != q.y
                && middle.x < (r.x - q.x) * (middle.y - q.y) / (r.y - q.y) + q.x
            {
                inside = !inside;
            }
            p = self.next(p);
            if p == a {
                return inside;
            }
        }
    }

    /// Splits the ring along `a`–`b`, duplicating both; returns the copy of `b`.
    fn split_polygon(&mut self, a: usize, b: usize) -> usize {
        let (an, bp) = (self.next(a), self.prev(b));
        let a2 = self.nodes.len();
        self.nodes.push(Node {
            i: self.nodes[a].i,
            p: self.p(a),
            prev: a2,
            next: a2,
            steiner: false,
        });
        let b2 = self.nodes.len();
        self.nodes.push(Node {
            i: self.nodes[b].i,
            p: self.p(b),
            prev: b2,
            next: b2,
            steiner: false,
        });
        self.nodes[a].next = b;
        self.nodes[b].prev = a;
        self.nodes[a2].next = an;
        self.nodes[an].prev = a2;
        self.nodes[b2].next = a2;
        self.nodes[a2].prev = b2;
        self.nodes[bp].next = b2;
        self.nodes[b2].prev = bp;
        b2
    }
}

/// Triangles (indices into `outer` followed by each hole's points) covering
/// the region inside `outer` and outside `holes`, wound counter-clockwise
/// (y up). Either winding is accepted for every loop. Non-finite or
/// degenerate input yields fewer (never overlapping) triangles.
pub(crate) fn triangulate(outer: &[Vec2], holes: &[Vec<Vec2>]) -> Vec<[u32; 3]> {
    let finite = |loop_: &[Vec2]| loop_.iter().all(|p| p.is_finite());
    if outer.len() < 3 || !finite(outer) {
        return Vec::new();
    }
    let total = outer.len() + holes.iter().map(Vec::len).sum::<usize>();
    let mut earcut = Earcut {
        nodes: Vec::with_capacity(total * 2),
        triangles: Vec::with_capacity(total),
        budget: total * total + 64,
    };
    let points: Vec<(u32, DVec2)> = outer
        .iter()
        .enumerate()
        .map(|(i, p)| (i as u32, p.as_dvec2()))
        .collect();
    let Some(mut start) = earcut.linked_list(&points, true) else {
        return Vec::new();
    };
    if earcut.next(start) == earcut.prev(start) {
        return Vec::new();
    }
    let mut offset = outer.len() as u32;
    let mut lists = Vec::new();
    for hole in holes {
        let indexed: Vec<(u32, DVec2)> = hole
            .iter()
            .enumerate()
            .map(|(k, p)| (offset + k as u32, p.as_dvec2()))
            .collect();
        offset += hole.len() as u32;
        if hole.len() >= 3
            && finite(hole)
            && let Some(list) = earcut.linked_list(&indexed, false)
        {
            lists.push(list);
        }
    }
    if !lists.is_empty() {
        start = earcut.eliminate_holes(lists, start);
    }
    earcut.earcut_linked(Some(start), 0);
    earcut.triangles
}

//! Kernel-free geometry behind the direct-modelling tools: dragging along an
//! axis, adaptive snapping, ghost prisms (with holes) and lathes, B-rep face
//! groups and edges of part tessellations, and Join/Cut inference.
//!
//! Everything here is pure and cheap enough to run per pointer move; the
//! kernel only sees the final feature.
use forma_core::{FeatureOperation, Mesh, Ray, SketchPlane};
use glam::{Mat4, Vec2, Vec3};
use std::collections::{BTreeMap, HashMap};

// ---------------------------------------------------------------------------
// Axis dragging and snapping

/// Parameter `t` of the point on the axis `origin + t * direction` closest to
/// `ray`, the standard closest-points-between-lines solution. `None` when the
/// ray runs (nearly) along the axis, where the closest point is undefined and
/// tiny pointer moves would fling the value.
pub(crate) fn axis_parameter(origin: Vec3, direction: Vec3, ray: Ray) -> Option<f32> {
    let d = direction.try_normalize()?;
    let r = ray.direction.try_normalize()?;
    let w = origin - ray.origin;
    let b = d.dot(r);
    let denominator = 1.0 - b * b;
    if denominator < 1.0e-4 {
        return None;
    }
    let t = (b * r.dot(w) - d.dot(w)) / denominator;
    (t.is_finite()).then(|| t / direction.length())
}

/// Snap increment for a distance shown at `pixels_per_unit`: the Draw grid's
/// 1–2–5 minor step, so drags snap to values that read cleanly at any zoom.
pub(crate) fn snap_step(pixels_per_unit: f32) -> f32 {
    crate::sketch::snap::grid_steps(pixels_per_unit).0
}

/// Snaps `value` to the nearest multiple of `step`, or to one of `targets`
/// (such as the height of another face) when one lies within `reach`.
/// Returns the value and the target index it snapped to.
pub(crate) fn snap_value(
    value: f32,
    step: f32,
    targets: &[f32],
    reach: f32,
) -> (f32, Option<usize>) {
    let nearest = targets
        .iter()
        .enumerate()
        .map(|(i, t)| (i, (t - value).abs()))
        .filter(|(_, d)| *d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((index, _)) = nearest {
        return (targets[index], Some(index));
    }
    if step > 0.0 && step.is_finite() {
        ((value / step).round() * step, None)
    } else {
        (value, None)
    }
}

/// Snaps an angle to `step` radians (15° in the revolve tool).
pub(crate) fn snap_angle(angle: f32, step: f32) -> f32 {
    (angle / step).round() * step
}

// ---------------------------------------------------------------------------
// Polygons and triangulation

/// Twice the signed area: positive for counter-clockwise loops.
pub(crate) fn signed_area(points: &[Vec2]) -> f32 {
    let mut sum = 0.0;
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        sum += a.perp_dot(b);
    }
    sum * 0.5
}

fn cross(o: Vec2, a: Vec2, b: Vec2) -> f32 {
    (a - o).perp_dot(b - o)
}

fn inside_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let (d0, d1, d2) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    let tolerance = -1.0e-12;
    d0 >= tolerance && d1 >= tolerance && d2 >= tolerance
}

/// Triangulates a region: a counter-clockwise outer loop with clockwise holes
/// (either winding is accepted and corrected). Holes are bridged into the
/// outer loop (Eberly's method), then ears are clipped. Returns triangles as
/// indices into the outer points followed by each hole's points, all wound
/// counter-clockwise.
pub(crate) fn triangulate(outer: &[Vec2], holes: &[Vec<Vec2>]) -> Vec<[u32; 3]> {
    let mut points: Vec<Vec2> = outer.to_vec();
    let mut ring: Vec<u32> = (0..outer.len() as u32).collect();
    if outer.len() < 3 {
        return Vec::new();
    }
    if signed_area(outer) < 0.0 {
        ring.reverse();
    }
    // Holes clockwise, merged right-most first so bridges never cross.
    let mut hole_rings: Vec<Vec<u32>> = Vec::new();
    for hole in holes.iter().filter(|h| h.len() >= 3) {
        let start = points.len() as u32;
        points.extend_from_slice(hole);
        let mut indices: Vec<u32> = (start..start + hole.len() as u32).collect();
        if signed_area(hole) > 0.0 {
            indices.reverse();
        }
        hole_rings.push(indices);
    }
    hole_rings.sort_by(|a, b| {
        let max = |r: &Vec<u32>| {
            r.iter()
                .map(|&i| points[i as usize].x)
                .fold(f32::NEG_INFINITY, f32::max)
        };
        max(b).total_cmp(&max(a))
    });
    for hole in hole_rings {
        bridge_hole(&points, &mut ring, &hole);
    }
    ear_clip(&points, ring)
}

/// Splices `hole` into `ring` through a mutually visible vertex pair.
fn bridge_hole(points: &[Vec2], ring: &mut Vec<u32>, hole: &[u32]) {
    let p = |i: u32| points[i as usize];
    // The hole's right-most vertex.
    let (m_index, &m) = hole
        .iter()
        .enumerate()
        .max_by(|a, b| p(*a.1).x.total_cmp(&p(*b.1).x))
        .unwrap();
    let mp = p(m);
    // Nearest ring edge hit by a ray from M toward +x.
    let mut best: Option<(f32, usize)> = None;
    for k in 0..ring.len() {
        let (a, b) = (p(ring[k]), p(ring[(k + 1) % ring.len()]));
        if (a.y > mp.y) == (b.y > mp.y) {
            continue;
        }
        let x = a.x + (mp.y - a.y) * (b.x - a.x) / (b.y - a.y);
        if x >= mp.x - 1.0e-9 && best.is_none_or(|(bx, _)| x < bx) {
            best = Some((x, k));
        }
    }
    let Some((hit_x, edge)) = best else {
        // Degenerate input: bridge to the nearest ring vertex.
        let nearest = (0..ring.len())
            .min_by(|&a, &b| p(ring[a]).distance(mp).total_cmp(&p(ring[b]).distance(mp)))
            .unwrap();
        splice(ring, nearest, hole, m_index);
        return;
    };
    let hit = Vec2::new(hit_x, mp.y);
    let (a, b) = (edge, (edge + 1) % ring.len());
    let mut candidate = if p(ring[a]).x > p(ring[b]).x { a } else { b };
    // A reflex ring vertex inside triangle (M, hit, candidate) blocks the
    // view; the one making the smallest angle with the ray is visible.
    let c = p(ring[candidate]);
    let (t0, t1, t2) = if cross(mp, hit, c) >= 0.0 {
        (mp, hit, c)
    } else {
        (mp, c, hit)
    };
    let mut best_angle = f32::INFINITY;
    for k in 0..ring.len() {
        if k == candidate {
            continue;
        }
        let v = p(ring[k]);
        let previous = p(ring[(k + ring.len() - 1) % ring.len()]);
        let next = p(ring[(k + 1) % ring.len()]);
        let reflex = cross(previous, v, next) <= 0.0;
        if reflex && v != mp && inside_triangle(v, t0, t1, t2) {
            let offset = v - mp;
            let angle = offset.y.abs().atan2(offset.x);
            if angle < best_angle {
                best_angle = angle;
                candidate = k;
            }
        }
    }
    splice(ring, candidate, hole, m_index);
}

fn splice(ring: &mut Vec<u32>, at: usize, hole: &[u32], start: usize) {
    let mut merged = Vec::with_capacity(ring.len() + hole.len() + 2);
    merged.extend_from_slice(&ring[..=at]);
    for k in 0..=hole.len() {
        merged.push(hole[(start + k) % hole.len()]);
    }
    merged.extend_from_slice(&ring[at..]);
    *ring = merged;
}

fn ear_clip(points: &[Vec2], mut ring: Vec<u32>) -> Vec<[u32; 3]> {
    let p = |i: u32| points[i as usize];
    let mut triangles = Vec::with_capacity(ring.len().saturating_sub(2));
    let mut guard = 0;
    while ring.len() > 3 && guard < ring.len() * ring.len() + 64 {
        guard += 1;
        let n = ring.len();
        let mut clipped = false;
        for k in 0..n {
            let (ia, ib, ic) = (ring[(k + n - 1) % n], ring[k], ring[(k + 1) % n]);
            let (a, b, c) = (p(ia), p(ib), p(ic));
            let area = cross(a, b, c);
            if area <= 1.0e-14 {
                // Collinear spikes from bridges: drop them without a triangle.
                if area.abs() <= 1.0e-14 && (a == c || (b - a).dot(c - b) < 0.0) {
                    ring.remove(k);
                    clipped = true;
                    break;
                }
                continue;
            }
            let blocked = ring.iter().any(|&j| {
                let q = p(j);
                q != a && q != b && q != c && inside_triangle(q, a, b, c)
            });
            if !blocked {
                triangles.push([ia, ib, ic]);
                ring.remove(k);
                clipped = true;
                break;
            }
        }
        if !clipped {
            // Self-touching input: fan what remains rather than looping.
            break;
        }
    }
    if ring.len() >= 3 {
        for k in 1..ring.len() - 1 {
            let (a, b, c) = (ring[0], ring[k], ring[k + 1]);
            if cross(p(a), p(b), p(c)) > 1.0e-14 {
                triangles.push([a, b, c]);
            }
        }
    }
    triangles
}

// ---------------------------------------------------------------------------
// Ghost geometry

/// A planar shape swept along its normal: the cached, distance-independent
/// part of an extrusion or push/pull preview. Points lie on the base plane
/// in world space; the paint step offsets them by the current distance.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Prism {
    pub points: Vec<Vec3>,
    /// Cap triangles over `points`, counter-clockwise about `normal`.
    pub triangles: Vec<[u32; 3]>,
    /// Boundary loops over `points` (outer loops counter-clockwise).
    pub loops: Vec<Vec<u32>>,
    /// Unit sweep direction in world space.
    pub normal: Vec3,
}

impl Prism {
    /// A prism over sketch regions: each region is an outer loop plus holes
    /// in plane coordinates.
    pub(crate) fn from_regions(plane: &SketchPlane, matrix: Mat4, regions: &[Vec<Vec<Vec2>>]) -> Self {
        let mut prism = Prism {
            normal: matrix
                .transform_vector3(plane.normal())
                .normalize_or_zero(),
            ..Default::default()
        };
        for region in regions {
            let Some((outer, holes)) = region.split_first() else {
                continue;
            };
            if outer.len() < 3 {
                continue;
            }
            let base = prism.points.len() as u32;
            let mut offset = base;
            for polygon in std::iter::once(outer).chain(holes) {
                prism.loops.push((offset..offset + polygon.len() as u32).collect());
                prism.points.extend(
                    polygon
                        .iter()
                        .map(|p| matrix.transform_point3(plane.to_world(*p))),
                );
                offset += polygon.len() as u32;
            }
            prism.triangles.extend(
                triangulate(outer, holes)
                    .into_iter()
                    .map(|t| t.map(|i| i + base)),
            );
        }
        prism.orient();
        prism
    }

    /// Makes caps wind counter-clockwise about `normal` (mirrored transforms
    /// flip them), so face culling at paint time is reliable.
    fn orient(&mut self) {
        let mut area = Vec3::ZERO;
        for t in &self.triangles {
            let [a, b, c] = t.map(|i| self.points[i as usize]);
            area += (b - a).cross(c - a);
        }
        if area.dot(self.normal) < 0.0 {
            for t in &mut self.triangles {
                t.swap(1, 2);
            }
            for l in &mut self.loops {
                l.reverse();
            }
        }
    }

    /// Area of the caps.
    pub(crate) fn area(&self) -> f32 {
        self.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.points[i as usize]);
                (b - a).cross(c - a).length() * 0.5
            })
            .sum()
    }

    /// Volume swept between offsets `start` and `end` along the normal.
    pub(crate) fn volume(&self, start: f32, end: f32) -> f32 {
        self.area() * (end - start).abs()
    }

    /// The centroid of the caps on the base plane.
    pub(crate) fn centroid(&self) -> Vec3 {
        let mut weighted = Vec3::ZERO;
        let mut total = 0.0;
        for t in &self.triangles {
            let [a, b, c] = t.map(|i| self.points[i as usize]);
            let area = (b - a).cross(c - a).length() * 0.5;
            weighted += (a + b + c) / 3.0 * area;
            total += area;
        }
        if total > 0.0 {
            weighted / total
        } else if !self.points.is_empty() {
            self.points.iter().sum::<Vec3>() / self.points.len() as f32
        } else {
            Vec3::ZERO
        }
    }

    /// Whether loop corner `k` of `points` is a crease (not a smooth arc
    /// sample), so the paint step draws a wall edge there.
    pub(crate) fn sharp_corner(&self, lp: &[u32], k: usize) -> bool {
        let n = lp.len();
        let p = |i: usize| self.points[lp[i % n] as usize];
        let (a, b, c) = (p(k + n - 1), p(k), p(k + 1));
        let (u, v) = ((b - a).normalize_or_zero(), (c - b).normalize_or_zero());
        u.dot(v) < 25f32.to_radians().cos()
    }
}

/// A profile revolved about an axis: loops in world space and the axis.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Lathe {
    pub loops: Vec<Vec<Vec3>>,
    pub prism: Prism,
    pub axis_origin: Vec3,
    pub axis_direction: Vec3,
}

/// `point` rotated by `angle` about the axis through `origin` along unit `axis`.
pub(crate) fn rotate_about(point: Vec3, origin: Vec3, axis: Vec3, angle: f32) -> Vec3 {
    origin + glam::Quat::from_axis_angle(axis, angle) * (point - origin)
}

// ---------------------------------------------------------------------------
// Face groups of part tessellations

/// Polygons of `group` (all polygons when the mesh has no groups and
/// `group` is a polygon index).
pub(crate) fn group_faces(mesh: &Mesh, group: u32) -> Vec<usize> {
    if mesh.has_groups() {
        (0..mesh.faces.len())
            .filter(|&f| mesh.groups[f] == group)
            .collect()
    } else if (group as usize) < mesh.faces.len() {
        vec![group as usize]
    } else {
        Vec::new()
    }
}

/// The plane of a set of polygons when every vertex of every polygon lies on
/// it, relative to the group's size: `(unit normal, point)`. The normal is
/// the area-weighted polygon normal, so it is outward for closed meshes.
/// Testing the whole group (not one triangle) rejects curved faces whose
/// individual facets are flat.
pub(crate) fn group_plane(mesh: &Mesh, faces: &[usize]) -> Option<(Vec3, Vec3)> {
    let mut area = Vec3::ZERO;
    let mut points = Vec::new();
    for &f in faces {
        let face = mesh.faces.get(f)?;
        let a = *mesh.positions.get(*face.first()? as usize)?;
        for w in face.windows(2).skip(1) {
            let (b, c) = (mesh.positions[w[0] as usize], mesh.positions[w[1] as usize]);
            area += (b - a).cross(c - a);
        }
        points.extend(face.iter().map(|&i| mesh.positions[i as usize]));
    }
    let normal = area.try_normalize()?;
    let centroid = points.iter().sum::<Vec3>() / points.len() as f32;
    let extent = points
        .iter()
        .map(|p| p.distance(centroid))
        .fold(0.0_f32, f32::max);
    let deviation = points
        .iter()
        .map(|p| (*p - centroid).dot(normal).abs())
        .fold(0.0_f32, f32::max);
    (deviation <= extent * 2.0e-3 + 1.0e-6).then_some((normal, centroid))
}

/// Directed boundary edges of a polygon set: edges used by exactly one of
/// its polygons, in that polygon's winding.
pub(crate) fn group_boundary(mesh: &Mesh, faces: &[usize]) -> Vec<[u32; 2]> {
    let mut uses: BTreeMap<(u32, u32), (u32, [u32; 2])> = BTreeMap::new();
    for &f in faces {
        let face = &mesh.faces[f];
        for k in 0..face.len() {
            let (a, b) = (face[k], face[(k + 1) % face.len()]);
            if a == b {
                continue;
            }
            let key = (a.min(b), a.max(b));
            uses.entry(key)
                .and_modify(|(count, _)| *count += 1)
                .or_insert((1, [a, b]));
        }
    }
    uses.into_values()
        .filter(|(count, _)| *count == 1)
        .map(|(_, edge)| edge)
        .collect()
}

/// Chains directed edges into closed loops (open chains are kept as is).
pub(crate) fn chain_loops(edges: &[[u32; 2]]) -> Vec<Vec<u32>> {
    let mut next: HashMap<u32, Vec<u32>> = HashMap::new();
    for e in edges {
        next.entry(e[0]).or_default().push(e[1]);
    }
    let mut used: HashMap<(u32, u32), bool> = HashMap::new();
    let mut loops = Vec::new();
    for e in edges {
        if used.contains_key(&(e[0], e[1])) {
            continue;
        }
        let start = e[0];
        let mut current = e[0];
        let mut lp = Vec::new();
        loop {
            let Some(candidates) = next.get(&current) else {
                break;
            };
            let Some(&to) = candidates
                .iter()
                .find(|&&to| !used.contains_key(&(current, to)))
            else {
                break;
            };
            used.insert((current, to), true);
            lp.push(current);
            current = to;
            if current == start || lp.len() > edges.len() {
                break;
            }
        }
        if lp.len() >= 2 {
            loops.push(lp);
        }
    }
    loops
}

/// Merges consecutive collinear segments of a closed loop, returning the
/// straight edges as point pairs (arcs become many short edges).
pub(crate) fn straight_edges(points: &[Vec3]) -> Vec<(Vec3, Vec3)> {
    let n = points.len();
    if n < 2 {
        return Vec::new();
    }
    let direction = |i: usize| (points[(i + 1) % n] - points[i]).normalize_or_zero();
    // Start at a corner so the first edge is not split in two.
    let start = (0..n)
        .find(|&i| direction((i + n - 1) % n).dot(direction(i)) < 1.0 - 1.0e-4)
        .unwrap_or(0);
    let mut edges = Vec::new();
    let mut first = points[start];
    for step in 0..n {
        let i = (start + step) % n;
        let next = (i + 1) % n;
        let corner = direction(i).dot(direction(next)) < 1.0 - 1.0e-4;
        if corner || step == n - 1 {
            edges.push((first, points[next]));
            first = points[next];
        }
    }
    edges
}

/// Picks the face-group polygon containing `point` (part space) whose plane
/// matches `normal`, for re-finding a selected face after regeneration.
pub(crate) fn resolve_group(mesh: &Mesh, point: Vec3, normal: Vec3) -> Option<u32> {
    let size = mesh_extent(mesh).max(1.0e-6);
    let mut best: Option<(f32, u32)> = None;
    for (triangle, face) in mesh.triangles_with_faces() {
        let [a, b, c] = triangle.map(|i| mesh.positions[i as usize]);
        let n = (b - a).cross(c - a).normalize_or_zero();
        if n.dot(normal) < 0.995 {
            continue;
        }
        let offset = (point - a).dot(n);
        let projected = point - n * offset;
        let lateral = projected.distance(closest_on_triangle(projected, a, b, c));
        let score = lateral + offset.abs();
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, mesh.face_group(face)?));
        }
    }
    best.filter(|(score, _)| *score <= size * 1.0e-3)
        .map(|(_, g)| g)
}

fn mesh_extent(mesh: &Mesh) -> f32 {
    let (lo, hi) = mesh.positions.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    if lo.is_finite() { lo.distance(hi) } else { 0.0 }
}

/// Closest point to `p` on triangle `abc` (Ericson, Real-Time Collision Detection).
pub(crate) fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denominator = 1.0 / (va + vb + vc);
    a + ab * (vb * denominator) + ac * (vc * denominator)
}

// ---------------------------------------------------------------------------
// B-rep edges

/// One B-rep edge of a part tessellation: a chain of display edges between
/// the same two face groups.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BrepEdge {
    /// Vertex indices along the edge.
    pub points: Vec<u32>,
    /// The face groups on either side (equal for a boundary of an open mesh).
    pub groups: (u32, u32),
    pub straight: bool,
}

/// Groups the display edges of a part tessellation into B-rep edges: a
/// curved edge (a circle's rim) is many tessellation segments but one edge.
pub(crate) fn brep_edges(mesh: &Mesh) -> Vec<BrepEdge> {
    // Each undirected edge with the groups using it.
    let mut sides: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new();
    for (f, face) in mesh.faces.iter().enumerate() {
        let group = mesh.face_group(f).unwrap_or(f as u32);
        for k in 0..face.len() {
            let (a, b) = (face[k], face[(k + 1) % face.len()]);
            if a != b {
                sides.entry((a.min(b), a.max(b))).or_default().push(group);
            }
        }
    }
    let display: Vec<((u32, u32), (u32, u32))> = mesh
        .display_edges()
        .into_iter()
        .filter_map(|[a, b]| {
            let key = (a.min(b), a.max(b));
            let groups = sides.get(&key)?;
            let g0 = groups[0];
            let g1 = groups.iter().copied().find(|g| *g != g0).unwrap_or(g0);
            Some((key, (g0.min(g1), g0.max(g1))))
        })
        .collect();
    // Vertex → display edges, to walk chains through degree-two vertices.
    let mut at: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, ((a, b), _)) in display.iter().enumerate() {
        at.entry(*a).or_default().push(i);
        at.entry(*b).or_default().push(i);
    }
    let continues = |vertex: u32, groups: (u32, u32)| -> bool {
        at.get(&vertex).is_some_and(|edges| {
            edges.len() == 2 && edges.iter().all(|&e| display[e].1 == groups)
        })
    };
    let mut used = vec![false; display.len()];
    let mut result = Vec::new();
    for start in 0..display.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let ((a, b), groups) = display[start];
        let mut chain = std::collections::VecDeque::from([a, b]);
        // Extend forward from the back, then backward from the front.
        for forward in [true, false] {
            loop {
                let end = if forward {
                    *chain.back().unwrap()
                } else {
                    *chain.front().unwrap()
                };
                if !continues(end, groups) {
                    break;
                }
                let Some(&next) = at[&end].iter().find(|&&e| !used[e]) else {
                    break;
                };
                used[next] = true;
                let ((p, q), _) = display[next];
                let other = if p == end { q } else { p };
                if forward {
                    chain.push_back(other);
                } else {
                    chain.push_front(other);
                }
                if chain.front() == chain.back() {
                    break;
                }
            }
        }
        let points: Vec<u32> = chain.into_iter().collect();
        let p = |i: u32| mesh.positions[i as usize];
        let first = p(points[0]);
        let last = p(*points.last().unwrap());
        let direction = (last - first).normalize_or_zero();
        let straight = direction != Vec3::ZERO
            && points.iter().all(|&i| {
                let offset = p(i) - first;
                (offset - direction * offset.dot(direction)).length()
                    <= first.distance(last) * 1.0e-4 + 1.0e-7
            });
        result.push(BrepEdge {
            points,
            groups,
            straight,
        });
    }
    result
}

// ---------------------------------------------------------------------------
// Operation inference

/// Join when the extrusion direction leaves the target face's material
/// (pulling outward), Cut when it goes into it (pushing inward).
pub(crate) fn infer_operation(direction: Vec3, outward: Vec3) -> FeatureOperation {
    if direction.dot(outward) < 0.0 {
        FeatureOperation::Cut
    } else {
        FeatureOperation::Join
    }
}

/// A planar face a sketch lies on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CoplanarFace {
    pub object: u64,
    pub group: u32,
    /// Outward face normal in world space.
    pub outward: Vec3,
}

/// The first part face whose plane coincides with `plane` (world space, within
/// a tolerance relative to the part's size) and that contains one of `samples`
/// (world points inside the picked regions).
pub(crate) fn find_coplanar_face(
    plane: &SketchPlane,
    samples: &[Vec3],
    parts: &[(u64, Mat4, &Mesh)],
) -> Option<CoplanarFace> {
    let normal = plane.normal().normalize_or_zero();
    for &(object, matrix, mesh) in parts {
        if !mesh.has_groups() || matrix.determinant().abs() < 1.0e-12 {
            continue;
        }
        let size = mesh_extent(mesh).max(1.0e-6) * matrix.x_axis.truncate().length().max(1.0e-6);
        let tolerance = size * 1.0e-4 + 1.0e-6;
        let inverse_transpose = matrix.inverse().transpose();
        for (triangle, face) in mesh.triangles_with_faces() {
            let [a, b, c] = triangle.map(|i| matrix.transform_point3(mesh.positions[i as usize]));
            let n = (b - a).cross(c - a).normalize_or_zero();
            if n.dot(normal).abs() < 1.0 - 1.0e-4 || (plane.origin - a).dot(n).abs() > tolerance {
                continue;
            }
            for sample in samples {
                if (*sample - a).dot(n).abs() <= tolerance
                    && closest_on_triangle(*sample, a, b, c).distance(*sample) <= tolerance
                {
                    let group = mesh.face_group(face)?;
                    let outward = inverse_transpose
                        .transform_vector3(mesh.face_normal(face))
                        .normalize_or_zero();
                    return Some(CoplanarFace {
                        object,
                        group,
                        outward: if outward == Vec3::ZERO { n } else { outward },
                    });
                }
            }
        }
    }
    None
}

/// Offsets along `normal` of the target's faces parallel to the sketch, from
/// one pass over its tessellation (groups whose every polygon is parallel at
/// one height).
pub(crate) fn parallel_heights(
    mesh: &Mesh,
    matrix: Mat4,
    plane_origin: Vec3,
    normal: Vec3,
) -> Vec<f32> {
    let mut groups: HashMap<u32, Option<f32>> = HashMap::new();
    let inverse_transpose = matrix.inverse().transpose();
    for (f, face) in mesh.faces.iter().enumerate() {
        let Some(group) = mesh.face_group(f) else {
            continue;
        };
        let n = inverse_transpose
            .transform_vector3(mesh.face_normal(f))
            .normalize_or_zero();
        let point = matrix.transform_point3(mesh.positions[face[0] as usize]);
        let height = (point - plane_origin).dot(normal);
        let parallel = n.dot(normal).abs() > 1.0 - 1.0e-4;
        let entry = groups.entry(group).or_insert(Some(height));
        if !parallel || entry.is_some_and(|h| (h - height).abs() > 1.0e-4) {
            *entry = None;
        }
    }
    let mut heights: Vec<f32> = groups
        .into_values()
        .flatten()
        .filter(|h| h.abs() > 1.0e-5)
        .collect();
    heights.sort_by(f32::total_cmp);
    heights.dedup_by(|a, b| (*a - *b).abs() < 1.0e-5);
    heights
}

/// Signed volume enclosed by a closed mesh (divergence theorem).
pub(crate) fn mesh_volume(mesh: &Mesh) -> f64 {
    mesh.triangles()
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| mesh.positions[i as usize].as_dvec3());
            a.dot(b.cross(c)) / 6.0
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::Primitive;
    use std::f32::consts::{FRAC_PI_2, PI, TAU};

    fn circle(center: Vec2, radius: f32, segments: usize, clockwise: bool) -> Vec<Vec2> {
        (0..segments)
            .map(|i| {
                let mut a = i as f32 / segments as f32 * TAU;
                if clockwise {
                    a = -a;
                }
                center + Vec2::from_angle(a) * radius
            })
            .collect()
    }

    fn triangles_area(points: &[Vec2], triangles: &[[u32; 3]]) -> f32 {
        triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| points[i as usize]);
                assert!(cross(a, b, c) > 0.0, "triangle {t:?} is not counter-clockwise");
                cross(a, b, c) * 0.5
            })
            .sum()
    }

    #[test]
    fn dragging_follows_the_closest_point_on_the_axis() {
        // Looking from the side at a vertical axis: the pointer ray passes
        // the axis at height 2.
        let ray = Ray {
            origin: Vec3::new(5.0, 2.0, 0.3),
            direction: Vec3::new(-1.0, 0.0, 0.0),
        };
        let t = axis_parameter(Vec3::ZERO, Vec3::Y, ray).unwrap();
        assert!((t - 2.0).abs() < 1.0e-5, "{t}");
        // A slanted ray through (0, 1.5, 0): the closest point is exact.
        let target = Vec3::new(0.0, 1.5, 0.0);
        let origin = Vec3::new(3.0, 4.0, 2.0);
        let ray = Ray {
            origin,
            direction: (target - origin).normalize(),
        };
        let t = axis_parameter(Vec3::ZERO, Vec3::Y, ray).unwrap();
        assert!((t - 1.5).abs() < 1.0e-4, "{t}");
        // Scaled axis directions report parameters in axis units.
        let t = axis_parameter(Vec3::ZERO, Vec3::Y * 2.0, ray).unwrap();
        assert!((t - 0.75).abs() < 1.0e-4, "{t}");
        // Looking straight down the axis there is no stable answer.
        let along = Ray {
            origin: Vec3::new(0.0, 9.0, 0.0),
            direction: Vec3::NEG_Y,
        };
        assert!(axis_parameter(Vec3::ZERO, Vec3::Y, along).is_none());
    }

    #[test]
    fn snapping_prefers_face_targets_then_clean_steps() {
        assert_eq!(snap_value(0.234, 0.05, &[], 0.0), (0.25, None));
        assert_eq!(snap_value(0.234, 0.05, &[0.24], 0.01), (0.24, Some(0)));
        assert_eq!(snap_value(-0.47, 0.1, &[0.24], 0.01).0, -0.5);
        // The step adapts to zoom: more pixels per metre, finer steps.
        assert!(snap_step(2000.0) < snap_step(20.0));
        assert!((snap_angle(0.27, 15f32.to_radians()) - 15f32.to_radians()).abs() < 1.0e-6);
    }

    #[test]
    fn triangulation_covers_regions_with_holes_exactly() {
        let square = vec![
            Vec2::new(-1.0, -1.0),
            Vec2::new(1.0, -1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(-1.0, 1.0),
        ];
        let triangles = triangulate(&square, &[]);
        assert_eq!(triangles.len(), 2);
        assert!((triangles_area(&square, &triangles) - 4.0).abs() < 1.0e-5);
        // A ring: the square with a circular hole, given clockwise.
        let hole = circle(Vec2::new(0.2, -0.1), 0.5, 48, true);
        let triangles = triangulate(&square, std::slice::from_ref(&hole));
        let mut points = square.clone();
        points.extend(&hole);
        let expected = 4.0 + signed_area(&hole);
        assert!(
            (triangles_area(&points, &triangles) - expected).abs() < 1.0e-4,
            "{} vs {expected}",
            triangles_area(&points, &triangles)
        );
        assert!(triangles.len() <= 4 + 48, "at most n + 2h - 2 triangles");
        // Two holes, a clockwise outer loop and a concave L-shape all work.
        let holes = vec![
            circle(Vec2::new(-0.5, 0.0), 0.3, 24, false),
            circle(Vec2::new(0.5, 0.0), 0.3, 24, false),
        ];
        let reversed: Vec<Vec2> = square.iter().rev().copied().collect();
        let triangles = triangulate(&reversed, &holes);
        let mut points = reversed.clone();
        for h in &holes {
            points.extend(h);
        }
        let expected = 4.0 - holes.iter().map(|h| signed_area(h)).sum::<f32>();
        assert!((triangles_area(&points, &triangles) - expected).abs() < 1.0e-4);
        let l_shape = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 2.0),
            Vec2::new(0.0, 2.0),
        ];
        let triangles = triangulate(&l_shape, &[]);
        assert!((triangles_area(&l_shape, &triangles) - 3.0).abs() < 1.0e-5);
    }

    #[test]
    fn prisms_from_regions_measure_area_volume_and_orientation() {
        let plane = SketchPlane::ground();
        let outer = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        let hole = circle(Vec2::new(1.0, 0.5), 0.25, 64, true);
        let prism = Prism::from_regions(&plane, Mat4::IDENTITY, &[vec![outer, hole.clone()]]);
        assert_eq!(prism.loops.len(), 2);
        assert!(prism.normal.distance(Vec3::Y) < 1.0e-6);
        let area = 2.0 + signed_area(&hole);
        assert!((prism.area() - area).abs() < 1.0e-4);
        assert!((prism.volume(-0.1, 0.4) - area * 0.5).abs() < 1.0e-4);
        // Caps face along the normal.
        for t in &prism.triangles {
            let [a, b, c] = t.map(|i| prism.points[i as usize]);
            assert!((b - a).cross(c - a).dot(prism.normal) > 0.0);
        }
        // Rectangle corners are creases; points along the hole circle are not.
        assert!(prism.sharp_corner(&prism.loops[0], 0));
        assert!(!prism.sharp_corner(&prism.loops[1], 5));
        // A mirrored placement still yields outward caps.
        let mirrored = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        let prism = Prism::from_regions(
            &plane,
            mirrored,
            &[vec![vec![Vec2::ZERO, Vec2::X, Vec2::ONE]]],
        );
        for t in &prism.triangles {
            let [a, b, c] = t.map(|i| prism.points[i as usize]);
            assert!((b - a).cross(c - a).dot(prism.normal) > 0.0);
        }
    }

    #[test]
    fn whole_face_groups_decide_planarity_and_boundaries() {
        // A cube whose faces are split into triangles, grouped per side.
        let cube = Mesh::primitive(Primitive::Cube);
        let mut mesh = Mesh::new(cube.positions.clone(), Vec::new());
        for (g, face) in cube.faces.iter().enumerate() {
            mesh.faces.push(vec![face[0], face[1], face[2]]);
            mesh.faces.push(vec![face[0], face[2], face[3]]);
            mesh.groups.extend([g as u32, g as u32]);
        }
        let top = (0..6u32)
            .find(|&g| {
                let faces = group_faces(&mesh, g);
                group_plane(&mesh, &faces).is_some_and(|(n, _)| n.y > 0.9)
            })
            .unwrap();
        let faces = group_faces(&mesh, top);
        assert_eq!(faces.len(), 2);
        let (normal, point) = group_plane(&mesh, &faces).unwrap();
        assert!(normal.distance(Vec3::Y) < 1.0e-5 && (point.y - 1.0).abs() < 1.0e-5);
        // The diagonal is interior: four boundary edges forming one loop.
        let boundary = group_boundary(&mesh, &faces);
        assert_eq!(boundary.len(), 4);
        let loops = chain_loops(&boundary);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 4);
        let points: Vec<Vec3> = loops[0].iter().map(|&i| mesh.positions[i as usize]).collect();
        assert_eq!(straight_edges(&points).len(), 4);
        // Two adjacent sides together are not planar, though each triangle is.
        let mut bent = faces.clone();
        bent.extend(group_faces(&mesh, (top + 1) % 6));
        assert!(group_plane(&mesh, &bent).is_none());
        // Selections survive regeneration by position and normal.
        assert_eq!(
            resolve_group(&mesh, Vec3::new(0.3, 1.0, -0.2), Vec3::Y),
            Some(top)
        );
        assert_eq!(resolve_group(&mesh, Vec3::new(0.3, 3.0, -0.2), Vec3::Y), None);
    }

    #[test]
    fn collinear_runs_merge_into_straight_edges() {
        let mut points = vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)];
        points.extend([Vec3::new(2.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)]);
        let edges = straight_edges(&points);
        assert_eq!(edges.len(), 4);
        let longest = edges
            .iter()
            .map(|(a, b)| a.distance(*b))
            .fold(0.0_f32, f32::max);
        assert!((longest - 2.0).abs() < 1.0e-6);
    }

    #[test]
    fn brep_edges_join_tessellation_segments_of_one_edge() {
        // A cylinder-like prism: a square top (one group), an n-gon would be
        // curved; here the side is one smooth group of quads around a hexagon.
        let n = 6;
        let mut positions = Vec::new();
        for i in 0..n {
            let a = i as f32 / n as f32 * TAU;
            positions.push(Vec3::new(a.cos(), 0.0, a.sin()));
            positions.push(Vec3::new(a.cos(), 1.0, a.sin()));
        }
        let mut faces = Vec::new();
        let mut groups = Vec::new();
        for i in 0..n as u32 {
            let j = (i + 1) % n as u32;
            faces.push(vec![2 * i, 2 * i + 1, 2 * j + 1, 2 * j]);
            groups.push(0);
        }
        faces.push((0..n as u32).map(|i| 2 * i + 1).rev().collect());
        groups.push(1);
        faces.push((0..n as u32).map(|i| 2 * i).collect());
        groups.push(2);
        let mesh = Mesh {
            positions,
            faces,
            groups,
        };
        let edges = brep_edges(&mesh);
        // Two rims (top and bottom), each one curved edge; no seams.
        assert_eq!(edges.len(), 2, "{edges:?}");
        assert!(edges.iter().all(|e| !e.straight && e.points.len() == n + 1));
        // A grouped cube has twelve straight edges.
        let cube = Mesh::primitive(Primitive::Cube);
        let grouped = Mesh {
            groups: (0..cube.faces.len() as u32).collect(),
            ..cube
        };
        let edges = brep_edges(&grouped);
        assert_eq!(edges.len(), 12);
        assert!(edges.iter().all(|e| e.straight && e.points.len() == 2));
    }

    #[test]
    fn operations_follow_the_drag_direction_against_the_face() {
        assert_eq!(infer_operation(Vec3::Y, Vec3::Y), FeatureOperation::Join);
        assert_eq!(infer_operation(Vec3::NEG_Y, Vec3::Y), FeatureOperation::Cut);
        // A sketch on the cube's top face is coplanar with it; on the ground
        // (well below the cube) it is not.
        let cube = Mesh::primitive(Primitive::Cube);
        let grouped = Mesh {
            groups: (0..cube.faces.len() as u32).collect(),
            ..cube
        };
        let lifted = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0));
        let parts = [(7, lifted, &grouped)];
        let on_top = SketchPlane {
            origin: Vec3::new(0.0, 2.0, 0.0),
            ..SketchPlane::ground()
        };
        let face = find_coplanar_face(&on_top, &[Vec3::new(0.2, 2.0, 0.1)], &parts).unwrap();
        assert_eq!(face.object, 7);
        assert!(face.outward.distance(Vec3::Y) < 1.0e-5);
        // Outside the face's outline it does not count.
        assert!(find_coplanar_face(&on_top, &[Vec3::new(3.0, 2.0, 0.0)], &parts).is_none());
        assert!(
            find_coplanar_face(&SketchPlane::ground(), &[Vec3::new(0.2, 0.0, 0.1)], &[(
                7,
                Mat4::from_translation(Vec3::new(0.0, 3.0, 0.0)),
                &grouped
            )])
            .is_none()
        );
        // A sketch facing down on the bottom face still finds it (outward −Y).
        let bottom = SketchPlane::from_point_normal(Vec3::ZERO, Vec3::NEG_Y).unwrap();
        let face = find_coplanar_face(&bottom, &[Vec3::new(0.1, 0.0, 0.1)], &parts).unwrap();
        assert!(face.outward.distance(Vec3::NEG_Y) < 1.0e-5);
        assert_eq!(
            infer_operation(bottom.normal(), face.outward),
            FeatureOperation::Join
        );
    }

    #[test]
    fn rotation_and_volume_helpers() {
        let p = rotate_about(Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO, Vec3::Y, FRAC_PI_2);
        assert!(p.distance(Vec3::new(0.0, 0.0, -1.0)) < 1.0e-5);
        let cube = Mesh::primitive(Primitive::Cube);
        assert!((mesh_volume(&cube) - 8.0).abs() < 1.0e-6);
        let _ = PI;
    }
}

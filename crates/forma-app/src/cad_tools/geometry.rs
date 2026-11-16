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
#[cfg(test)]
pub(crate) fn signed_area(points: &[Vec2]) -> f32 {
    let mut sum = 0.0;
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        sum += a.perp_dot(b);
    }
    sum * 0.5
}

#[cfg(test)]
fn cross(o: Vec2, a: Vec2, b: Vec2) -> f32 {
    (a - o).perp_dot(b - o)
}

/// Triangulates a region: an outer loop with holes, either winding (see
/// [`super::earcut`]). Returns triangles as indices into the outer points
/// followed by each hole's points, wound counter-clockwise.
pub(crate) fn triangulate(outer: &[Vec2], holes: &[Vec<Vec2>]) -> Vec<[u32; 3]> {
    super::earcut::triangulate(outer, holes)
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
    pub(crate) fn from_regions(
        plane: &SketchPlane,
        matrix: Mat4,
        regions: &[Vec<Vec<Vec2>>],
    ) -> Self {
        let mut prism = Prism {
            normal: matrix.transform_vector3(plane.normal()).normalize_or_zero(),
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
                prism
                    .loops
                    .push((offset..offset + polygon.len() as u32).collect());
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
    #[cfg(test)]
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

    /// Whether `point`, projected along the normal, lies on the caps (a
    /// plate's centroid can fall inside a hole or under a boss).
    pub(crate) fn cap_contains(&self, point: Vec3) -> bool {
        let (u, v) = self.normal.any_orthonormal_pair();
        let flat = |p: Vec3| Vec2::new(p.dot(u), p.dot(v));
        let q = flat(point);
        self.triangles.iter().any(|t| {
            let [a, b, c] = t.map(|i| flat(self.points[i as usize]));
            let (d1, d2, d3) = (
                (b - a).perp_dot(q - a),
                (c - b).perp_dot(q - b),
                (a - c).perp_dot(q - c),
            );
            (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
        })
    }

    /// A point on the caps near their centroid, for an arrow to stand on:
    /// the centroid itself when it lies on them, else the middle of the cap
    /// triangle furthest from the outline, nearer the centroid winning
    /// close calls. A ring's centroid is the middle of its hole; this lands
    /// on the ring.
    pub(crate) fn interior_point(&self) -> Vec3 {
        let centroid = self.centroid();
        if self.triangles.is_empty() || self.cap_contains(centroid) {
            return centroid;
        }
        let segments: Vec<(Vec3, Vec3)> = self
            .loops
            .iter()
            .flat_map(|lp| {
                (0..lp.len()).map(move |k| {
                    (
                        self.points[lp[k] as usize],
                        self.points[lp[(k + 1) % lp.len()] as usize],
                    )
                })
            })
            .collect();
        let clearance = |p: Vec3| {
            segments
                .iter()
                .map(|&(a, b)| {
                    let d = b - a;
                    let t = ((p - a).dot(d) / d.length_squared().max(1.0e-12)).clamp(0.0, 1.0);
                    p.distance(a + d * t)
                })
                .fold(f32::INFINITY, f32::min)
        };
        self.triangles
            .iter()
            .map(|t| t.map(|i| self.points[i as usize]))
            .map(|[a, b, c]| (a + b + c) / 3.0)
            .map(|p| (clearance(p) - 0.25 * p.distance(centroid), p))
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map_or(centroid, |(_, p)| p)
    }

    /// Whether loop corner `k` of `points` is a crease (not a smooth arc    /// Whether loop corner `k` of `points` is a crease (not a smooth arc
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
/// its polygons, in that polygon's winding. Vertices are welded by position
/// first (as [`weld`] does for the whole mesh), so a tessellation that
/// repeats a point inside one face does not outline the edges meeting
/// there: truck's mesh of the bracket's plate repeats the start of a hole's
/// rim 7 µm apart, and a long edge between two holes then used one copy on
/// each side and read as a line across the face. Returned indices are the
/// first vertex at each position.
pub(crate) fn group_boundary(mesh: &Mesh, faces: &[usize]) -> Vec<[u32; 2]> {
    let canon = weld_faces(mesh, faces);
    let canon = |i: u32| canon.get(&i).copied().unwrap_or(i);
    let mut uses: BTreeMap<(u32, u32), (u32, [u32; 2])> = BTreeMap::new();
    for &f in faces {
        let face = &mesh.faces[f];
        for k in 0..face.len() {
            let (a, b) = (canon(face[k]), canon(face[(k + 1) % face.len()]));
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
#[cfg(test)]
pub(crate) fn resolve_group(mesh: &Mesh, point: Vec3, normal: Vec3) -> Option<u32> {
    resolve_groups(mesh, &[(point, normal)])[0]
}

/// [`resolve_group`] for several references in one pass over the mesh's
/// triangles (selections are re-resolved after every regeneration).
pub(crate) fn resolve_groups(mesh: &Mesh, references: &[(Vec3, Vec3)]) -> Vec<Option<u32>> {
    let size = mesh_extent(mesh).max(1.0e-6);
    let mut best: Vec<Option<(f32, u32)>> = vec![None; references.len()];
    if references.is_empty() {
        return Vec::new();
    }
    for (triangle, face) in mesh.triangles_with_faces() {
        let [a, b, c] = triangle.map(|i| mesh.positions[i as usize]);
        let n = (b - a).cross(c - a).normalize_or_zero();
        let Some(group) = mesh.face_group(face) else {
            continue;
        };
        for (slot, (point, normal)) in best.iter_mut().zip(references) {
            if n.dot(*normal) < 0.995 {
                continue;
            }
            let offset = (*point - a).dot(n);
            if offset.abs() > size * 1.0e-3 {
                continue;
            }
            let projected = *point - n * offset;
            let lateral = projected.distance(closest_on_triangle(projected, a, b, c));
            let score = lateral + offset.abs();
            if slot.is_none_or(|(s, _)| score < s) {
                *slot = Some((score, group));
            }
        }
    }
    best.into_iter()
        .map(|b| {
            b.filter(|(score, _)| *score <= size * 1.0e-3)
                .map(|(_, g)| g)
        })
        .collect()
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

/// A canonical vertex per position: part tessellations weld vertices only
/// within each face group, so neighbouring faces repeat the points of the
/// edge they share (within a tiny tolerance). Neighbouring grid cells are
/// searched, since rounding alone splits points that straddle a cell edge.
pub(crate) fn weld(mesh: &Mesh) -> Vec<u32> {
    let tolerance = (mesh_extent(mesh) * 2.0e-5).max(1.0e-9);
    let cell = |p: Vec3| {
        let q = (p / tolerance).floor();
        (q.x as i64, q.y as i64, q.z as i64)
    };
    let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    let mut canon = Vec::with_capacity(mesh.positions.len());
    for (i, p) in mesh.positions.iter().enumerate() {
        let (x, y, z) = cell(*p);
        let mut found = None;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(&j) = grid.get(&(x + dx, y + dy, z + dz)).and_then(|list| {
                        list.iter()
                            .find(|&&j| mesh.positions[j as usize].distance(*p) <= tolerance)
                    }) {
                        found = Some(j);
                        break 'search;
                    }
                }
            }
        }
        let id = found.unwrap_or_else(|| {
            grid.entry((x, y, z)).or_default().push(i as u32);
            i as u32
        });
        canon.push(id);
    }
    canon
}

/// [`weld`] for the vertices of some polygons only: each vertex mapped to
/// the first of them at its position (within the same tolerance).
fn weld_faces(mesh: &Mesh, faces: &[usize]) -> HashMap<u32, u32> {
    let tolerance = (mesh_extent(mesh) * 2.0e-5).max(1.0e-9);
    let cell = |p: Vec3| {
        let q = (p / tolerance).floor();
        (q.x as i64, q.y as i64, q.z as i64)
    };
    let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    let mut canon: HashMap<u32, u32> = HashMap::new();
    for &f in faces {
        for &i in &mesh.faces[f] {
            if canon.contains_key(&i) {
                continue;
            }
            let p = mesh.positions[i as usize];
            let (x, y, z) = cell(p);
            let found = (-1..=1)
                .flat_map(|dx| (-1..=1).flat_map(move |dy| (-1..=1).map(move |dz| (dx, dy, dz))))
                .find_map(|(dx, dy, dz)| {
                    grid.get(&(x + dx, y + dy, z + dz)).and_then(|list| {
                        list.iter()
                            .copied()
                            .find(|&j| mesh.positions[j as usize].distance(p) <= tolerance)
                    })
                });
            let id = found.unwrap_or_else(|| {
                grid.entry((x, y, z)).or_default().push(i);
                i
            });
            canon.insert(i, id);
        }
    }
    canon
}

/// Groups the edges between face groups of a part tessellation into B-rep
/// edges: a curved edge (a circle's rim) is many tessellation segments but
/// one edge. Vertices are welded by position first (see [`weld`]); the
/// returned points are canonical vertex indices.
#[cfg(test)]
pub(crate) fn brep_edges(mesh: &Mesh) -> Vec<BrepEdge> {
    brep_edges_welded(mesh, &weld(mesh))
}

pub(crate) fn brep_edges_welded(mesh: &Mesh, canon: &[u32]) -> Vec<BrepEdge> {
    // Each undirected (welded) edge with the groups using it.
    let mut sides: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new();
    for (f, face) in mesh.faces.iter().enumerate() {
        let group = mesh.face_group(f).unwrap_or(f as u32);
        for k in 0..face.len() {
            let (a, b) = (
                canon[face[k] as usize],
                canon[face[(k + 1) % face.len()] as usize],
            );
            if a != b {
                sides.entry((a.min(b), a.max(b))).or_default().push(group);
            }
        }
    }
    // Edges between two groups, or open edges used once.
    let display: Vec<((u32, u32), (u32, u32))> = sides
        .iter()
        .filter_map(|(key, groups)| {
            let g0 = groups[0];
            let g1 = groups.iter().copied().find(|g| *g != g0).unwrap_or(g0);
            (g0 != g1 || groups.len() == 1).then_some((*key, (g0.min(g1), g0.max(g1))))
        })
        .collect();
    // Vertex → display edges, to walk chains through degree-two vertices.
    let mut at: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, ((a, b), _)) in display.iter().enumerate() {
        at.entry(*a).or_default().push(i);
        at.entry(*b).or_default().push(i);
    }
    let continues = |vertex: u32, groups: (u32, u32)| -> bool {
        at.get(&vertex)
            .is_some_and(|edges| edges.len() == 2 && edges.iter().all(|&e| display[e].1 == groups))
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
                assert!(
                    cross(a, b, c) > 0.0,
                    "triangle {t:?} is not counter-clockwise"
                );
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

    /// Deterministic xorshift for the property tests.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (hi - lo) * ((self.next() >> 40) as f32 / (1u64 << 24) as f32)
        }
    }

    /// Covered area and clockwise triangle count of a triangulation.
    fn coverage(outer: &[Vec2], holes: &[Vec<Vec2>]) -> (f32, f32, usize) {
        let triangles = triangulate(outer, holes);
        let mut points = outer.to_vec();
        for hole in holes {
            points.extend_from_slice(hole);
        }
        let mut sum = 0.0;
        let mut clockwise = 0;
        for t in &triangles {
            let [a, b, c] = t.map(|i| points[i as usize]);
            let area = cross(a, b, c) * 0.5;
            clockwise += usize::from(area < 0.0);
            sum += area;
        }
        let expected = signed_area(outer).abs()
            - holes
                .iter()
                .filter(|h| h.len() >= 3)
                .map(|h| signed_area(h).abs())
                .sum::<f32>();
        (sum, expected, clockwise)
    }

    /// Regression (review): with two or more holes, bridges could be made to
    /// the wrong copy of a vertex an earlier bridge duplicated, and the
    /// fallback fanned overlapping triangles (11 of 712 two-hole cases, 63 of
    /// 765 three-hole cases failed). Random star holes in random outer loops
    /// must cover exactly the region's area with counter-clockwise triangles.
    #[test]
    fn random_regions_with_many_holes_cover_their_exact_area() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut failures = Vec::new();
        for case in 0..3000 {
            let n = 8 + (rng.next() % 40) as usize;
            let outer: Vec<Vec2> = (0..n)
                .map(|i| {
                    let a = (i as f32 + rng.range(-0.3, 0.3)) / n as f32 * TAU;
                    Vec2::new(a.cos(), a.sin()) * rng.range(2.0, 4.0)
                })
                .collect();
            let centres = [
                Vec2::new(-0.9, 0.0),
                Vec2::new(0.9, 0.0),
                Vec2::new(0.0, 0.9),
                Vec2::new(0.0, -0.9),
            ];
            let count = (rng.next() % 5) as usize;
            let holes: Vec<Vec<Vec2>> = centres
                .iter()
                .take(count)
                .map(|c| {
                    // Jittered, evenly spread angles keep every gap below a
                    // half turn, so the star is simple (sorted random angles
                    // can leave a gap over 180° and cross themselves).
                    let m = 3 + (rng.next() % 12) as usize;
                    (0..m)
                        .map(|k| {
                            let a = (k as f32 + rng.range(-0.3, 0.3)) / m as f32 * TAU;
                            *c + Vec2::new(a.cos(), a.sin()) * rng.range(0.1, 0.4)
                        })
                        .collect::<Vec<Vec2>>()
                })
                .filter(|h| h.len() >= 3)
                .collect();
            let (sum, expected, clockwise) = coverage(&outer, &holes);
            if (sum - expected).abs() > expected * 1.0e-3 || clockwise > 0 {
                failures.push((case, holes.len(), sum, expected, clockwise));
            }
        }
        assert!(
            failures.is_empty(),
            "{} failures, first {:?}",
            failures.len(),
            &failures[..failures.len().min(5)]
        );
    }

    /// Regression (review): a plate with four corner bolt holes covered
    /// 15.01 instead of 7.22.
    #[test]
    fn plates_with_hole_patterns_triangulate_exactly() {
        let circle = |c: Vec2, r: f32, n: usize| -> Vec<Vec2> {
            (0..n)
                .map(|i| c + Vec2::from_angle(i as f32 / n as f32 * TAU) * r)
                .collect()
        };
        let rect = |w: f32, h: f32| {
            vec![
                Vec2::new(-w, -h),
                Vec2::new(w, -h),
                Vec2::new(w, h),
                Vec2::new(-w, h),
            ]
        };
        let layouts: Vec<(Vec<Vec2>, Vec<Vec<Vec2>>)> = vec![
            (
                rect(2.0, 1.0),
                [(-1.5, -0.6), (1.5, -0.6), (1.5, 0.6), (-1.5, 0.6)]
                    .iter()
                    .map(|&(x, y)| circle(Vec2::new(x, y), 0.25, 48))
                    .collect(),
            ),
            (
                rect(1.0, 2.0),
                [-1.0, 0.0, 1.0]
                    .iter()
                    .map(|&y| circle(Vec2::new(0.0, y), 0.3, 48))
                    .collect(),
            ),
            (
                rect(2.0, 1.5),
                [
                    (-1.2, -0.7),
                    (0.0, -0.7),
                    (1.2, -0.7),
                    (-1.2, 0.7),
                    (0.0, 0.7),
                    (1.2, 0.7),
                ]
                .iter()
                .map(|&(x, y)| circle(Vec2::new(x, y), 0.3, 40))
                .collect(),
            ),
            (
                circle(Vec2::ZERO, 2.0, 96),
                (0..6)
                    .map(|k| circle(Vec2::from_angle(k as f32 / 6.0 * TAU) * 1.4, 0.2, 32))
                    .chain([circle(Vec2::ZERO, 0.6, 64)])
                    .collect(),
            ),
            (
                rect(2.0, 1.0),
                vec![
                    vec![
                        Vec2::new(-1.5, -0.5),
                        Vec2::new(-0.5, -0.5),
                        Vec2::new(-0.5, 0.5),
                        Vec2::new(-1.5, 0.5),
                    ],
                    vec![
                        Vec2::new(0.5, -0.5),
                        Vec2::new(1.5, -0.5),
                        Vec2::new(1.5, 0.5),
                        Vec2::new(0.5, 0.5),
                    ],
                ],
            ),
        ];
        for (outer, holes) in layouts {
            let (sum, expected, clockwise) = coverage(&outer, &holes);
            assert!(
                (sum - expected).abs() <= expected * 1.0e-4 && clockwise == 0,
                "{} holes: {sum} vs {expected}",
                holes.len()
            );
        }
    }

    /// Degenerate input terminates and never covers more than its area.
    #[test]
    fn degenerate_regions_terminate_without_overlap() {
        let mut rng = Rng(12345);
        for _ in 0..2000 {
            let n = 3 + (rng.next() % 30) as usize;
            let outer: Vec<Vec2> = (0..n)
                .map(|_| {
                    Vec2::new(
                        (rng.range(-3.0, 3.0) * 2.0).round() / 2.0,
                        (rng.range(-3.0, 3.0) * 2.0).round() / 2.0,
                    )
                })
                .collect();
            let holes: Vec<Vec<Vec2>> = (0..(rng.next() % 3))
                .map(|_| {
                    (0..3 + rng.next() % 5)
                        .map(|_| Vec2::new(rng.range(-4.0, 4.0), rng.range(-4.0, 4.0)))
                        .collect()
                })
                .collect();
            let _ = triangulate(&outer, &holes);
        }
        assert!(triangulate(&[Vec2::NAN, Vec2::ONE, Vec2::X], &[]).is_empty());
        let _ = triangulate(
            &[Vec2::ZERO, Vec2::splat(f32::INFINITY), Vec2::X, Vec2::Y],
            &[vec![Vec2::NAN; 3]],
        );
        // A hole touching the outer loop at a vertex.
        let square = vec![
            Vec2::ZERO,
            Vec2::new(4.0, 0.0),
            Vec2::new(4.0, 4.0),
            Vec2::new(0.0, 4.0),
        ];
        let touching = vec![
            Vec2::new(4.0, 2.0),
            Vec2::new(3.0, 1.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(3.0, 3.0),
        ];
        let (sum, expected, clockwise) = coverage(&square, &[touching]);
        assert!(
            (sum - expected).abs() < 1.0e-4 && clockwise == 0,
            "{sum} vs {expected}"
        );
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
        let points: Vec<Vec3> = loops[0]
            .iter()
            .map(|&i| mesh.positions[i as usize])
            .collect();
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
        assert_eq!(
            resolve_group(&mesh, Vec3::new(0.3, 3.0, -0.2), Vec3::Y),
            None
        );
    }

    #[test]
    fn caps_contain_points_on_them_but_not_in_holes() {
        let outer = vec![
            Vec2::new(-2.0, -2.0),
            Vec2::new(2.0, -2.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(-2.0, 2.0),
        ];
        let hole = circle(Vec2::ZERO, 1.0, 32, true);
        let prism =
            Prism::from_regions(&SketchPlane::ground(), Mat4::IDENTITY, &[vec![outer, hole]]);
        // The ring's centroid is the middle of its hole.
        assert!(prism.centroid().distance(Vec3::ZERO) < 1.0e-4);
        assert!(!prism.cap_contains(prism.centroid()));
        assert!(prism.cap_contains(Vec3::new(1.5, 0.3, -1.5)));
        assert!(!prism.cap_contains(Vec3::new(2.5, 0.0, 0.0)));
        // An arrow stands on the ring, clear of both outlines.
        let inside = prism.interior_point();
        assert!(prism.cap_contains(inside), "{inside:?}");
        let radius = Vec2::new(inside.x, inside.z).length();
        assert!(
            radius > 1.1 && inside.x.abs().max(inside.z.abs()) < 1.9,
            "{inside:?}"
        );
    }

    /// A face whose tessellation repeats a vertex a hair apart (one copy on
    /// each side of an inner edge) is still outlined by its border only.
    #[test]
    fn boundaries_weld_repeated_points() {
        let positions = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, 1.0),
            // A copy of the first corner, 2 µm off.
            Vec3::new(2.0e-6, 0.0, 0.0),
        ];
        let mesh = Mesh {
            positions,
            faces: vec![vec![0, 2, 1], vec![4, 3, 2]],
            groups: vec![0, 0],
        };
        let boundary = group_boundary(&mesh, &[0, 1]);
        assert_eq!(boundary.len(), 4, "{boundary:?}");
        assert_eq!(chain_loops(&boundary).len(), 1);
    }

    #[test]
    fn collinear_runs_merge_into_straight_edges() {
        let mut points = vec![
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
        ];
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
            find_coplanar_face(
                &SketchPlane::ground(),
                &[Vec3::new(0.2, 0.0, 0.1)],
                &[(
                    7,
                    Mat4::from_translation(Vec3::new(0.0, 3.0, 0.0)),
                    &grouped
                )]
            )
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

//! Transactional mesh finishing tools. Surface orientation is derived from
//! shared edges rather than assuming that imported polygon winding is correct.
use crate::{Mesh, mesh::Triangulation};
use anyhow::{Result, ensure};
use glam::DVec3;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const GEOMETRY_LIMIT: usize = 2_000_000;
const BEVEL_PLANE_TEST_LIMIT: usize = 8_000_000;

impl Mesh {
    /// Replace selected polygons with their ear-clipped triangles, preserving
    /// winding and returning the new selection. Unselected polygons keep their
    /// geometry and relative order. Errors leave the mesh unchanged.
    pub fn triangulate_faces(&mut self, selected: &BTreeSet<usize>) -> Result<BTreeSet<usize>> {
        self.validate()?;
        check_selection(self, selected)?;
        let count = self.faces.len()
            + selected
                .iter()
                .map(|&i| self.faces[i].len() - 3)
                .sum::<usize>();
        ensure!(
            count <= GEOMETRY_LIMIT,
            "Triangulation would exceed the two million face limit"
        );
        let mut faces = Vec::with_capacity(count);
        let mut selection = BTreeSet::new();
        let mut scratch = Triangulation::default();
        for (index, face) in self.faces.iter().enumerate() {
            if selected.contains(&index) {
                let triangles = self.triangulate_face(face, &mut scratch);
                ensure!(
                    triangles.len() == face.len() - 2,
                    "Face {index} cannot be triangulated"
                );
                for triangle in triangles {
                    selection.insert(faces.len());
                    faces.push(triangle.to_vec());
                }
            } else {
                faces.push(face.clone());
            }
        }
        let edited = Self {
            positions: self.positions.clone(),
            faces,
        };
        edited.validate()?;
        *self = edited;
        Ok(selection)
    }

    /// Reverse selected polygon winding. The geometry and face indices stay
    /// unchanged. Deliberately permits a locally inconsistent orientation so
    /// users can choose which side of an open surface is its front.
    pub fn flip_faces(&mut self, selected: &BTreeSet<usize>) -> Result<()> {
        self.validate()?;
        check_selection(self, selected)?;
        let mut edited = self.clone();
        for &index in selected {
            edited.faces[index].reverse();
        }
        edited.validate()?;
        *self = edited;
        Ok(())
    }

    /// Orient neighboring faces consistently. Each closed component is made
    /// outward using its signed volume; an open component retains the side of
    /// its lowest-index face because an open surface has no intrinsic outside.
    /// Rejects non-manifold, non-orientable, or zero-volume closed surfaces.
    pub fn recalculate_normals(&mut self) -> Result<()> {
        self.validate()?;
        let topology = Topology::new(self)?;
        let (mut flips, components) = topology.orientations()?;
        let mut scratch = Triangulation::default();
        for component in components {
            if component.iter().any(|&f| topology.boundary_faces[f]) {
                continue;
            }
            let origin = self.positions[self.faces[component[0]][0] as usize].as_dvec3();
            let mut volume = 0.;
            let mut compensation = 0.;
            let mut roundoff = 0.;
            for &face in &component {
                for &[a, b, c] in self.triangulate_face(&self.faces[face], &mut scratch) {
                    let [a, b, c] =
                        [a, b, c].map(|i| self.positions[i as usize].as_dvec3() - origin);
                    let signed = a.dot(b.cross(c));
                    let term = if flips[face] { -signed } else { signed };
                    let corrected = term - compensation;
                    let sum = volume + corrected;
                    compensation = (sum - volume) - corrected;
                    volume = sum;
                    // Bound determinant roundoff using the actual products.
                    // A tolerance based on the longest dimension (even an
                    // axis-aligned bounding box's volume) wrongly rejects thin
                    // solids, especially after rotating them.
                    let [a, b, c] = [a, b, c].map(DVec3::abs);
                    roundoff += a.x * (b.y * c.z + b.z * c.y)
                        + a.y * (b.z * c.x + b.x * c.z)
                        + a.z * (b.x * c.y + b.y * c.x);
                }
            }
            let tolerance = roundoff * f64::EPSILON * 32.;
            ensure!(
                volume.abs() > tolerance,
                "Cannot determine outside of a zero-volume closed surface"
            );
            if volume < 0. {
                for face in component {
                    flips[face] = !flips[face];
                }
            }
        }
        let mut edited = self.clone();
        for (face, flip) in edited.faces.iter_mut().zip(flips) {
            if flip {
                face.reverse();
            }
        }
        edited.validate()?;
        *self = edited;
        Ok(())
    }

    /// Bevel every edge of closed convex components with one segment. `amount`
    /// is the inset distance along each original face, in mesh coordinates.
    /// The original face planes stay fixed; shared edge strips and triangulated
    /// corner caps close the surface. Concave, nonplanar, collapsed, or overly
    /// complex inputs are rejected without changing the mesh.
    pub fn bevel_edges(&mut self, amount: f32) -> Result<()> {
        ensure!(
            amount.is_finite() && amount > 0.,
            "Bevel width must be finite and positive"
        );
        // Bevel creates one output vertex per input face corner. Reject an
        // impossible output before validation, cloning, and the much larger
        // edge/fan maps used by normal repair and topology analysis.
        let corners = self.faces.iter().try_fold(0usize, |count, face| {
            let count = count.saturating_add(face.len());
            ensure!(
                count <= GEOMETRY_LIMIT,
                "Bevel would exceed the two million vertex/face limit"
            );
            Ok::<usize, anyhow::Error>(count)
        })?;
        self.validate()?;
        let mut source = self.clone();
        source.recalculate_normals()?;
        let topology = Topology::new(&source)?;
        ensure!(
            topology.edges.values().all(|uses| uses.len() == 2),
            "Bevel requires a closed mesh; fill open boundaries first"
        );
        let (_, components) = topology.orientations()?;
        let used_vertices: BTreeSet<_> = source.faces.iter().flatten().copied().collect();
        let cap_count = corners - 2 * used_vertices.len();
        ensure!(
            source.faces.len() + topology.edges.len() + cap_count <= GEOMETRY_LIMIT,
            "Bevel would exceed the two million vertex/face limit"
        );

        let mut normals = Vec::with_capacity(source.faces.len());
        for face in &source.faces {
            normals.push(normal(&source, face));
        }
        let mut vertex_centers = vec![DVec3::ZERO; source.positions.len()];
        let mut face_tolerances = vec![0.; source.faces.len()];
        // A full supporting-plane check catches self-intersecting and concave
        // shells. Bound this quadratic check so an interactive bevel cannot
        // spend unbounded time on a densely tessellated imported object.
        let mut comparisons = 0usize;
        for component in &components {
            let vertices: BTreeSet<_> = component
                .iter()
                .flat_map(|&i| source.faces[i].iter().copied())
                .collect();
            comparisons =
                comparisons.saturating_add(component.len().saturating_mul(vertices.len()));
            ensure!(
                comparisons <= BEVEL_PLANE_TEST_LIMIT,
                "Bevel convexity check is too large; simplify the mesh first"
            );
            let mut center = DVec3::ZERO;
            for &vertex in &vertices {
                let p = source.positions[vertex as usize].as_dvec3();
                center += p;
            }
            center /= vertices.len() as f64;
            for &vertex in &vertices {
                vertex_centers[vertex as usize] = center;
            }
            for &index in component {
                let face = &source.faces[index];
                let origin = source.positions[face[0] as usize].as_dvec3();
                let n = normals[index];
                let mut low = f64::INFINITY;
                let mut high = f64::NEG_INFINITY;
                let mut roundoff = 0.0_f64;
                for &vertex in &vertices {
                    let delta = source.positions[vertex as usize].as_dvec3() - origin;
                    let distance = delta.dot(n);
                    low = low.min(distance);
                    high = high.max(distance);
                    roundoff = roundoff.max(delta.abs().dot(n.abs()));
                }
                // Measure thickness along this face's normal. Using the
                // longest object dimension would hide dents in thin parts.
                let tolerance = (high - low) * 1.0e-6 + roundoff * f64::EPSILON * 64.;
                face_tolerances[index] = tolerance;
                ensure!(
                    face.iter()
                        .all(|&v| (source.positions[v as usize].as_dvec3() - origin)
                            .dot(n)
                            .abs()
                            <= tolerance),
                    "Bevel requires planar faces; triangulate warped polygons first"
                );
                ensure!(
                    high <= tolerance,
                    "Bevel currently supports convex meshes; this mesh contains concave geometry"
                );
            }
        }

        let mut output = Self {
            positions: Vec::with_capacity(corners),
            faces: Vec::new(),
        };
        let mut offsets = Vec::with_capacity(source.faces.len());
        let mut starts = vec![None; source.positions.len()];
        let mut next_corner = vec![u32::MAX; corners];
        for (index, face) in source.faces.iter().enumerate() {
            let offset = output.positions.len() as u32;
            offsets.push(offset);
            let n = normals[index];
            let mut inset = Vec::with_capacity(face.len());
            for (corner, &vertex) in face.iter().enumerate() {
                let p = source.positions[vertex as usize].as_dvec3();
                let before = source.positions
                    [face[(corner + face.len() - 1) % face.len()] as usize]
                    .as_dvec3();
                let after = source.positions[face[(corner + 1) % face.len()] as usize].as_dvec3();
                let a = (p - before).normalize();
                let b = (after - p).normalize();
                ensure!(
                    a.cross(b).dot(n) > 1.0e-10,
                    "Bevel requires strictly convex face corners; dissolve straight or concave corners first"
                );
                let denominator = 1. + a.dot(b);
                ensure!(denominator > 1.0e-12, "Bevel face corner is too narrow");
                let position = p + (n.cross(a) + n.cross(b)) * (f64::from(amount) / denominator);
                let stored = position.as_vec3();
                ensure!(
                    stored.is_finite()
                        && stored.abs().max_element() <= 1.0e8
                        && stored != source.positions[vertex as usize],
                    "Bevel width exceeds coordinate range or is too small for this mesh's precision"
                );
                inset.push(position);
                starts[vertex as usize] = Some(offset + corner as u32);
                output.positions.push(stored);
            }
            for corner in 0..face.len() {
                let next = (corner + 1) % face.len();
                let p = source.positions[face[corner] as usize].as_dvec3();
                let edge = source.positions[face[next] as usize].as_dvec3() - p;
                ensure!(
                    (inset[next] - inset[corner]).dot(edge) > edge.length_squared() * 1.0e-10,
                    "Bevel width is too large: a face would collapse"
                );
                let inward = n.cross(edge.normalize());
                let tolerance = face_tolerances[index].min(f64::from(amount) * 1.0e-5);
                ensure!(
                    inset
                        .iter()
                        .all(|&q| (q - p).dot(inward) >= f64::from(amount) - tolerance),
                    "Bevel width is too large for a narrow face"
                );
            }
            output
                .faces
                .push((0..face.len()).map(|i| offset + i as u32).collect());
        }
        for uses in topology.edges.values() {
            let [a, b] = [uses[0], uses[1]];
            let a_start = offsets[a.face] + a.corner as u32;
            let a_end = offsets[a.face] + ((a.corner + 1) % source.faces[a.face].len()) as u32;
            let b_start = offsets[b.face] + b.corner as u32;
            let b_end = offsets[b.face] + ((b.corner + 1) % source.faces[b.face].len()) as u32;
            output.faces.push(vec![a_end, a_start, b_end, b_start]);
            next_corner[b_end as usize] = a_start;
            next_corner[a_end as usize] = b_start;
        }
        let mut scratch = Triangulation::default();
        for (vertex, start) in starts.into_iter().enumerate() {
            let Some(start) = start else { continue };
            let mut cap = vec![start];
            let mut current = next_corner[start as usize];
            while current != start {
                ensure!(
                    current != u32::MAX && cap.len() < 4096,
                    "Bevel vertex has unsupported corner topology"
                );
                cap.push(current);
                current = next_corner[current as usize];
            }
            ensure!(
                cap.len() >= 3,
                "Bevel needs at least three faces around each vertex"
            );
            let triangles = output.triangulate_face(&cap, &mut scratch);
            ensure!(
                triangles.len() == cap.len() - 2,
                "Bevel corner cannot be triangulated; reduce the width"
            );
            let outward = source.positions[vertex].as_dvec3() - vertex_centers[vertex];
            for &[a, b, c] in triangles {
                let [p, q, r] = [a, b, c].map(|i| output.positions[i as usize].as_dvec3());
                ensure!(
                    (q - p).cross(r - p).dot(outward) > 0.,
                    "Bevel corner would fold over; reduce the width"
                );
                output.faces.push(vec![a, b, c]);
            }
        }
        output.validate()?;
        *self = output;
        Ok(())
    }
}

fn check_selection(mesh: &Mesh, selected: &BTreeSet<usize>) -> Result<()> {
    ensure!(!selected.is_empty(), "Select faces first");
    ensure!(
        selected.iter().all(|&i| i < mesh.faces.len()),
        "Selection contains a missing face"
    );
    Ok(())
}

fn normal(mesh: &Mesh, face: &[u32]) -> DVec3 {
    let origin = mesh.positions[face[0] as usize].as_dvec3();
    face[1..]
        .windows(2)
        .map(|pair| {
            let a = mesh.positions[pair[0] as usize].as_dvec3() - origin;
            let b = mesh.positions[pair[1] as usize].as_dvec3() - origin;
            a.cross(b)
        })
        .sum::<DVec3>()
        .normalize()
}

#[derive(Clone, Copy)]
struct EdgeUse {
    face: usize,
    corner: usize,
    from: u32,
    to: u32,
}

struct Topology {
    edges: BTreeMap<[u32; 2], Vec<EdgeUse>>,
    adjacent: Vec<Vec<(usize, bool)>>,
    boundary_faces: Vec<bool>,
}

impl Topology {
    /// Check both edge incidence and each vertex's connected fan. Merely
    /// checking two faces per edge misses shells pinched at a shared vertex.
    fn new(mesh: &Mesh) -> Result<Self> {
        let mut edges: BTreeMap<[u32; 2], Vec<EdgeUse>> = BTreeMap::new();
        let mut offsets = Vec::with_capacity(mesh.faces.len());
        let mut corners = 0;
        for (face, vertices) in mesh.faces.iter().enumerate() {
            offsets.push(corners);
            corners += vertices.len();
            for (corner, &from) in vertices.iter().enumerate() {
                let to = vertices[(corner + 1) % vertices.len()];
                let uses = edges.entry([from.min(to), from.max(to)]).or_default();
                uses.push(EdgeUse {
                    face,
                    corner,
                    from,
                    to,
                });
                ensure!(
                    uses.len() <= 2,
                    "Mesh has a non-manifold edge shared by more than two faces"
                );
            }
        }
        let mut adjacent = vec![Vec::new(); mesh.faces.len()];
        let mut boundary_faces = vec![false; mesh.faces.len()];
        let mut boundary_vertices = vec![0; mesh.positions.len()];
        let mut fans = DisjointSet::new(corners);
        for uses in edges.values() {
            if uses.len() == 1 {
                let edge = uses[0];
                boundary_faces[edge.face] = true;
                boundary_vertices[edge.from as usize] += 1;
                boundary_vertices[edge.to as usize] += 1;
                continue;
            }
            let [a, b] = [uses[0], uses[1]];
            let same_direction = a.from == b.from;
            adjacent[a.face].push((b.face, same_direction));
            adjacent[b.face].push((a.face, same_direction));
            let a_start = offsets[a.face] + a.corner;
            let a_end = offsets[a.face] + (a.corner + 1) % mesh.faces[a.face].len();
            let b_start = offsets[b.face] + b.corner;
            let b_end = offsets[b.face] + (b.corner + 1) % mesh.faces[b.face].len();
            if same_direction {
                fans.join(a_start, b_start);
                fans.join(a_end, b_end);
            } else {
                fans.join(a_start, b_end);
                fans.join(a_end, b_start);
            }
        }
        ensure!(
            boundary_vertices
                .iter()
                .all(|&count| count == 0 || count == 2),
            "Mesh has a non-manifold boundary vertex"
        );
        let mut roots = vec![None; mesh.positions.len()];
        for (face, vertices) in mesh.faces.iter().enumerate() {
            for (corner, &vertex) in vertices.iter().enumerate() {
                let root = fans.root(offsets[face] + corner);
                if let Some(existing) = roots[vertex as usize] {
                    ensure!(
                        existing == root,
                        "Mesh has a non-manifold vertex shared by disconnected surfaces"
                    );
                } else {
                    roots[vertex as usize] = Some(root);
                }
            }
        }
        Ok(Self {
            edges,
            adjacent,
            boundary_faces,
        })
    }

    fn orientations(&self) -> Result<(Vec<bool>, Vec<Vec<usize>>)> {
        let mut flips = vec![None; self.adjacent.len()];
        let mut components = Vec::new();
        for seed in 0..self.adjacent.len() {
            if flips[seed].is_some() {
                continue;
            }
            flips[seed] = Some(false);
            let mut queue = VecDeque::from([seed]);
            let mut component = Vec::new();
            while let Some(face) = queue.pop_front() {
                component.push(face);
                let current = flips[face].unwrap();
                for &(neighbor, opposite) in &self.adjacent[face] {
                    let expected = current ^ opposite;
                    if let Some(existing) = flips[neighbor] {
                        ensure!(
                            existing == expected,
                            "Mesh is non-orientable; its face winding cannot be made consistent"
                        );
                    } else {
                        flips[neighbor] = Some(expected);
                        queue.push_back(neighbor);
                    }
                }
            }
            components.push(component);
        }
        Ok((flips.into_iter().map(Option::unwrap).collect(), components))
    }
}

struct DisjointSet {
    parents: Vec<usize>,
    rank: Vec<u8>,
}

impl DisjointSet {
    fn new(count: usize) -> Self {
        Self {
            parents: (0..count).collect(),
            rank: vec![0; count],
        }
    }

    fn root(&mut self, mut value: usize) -> usize {
        while self.parents[value] != value {
            self.parents[value] = self.parents[self.parents[value]];
            value = self.parents[value];
        }
        value
    }

    fn join(&mut self, a: usize, b: usize) {
        let (mut a, mut b) = (self.root(a), self.root(b));
        if a == b {
            return;
        }
        if self.rank[a] < self.rank[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parents[b] = a;
        if self.rank[a] == self.rank[b] {
            self.rank[a] += 1;
        }
    }
}

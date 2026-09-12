use anyhow::{Result, ensure};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    Cube,
    Sphere,
    Cylinder,
    Torus,
    Plane,
}

impl Primitive {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cube => "Cube",
            Self::Sphere => "Sphere",
            Self::Cylinder => "Cylinder",
            Self::Torus => "Torus",
            Self::Plane => "Plane",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub faces: Vec<Vec<u32>>,
}

impl Mesh {
    pub fn primitive(primitive: Primitive) -> Self {
        match primitive {
            Primitive::Cube => Self::cube(),
            Primitive::Sphere => Self::sphere(32, 16),
            Primitive::Cylinder => Self::cylinder(32),
            Primitive::Torus => Self::torus(48, 16),
            Primitive::Plane => Self {
                positions: vec![
                    Vec3::new(-1., 0., -1.),
                    Vec3::new(-1., 0., 1.),
                    Vec3::new(1., 0., 1.),
                    Vec3::new(1., 0., -1.),
                ],
                faces: vec![vec![0, 1, 2, 3]],
            },
        }
    }

    /// Ear clipping preserves the winding of simple concave polygons. Quads
    /// and larger faces stay polygons in the editable mesh.
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        self.faces
            .iter()
            .flat_map(|face| self.triangulate_face(face))
            .collect()
    }

    /// Triangles with their original polygon index, for face picking.
    pub fn triangles_with_faces(&self) -> Vec<([u32; 3], usize)> {
        self.faces
            .iter()
            .enumerate()
            .flat_map(|(index, face)| {
                self.triangulate_face(face)
                    .into_iter()
                    .map(move |triangle| (triangle, index))
            })
            .collect()
    }

    pub fn edges(&self) -> Vec<[u32; 2]> {
        let mut edges = BTreeSet::new();
        for face in &self.faces {
            for (&a, &b) in face
                .iter()
                .zip(face.iter().cycle().skip(1))
                .take(face.len())
            {
                if a != b {
                    edges.insert(edge_key(a, b));
                }
            }
        }
        edges.into_iter().map(|(a, b)| [a, b]).collect()
    }

    pub fn face_normal(&self, face: usize) -> Vec3 {
        self.faces
            .get(face)
            .map(|face| polygon_normal(&self.positions, face).normalize_or_zero())
            .unwrap_or(Vec3::ZERO)
    }

    /// Catmull–Clark subdivision, including the cubic boundary rule for open
    /// meshes. A non-manifold edge is held at its midpoint. On invalid input or
    /// exhausted geometry budget, leaves the mesh unchanged; interactive tools
    /// should use `subdivide_checked` to present the error to the user.
    pub fn subdivide(&mut self) {
        let _ = self.subdivide_checked();
    }

    pub fn subdivide_checked(&mut self) -> Result<()> {
        self.validate()?;
        let output_faces: usize = self.faces.iter().map(Vec::len).sum();
        ensure!(
            output_faces <= 2_000_000,
            "Subdivision would exceed the two million face limit"
        );
        let face_points: Vec<Vec3> = self
            .faces
            .iter()
            .map(|face| {
                face.iter()
                    .map(|&i| self.positions[i as usize])
                    .sum::<Vec3>()
                    / face.len() as f32
            })
            .collect();
        let mut edge_faces: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
        let mut vertex_faces = vec![Vec::new(); self.positions.len()];
        let mut vertex_edges = vec![Vec::new(); self.positions.len()];
        for (index, face) in self.faces.iter().enumerate() {
            for (&a, &b) in face
                .iter()
                .zip(face.iter().cycle().skip(1))
                .take(face.len())
            {
                edge_faces.entry(edge_key(a, b)).or_default().push(index);
                vertex_faces[a as usize].push(index);
            }
        }
        for &(a, b) in edge_faces.keys() {
            vertex_edges[a as usize].push(b);
            vertex_edges[b as usize].push(a);
        }
        ensure!(
            self.positions.len() + edge_faces.len() + self.faces.len() <= 2_000_000,
            "Subdivision would exceed the two million vertex limit"
        );
        let mut positions = self.positions.clone();
        for (index, &point) in self.positions.iter().enumerate() {
            let neighbors = &vertex_edges[index];
            if neighbors.is_empty() {
                continue;
            }
            let boundary: Vec<_> = neighbors
                .iter()
                .copied()
                .filter(|&other| edge_faces[&edge_key(index as u32, other)].len() == 1)
                .collect();
            positions[index] = if boundary.len() == 2 {
                (point * 6.0
                    + self.positions[boundary[0] as usize]
                    + self.positions[boundary[1] as usize])
                    / 8.0
            } else if boundary.is_empty()
                && vertex_faces[index].len() == neighbors.len()
                && neighbors
                    .iter()
                    .all(|&other| edge_faces[&edge_key(index as u32, other)].len() == 2)
            {
                let n = neighbors.len() as f32;
                let f = vertex_faces[index]
                    .iter()
                    .map(|&face| face_points[face])
                    .sum::<Vec3>()
                    / n;
                let r = neighbors
                    .iter()
                    .map(|&other| (point + self.positions[other as usize]) * 0.5)
                    .sum::<Vec3>()
                    / n;
                (f + 2.0 * r + (n - 3.0) * point) / n
            } else {
                point
            };
        }
        let mut edge_indices = BTreeMap::new();
        for (&(a, b), adjacent) in &edge_faces {
            let midpoint = self.positions[a as usize] + self.positions[b as usize];
            let point = if adjacent.len() == 2 {
                (midpoint + face_points[adjacent[0]] + face_points[adjacent[1]]) * 0.25
            } else {
                midpoint * 0.5
            };
            edge_indices.insert((a, b), positions.len() as u32);
            positions.push(point);
        }
        let face_offset = positions.len() as u32;
        positions.extend(face_points);
        let mut faces = Vec::with_capacity(output_faces);
        for (index, face) in self.faces.iter().enumerate() {
            for (corner, &a) in face.iter().enumerate() {
                let b = face[(corner + 1) % face.len()];
                let previous = face[(corner + face.len() - 1) % face.len()];
                faces.push(vec![
                    a,
                    edge_indices[&edge_key(a, b)],
                    face_offset + index as u32,
                    edge_indices[&edge_key(previous, a)],
                ]);
            }
        }
        let output = Self { positions, faces };
        output.validate()?;
        *self = output;
        Ok(())
    }

    /// Move a polygon along its outward normal and connect its old boundary to
    /// the new cap. The original polygon is replaced, never left inside a solid.
    pub fn extrude_face(&mut self, face: usize, distance: f32) -> Result<()> {
        ensure!(
            distance.is_finite() && distance.abs() > 1.0e-6,
            "Extrusion distance must be finite and nonzero"
        );
        self.validate()?;
        let source = self
            .faces
            .get(face)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Face does not exist"))?;
        let normal = polygon_normal(&self.positions, &source).normalize_or_zero();
        ensure!(normal != Vec3::ZERO, "Cannot extrude a degenerate face");
        let first = self.positions.len() as u32;
        ensure!(
            self.positions.len() + source.len() <= 2_000_000
                && self.faces.len() + source.len() <= 2_000_000,
            "Mesh vertex/face limit exceeded"
        );
        let new_positions: Vec<_> = source
            .iter()
            .map(|&index| self.positions[index as usize] + normal * distance)
            .collect();
        ensure!(
            new_positions
                .iter()
                .zip(&source)
                .all(|(&point, &index)| point.is_finite()
                    && point.abs().max_element() <= 1.0e8
                    && point != self.positions[index as usize]),
            "Extrusion exceeds mesh coordinate range or precision"
        );
        self.positions.extend(new_positions);
        self.faces[face] = (0..source.len()).map(|i| first + i as u32).collect();
        for (i, &a) in source.iter().enumerate() {
            let j = (i + 1) % source.len();
            self.faces
                .push(vec![a, source[j], first + j as u32, first + i as u32]);
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(!self.positions.is_empty(), "Mesh has no vertices");
        ensure!(!self.faces.is_empty(), "Mesh has no faces");
        ensure!(
            self.positions.len() <= 2_000_000 && self.faces.len() <= 2_000_000,
            "Mesh exceeds the two million vertex/face limit"
        );
        ensure!(
            self.positions
                .iter()
                .all(|p| p.is_finite() && p.abs().max_element() <= 1.0e8),
            "Mesh contains a non-finite or out-of-range position"
        );
        for (index, face) in self.faces.iter().enumerate() {
            ensure!(
                (3..=4096).contains(&face.len()),
                "Face {index} must have 3–4096 vertices"
            );
            ensure!(
                face.iter().all(|&i| (i as usize) < self.positions.len()),
                "Face {index} references a missing vertex"
            );
            let unique: BTreeSet<_> = face.iter().collect();
            ensure!(unique.len() == face.len(), "Face {index} repeats a vertex");
            ensure!(
                polygon_normal(&self.positions, face).length_squared() > 1.0e-20,
                "Face {index} has zero area"
            );
            ensure!(
                simple_polygon(&self.project_face(face)),
                "Face {index} has overlapping or intersecting edges"
            );
            ensure!(
                self.triangulate_face(face).len() == face.len() - 2,
                "Face {index} is self-intersecting or cannot be triangulated"
            );
        }
        Ok(())
    }

    fn triangulate_face(&self, face: &[u32]) -> Vec<[u32; 3]> {
        if face.len() < 3 || face.iter().any(|&i| i as usize >= self.positions.len()) {
            return Vec::new();
        }
        if face.len() == 3 {
            return vec![[face[0], face[1], face[2]]];
        }
        let points = self.project_face(face);
        let area: f64 = points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .take(points.len())
            .map(|(a, b)| a.x as f64 * b.y as f64 - a.y as f64 * b.x as f64)
            .sum();
        if area.abs() < 1.0e-20 {
            return Vec::new();
        }
        let sign = area.signum();
        let mut remaining: Vec<usize> = (0..face.len()).collect();
        let mut triangles = Vec::with_capacity(face.len() - 2);
        while remaining.len() > 3 {
            let mut found = false;
            for i in 0..remaining.len() {
                let a = remaining[(i + remaining.len() - 1) % remaining.len()];
                let b = remaining[i];
                let c = remaining[(i + 1) % remaining.len()];
                if cross(points[a], points[b], points[c]) * sign <= 1.0e-14 {
                    continue;
                }
                if remaining.iter().any(|&p| {
                    p != a
                        && p != b
                        && p != c
                        && in_triangle(points[p], points[a], points[b], points[c], sign)
                }) {
                    continue;
                }
                triangles.push([face[a], face[b], face[c]]);
                remaining.remove(i);
                found = true;
                break;
            }
            if !found {
                return Vec::new();
            }
        }
        if cross(
            points[remaining[0]],
            points[remaining[1]],
            points[remaining[2]],
        ) * sign
            <= 1.0e-14
        {
            return Vec::new();
        }
        triangles.push([face[remaining[0]], face[remaining[1]], face[remaining[2]]]);
        triangles
    }

    fn project_face(&self, face: &[u32]) -> Vec<Vec2> {
        let normal = polygon_normal(&self.positions, face).abs();
        let axis = if normal.x >= normal.y && normal.x >= normal.z {
            0
        } else if normal.y >= normal.z {
            1
        } else {
            2
        };
        let origin = self.positions[face[0] as usize];
        face.iter()
            .map(|&index| {
                let p = self.positions[index as usize] - origin;
                match axis {
                    0 => Vec2::new(p.y, p.z),
                    1 => Vec2::new(p.z, p.x),
                    _ => Vec2::new(p.x, p.y),
                }
            })
            .collect()
    }

    fn cube() -> Self {
        Self {
            positions: vec![
                Vec3::new(-1., -1., -1.),
                Vec3::new(1., -1., -1.),
                Vec3::new(1., 1., -1.),
                Vec3::new(-1., 1., -1.),
                Vec3::new(-1., -1., 1.),
                Vec3::new(1., -1., 1.),
                Vec3::new(1., 1., 1.),
                Vec3::new(-1., 1., 1.),
            ],
            faces: vec![
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![3, 7, 6, 2],
                vec![0, 4, 7, 3],
                vec![1, 2, 6, 5],
            ],
        }
    }

    fn sphere(segments: u32, rings: u32) -> Self {
        let mut positions = vec![Vec3::Y];
        for ring in 1..rings {
            let theta = std::f32::consts::PI * ring as f32 / rings as f32;
            for segment in 0..segments {
                let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
                positions.push(Vec3::new(
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ));
            }
        }
        let south = positions.len() as u32;
        positions.push(-Vec3::Y);
        let mut faces = Vec::new();
        for s in 0..segments {
            let n = (s + 1) % segments;
            faces.push(vec![0, 1 + n, 1 + s]);
            for r in 0..rings - 2 {
                let row = 1 + r * segments;
                faces.push(vec![
                    row + s,
                    row + n,
                    row + segments + n,
                    row + segments + s,
                ]);
            }
            let last = 1 + (rings - 2) * segments;
            faces.push(vec![south, last + s, last + n]);
        }
        Self { positions, faces }
    }

    fn cylinder(segments: u32) -> Self {
        let mut positions = Vec::new();
        for y in [-1.0, 1.0] {
            for segment in 0..segments {
                let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
                positions.push(Vec3::new(phi.cos(), y, phi.sin()));
            }
        }
        let mut faces = Vec::new();
        for s in 0..segments {
            let n = (s + 1) % segments;
            faces.push(vec![s, s + segments, n + segments, n]);
        }
        faces.push((0..segments).collect());
        faces.push((segments..segments * 2).rev().collect());
        Self { positions, faces }
    }

    fn torus(segments: u32, sides: u32) -> Self {
        let mut positions = Vec::new();
        for segment in 0..segments {
            let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
            for side in 0..sides {
                let theta = std::f32::consts::TAU * side as f32 / sides as f32;
                let radial = 0.85 + 0.28 * theta.cos();
                positions.push(Vec3::new(
                    radial * phi.cos(),
                    0.28 * theta.sin(),
                    radial * phi.sin(),
                ));
            }
        }
        let mut faces = Vec::new();
        for s in 0..segments {
            for t in 0..sides {
                let next_s = (s + 1) % segments;
                let next_t = (t + 1) % sides;
                faces.push(vec![
                    s * sides + t,
                    s * sides + next_t,
                    next_s * sides + next_t,
                    next_s * sides + t,
                ]);
            }
        }
        Self { positions, faces }
    }
}

fn edge_key(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}

pub(crate) fn polygon_normal(positions: &[Vec3], face: &[u32]) -> Vec3 {
    // Cross products relative to the first vertex reduce cancellation for meshes
    // far from the world origin (the usual Newell sum is translation sensitive).
    let Some(&first) = face.first() else {
        return Vec3::ZERO;
    };
    let Some(&origin) = positions.get(first as usize) else {
        return Vec3::ZERO;
    };
    let mut normal = Vec3::ZERO;
    for pair in face[1..].windows(2) {
        let (Some(&a), Some(&b)) = (
            positions.get(pair[0] as usize),
            positions.get(pair[1] as usize),
        ) else {
            return Vec3::ZERO;
        };
        normal += (a - origin).cross(b - origin);
    }
    normal
}

fn cross(a: Vec2, b: Vec2, c: Vec2) -> f64 {
    (b.x as f64 - a.x as f64) * (c.y as f64 - a.y as f64)
        - (b.y as f64 - a.y as f64) * (c.x as f64 - a.x as f64)
}

fn in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2, sign: f64) -> bool {
    cross(a, b, p) * sign >= -1.0e-14
        && cross(b, c, p) * sign >= -1.0e-14
        && cross(c, a, p) * sign >= -1.0e-14
}

fn simple_polygon(points: &[Vec2]) -> bool {
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        if a == b {
            return false;
        }
        for j in i + 1..points.len() {
            if j == i + 1 || (i == 0 && j == points.len() - 1) {
                continue;
            }
            let c = points[j];
            let d = points[(j + 1) % points.len()];
            let (ab_c, ab_d, cd_a, cd_b) = (
                cross(a, b, c),
                cross(a, b, d),
                cross(c, d, a),
                cross(c, d, b),
            );
            if (ab_c > 0.0) != (ab_d > 0.0) && (cd_a > 0.0) != (cd_b > 0.0) {
                return false;
            }
            if (ab_c == 0.0 && on_segment(a, b, c))
                || (ab_d == 0.0 && on_segment(a, b, d))
                || (cd_a == 0.0 && on_segment(c, d, a))
                || (cd_b == 0.0 && on_segment(c, d, b))
            {
                return false;
            }
        }
    }
    true
}

fn on_segment(a: Vec2, b: Vec2, p: Vec2) -> bool {
    p.cmpge(a.min(b)).all() && p.cmple(a.max(b)).all()
}

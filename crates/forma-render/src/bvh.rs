use std::collections::HashSet;

use bytemuck::{Pod, Zeroable};
use forma_core::Scene;
use glam::Vec3;

/// Explicit float4 alignment mirrors Metal; no implicit host padding.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub(crate) struct Triangle {
    pub v0: [f32; 4],
    pub v1: [f32; 4],
    pub v2: [f32; 4],
    pub n0: [f32; 4],
    pub n1: [f32; 4],
    pub n2: [f32; 4],
    pub base_color: [f32; 4],
    pub emission: [f32; 4],
    pub params: [f32; 4],
}

impl Triangle {
    fn bounds(&self) -> Bounds {
        let a = Vec3::from_slice(&self.v0);
        let b = Vec3::from_slice(&self.v1);
        let c = Vec3::from_slice(&self.v2);
        Bounds {
            min: a.min(b).min(c),
            max: a.max(b).max(c),
        }
    }
    fn center(&self) -> Vec3 {
        (Vec3::from_slice(&self.v0) + Vec3::from_slice(&self.v1) + Vec3::from_slice(&self.v2)) / 3.0
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub(crate) struct Node {
    pub min: [f32; 4],
    pub max: [f32; 4],
    /// Interior: left, right, 0. Leaf: first triangle, count, 1.
    pub data: [u32; 4],
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}
impl Bounds {
    fn empty() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }
    fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }
    fn area(self) -> f32 {
        let d = (self.max - self.min).max(Vec3::ZERO);
        2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
    }
}

pub(crate) struct Geometry {
    pub triangles: Vec<Triangle>,
    pub nodes: Vec<Node>,
    pub lights: Vec<u32>,
}

fn vec4(v: Vec3) -> [f32; 4] {
    [v.x, v.y, v.z, 0.0]
}
fn edge_key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

impl Geometry {
    pub fn from_scene(scene: &Scene) -> Self {
        let mut triangles = Vec::new();
        for (object_index, object) in scene
            .objects
            .iter()
            .enumerate()
            .filter(|(_, object)| object.visible)
        {
            let transform = object.transform.matrix();
            let normal_transform = transform.inverse().transpose();
            let mesh = &object.mesh;
            let indices = mesh.triangles();
            let positions: Vec<_> = mesh
                .positions
                .iter()
                .map(|p| transform.transform_point3(*p))
                .collect();
            let edges: HashSet<_> = mesh
                .edges()
                .into_iter()
                .map(|[a, b]| edge_key(a, b))
                .collect();
            // Classify creases and angle-weight in mesh space, then transform
            // normals with the inverse transpose. Nonuniform object scaling must
            // not change smoothing groups or create seams on a flattened sphere.
            let mut adjacent = vec![Vec::<(Vec3, f32)>::new(); positions.len()];
            let face_normals: Vec<_> = indices
                .iter()
                .map(|face| {
                    let [a, b, c] = face.map(|index| mesh.positions[index as usize]);
                    let n = (b - a).cross(c - a).normalize_or_zero();
                    for corner in 0..3 {
                        let p = mesh.positions[face[corner] as usize];
                        let e0 = (mesh.positions[face[(corner + 1) % 3] as usize] - p)
                            .normalize_or_zero();
                        let e1 = (mesh.positions[face[(corner + 2) % 3] as usize] - p)
                            .normalize_or_zero();
                        let angle = e0.dot(e1).clamp(-1.0, 1.0).acos();
                        adjacent[face[corner] as usize].push((n, angle));
                    }
                    n
                })
                .collect();
            for (face, geometric) in indices.iter().zip(face_normals) {
                if geometric.length_squared() < 0.5 {
                    continue;
                }
                let normals = face.map(|index| {
                    let normal = adjacent[index as usize]
                        .iter()
                        .filter(|(normal, _)| {
                            normal.dot(geometric) > std::f32::consts::FRAC_1_SQRT_2
                        })
                        .map(|(normal, angle)| *normal * *angle)
                        .sum::<Vec3>()
                        .normalize_or_zero();
                    normal_transform
                        .transform_vector3(normal)
                        .normalize_or_zero()
                });
                let mut edge_bits = 0u32;
                for corner in 0..3 {
                    if edges.contains(&edge_key(face[(corner + 1) % 3], face[(corner + 2) % 3])) {
                        edge_bits |= 1 << corner;
                    }
                }
                triangles.push(Triangle {
                    v0: vec4(positions[face[0] as usize]),
                    v1: vec4(positions[face[1] as usize]),
                    v2: vec4(positions[face[2] as usize]),
                    n0: vec4(normals[0]),
                    n1: vec4(normals[1]),
                    n2: vec4(normals[2]),
                    base_color: vec4(object.material.base_color.clamp(Vec3::ZERO, Vec3::ONE)),
                    emission: vec4(object.material.emission.max(Vec3::ZERO)),
                    params: [
                        object.material.metallic.clamp(0.0, 1.0),
                        object.material.roughness.clamp(0.02, 1.0),
                        (object_index + 1) as f32,
                        edge_bits as f32,
                    ],
                });
            }
        }
        Self::build(triangles)
    }

    fn build(mut triangles: Vec<Triangle>) -> Self {
        let mut nodes = Vec::with_capacity(triangles.len().max(1));
        if !triangles.is_empty() {
            build_node(&mut triangles, 0, &mut nodes, 0);
        }
        let lights = triangles
            .iter()
            .enumerate()
            .filter(|(_, tri)| Vec3::from_slice(&tri.emission).max_element() > 0.0)
            .map(|(i, _)| i as u32)
            .collect();
        Self {
            triangles,
            nodes,
            lights,
        }
    }
}

/// Binned SAH partitions reduce ray traversal cost without expensive full sorting.
/// A depth cap keeps the shader's bounded traversal stack provably sufficient.
fn build_node(
    triangles: &mut [Triangle],
    offset: usize,
    nodes: &mut Vec<Node>,
    depth: usize,
) -> u32 {
    let bounds = triangles
        .iter()
        .fold(Bounds::empty(), |bounds, tri| bounds.union(tri.bounds()));
    let id = nodes.len() as u32;
    nodes.push(Node {
        min: vec4(bounds.min - Vec3::splat(1e-5)),
        max: vec4(bounds.max + Vec3::splat(1e-5)),
        data: [offset as u32, triangles.len() as u32, 1, 0],
    });
    if triangles.len() <= 4 || depth >= 48 {
        return id;
    }
    let centroids = triangles.iter().fold(Bounds::empty(), |bounds, tri| {
        let center = tri.center();
        bounds.union(Bounds {
            min: center,
            max: center,
        })
    });
    const BINS: usize = 12;
    let mut best = (f32::INFINITY, 0, 0);
    for axis in 0..3 {
        let span = centroids.max[axis] - centroids.min[axis];
        if span <= 1e-7 {
            continue;
        }
        let mut bins = [(Bounds::empty(), 0usize); BINS];
        for tri in triangles.iter() {
            let bin = (((tri.center()[axis] - centroids.min[axis]) / span * BINS as f32) as usize)
                .min(BINS - 1);
            bins[bin].0 = bins[bin].0.union(tri.bounds());
            bins[bin].1 += 1;
        }
        for split in 1..BINS {
            let mut left = (Bounds::empty(), 0usize);
            let mut right = left;
            for bin in &bins[..split] {
                left.0 = left.0.union(bin.0);
                left.1 += bin.1;
            }
            for bin in &bins[split..] {
                right.0 = right.0.union(bin.0);
                right.1 += bin.1;
            }
            if left.1 == 0 || right.1 == 0 {
                continue;
            }
            let cost = left.0.area() * left.1 as f32 + right.0.area() * right.1 as f32;
            if cost < best.0 {
                best = (cost, axis, split);
            }
        }
    }
    let middle = if best.0.is_finite() {
        let (_, axis, split) = best;
        let split_at = centroids.min[axis]
            + (centroids.max[axis] - centroids.min[axis]) * split as f32 / BINS as f32;
        let mut left = 0;
        for i in 0..triangles.len() {
            if triangles[i].center()[axis] < split_at {
                triangles.swap(left, i);
                left += 1;
            }
        }
        left
    } else {
        triangles.len() / 2
    };
    if middle == 0 || middle == triangles.len() {
        return id;
    }
    let (left, right) = triangles.split_at_mut(middle);
    let left = build_node(left, offset, nodes, depth + 1);
    let right = build_node(right, offset + middle, nodes, depth + 1);
    nodes[id as usize].data = [left, right, 0, 0];
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::Primitive;

    #[test]
    fn gpu_layout_has_no_implicit_padding() {
        assert_eq!(std::mem::size_of::<Triangle>(), 9 * 16);
        assert_eq!(std::mem::size_of::<Node>(), 3 * 16);
    }

    #[test]
    fn leaves_cover_each_triangle_once_and_enclose_vertices() {
        let mut scene = Scene::default();
        scene.add(Primitive::Sphere);
        let geometry = Geometry::from_scene(&scene);
        let mut coverage = vec![0; geometry.triangles.len()];
        for node in &geometry.nodes {
            if node.data[2] == 0 {
                continue;
            }
            for i in node.data[0]..node.data[0] + node.data[1] {
                coverage[i as usize] += 1;
                for p in [
                    geometry.triangles[i as usize].v0,
                    geometry.triangles[i as usize].v1,
                    geometry.triangles[i as usize].v2,
                ] {
                    let p = Vec3::from_slice(&p);
                    assert!(p.cmpge(Vec3::from_slice(&node.min)).all());
                    assert!(p.cmple(Vec3::from_slice(&node.max)).all());
                }
            }
        }
        assert!(coverage.iter().all(|count| *count == 1));
    }

    #[test]
    fn quad_triangulation_does_not_create_wire_diagonal() {
        let mut scene = Scene::default();
        scene.objects.clear();
        scene.add(Primitive::Plane);
        let geometry = Geometry::from_scene(&scene);
        assert_eq!(geometry.triangles.len(), 2);
        assert!(
            geometry
                .triangles
                .iter()
                .all(|tri| (tri.params[3] as u32).count_ones() == 2)
        );
    }

    #[test]
    fn nonuniform_object_scale_preserves_smooth_vertex_normals() {
        let mut scene = Scene::default();
        scene.objects.clear();
        let id = scene.add(Primitive::Sphere);
        scene.object_mut(id).unwrap().transform.scale = Vec3::new(1.0, 0.1, 1.0);
        let geometry = Geometry::from_scene(&scene);
        let mut normals = std::collections::HashMap::<[u32; 3], Vec3>::new();
        for triangle in &geometry.triangles {
            for (position, normal) in [
                (triangle.v0, triangle.n0),
                (triangle.v1, triangle.n1),
                (triangle.v2, triangle.n2),
            ] {
                let key = [
                    position[0].to_bits(),
                    position[1].to_bits(),
                    position[2].to_bits(),
                ];
                let normal = Vec3::from_slice(&normal);
                if let Some(previous) = normals.insert(key, normal) {
                    assert!(
                        previous.dot(normal) > 0.9999,
                        "Object scaling introduced a shading crease: {previous:?} vs {normal:?}"
                    );
                }
            }
        }
    }
}

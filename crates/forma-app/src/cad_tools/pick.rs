//! Picking faces under the pointer through a bounding volume hierarchy.
//!
//! `Scene::pick` tests every triangle of every mesh, which is fine for a
//! click but not for hover on every pointer move over a million-triangle
//! part. The tools build this index once per scene revision (for one object
//! when only that object matters) and query it per move.
use forma_core::{Ray, Scene, Vec3};
use glam::DVec3;
use std::ops::Range;

/// A pick result: the object, its polygon index and the world hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PickHit {
    pub object: u64,
    pub face: usize,
    pub distance: f32,
    pub position: Vec3,
}

struct Triangle {
    points: [Vec3; 3],
    low: Vec3,
    high: Vec3,
    object: u64,
    face: u32,
}

enum Contents {
    Triangles(Range<usize>),
    Children([usize; 2]),
}

struct Node {
    low: Vec3,
    high: Vec3,
    contents: Contents,
}

pub(crate) struct PickIndex {
    triangles: Vec<Triangle>,
    nodes: Vec<Node>,
}

impl PickIndex {
    /// Selectable, visible meshes of `scene` (only `object`, when given), as
    /// `Scene::pick` sees them.
    pub(crate) fn new(scene: &Scene, object: Option<u64>) -> Self {
        let mut triangles = Vec::new();
        for instance in scene.mesh_instances().filter(|i| {
            i.selectable
                && scene.is_effectively_visible(i.id)
                && object.is_none_or(|object| object == i.id)
        }) {
            let matrix = instance.world_transform;
            if !matrix.is_finite() || matrix.determinant() == 0.0 {
                continue;
            }
            let world: Vec<Vec3> = instance
                .mesh
                .positions
                .iter()
                .map(|p| matrix.transform_point3(*p))
                .collect();
            for (triangle, face) in instance.mesh.triangles_with_faces() {
                let points = triangle.map(|i| world[i as usize]);
                if points.iter().all(|p| p.is_finite()) {
                    triangles.push(Triangle {
                        low: points[0].min(points[1]).min(points[2]),
                        high: points[0].max(points[1]).max(points[2]),
                        points,
                        object: instance.id,
                        face: face as u32,
                    });
                }
            }
        }
        let mut index = Self {
            nodes: Vec::with_capacity(triangles.len() / 4 + 1),
            triangles,
        };
        if !index.triangles.is_empty() {
            index.build(0..index.triangles.len());
        }
        index
    }

    fn build(&mut self, range: Range<usize>) -> usize {
        let (mut low, mut high) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for t in &self.triangles[range.clone()] {
            low = low.min(t.low);
            high = high.max(t.high);
        }
        let node = self.nodes.len();
        self.nodes.push(Node {
            low,
            high,
            contents: Contents::Triangles(range.clone()),
        });
        if range.len() > 8 {
            let extent = high - low;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            let middle = range.len() / 2;
            self.triangles[range.clone()].select_nth_unstable_by(middle, |a, b| {
                (a.low[axis] + a.high[axis]).total_cmp(&(b.low[axis] + b.high[axis]))
            });
            let split = range.start + middle;
            let a = self.build(range.start..split);
            let b = self.build(split..range.end);
            self.nodes[node].contents = Contents::Children([a, b]);
        }
        node
    }

    /// The nearest triangle hit by `ray`.
    pub(crate) fn pick(&self, ray: Ray) -> Option<PickHit> {
        let direction = ray.direction.try_normalize()?;
        if self.nodes.is_empty() || !ray.origin.is_finite() {
            return None;
        }
        let inverse = direction.recip();
        let mut best: Option<(f32, usize)> = None;
        let mut stack = vec![0usize];
        while let Some(node) = stack.pop() {
            let node = &self.nodes[node];
            let Some(near) = slab(ray.origin, inverse, node.low, node.high) else {
                continue;
            };
            if best.is_some_and(|(distance, _)| near > distance) {
                continue;
            }
            match &node.contents {
                Contents::Children([a, b]) => {
                    stack.push(*b);
                    stack.push(*a);
                }
                Contents::Triangles(range) => {
                    for index in range.clone() {
                        let [a, b, c] = self.triangles[index].points;
                        if let Some(distance) = intersect(ray.origin, direction, a, b, c)
                            && best.is_none_or(|(d, _)| distance < d)
                        {
                            best = Some((distance, index));
                        }
                    }
                }
            }
        }
        let (distance, index) = best?;
        let triangle = &self.triangles[index];
        Some(PickHit {
            object: triangle.object,
            face: triangle.face as usize,
            distance,
            position: ray.origin + direction * distance,
        })
    }
}

/// Entry distance of a ray into a box, if it enters ahead of the origin.
fn slab(origin: Vec3, inverse: Vec3, low: Vec3, high: Vec3) -> Option<f32> {
    let a = (low - origin) * inverse;
    let b = (high - origin) * inverse;
    // NaN (a zero direction component on a slab boundary) must not exclude
    // the box: treat it as unbounded.
    let near = a.min(b);
    let far = a.max(b);
    let pick = |v: f32, fallback: f32| if v.is_nan() { fallback } else { v };
    let near = pick(near.x, f32::NEG_INFINITY)
        .max(pick(near.y, f32::NEG_INFINITY))
        .max(pick(near.z, f32::NEG_INFINITY))
        .max(0.0);
    let far = pick(far.x, f32::INFINITY)
        .min(pick(far.y, f32::INFINITY))
        .min(pick(far.z, f32::INFINITY));
    (near <= far).then_some(near)
}

/// Möller–Trumbore in double precision, as `Scene::pick` does.
fn intersect(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (origin, direction): (DVec3, DVec3) = (origin.as_dvec3(), direction.as_dvec3());
    let (a, b, c) = (a.as_dvec3(), b.as_dvec3(), c.as_dvec3());
    let (ab, ac) = (b - a, c - a);
    let p = direction.cross(ac);
    let determinant = ab.dot(p);
    if determinant.abs() <= 1.0e-12 * ab.length() * ac.length() * direction.length() {
        return None;
    }
    let inv = determinant.recip();
    let t = origin - a;
    let u = t.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = t.cross(ab);
    let v = direction.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = ac.dot(q) * inv;
    (distance > 0.0 && distance.is_finite() && distance < f32::MAX as f64)
        .then_some(distance as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::Primitive;

    /// The index agrees with `Scene::pick` (object, polygon and distance)
    /// on random rays through a scene of several transformed meshes.
    #[test]
    fn picks_match_the_scene_on_random_rays() {
        let mut scene = Scene::empty();
        for (k, primitive) in [
            Primitive::Cube,
            Primitive::Sphere,
            Primitive::Torus,
            Primitive::Cylinder,
        ]
        .into_iter()
        .enumerate()
        {
            let id = scene.add(primitive);
            let object = scene.object_mut(id).unwrap();
            object.transform.translation = Vec3::new(k as f32 * 2.5 - 3.0, 0.3 * k as f32, 0.0);
            object.transform.rotation = Vec3::new(0.3, 0.5 * k as f32, 0.1);
            object.transform.scale = Vec3::new(1.0, 0.8 + 0.1 * k as f32, 1.2);
        }
        let index = PickIndex::new(&scene, None);
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut random = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        };
        let mut hits = 0;
        for _ in 0..3000 {
            let origin = Vec3::new(random() * 6.0, random() * 4.0 + 1.0, 8.0);
            let target = Vec3::new(random() * 5.0, random() * 1.5, random());
            let ray = Ray {
                origin,
                direction: (target - origin).normalize(),
            };
            let expected = scene.pick(ray);
            let actual = index.pick(ray);
            match (expected, actual) {
                (None, None) => {}
                (Some(e), Some(a)) => {
                    hits += 1;
                    assert_eq!((e.object_id, e.face), (a.object, a.face));
                    assert!((e.distance - a.distance).abs() < 1.0e-3);
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(hits > 500, "too few hits to be meaningful: {hits}");
        // Restricting to one object ignores the others.
        let first = scene.objects[0].id;
        let only = PickIndex::new(&scene, Some(first));
        let through = Ray {
            origin: Vec3::new(-3.0, 0.0, 9.0),
            direction: Vec3::NEG_Z,
        };
        assert_eq!(only.pick(through).map(|h| h.object), Some(first));
    }
}

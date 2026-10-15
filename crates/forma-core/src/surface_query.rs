//! A temporary acceleration structure for batches of component visibility tests.
//! Reuse while scene geometry and visibility are unchanged, rather than
//! triangulating the entire scene for every selected face or vertex.
use crate::{Ray, Scene, Vec3};
use glam::{BVec3A, Vec3A};

struct Triangle {
    points: [Vec3; 3],
    low: Vec3,
    high: Vec3,
}
impl Triangle {
    fn new(points: [Vec3; 3]) -> Self {
        Self {
            low: points[0].min(points[1]).min(points[2]),
            high: points[0].max(points[1]).max(points[2]),
            points,
        }
    }
}
struct Node {
    low: Vec3,
    high: Vec3,
    contents: NodeContents,
}
enum NodeContents {
    Triangles(std::ops::Range<usize>),
    Children([usize; 2]),
}
struct QueryRay {
    ray: Ray,
    origin: Vec3A,
    direction: Vec3A,
    parallel: BVec3A,
}
impl QueryRay {
    fn new(ray: Ray) -> Self {
        let direction = Vec3A::from(ray.direction);
        Self {
            ray,
            origin: Vec3A::from(ray.origin),
            direction,
            parallel: direction.abs().cmplt(Vec3A::splat(1e-12)),
        }
    }

    #[inline]
    fn intersects(&self, low: Vec3, high: Vec3, distance: f32) -> bool {
        let low = Vec3A::from(low);
        let high = Vec3A::from(high);
        if ((self.origin.cmplt(low) | self.origin.cmpgt(high)) & self.parallel).any() {
            return false;
        }
        // Keep division rather than reciprocal multiplication: rounding a
        // grazing interval inward can incorrectly reject an occluding surface.
        // SIMD performs the same divisions together. Parallel axes are masked
        // out explicitly, including the 0/0 produced on a slab boundary.
        let a = (low - self.origin) / self.direction;
        let b = (high - self.origin) / self.direction;
        let near = Vec3A::select(self.parallel, Vec3A::splat(f32::NEG_INFINITY), a.min(b))
            .max_element()
            .max(0.);
        let far = Vec3A::select(self.parallel, Vec3A::splat(f32::INFINITY), a.max(b))
            .min_element()
            .min(distance);
        near <= far
    }
}
pub struct SurfaceQuery {
    triangles: Vec<Triangle>,
    nodes: Vec<Node>,
}

impl SurfaceQuery {
    pub fn new(scene: &Scene) -> Self {
        let mut triangles = Vec::new();
        let mut positions = Vec::new();
        for instance in scene
            .mesh_instances()
            .filter(|o| scene.is_effectively_visible(o.id))
        {
            let indices = instance.mesh.triangles();
            // Transform shared vertices once. Sparse meshes and disconnected
            // triangles do not benefit from caching every mesh position.
            if indices.len().saturating_mul(3) > instance.mesh.positions.len() {
                positions.clear();
                positions.extend(
                    instance
                        .mesh
                        .positions
                        .iter()
                        .map(|&point| instance.world_transform.transform_point3(point)),
                );
                triangles.extend(
                    indices
                        .into_iter()
                        .map(|t| Triangle::new(t.map(|i| positions[i as usize]))),
                );
            } else {
                triangles.extend(indices.into_iter().map(|t| {
                    Triangle::new(t.map(|i| {
                        instance
                            .world_transform
                            .transform_point3(instance.mesh.positions[i as usize])
                    }))
                }));
            }
        }
        // Median splitting produces at most this many leaves of eight triangles.
        let node_capacity = if triangles.is_empty() {
            0
        } else {
            triangles.len().div_ceil(8).next_power_of_two() * 2 - 1
        };
        let mut query = Self {
            triangles,
            nodes: Vec::with_capacity(node_capacity),
        };
        if !query.triangles.is_empty() {
            query.build(0..query.triangles.len());
        }
        query
    }

    fn build(&mut self, range: std::ops::Range<usize>) -> usize {
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        for triangle in &self.triangles[range.clone()] {
            low = low.min(triangle.low);
            high = high.max(triangle.high);
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            low,
            high,
            contents: NodeContents::Triangles(range.clone()),
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
            self.nodes[index].contents = NodeContents::Children([a, b]);
        }
        index
    }

    pub fn occludes(&self, ray: Ray, distance: f32) -> bool {
        if self.nodes.is_empty()
            || distance <= 0.
            || !ray.origin.is_finite()
            || !ray.direction.is_finite()
        {
            return false;
        }
        let ray = Ray {
            origin: ray.origin,
            direction: ray.direction.normalize_or_zero(),
        };
        if ray.direction == Vec3::ZERO {
            return false;
        }
        self.visit(0, &QueryRay::new(ray), distance)
    }

    fn visit(&self, index: usize, ray: &QueryRay, distance: f32) -> bool {
        let node = &self.nodes[index];
        if !ray.intersects(node.low, node.high, distance) {
            return false;
        }
        match &node.contents {
            NodeContents::Children([a, b]) => {
                self.visit(*a, ray, distance) || self.visit(*b, ray, distance)
            }
            NodeContents::Triangles(range) => {
                self.triangles[range.clone()].iter().any(|triangle| {
                    let [a, b, c] = triangle.points;
                    crate::scene::intersect_triangle(ray.ray.origin, ray.ray.direction, a, b, c)
                        .is_some_and(|t| t < distance)
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Keep the pre-SIMD slab test as an independent numerical reference. In
    // particular, reciprocal multiplication is not interchangeable with division
    // for rays grazing tiny boxes at large distances.
    fn scalar_intersects(ray: Ray, low: Vec3, high: Vec3, distance: f32) -> bool {
        let mut near: f32 = 0.;
        let mut far = distance;
        for axis in 0..3 {
            if ray.direction[axis].abs() < 1e-12 {
                if ray.origin[axis] < low[axis] || ray.origin[axis] > high[axis] {
                    return false;
                }
            } else {
                let a = (low[axis] - ray.origin[axis]) / ray.direction[axis];
                let b = (high[axis] - ray.origin[axis]) / ray.direction[axis];
                near = near.max(a.min(b));
                far = far.min(a.max(b));
                if near > far {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn vector_slabs_match_scalar_at_parallel_cutoff_boundaries_and_overflow() {
        let cutoff = 1.0e-12_f32;
        for component in [
            0.,
            -0.,
            cutoff.next_down(),
            cutoff,
            cutoff.next_up(),
            -cutoff.next_down(),
            -cutoff,
            -cutoff.next_up(),
        ] {
            for origin in [
                Vec3::ZERO,
                Vec3::ONE,
                -Vec3::ONE,
                Vec3::splat(1.0e30),
                Vec3::splat(-1.0e30),
                Vec3::splat(f32::MAX),
                Vec3::splat(-f32::MAX),
            ] {
                for (low, high) in [
                    (-Vec3::ONE, Vec3::ONE),
                    (-Vec3::splat(1e-30), Vec3::splat(1e-30)),
                    (-Vec3::splat(f32::MAX), Vec3::splat(f32::MAX)),
                    (origin, origin),
                ] {
                    for direction in [
                        Vec3::new(component, 0., 1.),
                        Vec3::new(0., -1., component),
                        Vec3::new(-1., component, 0.),
                    ] {
                        let ray = Ray { origin, direction };
                        for distance in [1e-30, 1., 1e30, f32::MAX, f32::INFINITY, f32::NAN] {
                            assert_eq!(
                                QueryRay::new(ray).intersects(low, high, distance),
                                scalar_intersects(ray, low, high, distance),
                                "ray={ray:?}, low={low}, high={high}, distance={distance}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn vector_slabs_match_scalar_for_varied_finite_boxes_and_rays() {
        let mut state = 0x4d595df4d0f33173_u64;
        let mut value = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            // Clear an exponent bit to exclude NaN and infinity while retaining
            // subnormal, tiny, ordinary and extreme finite values of either sign.
            f32::from_bits((state >> 32) as u32 & 0xfeff_ffff)
        };
        for _ in 0..20_000 {
            let a = Vec3::new(value(), value(), value());
            let b = Vec3::new(value(), value(), value());
            let low = a.min(b);
            let high = a.max(b);
            let origin = Vec3::new(value(), value(), value());
            let direction = Vec3::new(value(), value(), value()).normalize_or_zero();
            if direction == Vec3::ZERO {
                continue;
            }
            let ray = Ray { origin, direction };
            let distance = value().abs();
            assert_eq!(
                QueryRay::new(ray).intersects(low, high, distance),
                scalar_intersects(ray, low, high, distance),
                "ray={ray:?}, low={low}, high={high}, distance={distance}"
            );
        }
    }

    #[test]
    fn batch_visibility_matches_surface_picking_including_locked_objects() {
        let mut scene = Scene::empty();
        let id = scene.add(crate::Primitive::Sphere);
        scene.object_mut(id).unwrap().selectable = false;
        let query = SurfaceQuery::new(&scene);
        for y in 0..15 {
            for x in 0..15 {
                let ray = scene
                    .camera
                    .ray(crate::Vec2::new(x as f32 / 14., y as f32 / 14.), 1.);
                let hit = scene.pick_surface(ray);
                for distance in [1., 5., 10., 100.] {
                    assert_eq!(
                        query.occludes(ray, distance),
                        hit.is_some_and(|h| h.distance < distance)
                    );
                }
            }
        }
        scene.object_mut(id).unwrap().visible = false;
        let ray = Ray {
            origin: Vec3::Z * 4.,
            direction: -Vec3::Z,
        };
        assert!(!SurfaceQuery::new(&scene).occludes(ray, 10.));
    }
}

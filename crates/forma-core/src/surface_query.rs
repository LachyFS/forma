//! A temporary acceleration structure for batches of component visibility tests.
//! Build once per selection/overlay pass, rather than triangulating the entire
//! scene for every selected face or vertex.
use crate::{Ray, Scene, Vec3};

struct Triangle {
    points: [Vec3; 3],
    low: Vec3,
    high: Vec3,
}
struct Node {
    low: Vec3,
    high: Vec3,
    range: std::ops::Range<usize>,
    children: Option<[usize; 2]>,
}
pub struct SurfaceQuery {
    triangles: Vec<Triangle>,
    nodes: Vec<Node>,
}

impl SurfaceQuery {
    pub fn new(scene: &Scene) -> Self {
        let triangles = scene
            .mesh_instances()
            .filter(|o| scene.is_effectively_visible(o.id))
            .flat_map(|o| {
                o.mesh.triangles().into_iter().map(move |t| {
                    let points = t.map(|i| {
                        o.world_transform
                            .transform_point3(o.mesh.positions[i as usize])
                    });
                    Triangle {
                        low: points[0].min(points[1]).min(points[2]),
                        high: points[0].max(points[1]).max(points[2]),
                        points,
                    }
                })
            })
            .collect::<Vec<_>>();
        let mut query = Self {
            triangles,
            nodes: vec![],
        };
        if !query.triangles.is_empty() {
            query.build(0..query.triangles.len());
        }
        query
    }

    fn build(&mut self, range: std::ops::Range<usize>) -> usize {
        let low = self.triangles[range.clone()]
            .iter()
            .fold(Vec3::splat(f32::INFINITY), |a, t| a.min(t.low));
        let high = self.triangles[range.clone()]
            .iter()
            .fold(Vec3::splat(f32::NEG_INFINITY), |a, t| a.max(t.high));
        let index = self.nodes.len();
        self.nodes.push(Node {
            low,
            high,
            range: range.clone(),
            children: None,
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
            self.nodes[index].children = Some([a, b]);
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
        self.visit(0, ray, distance)
    }

    fn visit(&self, index: usize, ray: Ray, distance: f32) -> bool {
        let node = &self.nodes[index];
        let mut near: f32 = 0.;
        let mut far = distance;
        for axis in 0..3 {
            if ray.direction[axis].abs() < 1e-12 {
                if ray.origin[axis] < node.low[axis] || ray.origin[axis] > node.high[axis] {
                    return false;
                }
            } else {
                let a = (node.low[axis] - ray.origin[axis]) / ray.direction[axis];
                let b = (node.high[axis] - ray.origin[axis]) / ray.direction[axis];
                near = near.max(a.min(b));
                far = far.min(a.max(b));
                if near > far {
                    return false;
                }
            }
        }
        if let Some([a, b]) = node.children {
            return self.visit(a, ray, distance) || self.visit(b, ray, distance);
        }
        self.triangles[node.range.clone()].iter().any(|triangle| {
            let [a, b, c] = triangle.points;
            crate::scene::intersect_triangle(ray.origin, ray.direction, a, b, c)
                .is_some_and(|t| t < distance)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

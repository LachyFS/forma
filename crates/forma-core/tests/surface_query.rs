use forma_core::{Mesh, Primitive, Ray, Scene, SurfaceQuery, Vec2, Vec3};

fn world_triangles(scene: &Scene) -> Vec<[Vec3; 3]> {
    scene
        .mesh_instances()
        .filter(|instance| scene.is_effectively_visible(instance.id))
        .flat_map(|instance| {
            instance.mesh.triangles().into_iter().map(move |indices| {
                indices.map(|i| {
                    instance
                        .world_transform
                        .transform_point3(instance.mesh.positions[i as usize])
                })
            })
        })
        .collect()
}

// Exhaustive reference, independent of the query's bounds and traversal order.
// Use double precision like surface picking, including its scale-relative
// parallel threshold, positive-distance rule, and final f32 distance rounding.
fn exhaustive_hit(triangles: &[[Vec3; 3]], ray: Ray) -> Option<f32> {
    if !ray.origin.is_finite() || !ray.direction.is_finite() {
        return None;
    }
    let direction = ray.direction.normalize_or_zero();
    if direction == Vec3::ZERO {
        return None;
    }
    let direction = direction.as_dvec3();
    let origin = ray.origin.as_dvec3();
    triangles
        .iter()
        .filter_map(|points| {
            let [a, b, c] = points.map(|p| p.as_dvec3());
            let ab = b - a;
            let ac = c - a;
            let p = direction.cross(ac);
            let determinant = ab.dot(p);
            if determinant.abs() <= 1.0e-12 * ab.length() * ac.length() * direction.length() {
                return None;
            }
            let inverse = determinant.recip();
            let offset = origin - a;
            let u = offset.dot(p) * inverse;
            if !(0.0..=1.0).contains(&u) {
                return None;
            }
            let q = offset.cross(ab);
            let v = direction.dot(q) * inverse;
            if v < 0.0 || u + v > 1.0 {
                return None;
            }
            let distance = ac.dot(q) * inverse;
            (distance > 0.0 && distance.is_finite() && distance < f32::MAX as f64)
                .then_some(distance as f32)
        })
        .min_by(f32::total_cmp)
}

#[test]
fn batches_match_exhaustive_triangles_on_default_and_v8_scenes() {
    let v8 =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/scenes/v8-engine.forma");
    for scene in [Scene::default(), Scene::load(&v8).unwrap()] {
        let triangles = world_triangles(&scene);
        let query = SurfaceQuery::new(&scene);
        for opposite in [false, true] {
            let mut camera = scene.camera;
            if opposite {
                camera.yaw += std::f32::consts::PI;
            }
            for y in 0..9 {
                for x in 0..9 {
                    let ray =
                        camera.ray(Vec2::new((x as f32 + 0.5) / 9., (y as f32 + 0.5) / 9.), 1.);
                    let hit = exhaustive_hit(&triangles, ray);
                    for distance in [
                        camera.distance * 0.7,
                        camera.distance,
                        camera.distance * 2.,
                        f32::INFINITY,
                    ] {
                        assert_eq!(
                            query.occludes(ray, distance),
                            hit.is_some_and(|t| t < distance),
                            "ray={ray:?}, distance={distance}"
                        );
                    }
                    if let Some(hit) = hit {
                        assert!(!query.occludes(ray, hit), "strict hit distance {hit}");
                        assert!(
                            query.occludes(ray, hit.next_up()),
                            "next representable distance after {hit}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn axis_parallel_rays_boundaries_and_huge_origins_match_exhaustive_reference() {
    let mut scene = Scene::empty();
    scene.add(Primitive::Cube);
    let query = SurfaceQuery::new(&scene);
    let triangles = world_triangles(&scene);
    for origin in [2., 1.0e10, 1.0e20, 1.0e30] {
        for x in [-1.0_f32, -0.0, 0., 1., 1.0_f32.next_up()] {
            for direction in [
                Vec3::new(0., 0., -1.),
                Vec3::new(-0., 0., -7.),
                Vec3::new(1.0e-13, 0., -1.),
            ] {
                let ray = Ray {
                    origin: Vec3::new(x, 0., origin),
                    direction,
                };
                let hit = exhaustive_hit(&triangles, ray);
                // The existing slab test deliberately treats components under
                // 1e-12 as parallel; test those only where that cannot reject
                // an actual distant hit.
                if direction.x != 0. && origin > 2. {
                    continue;
                }
                for distance in [0., origin, origin * 2., f32::INFINITY, f32::NAN] {
                    assert_eq!(
                        query.occludes(ray, distance),
                        hit.is_some_and(|t| t < distance),
                        "ray={ray:?}, distance={distance}"
                    );
                }
            }
        }
    }
}

#[test]
fn tiny_and_large_parented_triangles_match_reference() {
    for scale in [1.0e-8, 1.0e-4, 1., 1.0e4, 1.0e8] {
        let mut scene = Scene::empty();
        let id = scene.add(Primitive::Plane);
        *scene.object_mesh_mut(id).unwrap() = Mesh {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            faces: vec![vec![0, 1, 2]],
        };
        let parent = scene.add_empty("parent").unwrap();
        scene.object_mut(parent).unwrap().transform.rotation = Vec3::new(0.3, 0.2, -0.4);
        scene.object_mut(parent).unwrap().transform.scale =
            Vec3::new(-scale, scale * 2., scale * 0.5);
        scene.set_parent(id, Some(parent), false).unwrap();
        let triangles = world_triangles(&scene);
        let [a, b, c] = triangles[0];
        let center = a + (b - a) * 0.2 + (c - a) * 0.3;
        let normal = (b - a)
            .as_dvec3()
            .cross((c - a).as_dvec3())
            .normalize()
            .as_vec3();
        let query = SurfaceQuery::new(&scene);
        for side in [-1., 1.] {
            let ray = Ray {
                origin: center + normal * scale * 3. * side,
                direction: -normal * side * 7.,
            };
            let hit = exhaustive_hit(&triangles, ray).unwrap();
            for distance in [hit * 0.5, hit, hit.next_up(), hit * 2.] {
                assert_eq!(
                    query.occludes(ray, distance),
                    hit < distance,
                    "scale={scale}, distance={distance}"
                );
            }
        }
    }
}

#[test]
fn visibility_locked_objects_empty_queries_and_invalid_rays() {
    let ray = Ray {
        origin: Vec3::Z * 4.,
        direction: -Vec3::Z,
    };
    let mut scene = Scene::empty();
    assert!(!SurfaceQuery::new(&scene).occludes(ray, 10.));
    let id = scene.add(Primitive::Cube);
    scene.object_mut(id).unwrap().selectable = false;
    assert!(SurfaceQuery::new(&scene).occludes(ray, 10.));
    assert!(scene.pick_surface(ray).is_some());
    assert!(scene.pick(ray).is_none());
    let query = SurfaceQuery::new(&scene);
    for origin in [
        Vec3::ZERO,
        Vec3::splat(f32::NAN),
        Vec3::splat(f32::INFINITY),
    ] {
        for direction in [
            Vec3::ZERO,
            Vec3::splat(f32::NAN),
            Vec3::splat(f32::INFINITY),
            Vec3::splat(f32::MAX),
        ] {
            assert!(!query.occludes(Ray { origin, direction }, f32::INFINITY));
        }
    }
    scene.object_mut(id).unwrap().visible = false;
    assert!(!SurfaceQuery::new(&scene).occludes(ray, 10.));
    scene.object_mut(id).unwrap().visible = true;
    scene.collection_mut(scene.root_collection).unwrap().visible = false;
    assert!(!SurfaceQuery::new(&scene).occludes(ray, 10.));
}

use forma_core::{Hit, Mat4, Mesh, Primitive, Ray, Scene, Transform, Vec2, Vec3};

// The exhaustive pre-broad-phase picker is deliberately kept independent of
// the implementation being tested, including its double-precision triangle
// test. Match face IDs, tie ordering, and exact world distances, not just hits.
fn exhaustive_pick(scene: &Scene, ray: Ray, selectable_only: bool) -> Option<Hit> {
    if !ray.origin.is_finite() || !ray.direction.is_finite() {
        return None;
    }
    let direction = ray.direction.normalize_or_zero();
    if direction == Vec3::ZERO {
        return None;
    }
    let mut closest: Option<Hit> = None;
    for instance in scene.mesh_instances().filter(|instance| {
        (!selectable_only || instance.selectable) && scene.is_effectively_visible(instance.id)
    }) {
        let matrix = instance.world_transform;
        if !matrix.is_finite() || matrix.determinant() == 0.0 {
            continue;
        }
        let inverse = matrix.inverse();
        if !inverse.is_finite() {
            continue;
        }
        let origin = inverse.transform_point3(ray.origin).as_dvec3();
        let local_direction = inverse.transform_vector3(direction).as_dvec3();
        for (triangle, face) in instance.mesh.triangles_with_faces() {
            let [a, b, c] =
                triangle.map(|index| instance.mesh.positions[index as usize].as_dvec3());
            let ab = b - a;
            let ac = c - a;
            let p = local_direction.cross(ac);
            let determinant = ab.dot(p);
            if determinant.abs() <= 1.0e-12 * ab.length() * ac.length() * local_direction.length() {
                continue;
            }
            let inv = determinant.recip();
            let t = origin - a;
            let u = t.dot(p) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = t.cross(ab);
            let v = local_direction.dot(q) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let distance = ac.dot(q) * inv;
            if distance > 0.0 && distance.is_finite() && distance < f32::MAX as f64 {
                let distance = distance as f32;
                if closest.is_none_or(|hit| distance < hit.distance) {
                    closest = Some(Hit {
                        object_id: instance.id,
                        face,
                        distance,
                        position: ray.origin + direction * distance,
                    });
                }
            }
        }
    }
    closest
}

fn assert_same(scene: &Scene, ray: Ray) {
    for selectable_only in [true, false] {
        let expected = exhaustive_pick(scene, ray, selectable_only);
        let actual = if selectable_only {
            scene.pick(ray)
        } else {
            scene.pick_surface(ray)
        };
        let signature = |hit: Hit| (hit.object_id, hit.face, hit.distance, hit.position);
        assert_eq!(
            actual.map(signature),
            expected.map(signature),
            "ray={ray:?}, selectable_only={selectable_only}"
        );
    }
}

#[test]
fn matches_exhaustive_for_mixed_transformed_parented_and_locked_geometry() {
    let mut scene = Scene::default();
    let parent = scene.add_empty("parent").unwrap();
    scene.object_mut(parent).unwrap().transform = Transform {
        translation: Vec3::new(-1.0, 0.5, 2.0),
        rotation: Vec3::new(0.3, -0.5, 0.7),
        scale: Vec3::new(2.0, -0.4, 1.3),
    };
    for (index, primitive) in [Primitive::Cube, Primitive::Plane, Primitive::Cylinder]
        .into_iter()
        .enumerate()
    {
        let id = scene.add(primitive);
        let object = scene.object_mut(id).unwrap();
        object.transform = Transform {
            translation: Vec3::new(index as f32, 0.1, -0.3),
            rotation: Vec3::new(-0.3, 0.7, 0.1),
            scale: Vec3::new(-0.7, 1.2, 0.9),
        };
        object.parent = Some(parent);
        object.parent_inverse = Mat4::from_rotation_y(0.2);
        object.selectable = index != 1;
    }
    let hidden_collection = scene.add_collection("hidden", None).unwrap();
    scene
        .collections
        .iter_mut()
        .find(|c| c.id == hidden_collection)
        .unwrap()
        .visible = false;
    let hidden = scene.add(Primitive::Cube);
    scene.object_mut(hidden).unwrap().collections = vec![hidden_collection];
    let linked = scene.duplicate_linked(scene.objects[0].id).unwrap();
    scene.object_mut(linked).unwrap().transform.translation.x += 3.0;
    for y in 0..19 {
        for x in 0..27 {
            assert_same(
                &scene,
                scene
                    .camera
                    .ray(Vec2::new(x as f32 / 26.0, y as f32 / 18.0), 1.5),
            );
        }
    }
}

#[test]
fn preserves_boundary_inside_parallel_and_nearly_parallel_rays() {
    for primitive in [Primitive::Cube, Primitive::Plane] {
        let mut scene = Scene::empty();
        scene.add(primitive);
        for x in [-1.0, -0.9999999, 0.0, 0.9999999, 1.0, 1.0000001] {
            for z in [-1.0, 0.0, 1.0] {
                for origin_y in [-3.0, 0.0, 3.0] {
                    for direction in [Vec3::Y, Vec3::NEG_Y, Vec3::new(-0.0, -2.0, 0.0)] {
                        assert_same(
                            &scene,
                            Ray {
                                origin: Vec3::new(x, origin_y, z),
                                direction,
                            },
                        );
                    }
                }
            }
        }
    }
    let mut scene = Scene::empty();
    let id = scene.add(Primitive::Plane);
    scene.object_mut(id).unwrap().transform.scale = Vec3::splat(1000.0);
    for slope in [1.0e-11, 1.0e-9, 1.0e-6] {
        let ray = Ray {
            origin: Vec3::new(0.0, 1.0e7 * slope, 1.0e7),
            direction: Vec3::new(0.0, -slope, -1.0),
        };
        assert!(exhaustive_pick(&scene, ray, true).is_some());
        assert_same(&scene, ray);
    }
}

#[test]
fn tiny_huge_mirrored_and_translated_meshes_match_exhaustive() {
    for mesh_scale in [1.0e-8, 1.0, 1.0e8] {
        for object_scale in [1.0e-5, 1.0, 1.0e5] {
            let mut scene = Scene::empty();
            let id = scene.add(Primitive::Cube);
            for point in &mut scene.object_mesh_mut(id).unwrap().positions {
                *point *= mesh_scale;
            }
            let extent = mesh_scale * object_scale;
            scene.object_mut(id).unwrap().transform = Transform {
                translation: Vec3::new(1.3, -0.7, 0.5) * extent,
                rotation: Vec3::new(0.1, -0.3, 0.5),
                scale: Vec3::new(-1.0, 2.0, 0.7) * object_scale,
            };
            let matrix = scene.world_transform(id).unwrap();
            for x in [-1.0, -0.5, 0.0, 0.5, 1.0, 1.001] {
                let origin = matrix.transform_point3(Vec3::new(x, 0.0, 5.0) * mesh_scale);
                let direction = matrix.transform_vector3(Vec3::NEG_Z).normalize();
                assert_same(&scene, Ray { origin, direction });
            }
        }
    }
}

#[test]
fn nearest_and_equal_distance_hits_keep_order_and_mutations_are_immediate() {
    let mut scene = Scene::empty();
    let far = scene.add(Primitive::Cube);
    scene.object_mut(far).unwrap().transform.translation.z = -5.0;
    let near = scene.add(Primitive::Cube);
    let same = scene.add(Primitive::Cube);
    let ray = Ray {
        origin: Vec3::Z * 10.0,
        direction: Vec3::NEG_Z * 3.0,
    };
    assert_eq!(scene.pick(ray).unwrap().object_id, near);
    assert_same(&scene, ray);
    scene.object_mut(near).unwrap().selectable = false;
    assert_eq!(scene.pick(ray).unwrap().object_id, same);
    assert_eq!(scene.pick_surface(ray).unwrap().object_id, near);
    assert_same(&scene, ray);
    scene.object_mut(near).unwrap().visible = false;
    for point in &mut scene.object_mesh_mut(same).unwrap().positions {
        point.x += 100.0;
    }
    assert_eq!(scene.pick(ray).unwrap().object_id, far);
    assert_same(&scene, ray);
    let linked = scene.duplicate_linked(same).unwrap();
    scene.object_mut(linked).unwrap().transform.translation = Vec3::new(-100.0, 0.0, 2.0);
    assert_eq!(scene.pick(ray).unwrap().object_id, linked);
    assert_same(&scene, ray);
}

#[test]
fn invalid_rays_empty_mesh_and_nonfinite_unused_vertices_match_exhaustive() {
    let mut scene = Scene::empty();
    let id = scene.add(Primitive::Cube);
    for ray in [
        Ray {
            origin: Vec3::NAN,
            direction: Vec3::NEG_Z,
        },
        Ray {
            origin: Vec3::Z * 5.0,
            direction: Vec3::ZERO,
        },
        Ray {
            origin: Vec3::Z * 5.0,
            direction: Vec3::splat(f32::INFINITY),
        },
    ] {
        assert_same(&scene, ray);
    }
    let ray = Ray {
        origin: Vec3::Z * 5.0,
        direction: Vec3::NEG_Z,
    };
    scene.object_mesh_mut(id).unwrap().positions.push(Vec3::NAN);
    assert!(scene.pick(ray).is_some());
    assert_same(&scene, ray);
    *scene.object_mesh_mut(id).unwrap() = Mesh {
        positions: vec![],
        faces: vec![],
    };
    assert_same(&scene, ray);
}

#[test]
fn bundled_engine_camera_rays_match_exhaustive() {
    let scene = Scene::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/scenes/v8-engine.forma"),
    )
    .unwrap();
    for y in 0..5 {
        for x in 0..7 {
            assert_same(
                &scene,
                scene.camera.ray(
                    Vec2::new((x as f32 + 0.5) / 7.0, (y as f32 + 0.5) / 5.0),
                    1.5,
                ),
            );
        }
    }
}

#[test]
fn grazing_and_skinny_triangle_rays_match_exhaustive_across_scales() {
    let mut scene = Scene::empty();
    let id = scene.add(Primitive::Cube);
    let mut seed = 0x6391_ef02_u32;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed >> 8) as f32 / (1_u32 << 24) as f32
    };
    let mut hits = 0;
    for sample in 0..4096 {
        let size = 10.0_f32.powf(random() * 16.0 - 8.0);
        let width = 10.0_f32.powf(-random() * 10.0);
        let shape = [
            Vec3::new(-1.0, -width, 0.0),
            Vec3::new(1.0, -width, 0.0),
            Vec3::new(1.0, width, 0.0),
        ];
        *scene.object_mesh_mut(id).unwrap() = Mesh {
            positions: shape.into_iter().map(|point| point * size).collect(),
            faces: vec![vec![0, 1, 2]],
        };
        scene.object_mut(id).unwrap().transform = Transform {
            translation: Vec3::new(random() - 0.5, random() - 0.5, random() - 0.5) * size,
            rotation: Vec3::new(random(), random(), random()) * 6.0,
            scale: Vec3::new(-1.0, 0.7, 1.3) * 10.0_f32.powf(random() * 10.0 - 5.0),
        };
        // Include exact vertices, points only a float ulp beyond the AABB,
        // interior hits, shallow incidence, and cancellation from far origins.
        let target = match sample % 4 {
            0 => shape[sample % 3],
            1 => Vec3::new(1.0 + f32::EPSILON, 0.0, 0.0),
            _ => Vec3::new(0.5, -0.25 * width, 0.0),
        } * size;
        let offset = Vec3::new(
            random() - 0.5,
            (random() - 0.5) * width,
            10.0_f32.powf(-random() * 10.0),
        ) * size
            * 10.0_f32.powf(random() * 12.0);
        let matrix = scene.world_transform(id).unwrap();
        let origin = matrix.transform_point3(target + offset);
        let direction = matrix.transform_vector3(-offset).normalize_or_zero();
        let ray = Ray { origin, direction };
        hits += usize::from(exhaustive_pick(&scene, ray, true).is_some());
        assert_same(&scene, ray);
    }
    assert!(
        hits > 100,
        "stress corpus must exercise actual triangle hits"
    );
}

#[test]
fn finite_extreme_direction_lengths_preserve_existing_normalization_behavior() {
    let mut scene = Scene::empty();
    let id = scene.add(Primitive::Cube);
    for scale in [1.0e-5, 1.0, 1.0e5] {
        scene.object_mut(id).unwrap().transform.scale = Vec3::new(-1.0, 0.5, 2.0) * scale;
        for length in [f32::MIN_POSITIVE, 1.0e-18, 1.0, 1.0e18, f32::MAX] {
            assert_same(
                &scene,
                Ray {
                    origin: Vec3::new(0.0, 0.0, 5.0) * scale,
                    direction: Vec3::NEG_Z * length,
                },
            );
        }
    }
}

#[test]
fn extreme_finite_anisotropic_scales_preserve_small_nonzero_local_directions() {
    let mut scene = Scene::empty();
    let id = scene.add(Primitive::Cube);
    let scale = Vec3::new(1.0e30, -1.0e-30, 1.0);
    scene.object_mut(id).unwrap().transform.scale = scale;
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        for sign in [-1.0, 1.0] {
            let ray = Ray {
                origin: axis * scale.abs() * (3.0 * sign),
                direction: axis * -sign,
            };
            assert!(exhaustive_pick(&scene, ray, true).is_some());
            assert_same(&scene, ray);
        }
    }
}

#[test]
fn loose_vertices_and_large_polygons_match_exhaustive() {
    let mut scene = Scene::empty();
    let id = scene.add(Primitive::Cube);
    scene
        .object_mesh_mut(id)
        .unwrap()
        .positions
        .extend(std::iter::repeat_n(Vec3::splat(100.0), 100_000));
    for x in [0.0, 2.0, 100.0] {
        assert_same(
            &scene,
            Ray {
                origin: Vec3::new(x, 0.0, 5.0),
                direction: Vec3::NEG_Z,
            },
        );
    }
    *scene.object_mesh_mut(id).unwrap() = Mesh {
        positions: (0..64)
            .map(|index| {
                let angle = index as f32 * std::f32::consts::TAU / 64.0;
                Vec3::new(angle.cos(), angle.sin(), 0.0)
            })
            .collect(),
        faces: vec![(0..64).collect()],
    };
    for x in [0.0, 0.5, 2.0] {
        assert_same(
            &scene,
            Ray {
                origin: Vec3::new(x, 0.0, 5.0),
                direction: Vec3::NEG_Z,
            },
        );
    }
}

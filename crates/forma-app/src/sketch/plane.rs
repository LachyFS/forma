//! Choosing the plane a new sketch is drawn on, and the camera that looks
//! straight at it.
use forma_core::{Camera, Ray, Scene, SketchCurve, SketchPlane, Vec3};
use std::f32::consts::FRAC_PI_2;
use std::time::{Duration, Instant};

/// A plane for a new sketch, in world space.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlaneChoice {
    pub plane: SketchPlane,
    pub label: String,
    /// The picked face's outline in plane coordinates: snapping reference.
    pub reference: Vec<SketchCurve>,
    /// World point the camera should centre on.
    pub focus: Vec3,
}

/// Plane under the pointer: a planar face, else the named orthographic view's
/// plane, else the ground.
pub(crate) fn choose(scene: &Scene, camera: Camera, ray: Option<Ray>) -> PlaneChoice {
    if let Some(choice) = ray.and_then(|ray| face_plane(scene, camera, ray)) {
        return choice;
    }
    if let Some((plane, label)) = named_view_plane(camera) {
        return PlaneChoice {
            focus: project_onto(&plane, camera.target),
            plane,
            label: label.into(),
            reference: Vec::new(),
        };
    }
    let plane = SketchPlane::ground();
    PlaneChoice {
        focus: project_onto(&plane, camera.target),
        plane,
        label: "Ground plane".into(),
        reference: Vec::new(),
    }
}

fn project_onto(plane: &SketchPlane, point: Vec3) -> Vec3 {
    plane.to_world(plane.to_local(point))
}

/// Camera screen right and up vectors, matching `Camera`'s yaw/pitch basis.
fn camera_axes(camera: Camera) -> (Vec3, Vec3, Vec3) {
    let inverse = camera.view_matrix().inverse();
    (
        inverse.x_axis.truncate(),
        inverse.y_axis.truncate(),
        inverse.z_axis.truncate(),
    )
}

/// The plane through the world origin facing an exact orthographic view, with
/// its axes along the screen so drawing matches the view.
pub(crate) fn named_view_plane(camera: Camera) -> Option<(SketchPlane, &'static str)> {
    if !camera.orthographic {
        return None;
    }
    let (right, up, backward) = camera_axes(camera);
    let snap = |v: Vec3| {
        let rounded = v.round();
        (rounded.abs().element_sum() == 1.0 && rounded.distance(v) < 1.0e-3).then_some(rounded)
    };
    let (x_axis, y_axis, normal) = (snap(right)?, snap(up)?, snap(backward)?);
    let label = match (normal.x, normal.y, normal.z) {
        (_, y, _) if y > 0.5 => "Top plane",
        (_, y, _) if y < -0.5 => "Bottom plane",
        (_, _, z) if z > 0.5 => "Front plane",
        (_, _, z) if z < -0.5 => "Back plane",
        (x, _, _) if x > 0.5 => "Right plane",
        _ => "Left plane",
    };
    let plane = SketchPlane {
        origin: Vec3::ZERO,
        x_axis,
        y_axis,
    };
    plane.is_valid().then_some((plane, label))
}

/// The plane of a planar face hit by `ray`: one polygon of a mesh, or a
/// whole B-rep face group of a part. Axes follow the face's longest straight
/// edge, choosing the orientation the camera can show upright; the origin
/// sits on a face corner so the grid lines up with the face's edges.
pub(crate) fn face_plane(scene: &Scene, camera: Camera, ray: Ray) -> Option<PlaneChoice> {
    let hit = scene.pick(ray)?;
    let instance = scene.mesh_instance(hit.object_id)?;
    let group = instance.mesh.face_group(hit.face)?;
    group_plane_choice(
        scene,
        camera,
        hit.object_id,
        group,
        Some((ray.direction, hit.position)),
    )
}

/// The plane of face group `group` of `object` (a polygon index for meshes
/// without groups), if every vertex of every polygon in it is coplanar —
/// tested across the whole group, so a curved B-rep face whose facets are
/// each flat is rejected. `pick` is the viewing ray direction and hit point.
pub(crate) fn group_plane_choice(
    scene: &Scene,
    camera: Camera,
    object: u64,
    group: u32,
    pick: Option<(Vec3, Vec3)>,
) -> Option<PlaneChoice> {
    use crate::cad_tools::geometry;
    let instance = scene.mesh_instance(object)?;
    let mesh = instance.mesh;
    let faces = geometry::group_faces(mesh, group);
    let to_world = |i: u32| {
        mesh.positions
            .get(i as usize)
            .map(|p| instance.world_transform.transform_point3(*p))
    };
    let mut normal = Vec3::ZERO;
    let mut world: Vec<Vec3> = Vec::new();
    for &f in &faces {
        let polygon: Vec<Vec3> = mesh.faces[f].iter().filter_map(|&i| to_world(i)).collect();
        for (i, a) in polygon.iter().enumerate() {
            let b = polygon[(i + 1) % polygon.len()];
            normal += Vec3::new(
                (a.y - b.y) * (a.z + b.z),
                (a.z - b.z) * (a.x + b.x),
                (a.x - b.x) * (a.y + b.y),
            );
        }
        world.extend(polygon);
    }
    if world.len() < 3 {
        return None;
    }
    let mut normal = normal.try_normalize()?;
    let toward_viewer = match pick {
        Some((direction, _)) => -direction,
        None => camera.position() - world[0],
    };
    if normal.dot(toward_viewer) < 0.0 {
        normal = -normal;
    }
    let centroid = world.iter().sum::<Vec3>() / world.len() as f32;
    let extent = world
        .iter()
        .map(|p| p.distance(centroid))
        .fold(0.0_f32, f32::max);
    let deviation = world
        .iter()
        .map(|p| (*p - centroid).dot(normal).abs())
        .fold(0.0_f32, f32::max);
    if deviation > extent * 2.0e-3 + 1.0e-6 {
        return None;
    }
    // The group's outline, with collinear tessellation segments merged.
    let boundary = geometry::group_boundary(mesh, &faces);
    let edges: Vec<(Vec3, Vec3)> = geometry::chain_loops(&boundary)
        .iter()
        .flat_map(|lp| {
            let points: Vec<Vec3> = lp.iter().filter_map(|&i| to_world(i)).collect();
            geometry::straight_edges(&points)
        })
        .collect();
    let (start, end) = edges
        .iter()
        .copied()
        .max_by(|a, b| a.0.distance(a.1).total_cmp(&b.0.distance(b.1)))?;
    let world: Vec<Vec3> = edges.iter().map(|e| e.0).collect();
    let edge = (end - start - normal * (end - start).dot(normal)).try_normalize()?;
    let candidates = [edge, -edge, normal.cross(edge), -normal.cross(edge)];
    let desired = if normal.y.abs() > 0.9999 {
        camera_axes(camera).0
    } else {
        Vec3::new(normal.z, 0.0, -normal.x).normalize()
    };
    let x_axis = candidates
        .into_iter()
        .max_by(|a, b| a.dot(desired).total_cmp(&b.dot(desired)))?;
    let y_axis = normal.cross(x_axis).normalize();
    let on_plane = |p: Vec3| p - normal * (p - centroid).dot(normal);
    let corner = world
        .iter()
        .copied()
        .min_by(|a, b| {
            let key = |p: &Vec3| (*p - centroid).dot(x_axis) + (*p - centroid).dot(y_axis);
            key(a).total_cmp(&key(b))
        })
        .map(on_plane)?;
    let plane = SketchPlane {
        origin: corner,
        x_axis,
        y_axis,
    };
    if !plane.is_valid() {
        return None;
    }
    let reference = edges
        .iter()
        .map(|(a, b)| SketchCurve::Line {
            start: plane.to_local(*a),
            end: plane.to_local(*b),
        })
        .filter(SketchCurve::is_valid)
        .collect();
    Some(PlaneChoice {
        plane,
        label: format!("Face of {}", instance.object.name),
        reference,
        focus: project_onto(&plane, pick.map_or(centroid, |(_, hit)| hit)),
    })
}

/// `camera` turned to look straight at `plane` (world space) around `focus`,
/// with the plane's x axis pointing right wherever the camera can roll it.
pub(crate) fn look_at(camera: Camera, plane: &SketchPlane, focus: Vec3) -> Camera {
    let normal = plane.normal();
    let mut result = camera;
    if normal.y.abs() > 0.9999 {
        result.pitch = FRAC_PI_2 * normal.y.signum();
        result.yaw = (-plane.x_axis.z).atan2(plane.x_axis.x);
    } else {
        result.pitch = normal.y.clamp(-1.0, 1.0).asin();
        result.yaw = normal.x.atan2(normal.z);
    }
    result.yaw = result.yaw.rem_euclid(std::f32::consts::TAU);
    if focus.is_finite() {
        result.target = focus;
    }
    result
}

/// Angle between the camera's viewing direction and the plane's normal view.
pub(crate) fn view_angle(camera: Camera, plane: &SketchPlane) -> f32 {
    let (_, _, backward) = camera_axes(camera);
    backward
        .normalize_or_zero()
        .dot(plane.normal())
        .clamp(-1.0, 1.0)
        .acos()
}

/// A short eased camera move. Interpolates yaw along the shorter way round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CameraAnimation {
    pub from: Camera,
    pub to: Camera,
    pub started: Instant,
    pub duration: Duration,
    /// The camera this animation last set; anything else means the user
    /// navigated and the animation yields.
    pub last: Camera,
}

impl CameraAnimation {
    pub(crate) fn new(from: Camera, to: Camera) -> Self {
        Self {
            from,
            to,
            started: Instant::now(),
            duration: Duration::from_millis(320),
            last: from,
        }
    }

    /// Camera at `now`, and whether the animation has finished.
    pub(crate) fn sample(&self, now: Instant) -> (Camera, bool) {
        let t = (now.saturating_duration_since(self.started).as_secs_f32()
            / self.duration.as_secs_f32().max(1.0e-3))
        .clamp(0.0, 1.0);
        (interpolate(self.from, self.to, ease(t)), t >= 1.0)
    }
}

fn ease(t: f32) -> f32 {
    // Cubic ease in-out.
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) * 0.5
    }
}

pub(crate) fn interpolate(from: Camera, to: Camera, t: f32) -> Camera {
    if t >= 1.0 {
        return to;
    }
    let tau = std::f32::consts::TAU;
    let mut yaw_delta = (to.yaw - from.yaw).rem_euclid(tau);
    if yaw_delta > std::f32::consts::PI {
        yaw_delta -= tau;
    }
    Camera {
        target: from.target.lerp(to.target, t),
        yaw: (from.yaw + yaw_delta * t).rem_euclid(tau),
        pitch: from.pitch + (to.pitch - from.pitch) * t,
        distance: (from.distance.ln() + (to.distance.ln() - from.distance.ln()) * t).exp(),
        fov_y: from.fov_y + (to.fov_y - from.fov_y) * t,
        orthographic: if t < 0.5 {
            from.orthographic
        } else {
            to.orthographic
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::Vec2;

    fn screen_axes(camera: Camera) -> (Vec3, Vec3) {
        let (right, up, _) = camera_axes(camera);
        (right, up)
    }

    #[test]
    fn looking_at_planes_puts_their_axes_on_screen() {
        for plane in [
            SketchPlane::ground(),
            SketchPlane::front(),
            SketchPlane::right(),
            SketchPlane::from_point_normal(Vec3::ZERO, Vec3::new(0.3, 0.2, 0.9)).unwrap(),
            SketchPlane::from_point_normal(Vec3::ZERO, Vec3::NEG_Y).unwrap(),
        ] {
            let camera = look_at(Camera::default(), &plane, Vec3::new(1.0, 2.0, 3.0));
            assert!(view_angle(camera, &plane) < 1.0e-3, "{plane:?}");
            let (right, up) = screen_axes(camera);
            if plane.x_axis.y.abs() < 1.0e-4 {
                assert!(right.distance(plane.x_axis) < 1.0e-3, "{plane:?} {right}");
                assert!(up.distance(plane.y_axis) < 1.0e-3, "{plane:?} {up}");
            }
            assert_eq!(camera.target, Vec3::new(1.0, 2.0, 3.0));
        }
    }

    #[test]
    fn named_views_choose_screen_aligned_planes() {
        let mut camera = Camera {
            orthographic: true,
            yaw: 0.0,
            pitch: FRAC_PI_2,
            ..Camera::default()
        };
        let (plane, label) = named_view_plane(camera).unwrap();
        assert_eq!(label, "Top plane");
        assert!(plane.normal().distance(Vec3::Y) < 1.0e-5);
        assert!(plane.x_axis.distance(Vec3::X) < 1.0e-5);
        camera.pitch = 0.0;
        camera.yaw = FRAC_PI_2;
        let (plane, label) = named_view_plane(camera).unwrap();
        assert_eq!(label, "Right plane");
        assert!(plane.normal().distance(Vec3::X) < 1.0e-5);
        camera.yaw = 0.3;
        assert!(named_view_plane(camera).is_none());
        camera.orthographic = false;
        camera.yaw = 0.0;
        assert!(named_view_plane(camera).is_none());
    }

    #[test]
    fn faces_give_corner_aligned_planes() {
        let mut scene = Scene::empty();
        let cube = scene.add(forma_core::Primitive::Cube);
        scene.object_mut(cube).unwrap().transform.rotation.y = 0.4;
        let camera = Camera::default();
        let choice = choose(
            &scene,
            camera,
            Some(Ray {
                origin: Vec3::new(0.1, 5.0, 0.2),
                direction: Vec3::NEG_Y,
            }),
        );
        assert!(choice.label.starts_with("Face of"));
        assert!(choice.plane.normal().distance(Vec3::Y) < 1.0e-4);
        assert!((choice.plane.origin.y - 1.0).abs() < 1.0e-4);
        // The cube's top face outline is a 2 x 2 square starting at the origin.
        assert_eq!(choice.reference.len(), 4);
        let (min, max) =
            choice
                .reference
                .iter()
                .fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(lo, hi), c| {
                    let (a, b) = c.bounds();
                    (lo.min(a), hi.max(b))
                });
        assert!(min.distance(Vec2::ZERO) < 1.0e-4 && max.distance(Vec2::splat(2.0)) < 1.0e-4);
        // Missing the cube falls back to the ground plane.
        let miss = choose(
            &scene,
            camera,
            Some(Ray {
                origin: Vec3::new(9.0, 5.0, 9.0),
                direction: Vec3::NEG_Y,
            }),
        );
        assert_eq!(miss.label, "Ground plane");
    }

    #[test]
    fn animation_eases_to_its_target_the_short_way_round() {
        let from = Camera {
            yaw: 6.1,
            ..Camera::default()
        };
        let to = Camera {
            yaw: 0.1,
            pitch: 1.2,
            distance: 2.0,
            ..Camera::default()
        };
        let middle = interpolate(from, to, 0.5);
        assert!(middle.yaw < 0.2 || middle.yaw > 6.0, "{}", middle.yaw);
        assert_eq!(interpolate(from, to, 1.0), to);
        let animation = CameraAnimation::new(from, to);
        let (camera, done) = animation.sample(animation.started + Duration::from_secs(1));
        assert!(done);
        assert_eq!(camera, to);
    }
}

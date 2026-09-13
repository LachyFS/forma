use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
    pub orthographic: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 0.45, 0.0),
            yaw: 0.65,
            pitch: 0.36,
            distance: 9.5,
            fov_y: 45.0_f32.to_radians(),
            orthographic: false,
        }
    }
}

impl Camera {
    pub fn position(&self) -> Vec3 {
        let (_, _, forward) = self.basis();
        self.target - self.distance * forward
    }

    pub fn view_matrix(&self) -> Mat4 {
        let (_, up, forward) = self.basis();
        Mat4::look_to_rh(self.position(), forward, up)
    }

    /// Projection uses a 0..1 depth range, as required by Metal.
    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        let aspect = aspect.max(0.001);
        let near = (self.distance * 0.001).clamp(0.0001, 0.1);
        let far = (self.distance * 1000.0).max(1000.0);
        if self.orthographic {
            let half_height = self.distance * (self.fov_y * 0.5).tan();
            Mat4::orthographic_rh(
                -half_height * aspect,
                half_height * aspect,
                -half_height,
                half_height,
                near,
                far,
            )
        } else {
            Mat4::perspective_rh(self.fov_y, aspect, near, far)
        }
    }

    /// A normalized world ray through a top-left-origin viewport coordinate.
    pub fn ray(&self, uv: Vec2, aspect: f32) -> Ray {
        let (right, up, forward) = self.basis();
        let half_height = (self.fov_y * 0.5).tan();
        let offset = right * ((uv.x * 2.0 - 1.0) * half_height * aspect.max(0.001))
            + up * ((1.0 - uv.y * 2.0) * half_height);
        if self.orthographic {
            Ray {
                origin: self.position() + offset * self.distance,
                direction: forward,
            }
        } else {
            Ray {
                origin: self.position(),
                direction: (forward + offset).normalize(),
            }
        }
    }

    /// Drag deltas are viewport pixels; positive X turns the camera left.
    pub fn orbit(&mut self, delta: Vec2) {
        if delta.is_finite() {
            self.yaw = (self.yaw - delta.x * 0.007).rem_euclid(std::f32::consts::TAU);
            self.pitch = (self.pitch + delta.y * 0.007).clamp(-1.5533, 1.5533);
        }
    }

    /// Pan deltas are pixels, calibrated for a roughly 900px-tall viewport.
    pub fn pan(&mut self, delta: Vec2) {
        self.pan_in_viewport(delta, 900.0);
    }

    /// Pan at the target depth using logical pixels, independent of window size or DPI.
    pub fn pan_in_viewport(&mut self, delta: Vec2, height: f32) {
        if !delta.is_finite() || !height.is_finite() || height <= 0.0 {
            return;
        }
        let (right, up, _) = self.basis();
        let scale = self.distance * (self.fov_y * 0.5).tan() * 2.0 / height;
        self.target += (-right * delta.x + up * delta.y) * scale;
    }

    /// Positive wheel deltas dolly in, preserving an exponential zoom scale.
    pub fn zoom(&mut self, delta: f32) {
        if delta.is_finite() {
            self.distance = (self.distance * (-delta * 0.01).exp()).clamp(0.02, 100_000.0);
        }
    }

    pub fn frame(&mut self, center: Vec3, radius: f32) {
        self.frame_in_viewport(center, radius, 1.0);
    }

    /// Fit a bounding sphere with padding in both viewport dimensions.
    pub fn frame_in_viewport(&mut self, center: Vec3, radius: f32, aspect: f32) {
        if center.is_finite() && radius.is_finite() && aspect.is_finite() && aspect > 0.0 {
            let half_angle = ((self.fov_y * 0.5).tan() * aspect.min(1.0)).atan();
            let extent = if self.orthographic {
                half_angle.tan()
            } else {
                half_angle.sin()
            };
            self.target = center;
            self.distance = (radius.max(0.01) / extent * 1.2).clamp(0.02, 100_000.0);
        }
    }

    /// Yaw defines a stable screen-right direction even at exact top/bottom
    /// views, where crossing a viewing direction with world-up is singular.
    fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let right = Vec3::new(cy, 0.0, -sy);
        let backward = Vec3::new(sy * cp, sp, cy * cp);
        (right, backward.cross(right), -backward)
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;

    #[test]
    fn pan_tracks_screen_pixels_at_target_depth_in_both_projections() {
        for orthographic in [false, true] {
            for height in [400., 900., 1600.] {
                let mut camera = Camera {
                    orthographic,
                    ..Camera::default()
                };
                let target = camera.target;
                let delta = Vec2::new(35.25, -18.5);
                camera.pan_in_viewport(delta, height);
                let clip = camera.projection_matrix(1.5) * camera.view_matrix() * target.extend(1.);
                let ndc = clip.truncate() / clip.w;
                let displacement = Vec2::new(ndc.x * height * 1.5 * 0.5, -ndc.y * height * 0.5);
                assert!(displacement.distance(delta) < 0.001);
            }
        }
    }

    #[test]
    fn framing_fits_sphere_in_portrait_and_landscape_in_both_projections() {
        for orthographic in [false, true] {
            for aspect in [0.3, 1.0, 2.0] {
                let mut camera = Camera {
                    orthographic,
                    ..Camera::default()
                };
                let center = Vec3::new(12., -3., 7.);
                let radius = 2.5;
                let orientation = (camera.yaw, camera.pitch);
                camera.frame_in_viewport(center, radius, aspect);
                assert_eq!(camera.target, center);
                assert_eq!((camera.yaw, camera.pitch), orientation);
                let matrix = camera.projection_matrix(aspect) * camera.view_matrix();
                for latitude in -9..=9 {
                    for longitude in 0..36 {
                        let phi = latitude as f32 * std::f32::consts::PI / 18.;
                        let theta = longitude as f32 * std::f32::consts::TAU / 36.;
                        let offset =
                            Vec3::new(phi.cos() * theta.cos(), phi.sin(), phi.cos() * theta.sin());
                        let clip = matrix * (center + offset * radius).extend(1.);
                        let ndc = clip.truncate() / clip.w;
                        assert!(ndc.x.abs() < 1. && ndc.y.abs() < 1.);
                        assert!((0. ..=1.).contains(&ndc.z));
                    }
                }
            }
        }
    }

    #[test]
    fn zoom_composes_and_stays_bounded() {
        let mut continuous = Camera::default();
        let mut batched = continuous;
        for _ in 0..100 {
            continuous.zoom(0.1);
        }
        batched.zoom(10.);
        assert!((continuous.distance - batched.distance).abs() < 0.0001);
        continuous.zoom(f32::MAX);
        assert_eq!(continuous.distance, 0.02);
        continuous.zoom(-f32::MAX);
        assert_eq!(continuous.distance, 100_000.);
        let before = continuous;
        continuous.zoom(f32::NAN);
        continuous.pan_in_viewport(Vec2::ONE, 0.);
        assert_eq!(continuous, before);
    }
}

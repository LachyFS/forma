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
        if !delta.is_finite() {
            return;
        }
        let (right, up, _) = self.basis();
        let scale = self.distance * (self.fov_y * 0.5).tan() * 2.0 / 900.0;
        self.target += (-right * delta.x + up * delta.y) * scale;
    }

    /// Positive wheel deltas dolly in, preserving an exponential zoom scale.
    pub fn zoom(&mut self, delta: f32) {
        if delta.is_finite() {
            self.distance = (self.distance * (-delta * 0.01).exp()).clamp(0.02, 100_000.0);
        }
    }

    pub fn frame(&mut self, center: Vec3, radius: f32) {
        if center.is_finite() && radius.is_finite() {
            self.target = center;
            self.distance =
                (radius.max(0.01) / (self.fov_y * 0.5).sin() * 1.2).clamp(0.02, 100_000.0);
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

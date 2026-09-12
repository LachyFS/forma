//! Geometry and scene data shared by the native editor and renderer.
//! Coordinates are right handed, +Y is up, and material colors are linear RGB.

mod camera;
mod history;
mod mesh;
mod obj;
mod scene;

pub use camera::{Camera, Ray};
pub use glam::{Mat4, Vec2, Vec3};
pub use history::History;
pub use mesh::{Mesh, Primitive};
pub use scene::{Hit, Material, Object, RenderPreferences, Scene, Transform, World};

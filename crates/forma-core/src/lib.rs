//! Geometry and scene data shared by the native editor and renderer.
//! Coordinates are right handed, +Y is up, and material colors are linear RGB.

mod camera;
mod history;
mod material;
mod mesh;
mod obj;
mod scene;

pub use camera::{Camera, Ray};
pub use glam::{Mat4, Vec2, Vec3};
pub use history::History;
pub use mesh::{Mesh, Primitive};
pub use scene::{
    CameraData, CameraProjection, Collection, Hit, Light, LightKind, MaterialData, MeshData,
    MeshInstance, Object, ObjectData, RenderPreferences, Scene, Transform, World,
};

pub use material::{
    DEFAULT_SHADER_CODE, MAX_SCENE_TEXTURE_BYTES, MAX_SHADER_BYTES, MAX_TEXTURE_PIXELS, Material,
    ShaderKind, TextureImage, TextureMapping, TextureSlot,
};

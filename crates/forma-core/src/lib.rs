//! Geometry and scene data shared by the native editor and renderer.
//! Coordinates are right handed, +Y is up, and material colors are linear RGB.

mod camera;
mod edit;
mod history;
mod material;
mod mesh;
mod mesh_tools;
mod obj;
mod object_ops;
mod part;
mod scene;
mod sketch;
mod stl;
mod surface_query;
pub use part::{
    BaseBody, BaseBodyFeature, ChamferFeature, CombineFeature, EdgeRef, ExtrudeExtent,
    ExtrudeFeature, FaceRef, Feature, FeatureGeometry, FeatureIssue, FeatureKind, FeatureOperation,
    FeaturesGeometry, MAX_FEATURE_REFERENCES, MAX_PART_FEATURES, MAX_PART_LENGTH,
    MAX_PART_STEP_BYTES, MAX_TOOL_DEPTH, MIN_PART_LENGTH, PartData, PartEvaluation, Placement,
    PrimitiveFeature, PrimitiveShape, PushPullFeature, RevolveFeature, TessellationQuality,
    ToolBody, all_features,
};
pub use sketch::{
    MAX_SKETCH_COORDINATE, MAX_SKETCH_ENTITIES, Profile, ProfileLoop, SKETCH_TOLERANCE, Sketch,
    SketchCurve, SketchEntity, SketchPlane, TrimPiece,
};
pub use surface_query::SurfaceQuery;

pub use camera::{Camera, Ray};
pub use edit::MeshElement;
pub use glam::{Mat4, Vec2, Vec3};
pub use history::History;
pub use mesh::{Mesh, Primitive};
pub use object_ops::ApplyTransform;
pub use scene::{
    CameraData, CameraProjection, Collection, DenoiseQuality, DenoiseSettings, Hit, Light,
    LightKind, MAX_SCENE_PARTS, MaterialData, MeshData, MeshInstance, Object, ObjectData,
    RenderPreferences, Scene, Transform, World,
};

pub use material::{
    DEFAULT_SHADER_CODE, DEFAULT_WGSL_CODE, MAX_SCENE_TEXTURE_BYTES, MAX_SHADER_BYTES,
    MAX_TEXTURE_PIXELS, Material, ShaderKind, ShaderLanguage, TextureImage, TextureMapping,
    TextureSlot,
};

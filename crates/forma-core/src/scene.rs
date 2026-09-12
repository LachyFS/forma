use crate::{Camera, Mesh, Primitive, Ray};
use anyhow::{Context, Result, ensure};
use glam::{EulerRot, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    ops::Deref,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

pub(crate) const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub base_color: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub emission: Vec3,
}

/// A named, reusable material data block. Objects refer to this by ID instead
/// of embedding a private material, so linked duplicates and future material
/// slots do not duplicate data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaterialData {
    pub id: u64,
    pub name: String,
    pub material: Material,
}

/// A named, reusable editable mesh data block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshData {
    pub id: u64,
    pub name: String,
    pub mesh: Mesh,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base_color: Vec3::new(0.48, 0.53, 0.59),
            metallic: 0.0,
            roughness: 0.36,
            emission: Vec3::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            self.scale,
            Quat::from_euler(
                EulerRot::XYZ,
                self.rotation.x,
                self.rotation.y,
                self.rotation.z,
            ),
            self.translation,
        )
    }

    /// Decompose an affine matrix back into Forma's local transform format.
    /// This is primarily useful when re-parenting while retaining world space.
    pub fn from_matrix(matrix: Mat4) -> Option<Self> {
        if !matrix.is_finite() || matrix.determinant().abs() < 1.0e-12 {
            return None;
        }
        let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
        let (x, y, z) = rotation.to_euler(EulerRot::XYZ);
        let transform = Self {
            translation,
            rotation: Vec3::new(x, y, z),
            scale,
        };
        (transform.translation.is_finite()
            && transform.rotation.is_finite()
            && transform.scale.is_finite())
        .then_some(transform)
    }
}

/// The payload of a scene object. Transform, visibility and hierarchy are
/// deliberately stored on `Object`; type-specific data lives here or in a
/// referenced data block. Adding a new object type does not change mesh code.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ObjectData {
    Mesh {
        mesh: u64,
        #[serde(default)]
        materials: Vec<u64>,
    },
    Empty {
        display_size: f32,
    },
    Light {
        light: Light,
    },
    Camera {
        camera: CameraData,
    },
    /// A durable extension point for object kinds Forma does not understand
    /// yet. Core graph operations preserve these properties losslessly.
    Custom {
        kind: String,
        #[serde(default)]
        properties: BTreeMap<String, serde_json::Value>,
    },
}

impl ObjectData {
    pub fn kind(&self) -> &str {
        match self {
            Self::Mesh { .. } => "mesh",
            Self::Empty { .. } => "empty",
            Self::Light { .. } => "light",
            Self::Camera { .. } => "camera",
            Self::Custom { kind, .. } => kind,
        }
    }

    pub fn mesh_id(&self) -> Option<u64> {
        match self {
            Self::Mesh { mesh, .. } => Some(*mesh),
            _ => None,
        }
    }

    pub fn material_ids(&self) -> &[u64] {
        match self {
            Self::Mesh { materials, .. } => materials,
            _ => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LightKind {
    Point,
    Sun,
    Spot,
    Area,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Light {
    pub kind: LightKind,
    pub color: Vec3,
    pub energy: f32,
    pub radius: f32,
    pub spot_angle: f32,
}

impl Default for Light {
    fn default() -> Self {
        Self {
            kind: LightKind::Point,
            color: Vec3::ONE,
            energy: 1000.0,
            radius: 0.25,
            spot_angle: std::f32::consts::FRAC_PI_4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraData {
    pub projection: CameraProjection,
    pub focal_length_mm: f32,
    pub sensor_width_mm: f32,
    pub orthographic_scale: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for CameraData {
    fn default() -> Self {
        Self {
            projection: CameraProjection::Perspective,
            focal_length_mm: 50.0,
            sensor_width_mm: 36.0,
            orthographic_scale: 10.0,
            near: 0.01,
            far: 10_000.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CameraProjection {
    Perspective,
    Orthographic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub id: u64,
    pub name: String,
    pub transform: Transform,
    #[serde(default)]
    pub parent: Option<u64>,
    /// Compensates a parent's world transform when parenting with "keep
    /// transform". Keeping this matrix avoids losing shear under non-uniform
    /// parent scale while the user-facing local transform remains decomposed.
    #[serde(default = "identity_matrix")]
    pub parent_inverse: Mat4,
    pub data: ObjectData,
    pub visible: bool,
    #[serde(default = "default_true")]
    pub selectable: bool,
    /// An object may be linked into more than one collection, as in Blender.
    pub collections: Vec<u64>,
}

impl Object {
    pub fn kind(&self) -> &str {
        self.data.kind()
    }

    pub fn mesh_id(&self) -> Option<u64> {
        self.data.mesh_id()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    pub id: u64,
    pub name: String,
    pub parent: Option<u64>,
    pub visible: bool,
}

/// A resolved render/edit view over a mesh object and its linked data blocks.
/// Consumers no longer need to know how the scene stores reusable data.
#[derive(Clone, Copy, Debug)]
pub struct MeshInstance<'a> {
    pub object: &'a Object,
    pub mesh: &'a Mesh,
    pub material: &'a Material,
    pub world_transform: Mat4,
}

impl Deref for MeshInstance<'_> {
    type Target = Object;

    fn deref(&self) -> &Self::Target {
        self.object
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct World {
    pub color: Vec3,
    pub strength: f32,
}

impl Default for World {
    fn default() -> Self {
        Self {
            color: Vec3::new(0.48, 0.58, 0.72),
            strength: 0.35,
        }
    }
}

/// Document-owned render quality and display exposure. These settings travel
/// with a scene through saving, reopening, and undo/redo transactions.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderPreferences {
    pub exposure: f32,
    pub max_samples: u32,
    pub max_bounces: u32,
    #[serde(default)]
    pub denoise: DenoiseSettings,
}

impl Default for RenderPreferences {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            max_samples: 128,
            max_bounces: 8,
            denoise: DenoiseSettings::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenoiseQuality {
    Fast,
    #[default]
    Balanced,
    High,
}

impl DenoiseQuality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Balanced => "Balanced",
            Self::High => "High",
        }
    }
}

/// Denoising changes the displayed image, never the Monte Carlo estimator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DenoiseSettings {
    pub viewport: bool,
    pub render: bool,
    pub start_sample: u32,
    pub quality: DenoiseQuality,
}

impl Default for DenoiseSettings {
    fn default() -> Self {
        Self {
            viewport: true,
            render: true,
            start_sample: 8,
            quality: DenoiseQuality::Balanced,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub objects: Vec<Object>,
    pub meshes: Vec<MeshData>,
    pub materials: Vec<MaterialData>,
    pub collections: Vec<Collection>,
    pub root_collection: u64,
    pub camera: Camera,
    pub world: World,
    #[serde(default)]
    pub render: RenderPreferences,
    pub next_id: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub object_id: u64,
    pub face: usize,
    /// World-space distance, independent of the incoming ray's length.
    pub distance: f32,
    pub position: Vec3,
}

#[derive(Serialize, Deserialize)]
struct SceneFile {
    format: String,
    version: u32,
    scene: serde_json::Value,
}

#[derive(Deserialize)]
struct LegacyScene {
    objects: Vec<LegacyObject>,
    camera: Camera,
    world: World,
    #[serde(default)]
    render: RenderPreferences,
    next_id: u64,
}

#[derive(Deserialize)]
struct LegacyObject {
    id: u64,
    name: String,
    mesh: Mesh,
    transform: Transform,
    material: Material,
    visible: bool,
}

const FILE_VERSION: u32 = 2;

impl Default for Scene {
    fn default() -> Self {
        let mut scene = Self::empty();
        let torus = scene.add(Primitive::Torus);
        {
            let object = scene.object_mut(torus).unwrap();
            object.name = "Torus".into();
            object.transform.translation = Vec3::new(-0.75, 1.12, 0.0);
            object.transform.rotation = Vec3::new(1.22, 0.0, -0.23);
        }
        *scene.object_material_mut(torus).unwrap() = Material {
            base_color: Vec3::new(0.035, 0.46, 0.38),
            metallic: 0.45,
            roughness: 0.24,
            emission: Vec3::ZERO,
        };

        let sphere = scene.add(Primitive::Sphere);
        {
            let object = scene.object_mut(sphere).unwrap();
            object.transform.translation = Vec3::new(1.05, 0.61, 0.6);
            object.transform.scale = Vec3::splat(0.6);
        }
        *scene.object_material_mut(sphere).unwrap() = Material {
            base_color: Vec3::new(0.72, 0.50, 0.30),
            metallic: 0.72,
            roughness: 0.19,
            emission: Vec3::ZERO,
        };

        let cube = scene.add(Primitive::Cube);
        scene.object_mesh_mut(cube).unwrap().subdivide();
        scene.object_mesh_mut(cube).unwrap().subdivide();
        {
            let object = scene.object_mut(cube).unwrap();
            object.name = "Rounded cube".into();
            object.transform.translation = Vec3::new(1.0, 0.59, -1.0);
            object.transform.scale = Vec3::splat(0.8);
            object.transform.rotation.y = 0.22;
        }
        let material = scene.object_material_mut(cube).unwrap();
        material.base_color = Vec3::new(0.61, 0.65, 0.71);
        material.roughness = 0.28;

        let platform = scene.add(Primitive::Cube);
        {
            let object = scene.object_mut(platform).unwrap();
            object.name = "Studio plinth".into();
            object.transform.translation = Vec3::new(0.0, -0.15, 0.0);
            object.transform.scale = Vec3::new(3.4, 0.15, 2.6);
        }
        let material = scene.object_material_mut(platform).unwrap();
        material.base_color = Vec3::new(0.16, 0.18, 0.22);
        material.roughness = 0.55;

        let light = scene.add(Primitive::Plane);
        {
            let object = scene.object_mut(light).unwrap();
            object.name = "Key · area light".into();
            object.transform.translation = Vec3::new(-2.0, 5.0, 2.0);
            object.transform.rotation.x = std::f32::consts::PI;
            object.transform.scale = Vec3::new(1.6, 1.0, 1.25);
        }
        let material = scene.object_material_mut(light).unwrap();
        material.base_color = Vec3::ONE;
        material.emission = Vec3::new(12.0, 10.5, 9.0);
        scene
    }
}

impl Scene {
    /// A valid scene with an empty root collection and no objects or data.
    pub fn empty() -> Self {
        Self {
            objects: Vec::new(),
            meshes: Vec::new(),
            materials: Vec::new(),
            collections: vec![Collection {
                id: 1,
                name: "Scene Collection".into(),
                parent: None,
                visible: true,
            }],
            root_collection: 1,
            camera: Camera::default(),
            world: World::default(),
            render: RenderPreferences::default(),
            next_id: 2,
        }
    }

    pub fn add(&mut self, primitive: Primitive) -> u64 {
        self.add_mesh_object(primitive.label(), Mesh::primitive(primitive))
    }

    fn add_mesh_object(&mut self, name: impl Into<String>, mesh: Mesh) -> u64 {
        let name = name.into();
        let mesh_id = self.allocate_id();
        let material_id = self.allocate_id();
        let id = self.allocate_id();
        self.meshes.push(MeshData {
            id: mesh_id,
            name: self.unique_mesh_name(&name),
            mesh,
        });
        self.materials.push(MaterialData {
            id: material_id,
            name: self.unique_material_name("Material"),
            material: Material::default(),
        });
        self.objects.push(Object {
            id,
            name: self.unique_object_name(&name),
            transform: Transform::default(),
            parent: None,
            parent_inverse: Mat4::IDENTITY,
            data: ObjectData::Mesh {
                mesh: mesh_id,
                materials: vec![material_id],
            },
            visible: true,
            selectable: true,
            collections: vec![self.root_collection],
        });
        id
    }

    /// Add mesh data without creating an object. This is the low-level path for
    /// importers, generators and procedural tools that want to instance it later.
    pub fn create_mesh_data(&mut self, name: impl Into<String>, mesh: Mesh) -> Result<u64> {
        mesh.validate().context("Invalid mesh data")?;
        ensure!(self.meshes.len() < 10_000, "Scene exceeds 10,000 meshes");
        let vertices: usize = self
            .meshes
            .iter()
            .map(|data| data.mesh.positions.len())
            .sum();
        let faces: usize = self.meshes.iter().map(|data| data.mesh.faces.len()).sum();
        ensure!(
            vertices + mesh.positions.len() <= 5_000_000 && faces + mesh.faces.len() <= 5_000_000,
            "Scene exceeds five million vertices or faces"
        );
        let name = name.into();
        ensure!(valid_name(&name), "Invalid mesh data-block name");
        let id = self.allocate_id();
        self.meshes.push(MeshData {
            id,
            name: self.unique_mesh_name(&name),
            mesh,
        });
        Ok(id)
    }

    /// Add material data without assigning it to an object.
    pub fn create_material_data(
        &mut self,
        name: impl Into<String>,
        material: Material,
    ) -> Result<u64> {
        validate_material(&material)?;
        ensure!(
            self.materials.len() < 100_000,
            "Scene exceeds 100,000 materials"
        );
        let name = name.into();
        ensure!(valid_name(&name), "Invalid material data-block name");
        let id = self.allocate_id();
        self.materials.push(MaterialData {
            id,
            name: self.unique_material_name(&name),
            material,
        });
        Ok(id)
    }

    /// Create an object that instances existing mesh and material data blocks.
    pub fn instantiate_mesh(
        &mut self,
        name: impl Into<String>,
        mesh: u64,
        materials: Vec<u64>,
    ) -> Result<u64> {
        ensure!(self.mesh(mesh).is_some(), "Mesh data does not exist");
        ensure!(
            !materials.is_empty() && materials.iter().all(|id| self.material(*id).is_some()),
            "At least one valid material is required"
        );
        ensure!(self.objects.len() < 10_000, "Scene exceeds 10,000 objects");
        self.add_object(name, ObjectData::Mesh { mesh, materials })
    }

    pub fn add_empty(&mut self, name: impl Into<String>) -> Result<u64> {
        self.add_object(name, ObjectData::Empty { display_size: 1.0 })
    }

    pub fn add_light(&mut self, name: impl Into<String>, light: Light) -> Result<u64> {
        self.add_object(name, ObjectData::Light { light })
    }

    pub fn add_camera(&mut self, name: impl Into<String>, camera: CameraData) -> Result<u64> {
        self.add_object(name, ObjectData::Camera { camera })
    }

    pub fn add_object(&mut self, name: impl Into<String>, data: ObjectData) -> Result<u64> {
        let name = name.into();
        ensure!(self.objects.len() < 10_000, "Scene exceeds 10,000 objects");
        ensure!(valid_name(&name), "Invalid object name");
        match &data {
            ObjectData::Mesh { mesh, materials } => {
                ensure!(self.mesh(*mesh).is_some(), "Mesh data does not exist");
                ensure!(
                    !materials.is_empty()
                        && materials.len() <= 256
                        && materials.iter().all(|id| self.material(*id).is_some()),
                    "One to 256 valid materials are required"
                );
            }
            ObjectData::Empty { display_size } => ensure!(
                display_size.is_finite() && (0.0001..=1.0e6).contains(display_size),
                "Invalid empty display size"
            ),
            ObjectData::Light { light } => validate_light(light)?,
            ObjectData::Camera { camera } => validate_camera_data(camera)?,
            ObjectData::Custom { kind, properties } => {
                ensure!(valid_name(kind), "Invalid custom object type");
                ensure!(
                    properties.len() <= 1024,
                    "Custom object has too many properties"
                );
            }
        }
        let id = self.allocate_id();
        self.objects.push(Object {
            id,
            name: self.unique_object_name(&name),
            transform: Transform::default(),
            parent: None,
            parent_inverse: Mat4::IDENTITY,
            data,
            visible: true,
            selectable: true,
            collections: vec![self.root_collection],
        });
        Ok(id)
    }

    pub fn object(&self, id: u64) -> Option<&Object> {
        let index = id_index(&self.objects, id, |object| object.id)?;
        self.objects.get(index)
    }
    pub fn object_mut(&mut self, id: u64) -> Option<&mut Object> {
        let index = id_index(&self.objects, id, |object| object.id)?;
        self.objects.get_mut(index)
    }

    pub fn mesh(&self, id: u64) -> Option<&MeshData> {
        let index = id_index(&self.meshes, id, |mesh| mesh.id)?;
        self.meshes.get(index)
    }

    pub fn mesh_mut(&mut self, id: u64) -> Option<&mut MeshData> {
        let index = id_index(&self.meshes, id, |mesh| mesh.id)?;
        self.meshes.get_mut(index)
    }

    pub fn material(&self, id: u64) -> Option<&MaterialData> {
        let index = id_index(&self.materials, id, |material| material.id)?;
        self.materials.get(index)
    }

    pub fn material_mut(&mut self, id: u64) -> Option<&mut MaterialData> {
        let index = id_index(&self.materials, id, |material| material.id)?;
        self.materials.get_mut(index)
    }

    pub fn collection(&self, id: u64) -> Option<&Collection> {
        let index = id_index(&self.collections, id, |collection| collection.id)?;
        self.collections.get(index)
    }

    pub fn collection_mut(&mut self, id: u64) -> Option<&mut Collection> {
        let index = id_index(&self.collections, id, |collection| collection.id)?;
        self.collections.get_mut(index)
    }

    pub fn object_mesh(&self, object_id: u64) -> Option<&Mesh> {
        let mesh_id = self.object(object_id)?.mesh_id()?;
        Some(&self.mesh(mesh_id)?.mesh)
    }

    pub fn object_mesh_mut(&mut self, object_id: u64) -> Option<&mut Mesh> {
        let mesh_id = self.object(object_id)?.mesh_id()?;
        Some(&mut self.mesh_mut(mesh_id)?.mesh)
    }

    pub fn object_material(&self, object_id: u64) -> Option<&Material> {
        let material_id = self.object(object_id)?.data.material_ids().first()?;
        Some(&self.material(*material_id)?.material)
    }

    pub fn object_material_mut(&mut self, object_id: u64) -> Option<&mut Material> {
        let material_id = *self.object(object_id)?.data.material_ids().first()?;
        Some(&mut self.material_mut(material_id)?.material)
    }

    pub fn mesh_users(&self, mesh_id: u64) -> usize {
        self.objects
            .iter()
            .filter(|object| object.mesh_id() == Some(mesh_id))
            .count()
    }

    pub fn material_users(&self, material_id: u64) -> usize {
        self.objects
            .iter()
            .filter(|object| object.data.material_ids().contains(&material_id))
            .count()
    }

    pub fn assign_mesh_data(&mut self, object_id: u64, mesh_id: u64) -> Result<()> {
        ensure!(self.mesh(mesh_id).is_some(), "Mesh data does not exist");
        let object = self
            .object_mut(object_id)
            .context("Object does not exist")?;
        let ObjectData::Mesh { mesh, .. } = &mut object.data else {
            anyhow::bail!("Object is not a mesh");
        };
        *mesh = mesh_id;
        Ok(())
    }

    pub fn assign_material(&mut self, object_id: u64, slot: usize, material_id: u64) -> Result<()> {
        ensure!(
            self.material(material_id).is_some(),
            "Material data does not exist"
        );
        let object = self
            .object_mut(object_id)
            .context("Object does not exist")?;
        let ObjectData::Mesh { materials, .. } = &mut object.data else {
            anyhow::bail!("Object is not a mesh");
        };
        let target = materials
            .get_mut(slot)
            .context("Material slot does not exist")?;
        *target = material_id;
        Ok(())
    }

    pub fn add_material_slot(&mut self, object_id: u64, material_id: u64) -> Result<usize> {
        ensure!(
            self.material(material_id).is_some(),
            "Material data does not exist"
        );
        let object = self
            .object_mut(object_id)
            .context("Object does not exist")?;
        let ObjectData::Mesh { materials, .. } = &mut object.data else {
            anyhow::bail!("Object is not a mesh");
        };
        ensure!(materials.len() < 256, "Object exceeds 256 material slots");
        materials.push(material_id);
        Ok(materials.len() - 1)
    }

    /// Give one object a private copy of linked mesh data before a local edit.
    pub fn make_mesh_single_user(&mut self, object_id: u64) -> Result<u64> {
        let mesh_id = self
            .object(object_id)
            .and_then(Object::mesh_id)
            .context("Object is not a mesh")?;
        if self.mesh_users(mesh_id) <= 1 {
            return Ok(mesh_id);
        }
        let source = self.mesh(mesh_id).unwrap().clone();
        let new_id = self.create_mesh_data(source.name, source.mesh)?;
        let object = self.object_mut(object_id).unwrap();
        if let ObjectData::Mesh { mesh, .. } = &mut object.data {
            *mesh = new_id;
        }
        Ok(new_id)
    }

    /// Give one object's material slot a private copy before a local edit.
    pub fn make_material_single_user(&mut self, object_id: u64, slot: usize) -> Result<u64> {
        let material_id = *self
            .object(object_id)
            .context("Object does not exist")?
            .data
            .material_ids()
            .get(slot)
            .context("Material slot does not exist")?;
        if self.material_users(material_id) <= 1 {
            return Ok(material_id);
        }
        let source = self.material(material_id).unwrap().clone();
        let new_id = self.create_material_data(source.name, source.material)?;
        let object = self.object_mut(object_id).unwrap();
        if let ObjectData::Mesh { materials, .. } = &mut object.data {
            materials[slot] = new_id;
        }
        Ok(new_id)
    }

    pub fn mesh_instance(&self, id: u64) -> Option<MeshInstance<'_>> {
        let object = self.object(id)?;
        Some(MeshInstance {
            object,
            mesh: self.object_mesh(id)?,
            material: self.object_material(id)?,
            world_transform: self.world_transform(id)?,
        })
    }

    pub fn mesh_instances(&self) -> impl Iterator<Item = MeshInstance<'_>> + '_ {
        self.objects
            .iter()
            .filter_map(|object| self.mesh_instance(object.id))
    }

    pub fn world_transform(&self, id: u64) -> Option<Mat4> {
        let mut current = Some(id);
        let mut matrix = Mat4::IDENTITY;
        let mut visited = BTreeSet::new();
        while let Some(object_id) = current {
            if !visited.insert(object_id) {
                return None;
            }
            let object = self.object(object_id)?;
            let local = object.parent_inverse * object.transform.matrix();
            matrix = local * matrix;
            current = object.parent;
        }
        Some(matrix)
    }

    pub fn is_effectively_visible(&self, id: u64) -> bool {
        let Some(object) = self.object(id) else {
            return false;
        };
        let mut current = Some(id);
        let mut visited = BTreeSet::new();
        while let Some(object_id) = current {
            let Some(object) = self.object(object_id) else {
                return false;
            };
            if !visited.insert(object_id) || !object.visible {
                return false;
            }
            current = object.parent;
        }
        object.collections.iter().any(|&collection_id| {
            let mut current = Some(collection_id);
            let mut visited = BTreeSet::new();
            while let Some(id) = current {
                let Some(collection) = self.collection(id) else {
                    return false;
                };
                if !visited.insert(id) || !collection.visible {
                    return false;
                }
                current = collection.parent;
            }
            true
        })
    }

    pub fn set_parent(&mut self, child: u64, parent: Option<u64>, keep_world: bool) -> Result<()> {
        ensure!(self.object(child).is_some(), "Child object does not exist");
        if let Some(parent) = parent {
            ensure!(
                self.object(parent).is_some(),
                "Parent object does not exist"
            );
            ensure!(parent != child, "An object cannot parent itself");
            let mut current = Some(parent);
            while let Some(id) = current {
                ensure!(id != child, "Parenting would create a cycle");
                current = self.object(id).and_then(|object| object.parent);
            }
        }
        let (local, parent_inverse) = if keep_world {
            let child_world = self
                .world_transform(child)
                .context("Invalid child hierarchy")?;
            let parent_world = parent
                .map(|parent| {
                    self.world_transform(parent)
                        .context("Invalid parent hierarchy")
                })
                .transpose()?
                .unwrap_or(Mat4::IDENTITY);
            let local = self.object(child).unwrap().transform;
            (
                local,
                parent_world.inverse() * child_world * local.matrix().inverse(),
            )
        } else {
            (self.object(child).unwrap().transform, Mat4::IDENTITY)
        };
        ensure!(
            parent_inverse.is_finite() && parent_inverse.determinant().abs() >= 1.0e-12,
            "Could not preserve an invertible parent transform"
        );
        let object = self.object_mut(child).unwrap();
        object.parent = parent;
        object.transform = local;
        object.parent_inverse = parent_inverse;
        Ok(())
    }

    pub fn add_collection(&mut self, name: impl Into<String>, parent: Option<u64>) -> Result<u64> {
        let parent = parent.unwrap_or(self.root_collection);
        ensure!(
            self.collection(parent).is_some(),
            "Parent collection does not exist"
        );
        let name = name.into();
        let id = self.allocate_id();
        self.collections.push(Collection {
            id,
            name: self.unique_collection_name(&name),
            parent: Some(parent),
            visible: true,
        });
        Ok(id)
    }

    pub fn set_collection_parent(&mut self, child: u64, parent: u64) -> Result<()> {
        ensure!(
            child != self.root_collection,
            "Root collection cannot be parented"
        );
        ensure!(child != parent, "A collection cannot parent itself");
        ensure!(
            self.collection(child).is_some(),
            "Collection does not exist"
        );
        ensure!(
            self.collection(parent).is_some(),
            "Parent collection does not exist"
        );
        let mut current = Some(parent);
        while let Some(id) = current {
            ensure!(id != child, "Parenting would create a collection cycle");
            current = self.collection(id).and_then(|collection| collection.parent);
        }
        self.collection_mut(child).unwrap().parent = Some(parent);
        Ok(())
    }

    /// Remove a non-root collection while preserving its objects and children
    /// by linking them to the removed collection's parent.
    pub fn remove_collection(&mut self, id: u64) -> Result<()> {
        ensure!(
            id != self.root_collection,
            "Root collection cannot be removed"
        );
        let parent = self
            .collection(id)
            .context("Collection does not exist")?
            .parent
            .unwrap_or(self.root_collection);
        for object in &mut self.objects {
            if object.collections.contains(&id) {
                object.collections.retain(|collection| *collection != id);
                if !object.collections.contains(&parent) {
                    object.collections.push(parent);
                }
            }
        }
        for collection in &mut self.collections {
            if collection.parent == Some(id) {
                collection.parent = Some(parent);
            }
        }
        self.collections.retain(|collection| collection.id != id);
        Ok(())
    }

    pub fn link_object(&mut self, object_id: u64, collection_id: u64) -> Result<()> {
        ensure!(
            self.collection(collection_id).is_some(),
            "Collection does not exist"
        );
        let object = self
            .object_mut(object_id)
            .context("Object does not exist")?;
        if !object.collections.contains(&collection_id) {
            object.collections.push(collection_id);
        }
        Ok(())
    }

    pub fn unlink_object(&mut self, object_id: u64, collection_id: u64) -> Result<()> {
        let root = self.root_collection;
        let object = self
            .object_mut(object_id)
            .context("Object does not exist")?;
        object.collections.retain(|&id| id != collection_id);
        if object.collections.is_empty() {
            object.collections.push(root);
        }
        Ok(())
    }
    pub fn remove(&mut self, id: u64) {
        let children: Vec<_> = self
            .objects
            .iter()
            .filter(|object| object.parent == Some(id))
            .filter_map(|object| {
                self.world_transform(object.id)
                    .map(|world| (object.id, world))
            })
            .collect();
        self.objects.retain(|o| o.id != id);
        for (child, world) in children {
            if let Some(object) = self.object_mut(child) {
                object.parent = None;
                object.parent_inverse = world * object.transform.matrix().inverse();
            }
        }
    }

    pub fn duplicate(&mut self, id: u64) -> Option<u64> {
        self.duplicate_impl(id, false)
    }

    /// Duplicate only the object, sharing its data blocks like Blender's
    /// linked duplicate. Editing the mesh or material updates every instance.
    pub fn duplicate_linked(&mut self, id: u64) -> Option<u64> {
        self.duplicate_impl(id, true)
    }

    fn duplicate_impl(&mut self, id: u64, linked: bool) -> Option<u64> {
        let mut object = self.object(id)?.clone();
        if !linked && let ObjectData::Mesh { mesh, materials } = &mut object.data {
            let source_mesh = self.mesh(*mesh)?.clone();
            let mesh_id = self.allocate_id();
            self.meshes.push(MeshData {
                id: mesh_id,
                name: self.unique_mesh_name(&source_mesh.name),
                mesh: source_mesh.mesh,
            });
            *mesh = mesh_id;
            let source_materials: Vec<_> = materials
                .iter()
                .filter_map(|id| self.material(*id).cloned())
                .collect();
            materials.clear();
            for source in source_materials {
                let material_id = self.allocate_id();
                self.materials.push(MaterialData {
                    id: material_id,
                    name: self.unique_material_name(&source.name),
                    material: source.material,
                });
                materials.push(material_id);
            }
        }
        let new_id = self.allocate_id();
        object.id = new_id;
        object.name = self.unique_object_name(&object.name);
        object.transform.translation += Vec3::new(0.4, 0.0, 0.4);
        self.objects.push(object);
        Some(new_id)
    }

    /// Remove mesh and material blocks that no object references.
    pub fn purge_orphans(&mut self) -> (usize, usize) {
        let meshes: BTreeSet<_> = self.objects.iter().filter_map(Object::mesh_id).collect();
        let materials: BTreeSet<_> = self
            .objects
            .iter()
            .flat_map(|object| object.data.material_ids().iter().copied())
            .collect();
        let before_meshes = self.meshes.len();
        let before_materials = self.materials.len();
        self.meshes.retain(|mesh| meshes.contains(&mesh.id));
        self.materials
            .retain(|material| materials.contains(&material.id));
        (
            before_meshes - self.meshes.len(),
            before_materials - self.materials.len(),
        )
    }

    pub fn pick(&self, ray: Ray) -> Option<Hit> {
        if !ray.origin.is_finite() || !ray.direction.is_finite() {
            return None;
        }
        let direction = ray.direction.normalize_or_zero();
        if direction == Vec3::ZERO {
            return None;
        }
        let mut closest: Option<Hit> = None;
        for instance in self
            .mesh_instances()
            .filter(|instance| instance.selectable && self.is_effectively_visible(instance.id))
        {
            let object = instance.object;
            let matrix = instance.world_transform;
            if !matrix.is_finite() || matrix.determinant() == 0.0 {
                continue;
            }
            let inverse = matrix.inverse();
            if !inverse.is_finite() {
                continue;
            }
            let origin = inverse.transform_point3(ray.origin);
            // Keep this direction unnormalized: t remains world-space distance
            // under non-uniform object scaling.
            let local_direction = inverse.transform_vector3(direction);
            for (triangle, face) in instance.mesh.triangles_with_faces() {
                let a = instance.mesh.positions[triangle[0] as usize];
                let b = instance.mesh.positions[triangle[1] as usize];
                let c = instance.mesh.positions[triangle[2] as usize];
                if let Some(distance) = intersect_triangle(origin, local_direction, a, b, c)
                    && closest.is_none_or(|hit| distance < hit.distance)
                {
                    closest = Some(Hit {
                        object_id: object.id,
                        face,
                        distance,
                        position: ray.origin + direction * distance,
                    });
                }
            }
        }
        closest
    }

    pub fn bounds(&self, id: u64) -> Option<(Vec3, f32)> {
        let instance = self.mesh_instance(id)?;
        if instance.mesh.positions.is_empty() {
            return None;
        }
        let matrix = instance.world_transform;
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for point in &instance.mesh.positions {
            let point = matrix.transform_point3(*point);
            min = min.min(point);
            max = max.max(point);
        }
        let center = (min + max) * 0.5;
        let radius = instance
            .mesh
            .positions
            .iter()
            .map(|p| matrix.transform_point3(*p).distance(center))
            .fold(0.0_f32, f32::max);
        Some((center, radius.max(0.001)))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate().context("Cannot save an invalid scene")?;
        let document = SceneFile {
            format: "forma".into(),
            version: FILE_VERSION,
            scene: serde_json::to_value(self).context("Could not serialize scene")?,
        };
        let bytes = serde_json::to_vec_pretty(&document).context("Could not serialize scene")?;
        ensure!(
            bytes.len() as u64 <= MAX_FILE_BYTES,
            "Scene exceeds the 256 MiB file limit"
        );
        atomic_write(path, &bytes).with_context(|| format!("Could not save {}", path.display()))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_limited(path)?;
        let document: SceneFile =
            serde_json::from_slice(&bytes).context("Invalid Forma scene JSON")?;
        ensure!(document.format == "forma", "This is not a Forma scene");
        let scene = match document.version {
            1 => Self::from_legacy(
                serde_json::from_value(document.scene).context("Invalid version 1 scene")?,
            )?,
            FILE_VERSION => {
                serde_json::from_value(document.scene).context("Invalid version 2 scene")?
            }
            version => anyhow::bail!("Unsupported Forma scene version {version}"),
        };
        scene.validate().context("Invalid scene data")?;
        Ok(scene)
    }

    pub fn import_obj(&mut self, path: &Path) -> Result<u64> {
        ensure!(
            self.objects.len() < 10_000,
            "Import would exceed the 10,000 object limit"
        );
        let mesh = crate::obj::read(path)?;
        let vertices: usize = self
            .meshes
            .iter()
            .map(|data| data.mesh.positions.len())
            .sum();
        let faces: usize = self.meshes.iter().map(|data| data.mesh.faces.len()).sum();
        ensure!(
            vertices + mesh.positions.len() <= 5_000_000 && faces + mesh.faces.len() <= 5_000_000,
            "Import would exceed the five million scene vertex/face limit"
        );
        ensure!(
            self.next_id < u64::MAX - 3,
            "Scene has exhausted object IDs"
        );
        let label = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Imported mesh");
        let label: String = label.chars().take(100).collect();
        Ok(self.add_mesh_object(label, mesh))
    }

    pub fn export_obj(&self, path: &Path) -> Result<()> {
        crate::obj::write(self, path)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.objects.len() <= 10_000, "Scene exceeds 10,000 objects");
        ensure!(self.meshes.len() <= 10_000, "Scene exceeds 10,000 meshes");
        ensure!(
            self.materials.len() <= 100_000,
            "Scene exceeds 100,000 materials"
        );
        ensure!(
            !self.collections.is_empty() && self.collections.len() <= 10_000,
            "Scene must contain 1–10,000 collections"
        );
        let mut ids = BTreeSet::new();
        let mut total_vertices = 0_usize;
        let mut total_faces = 0_usize;
        for collection in &self.collections {
            ensure!(
                collection.id > 0 && ids.insert(collection.id),
                "Data-block IDs must be nonzero and globally unique"
            );
            ensure!(valid_name(&collection.name), "Invalid collection name");
            if let Some(parent) = collection.parent {
                ensure!(
                    self.collection(parent).is_some(),
                    "Collection {} has a missing parent",
                    collection.name
                );
            }
            if collection.id != self.root_collection {
                ensure!(
                    collection.parent.is_some(),
                    "Only the root collection may have no parent"
                );
            }
        }
        let root = self
            .collection(self.root_collection)
            .context("Root collection does not exist")?;
        ensure!(
            root.parent.is_none(),
            "Root collection cannot have a parent"
        );
        for collection in &self.collections {
            self.validate_collection_chain(collection.id)?;
        }
        for data in &self.meshes {
            ensure!(
                data.id > 0 && ids.insert(data.id),
                "Data-block IDs must be nonzero and globally unique"
            );
            ensure!(valid_name(&data.name), "Invalid mesh data-block name");
            data.mesh
                .validate()
                .with_context(|| format!("Invalid mesh data in {}", data.name))?;
            total_vertices += data.mesh.positions.len();
            total_faces += data.mesh.faces.len();
            ensure!(
                total_vertices <= 5_000_000 && total_faces <= 5_000_000,
                "Scene exceeds five million vertices or faces"
            );
        }
        for data in &self.materials {
            ensure!(
                data.id > 0 && ids.insert(data.id),
                "Data-block IDs must be nonzero and globally unique"
            );
            ensure!(valid_name(&data.name), "Invalid material data-block name");
            validate_material(&data.material)
                .with_context(|| format!("Invalid material in {}", data.name))?;
        }
        for object in &self.objects {
            ensure!(
                object.id > 0 && ids.insert(object.id),
                "Data-block IDs must be nonzero and globally unique"
            );
            ensure!(valid_name(&object.name), "Invalid object name");
            let t = object.transform;
            ensure!(
                t.translation.is_finite() && t.rotation.is_finite() && t.scale.is_finite(),
                "Non-finite transform in {}",
                object.name
            );
            ensure!(
                object.parent_inverse.is_finite()
                    && object.parent_inverse.determinant().abs() >= 1.0e-12,
                "Invalid parent inverse in {}",
                object.name
            );
            ensure!(
                t.translation.abs().max_element() <= 1.0e8
                    && t.scale.abs().max_element() <= 1.0e6
                    && t.scale.abs().min_element() >= 1.0e-5,
                "Transform out of range in {}",
                object.name
            );
            ensure!(
                !object.collections.is_empty()
                    && object
                        .collections
                        .iter()
                        .all(|id| self.collection(*id).is_some()),
                "Object {} is not linked to a valid collection",
                object.name
            );
            ensure!(
                object
                    .collections
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .len()
                    == object.collections.len(),
                "Object {} repeats a collection link",
                object.name
            );
            if let Some(parent) = object.parent {
                ensure!(
                    self.object(parent).is_some(),
                    "Object {} has a missing parent",
                    object.name
                );
            }
            match &object.data {
                ObjectData::Mesh { mesh, materials } => {
                    ensure!(
                        self.mesh(*mesh).is_some(),
                        "Object {} has a missing mesh",
                        object.name
                    );
                    ensure!(
                        !materials.is_empty()
                            && materials.len() <= 256
                            && materials.iter().all(|id| self.material(*id).is_some()),
                        "Object {} has missing material data",
                        object.name
                    );
                }
                ObjectData::Empty { display_size } => ensure!(
                    display_size.is_finite() && (0.0001..=1.0e6).contains(display_size),
                    "Invalid empty display size in {}",
                    object.name
                ),
                ObjectData::Light { light } => validate_light(light)
                    .with_context(|| format!("Invalid light in {}", object.name))?,
                ObjectData::Camera { camera } => validate_camera_data(camera)
                    .with_context(|| format!("Invalid camera data in {}", object.name))?,
                ObjectData::Custom { kind, properties } => {
                    ensure!(valid_name(kind), "Invalid custom object type");
                    ensure!(
                        properties.len() <= 1024,
                        "Custom object has too many properties"
                    );
                }
            }
        }
        for object in &self.objects {
            self.validate_object_chain(object.id)?;
        }
        ensure!(
            self.next_id > ids.last().copied().unwrap_or(0) && self.next_id < u64::MAX,
            "Next object ID must exceed existing IDs"
        );
        ensure!(
            valid_color(self.world.color, 1.0e6)
                && self.world.strength.is_finite()
                && (0.0..=1.0e6).contains(&self.world.strength),
            "Invalid world illumination"
        );
        let c = self.camera;
        ensure!(
            c.target.is_finite()
                && c.target.abs().max_element() <= 1.0e8
                && c.yaw.is_finite()
                && c.pitch.is_finite()
                && c.pitch.abs() <= std::f32::consts::FRAC_PI_2
                && c.distance.is_finite()
                && (0.001..=1.0e6).contains(&c.distance)
                && c.fov_y.is_finite()
                && (0.01..3.13).contains(&c.fov_y),
            "Invalid camera"
        );
        let render = self.render;
        ensure!(
            render.exposure.is_finite()
                && (-10.0..=10.0).contains(&render.exposure)
                && (1..=4096).contains(&render.max_samples)
                && (1..=32).contains(&render.max_bounces)
                && (1..=4096).contains(&render.denoise.start_sample),
            "Invalid render preferences"
        );
        Ok(())
    }

    fn allocate_id(&mut self) -> u64 {
        let highest = self
            .objects
            .iter()
            .map(|entry| entry.id)
            .chain(self.meshes.iter().map(|entry| entry.id))
            .chain(self.materials.iter().map(|entry| entry.id))
            .chain(self.collections.iter().map(|entry| entry.id))
            .max()
            .unwrap_or(0);
        let id = self.next_id.max(highest.saturating_add(1)).max(1);
        // Validated documents reserve the final ID; exhausting u64 IDs is not
        // possible through normal editor use.
        self.next_id = id.saturating_add(1);
        id
    }

    fn unique_object_name(&self, stem: &str) -> String {
        unique_name(stem, self.objects.iter().map(|entry| entry.name.as_str()))
    }

    fn unique_mesh_name(&self, stem: &str) -> String {
        unique_name(stem, self.meshes.iter().map(|entry| entry.name.as_str()))
    }

    fn unique_material_name(&self, stem: &str) -> String {
        unique_name(stem, self.materials.iter().map(|entry| entry.name.as_str()))
    }

    fn unique_collection_name(&self, stem: &str) -> String {
        unique_name(
            stem,
            self.collections.iter().map(|entry| entry.name.as_str()),
        )
    }

    fn validate_object_chain(&self, id: u64) -> Result<()> {
        let mut current = Some(id);
        let mut visited = BTreeSet::new();
        while let Some(id) = current {
            ensure!(visited.insert(id), "Object hierarchy contains a cycle");
            current = self
                .object(id)
                .context("Object hierarchy is broken")?
                .parent;
        }
        Ok(())
    }

    fn validate_collection_chain(&self, id: u64) -> Result<()> {
        let mut current = Some(id);
        let mut visited = BTreeSet::new();
        while let Some(id) = current {
            ensure!(visited.insert(id), "Collection hierarchy contains a cycle");
            current = self
                .collection(id)
                .context("Collection hierarchy is broken")?
                .parent;
        }
        Ok(())
    }

    fn from_legacy(legacy: LegacyScene) -> Result<Self> {
        let highest = legacy
            .objects
            .iter()
            .map(|object| object.id)
            .max()
            .unwrap_or(0);
        let mut scene = Self {
            objects: Vec::new(),
            meshes: Vec::new(),
            materials: Vec::new(),
            collections: Vec::new(),
            root_collection: 0,
            camera: legacy.camera,
            world: legacy.world,
            render: legacy.render,
            next_id: legacy.next_id.max(highest.saturating_add(1)).max(1),
        };
        let root = scene.allocate_id();
        scene.root_collection = root;
        scene.collections.push(Collection {
            id: root,
            name: "Scene Collection".into(),
            parent: None,
            visible: true,
        });
        for object in legacy.objects {
            let mesh_id = scene.allocate_id();
            let material_id = scene.allocate_id();
            scene.meshes.push(MeshData {
                id: mesh_id,
                name: scene.unique_mesh_name(&object.name),
                mesh: object.mesh,
            });
            scene.materials.push(MaterialData {
                id: material_id,
                name: scene.unique_material_name("Material"),
                material: object.material,
            });
            scene.objects.push(Object {
                id: object.id,
                name: object.name,
                transform: object.transform,
                parent: None,
                parent_inverse: Mat4::IDENTITY,
                data: ObjectData::Mesh {
                    mesh: mesh_id,
                    materials: vec![material_id],
                },
                visible: object.visible,
                selectable: true,
                collections: vec![root],
            });
        }
        Ok(scene)
    }
}

fn unique_name<'a>(stem: &str, existing: impl Iterator<Item = &'a str>) -> String {
    let existing: BTreeSet<_> = existing.collect();
    if !existing.contains(stem) {
        return stem.to_owned();
    }
    for suffix in 1.. {
        let candidate = format!("{stem}.{suffix:03}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!()
}

/// Allocated data normally remains ID-sorted, making lookups logarithmic. The
/// fallback keeps APIs useful while an editor is explicitly reordering vectors
/// or diagnosing an invalid third-party document.
fn id_index<T>(values: &[T], id: u64, key: impl Fn(&T) -> u64) -> Option<usize> {
    values
        .binary_search_by_key(&id, &key)
        .ok()
        .or_else(|| values.iter().position(|value| key(value) == id))
}

fn default_true() -> bool {
    true
}

fn identity_matrix() -> Mat4 {
    Mat4::IDENTITY
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 512 && !name.chars().any(char::is_control)
}

fn validate_material(material: &Material) -> Result<()> {
    ensure!(
        valid_color(material.base_color, 1.0)
            && valid_color(material.emission, 1.0e6)
            && material.metallic.is_finite()
            && (0.0..=1.0).contains(&material.metallic)
            && material.roughness.is_finite()
            && (0.0..=1.0).contains(&material.roughness),
        "Invalid material values"
    );
    Ok(())
}

fn validate_light(light: &Light) -> Result<()> {
    ensure!(
        valid_color(light.color, 1.0)
            && light.energy.is_finite()
            && (0.0..=1.0e9).contains(&light.energy)
            && light.radius.is_finite()
            && (0.0..=1.0e6).contains(&light.radius)
            && light.spot_angle.is_finite()
            && (0.001..=std::f32::consts::PI).contains(&light.spot_angle),
        "Invalid light values"
    );
    Ok(())
}

fn validate_camera_data(camera: &CameraData) -> Result<()> {
    ensure!(
        camera.focal_length_mm.is_finite()
            && (1.0..=10_000.0).contains(&camera.focal_length_mm)
            && camera.sensor_width_mm.is_finite()
            && (1.0..=1_000.0).contains(&camera.sensor_width_mm)
            && camera.orthographic_scale.is_finite()
            && (0.0001..=1.0e8).contains(&camera.orthographic_scale)
            && camera.near.is_finite()
            && camera.far.is_finite()
            && camera.near > 0.0
            && camera.far > camera.near,
        "Invalid camera values"
    );
    Ok(())
}

fn valid_color(value: Vec3, max: f32) -> bool {
    value.is_finite() && value.min_element() >= 0.0 && value.max_element() <= max
}

fn intersect_triangle(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    // Double precision and a scale-relative parallel threshold keep selection
    // reliable for very small imported parts and very large object transforms.
    let (origin, direction, a, b, c) = (
        origin.as_dvec3(),
        direction.as_dvec3(),
        a.as_dvec3(),
        b.as_dvec3(),
        c.as_dvec3(),
    );
    let ab = b - a;
    let ac = c - a;
    let p = direction.cross(ac);
    let determinant = ab.dot(p);
    if determinant.abs() <= 1.0e-12 * ab.length() * ac.length() * direction.length() {
        return None;
    }
    let inv = determinant.recip();
    let t = origin - a;
    let u = t.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = t.cross(ab);
    let v = direction.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = ac.dot(q) * inv;
    (distance > 0.0 && distance.is_finite() && distance < f32::MAX as f64)
        .then_some(distance as f32)
}

pub(crate) fn read_limited(path: &Path) -> Result<Vec<u8>> {
    let file =
        fs::File::open(path).with_context(|| format!("Could not open {}", path.display()))?;
    ensure!(
        file.metadata()?.len() <= MAX_FILE_BYTES,
        "File exceeds the 256 MiB limit"
    );
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE_BYTES,
        "File exceeds the 256 MiB limit"
    );
    Ok(bytes)
}

/// A same-directory temporary file followed by rename prevents a failed save
/// from truncating the user's existing document.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path.file_name().context("A file name is required")?;
    let mut temporary_name = name.to_os_string();
    temporary_name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = parent.join(temporary_name);
    let mut created = false;
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        created = true;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        // Best effort directory sync (unsupported by some platforms). The data
        // itself has already been synced and atomically installed.
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(&temporary);
    }
    result
}

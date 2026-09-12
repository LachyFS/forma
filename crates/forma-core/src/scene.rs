use crate::{Camera, Mesh, Primitive, Ray};
use anyhow::{Context, Result, ensure};
use glam::{EulerRot, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub id: u64,
    pub name: String,
    pub mesh: Mesh,
    pub transform: Transform,
    pub material: Material,
    pub visible: bool,
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
}

impl Default for RenderPreferences {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            max_samples: 128,
            max_bounces: 8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub objects: Vec<Object>,
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
    scene: Scene,
}

impl Default for Scene {
    fn default() -> Self {
        let mut scene = Self {
            objects: Vec::new(),
            camera: Camera::default(),
            world: World::default(),
            render: RenderPreferences::default(),
            next_id: 1,
        };
        let torus = scene.add(Primitive::Torus);
        let object = scene.object_mut(torus).unwrap();
        object.name = "Torus".into();
        object.transform.translation = Vec3::new(-0.75, 1.12, 0.0);
        object.transform.rotation = Vec3::new(1.22, 0.0, -0.23);
        object.material = Material {
            base_color: Vec3::new(0.035, 0.46, 0.38),
            metallic: 0.45,
            roughness: 0.24,
            emission: Vec3::ZERO,
        };

        let sphere = scene.add(Primitive::Sphere);
        let object = scene.object_mut(sphere).unwrap();
        object.transform.translation = Vec3::new(1.05, 0.61, 0.6);
        object.transform.scale = Vec3::splat(0.6);
        object.material = Material {
            base_color: Vec3::new(0.72, 0.50, 0.30),
            metallic: 0.72,
            roughness: 0.19,
            emission: Vec3::ZERO,
        };

        let cube = scene.add(Primitive::Cube);
        let object = scene.object_mut(cube).unwrap();
        object.name = "Rounded cube".into();
        object.mesh.subdivide();
        object.mesh.subdivide();
        object.transform.translation = Vec3::new(1.0, 0.59, -1.0);
        object.transform.scale = Vec3::splat(0.8);
        object.transform.rotation.y = 0.22;
        object.material.base_color = Vec3::new(0.61, 0.65, 0.71);
        object.material.roughness = 0.28;

        let platform = scene.add(Primitive::Cube);
        let object = scene.object_mut(platform).unwrap();
        object.name = "Studio plinth".into();
        object.transform.translation = Vec3::new(0.0, -0.15, 0.0);
        object.transform.scale = Vec3::new(3.4, 0.15, 2.6);
        object.material.base_color = Vec3::new(0.16, 0.18, 0.22);
        object.material.roughness = 0.55;

        let light = scene.add(Primitive::Plane);
        let object = scene.object_mut(light).unwrap();
        object.name = "Key · area light".into();
        object.transform.translation = Vec3::new(-2.0, 5.0, 2.0);
        object.transform.rotation.x = std::f32::consts::PI;
        object.transform.scale = Vec3::new(1.6, 1.0, 1.25);
        object.material.base_color = Vec3::ONE;
        object.material.emission = Vec3::new(12.0, 10.5, 9.0);
        scene
    }
}

impl Scene {
    pub fn add(&mut self, primitive: Primitive) -> u64 {
        let id = self.allocate_id();
        self.objects.push(Object {
            id,
            name: self.unique_name(primitive.label()),
            mesh: Mesh::primitive(primitive),
            transform: Transform::default(),
            material: Material::default(),
            visible: true,
        });
        id
    }

    pub fn object(&self, id: u64) -> Option<&Object> {
        self.objects.iter().find(|o| o.id == id)
    }
    pub fn object_mut(&mut self, id: u64) -> Option<&mut Object> {
        self.objects.iter_mut().find(|o| o.id == id)
    }
    pub fn remove(&mut self, id: u64) {
        self.objects.retain(|o| o.id != id);
    }

    pub fn duplicate(&mut self, id: u64) -> Option<u64> {
        let mut object = self.object(id)?.clone();
        let new_id = self.allocate_id();
        object.id = new_id;
        object.name = self.unique_name(&object.name);
        object.transform.translation += Vec3::new(0.4, 0.0, 0.4);
        self.objects.push(object);
        Some(new_id)
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
        for object in self.objects.iter().filter(|object| object.visible) {
            let matrix = object.transform.matrix();
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
            for (triangle, face) in object.mesh.triangles_with_faces() {
                let a = object.mesh.positions[triangle[0] as usize];
                let b = object.mesh.positions[triangle[1] as usize];
                let c = object.mesh.positions[triangle[2] as usize];
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
        let object = self.object(id)?;
        if object.mesh.positions.is_empty() {
            return None;
        }
        let matrix = object.transform.matrix();
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for point in &object.mesh.positions {
            let point = matrix.transform_point3(*point);
            min = min.min(point);
            max = max.max(point);
        }
        let center = (min + max) * 0.5;
        let radius = object
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
            version: 1,
            scene: self.clone(),
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
        ensure!(
            document.version == 1,
            "Unsupported Forma scene version {}",
            document.version
        );
        document.scene.validate().context("Invalid scene data")?;
        Ok(document.scene)
    }

    pub fn import_obj(&mut self, path: &Path) -> Result<u64> {
        ensure!(
            self.objects.len() < 10_000,
            "Import would exceed the 10,000 object limit"
        );
        let mesh = crate::obj::read(path)?;
        let vertices: usize = self.objects.iter().map(|o| o.mesh.positions.len()).sum();
        let faces: usize = self.objects.iter().map(|o| o.mesh.faces.len()).sum();
        ensure!(
            vertices + mesh.positions.len() <= 5_000_000 && faces + mesh.faces.len() <= 5_000_000,
            "Import would exceed the five million scene vertex/face limit"
        );
        ensure!(
            self.next_id < u64::MAX - 1
                && self.objects.iter().all(|object| object.id < u64::MAX - 2),
            "Scene has exhausted object IDs"
        );
        let label = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Imported mesh");
        let label: String = label.chars().take(100).collect();
        let id = self.allocate_id();
        self.objects.push(Object {
            id,
            name: self.unique_name(&label),
            mesh,
            transform: Transform::default(),
            material: Material::default(),
            visible: true,
        });
        Ok(id)
    }

    pub fn export_obj(&self, path: &Path) -> Result<()> {
        crate::obj::write(self, path)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.objects.len() <= 10_000, "Scene exceeds 10,000 objects");
        let mut ids = BTreeSet::new();
        let mut total_vertices = 0_usize;
        let mut total_faces = 0_usize;
        for object in &self.objects {
            ensure!(
                object.id > 0 && ids.insert(object.id),
                "Object IDs must be nonzero and unique"
            );
            ensure!(
                !object.name.trim().is_empty() && object.name.len() <= 512,
                "Object name is empty or too long"
            );
            object
                .mesh
                .validate()
                .with_context(|| format!("Invalid mesh in {}", object.name))?;
            total_vertices += object.mesh.positions.len();
            total_faces += object.mesh.faces.len();
            ensure!(
                total_vertices <= 5_000_000 && total_faces <= 5_000_000,
                "Scene exceeds five million vertices or faces"
            );
            let t = object.transform;
            ensure!(
                t.translation.is_finite() && t.rotation.is_finite() && t.scale.is_finite(),
                "Non-finite transform in {}",
                object.name
            );
            ensure!(
                t.translation.abs().max_element() <= 1.0e8
                    && t.scale.abs().max_element() <= 1.0e6
                    && t.scale.abs().min_element() >= 1.0e-5,
                "Transform out of range in {}",
                object.name
            );
            let m = &object.material;
            ensure!(
                valid_color(m.base_color, 1.0)
                    && valid_color(m.emission, 1.0e6)
                    && m.metallic.is_finite()
                    && (0.0..=1.0).contains(&m.metallic)
                    && m.roughness.is_finite()
                    && (0.0..=1.0).contains(&m.roughness),
                "Invalid material in {}",
                object.name
            );
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
                && (1..=32).contains(&render.max_bounces),
            "Invalid render preferences"
        );
        Ok(())
    }

    fn allocate_id(&mut self) -> u64 {
        let highest = self.objects.iter().map(|o| o.id).max().unwrap_or(0);
        let id = self.next_id.max(highest.saturating_add(1)).max(1);
        // Validated documents reserve the final ID; exhausting u64 IDs is not
        // possible through normal editor use.
        self.next_id = id.saturating_add(1);
        id
    }

    fn unique_name(&self, stem: &str) -> String {
        if !self.objects.iter().any(|o| o.name == stem) {
            return stem.to_owned();
        }
        for suffix in 1.. {
            let candidate = format!("{stem}.{suffix:03}");
            if !self.objects.iter().any(|o| o.name == candidate) {
                return candidate;
            }
        }
        unreachable!()
    }
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

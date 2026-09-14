//! Common GPU layout and scene fingerprint shared by Metal and wgpu.
use crate::{RenderMode, RenderSettings};
use bytemuck::{Pod, Zeroable};
use forma_core::Scene;
use glam::Vec3;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Uniforms {
    pub cam_origin: [f32; 4],
    pub cam_right: [f32; 4],
    pub cam_up: [f32; 4],
    pub cam_forward: [f32; 4],
    pub world: [f32; 4],
    pub camera: [f32; 4],
    pub image: [u32; 4],
    pub scene: [u32; 4],
    pub settings: [f32; 4],
    pub preview_lighting: [f32; 4],
    pub preview_display: [f32; 4],
}

impl Uniforms {
    pub fn new(
        scene: &Scene,
        settings: &RenderSettings,
        width: u32,
        height: u32,
        sample: u32,
        counts: [u32; 3],
    ) -> Self {
        let camera = scene.camera;
        let view_inverse = camera.view_matrix().inverse();
        let vector = |v: Vec3| {
            let v = view_inverse.transform_vector3(v).normalize();
            [v.x, v.y, v.z, 0.0]
        };
        let position = camera.position();
        Self {
            // The spare camera-origin lane carries component overlay mode.
            cam_origin: [
                position.x,
                position.y,
                position.z,
                if settings.edit_vertices {
                    2.0
                } else {
                    u32::from(settings.edit_wireframe) as f32
                },
            ],
            cam_right: vector(Vec3::X),
            cam_up: vector(Vec3::Y),
            cam_forward: vector(Vec3::NEG_Z),
            world: if settings.mode == RenderMode::MaterialPreview
                && !settings.preview.use_scene_world
            {
                [0.0; 4]
            } else {
                [
                    scene.world.color.x,
                    scene.world.color.y,
                    scene.world.color.z,
                    scene.world.strength,
                ]
            },
            camera: [
                (camera.fov_y * 0.5).tan(),
                width as f32 / height as f32,
                camera.distance * (camera.fov_y * 0.5).tan(),
                u32::from(camera.orthographic) as f32,
            ],
            image: [width, height, sample, settings.mode as u32],
            scene: [
                counts[0],
                counts[1],
                counts[2],
                if settings.mode == RenderMode::Rendered {
                    settings.max_bounces.clamp(1, 32)
                } else {
                    1
                },
            ],
            settings: [
                settings.exposure,
                u32::from(settings.show_grid) as f32,
                settings
                    .selected
                    .and_then(|id| scene.objects.iter().position(|object| object.id == id))
                    .map_or(0.0, |index| (index + 1) as f32),
                // Display refresh flag, set by the renderer itself when only the
                // selection overlay of a finished film has to be redrawn.
                0.0,
            ],
            preview_lighting: if settings.mode == RenderMode::MaterialPreview {
                if settings.preview.use_scene_world {
                    [1.0, 0.0, 1.0, 1.0]
                } else {
                    [
                        settings.preview.rotation.cos(),
                        settings.preview.rotation.sin(),
                        settings.preview.strength,
                        0.0,
                    ]
                }
            } else {
                [0.0; 4]
            },
            preview_display: if settings.mode == RenderMode::MaterialPreview {
                [
                    settings.preview.world_opacity,
                    if settings.preview.use_scene_world {
                        0.0
                    } else {
                        settings.preview.background_blur
                    },
                    u32::from(settings.preview.ambient_occlusion) as f32,
                    0.5,
                ]
            } else {
                [0.0; 4]
            },
        }
    }
}

pub(crate) fn geometry_hash(scene: &Scene) -> u64 {
    let mut hasher = DefaultHasher::new();
    scene.collections.len().hash(&mut hasher);
    for collection in &scene.collections {
        collection.id.hash(&mut hasher);
        collection.parent.hash(&mut hasher);
        collection.visible.hash(&mut hasher);
    }
    for data in &scene.materials {
        let m = &data.material;
        data.id.hash(&mut hasher);
        m.shader.hash(&mut hasher);
        m.mapping.hash(&mut hasher);
        m.custom_code.hash(&mut hasher);
        m.custom_language.hash(&mut hasher);
        m.textures.hash(&mut hasher);
        for v in [
            m.ior,
            m.normal_strength,
            m.texture_scale.x,
            m.texture_scale.y,
            m.texture_offset.x,
            m.texture_offset.y,
        ] {
            v.to_bits().hash(&mut hasher);
        }
    }
    scene.objects.len().hash(&mut hasher);
    for object in &scene.objects {
        object.id.hash(&mut hasher);
        object.visible.hash(&mut hasher);
        object.data.material_ids().hash(&mut hasher);
        object.parent.hash(&mut hasher);
        object.collections.hash(&mut hasher);
        if !scene.is_effectively_visible(object.id) {
            continue;
        }
        let Some(instance) = scene.mesh_instance(object.id) else {
            continue;
        };
        for p in &instance.mesh.positions {
            for n in p.to_array() {
                n.to_bits().hash(&mut hasher);
            }
        }
        instance.mesh.faces.hash(&mut hasher);
        for n in instance.world_transform.to_cols_array() {
            n.to_bits().hash(&mut hasher);
        }
        for n in object
            .data
            .material_ids()
            .iter()
            .filter_map(|id| scene.material(*id))
            .flat_map(|data| {
                data.material
                    .base_color
                    .to_array()
                    .into_iter()
                    .chain(data.material.emission.to_array())
                    .chain([data.material.roughness, data.material.metallic])
            })
        {
            n.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

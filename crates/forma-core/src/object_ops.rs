//! Mesh object transform utilities. Geometry is baked in local coordinates;
//! direct children retain their world transforms through parent compensation.
use crate::{Mat4, Scene, Transform, Vec3};
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

/// Which local transform components to bake into a mesh object's vertices.
/// Translation and the parent relationship are retained in both modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyTransform {
    Scale,
    RotationScale,
}

impl Scene {
    /// Bake the requested local components into each selected mesh, retaining
    /// world geometry, material links, and descendant world transforms. Mesh
    /// data is made single-user only for objects whose transform changes.
    ///
    /// The selection may contain both parents and descendants. Compensation
    /// composes in their parent-inverse matrices without decomposing shear,
    /// so deterministic ID order is sufficient regardless of hierarchy order.
    /// Any invalid selection, geometry, or resulting transform rejects the
    /// entire operation, including data-block allocations.
    pub fn apply_object_transforms(
        &mut self,
        ids: &BTreeSet<u64>,
        mode: ApplyTransform,
    ) -> Result<()> {
        self.validate_mesh_operation(ids)?;
        if ids.iter().all(|&id| {
            let transform = self.object(id).unwrap().transform;
            transform.scale == Vec3::ONE
                && (mode == ApplyTransform::Scale || transform.rotation == Vec3::ZERO)
        }) {
            return Ok(());
        }
        let mut candidate = self.clone();
        for &id in ids {
            let original = candidate.object(id).unwrap().transform;
            let mut transform = original;
            transform.scale = Vec3::ONE;
            if mode == ApplyTransform::RotationScale {
                transform.rotation = Vec3::ZERO;
            }
            if transform == original {
                continue;
            }
            // Construct the exact factored component rather than inverting
            // TRS matrices. This also supports very small and mirrored scales.
            let bake = match mode {
                ApplyTransform::Scale => Mat4::from_scale(original.scale),
                ApplyTransform::RotationScale => Transform {
                    translation: Vec3::ZERO,
                    ..original
                }
                .matrix(),
            };
            candidate.bake_object_coordinates(id, transform, bake)?;
        }
        candidate
            .validate()
            .context("Applied transforms exceed the scene's geometry or transform limits")?;
        *self = candidate;
        Ok(())
    }

    /// Move each selected origin to the center of its mesh's local bounding
    /// box. World geometry and descendant world transforms stay in place.
    /// Rotation, scale, material links and parent relationships are retained.
    /// Linked meshes are separated only when their origin actually changes.
    /// Any failure leaves the complete scene and its ID sequence untouched.
    pub fn center_object_origins(&mut self, ids: &BTreeSet<u64>) -> Result<()> {
        self.validate_mesh_operation(ids)?;
        // Linked instances share the same local bounds. Plan from original
        // data once per mesh, avoiding repeated dense vertex scans and avoiding
        // a whole-scene clone for already centered selections.
        let mut centers = BTreeMap::new();
        let mut changes = Vec::new();
        for &id in ids {
            let mesh_id = self.object(id).unwrap().mesh_id().unwrap();
            let center = *centers.entry(mesh_id).or_insert_with(|| {
                let mesh = &self.mesh(mesh_id).unwrap().mesh;
                let mut low = Vec3::splat(f32::INFINITY);
                let mut high = Vec3::splat(f32::NEG_INFINITY);
                for &position in &mesh.positions {
                    low = low.min(position);
                    high = high.max(position);
                }
                low * 0.5 + high * 0.5
            });
            if center != Vec3::ZERO {
                changes.push((id, center));
            }
        }
        if changes.is_empty() {
            return Ok(());
        }
        let mut candidate = self.clone();
        for (id, center) in changes {
            let original = candidate.object(id).unwrap().transform;
            let transform = Transform {
                translation: original.matrix().transform_point3(center),
                ..original
            };
            candidate.bake_object_coordinates(id, transform, Mat4::from_translation(-center))?;
        }
        candidate
            .validate()
            .context("Centered origins exceed the scene's geometry or transform limits")?;
        *self = candidate;
        Ok(())
    }

    fn validate_mesh_operation(&self, ids: &BTreeSet<u64>) -> Result<()> {
        ensure!(!ids.is_empty(), "Select at least one mesh object");
        self.validate()
            .context("The scene must be valid before editing object transforms")?;
        for &id in ids {
            let object = self
                .object(id)
                .with_context(|| format!("Object {id} does not exist"))?;
            ensure!(
                object.mesh_id().is_some(),
                "Object '{}' is not a mesh",
                object.name
            );
        }
        Ok(())
    }

    /// `transform.matrix() * bake` must equal the previous local matrix.
    /// Moving `bake` into the mesh preserves world vertices; inserting it in
    /// direct children's parent-inverse matrices preserves their entire
    /// subtrees, including sheared or negatively scaled descendants.
    fn bake_object_coordinates(&mut self, id: u64, transform: Transform, bake: Mat4) -> Result<()> {
        self.make_mesh_single_user(id)?;
        let mesh = self.object_mesh_mut(id).unwrap();
        for position in &mut mesh.positions {
            *position = bake.transform_point3(*position);
        }
        // Rendering transforms normals with the inverse transpose, and both
        // exporters compensate reflected instances. Once the reflection moves
        // into vertex coordinates, reverse winding to retain that orientation.
        if bake.as_dmat4().determinant() < 0. {
            for face in &mut mesh.faces {
                face.reverse();
            }
        }
        mesh.validate()
            .context("Baked mesh exceeds supported geometry precision or range")?;
        self.object_mut(id).unwrap().transform = transform;
        for child in self
            .objects
            .iter_mut()
            .filter(|object| object.parent == Some(id))
        {
            child.parent_inverse = bake * child.parent_inverse;
        }
        Ok(())
    }
}

//! Transactional modelling tools: preview from immutable input, commit once,
//! and restore both geometry and selection when the operation is cancelled.
use crate::app::{EditMode, Studio, Tool, TransformDrag};
use crate::viewport::{mouse_point, project};
use glam::{Mat4, Quat, Vec2, Vec3};
use gpui::Context;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeshTool {
    Extrude,
    Inset,
}

impl Studio {
    pub(crate) fn start_mesh_tool(&mut self, tool: MeshTool, cx: &mut Context<Self>) {
        if !self.edit_mode.is_component() || self.selected_faces().is_empty() {
            self.status = "Select a face region first".into();
            cx.notify();
            return;
        }
        let faces = self.selected_faces();
        let selection_before = self.component_elements();
        let mode_before = self.edit_mode;
        self.edit_mode = EditMode::Face;
        self.set_components(
            faces
                .iter()
                .copied()
                .map(forma_core::MeshElement::Face)
                .collect(),
        );
        self.start_transform(Tool::Move, None, true, cx);
        if let Some(drag) = self.transform_drag.as_mut() {
            drag.mesh_tool = Some(tool);
            drag.mesh_faces = faces;
            drag.selection_before = selection_before;
            drag.selection_mode_before = mode_before;
        }
        self.status = format!(
            "{tool:?} · Move pointer or type a distance · Ctrl snap · Enter confirm · Esc cancel"
        );
        cx.notify();
    }

    fn update_mesh_tool(
        &mut self,
        position: Vec2,
        precise: bool,
        snapping: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.transform_drag.as_mut() else {
            return;
        };
        let tool = drag.mesh_tool.unwrap();
        drag.motion.update(position, drag.motion.start, precise);
        let delta = drag.motion.effective - drag.motion.start;
        let explicit = crate::transform::numeric_value(&drag.numeric);
        if !drag.numeric.is_empty() && explicit.is_none() {
            drag.error = Some("Finish the numeric expression".into());
            self.status = format!(
                "{tool:?} · {} · Finish the numeric expression",
                drag.numeric
            );
            cx.notify();
            return;
        }
        let faces = &drag.mesh_faces;
        let source = drag.before.object_mesh(drag.original.id).unwrap();
        let world = drag.before.world_transform(drag.original.id).unwrap();
        let inverse_view = self.scene.camera.view_matrix().inverse();
        let pixel_scale = self.scene.camera.distance * (self.scene.camera.fov_y * 0.5).tan() * 2.
            / f32::from(self.bounds.get().size.height).max(1.);
        let normal = faces
            .iter()
            .map(|&i| {
                world
                    .inverse()
                    .transpose()
                    .transform_vector3(source.face_normal(i))
            })
            .sum::<Vec3>()
            .normalize_or_zero();
        let basis = if drag.constraint.orientation == crate::transform::Orientation::Local {
            drag.basis
        } else {
            glam::Mat3::IDENTITY
        };
        let direction = drag.constraint.axis.map(|i| basis.col(i)).unwrap_or(normal);
        let projected = Vec2::new(
            direction.dot(inverse_view.x_axis.truncate()),
            -direction.dot(inverse_view.y_axis.truncate()),
        );
        let raw = if tool == MeshTool::Inset || projected.length_squared() < 0.02 {
            (delta.x - delta.y) * pixel_scale
        } else {
            delta.dot(projected) / projected.length_squared() * pixel_scale
        };
        let mut amount = explicit.unwrap_or(raw);
        if snapping && explicit.is_none() {
            amount = crate::transform::snap(amount, if precise { 0.01 } else { 0.1 });
        }
        let mut mesh = source.clone();
        let result = if amount.abs() < 1e-6 {
            Ok(())
        } else if tool == MeshTool::Inset {
            // Offset in world space so non-uniform object scale does not make
            // inset thickness vary from one side of the face to another.
            for position in &mut mesh.positions {
                *position = world.transform_point3(*position);
            }
            let result = mesh.inset_region(faces, amount);
            if result.is_ok() {
                let inverse = world.inverse();
                for (i, position) in mesh.positions.iter_mut().enumerate() {
                    *position = source
                        .positions
                        .get(i)
                        .copied()
                        .unwrap_or_else(|| inverse.transform_point3(*position));
                }
            }
            result
        } else {
            let mut offset = direction * amount;
            if drag.constraint.plane {
                let screen_move = (inverse_view.x_axis.truncate() * delta.x
                    - inverse_view.y_axis.truncate() * delta.y)
                    * pixel_scale;
                offset = screen_move - direction * screen_move.dot(direction);
                if let Some(value) = explicit {
                    let i = drag.constraint.axis.unwrap();
                    offset = (basis.col((i + 1) % 3) + basis.col((i + 2) % 3)) * value;
                }
            }
            mesh.extrude_region(faces, world.inverse().transform_vector3(offset))
        };
        match result {
            Ok(()) => {
                drag.error = None;
                *self.scene.object_mesh_mut(drag.original.id).unwrap() = mesh;
                drag.changed = amount.abs() >= 1e-6;
                self.status = format!(
                    "{tool:?} · {amount:.3} m · {} · Enter confirm · Esc cancel",
                    if drag.constraint.axis.is_none() {
                        "Normal".into()
                    } else {
                        drag.constraint.label()
                    }
                );
                self.invalidate(true, cx);
            }
            Err(error) => {
                drag.error = Some(error.to_string());
                self.status = format!("{tool:?} · {error} · Adjust distance or Esc cancel");
                cx.notify();
            }
        }
    }

    pub(crate) fn start_transform(
        &mut self,
        tool: Tool,
        axis: Option<usize>,
        modal: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(original) = self
            .selected_object()
            .map(|instance| instance.object.clone())
        else {
            self.status = "Select an object first".into();
            cx.notify();
            return;
        };
        if self.edit_mode.is_component() && self.component_vertices().is_empty() {
            self.status = "Select a component first".into();
            cx.notify();
            return;
        }
        self.tool = tool;
        self.transform_drag = Some(TransformDrag {
            tool,
            constraint: crate::transform::Constraint {
                axis,
                ..Default::default()
            },
            motion: crate::transform::PointerMotion::new(self.last_mouse),
            pivot: self.selection_center().unwrap(),
            basis: glam::Mat3::from_quat(
                self.scene
                    .world_transform(original.id)
                    .unwrap()
                    .to_scale_rotation_translation()
                    .1,
            ),
            objects: self
                .selected_ids()
                .iter()
                .filter_map(|id| self.scene.object(*id).cloned())
                .collect(),
            objects_before: self.selected_ids(),
            original,
            before: self.scene.clone(),
            numeric: String::new(),
            modal,
            changed: false,
            vertices: self.component_vertices(),
            mesh_tool: None,
            mesh_faces: Default::default(),
            selection_before: self.component_elements(),
            selection_object_before: self.selected,
            selection_mode_before: self.edit_mode,
            history_before: None,
            error: None,
        });
        self.status = format!("{tool:?} · X / Y / Z to constrain · Type a value · Return to apply");
        cx.notify();
    }

    pub(crate) fn update_transform(
        &mut self,
        position: Vec2,
        precise: bool,
        snapping: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .transform_drag
            .as_ref()
            .is_some_and(|d| d.mesh_tool.is_some())
        {
            self.update_mesh_tool(position, precise, snapping, cx);
            return;
        }
        let Some(drag) = &mut self.transform_drag else {
            return;
        };
        use crate::transform::{Orientation, numeric_value, snap};
        let camera = self.scene.camera;
        let bounds = self.bounds.get();
        let aspect = f32::from(bounds.size.width) / f32::from(bounds.size.height).max(1.);
        let matrix = camera.projection_matrix(aspect) * camera.view_matrix();
        let pivot_screen = project(drag.pivot, matrix, bounds)
            .map(mouse_point)
            .unwrap_or(drag.motion.start);
        drag.motion.update(position, pivot_screen, precise);
        let delta = drag.motion.effective - drag.motion.start;
        let explicit = numeric_value(&drag.numeric);
        if !drag.numeric.is_empty() && explicit.is_none() {
            self.status = format!("{} · Finish the numeric expression", drag.numeric);
            cx.notify();
            return;
        }
        let original = &drag.original;
        let inverse_view = camera.view_matrix().inverse();
        let right = inverse_view.x_axis.truncate();
        let up = inverse_view.y_axis.truncate();
        let depth = if camera.orthographic {
            camera.distance
        } else {
            -camera.view_matrix().transform_point3(drag.pivot).z
        };
        let pixel_scale = depth.max(0.001) * (camera.fov_y * 0.5).tan() * 2.
            / f32::from(bounds.size.height).max(1.);
        let basis = if drag.constraint.orientation == Orientation::Local {
            drag.basis
        } else {
            glam::Mat3::IDENTITY
        };
        let mut translation = right * delta.x * pixel_scale - up * delta.y * pixel_scale;
        if let Some(axis) = drag.constraint.axis {
            let direction = basis.col(axis);
            if drag.constraint.plane {
                // Intersect the pointer rays with the constrained plane. Fall back
                // to projected screen motion when that plane is edge-on.
                let at = |point: Vec2| {
                    let uv = (point - mouse_point(bounds.origin))
                        / Vec2::new(bounds.size.width.into(), bounds.size.height.into());
                    let ray = camera.ray(uv, aspect);
                    let denominator = ray.direction.dot(direction);
                    (denominator.abs() > 1e-4).then(|| {
                        ray.origin
                            + ray.direction
                                * ((drag.pivot - ray.origin).dot(direction) / denominator)
                    })
                };
                translation = at(drag.motion.start)
                    .zip(at(drag.motion.effective))
                    .map(|(a, b)| b - a)
                    .unwrap_or(translation - direction * translation.dot(direction));
                if let Some(value) = explicit {
                    translation = (basis.col((axis + 1) % 3) + basis.col((axis + 2) % 3)) * value;
                }
            } else {
                let projected = Vec2::new(direction.dot(right), -direction.dot(up));
                let amount = if projected.length_squared() > 0.02 {
                    delta.dot(projected) / projected.length_squared() * pixel_scale
                } else {
                    delta.x * pixel_scale
                };
                translation = direction * explicit.unwrap_or(amount);
            }
        } else if let Some(value) = explicit {
            translation = Vec3::X * value;
        }
        if snapping && explicit.is_none() {
            let step = if precise { 0.1 } else { 1. };
            let local = basis.transpose() * translation;
            translation = basis
                * Vec3::new(
                    snap(local.x, step),
                    snap(local.y, step),
                    snap(local.z, step),
                );
        }
        let mut angle = explicit.map(f32::to_radians).unwrap_or(drag.motion.angle);
        if let Some(axis) = drag.constraint.axis {
            // Clockwise mouse motion should feel the same when viewing an axis
            // from either side, while numeric angles follow its positive direction.
            if explicit.is_none() && basis.col(axis).dot(inverse_view.z_axis.truncate()) < 0. {
                angle = -angle;
            }
        }
        let mut factor = explicit.unwrap_or(drag.motion.scale(pivot_screen));
        if snapping && explicit.is_none() {
            angle = snap(
                angle,
                if precise {
                    1_f32.to_radians()
                } else {
                    5_f32.to_radians()
                },
            );
            factor = snap(factor, if precise { 0.01 } else { 0.1 });
        }

        if drag.tool == Tool::Scale && !self.edit_mode.is_component() && factor.abs() < 1e-5 {
            drag.error =
                Some("Object scale cannot be zero; use Edit mode to flatten geometry".into());
            self.status = drag.error.clone().unwrap();
            cx.notify();
            return;
        }
        let rotation_axis = drag
            .constraint
            .axis
            .map(|a| basis.col(a))
            .unwrap_or(inverse_view.z_axis.truncate());
        let mut scaling = Vec3::splat(factor);
        if let Some(axis) = drag.constraint.axis {
            if drag.constraint.plane {
                scaling[axis] = 1.;
            } else {
                scaling = Vec3::ONE;
                scaling[axis] = factor;
            }
        }
        let operation = match drag.tool {
            Tool::Move => Mat4::from_translation(translation),
            Tool::Rotate => {
                Mat4::from_translation(drag.pivot)
                    * Mat4::from_quat(Quat::from_axis_angle(rotation_axis, angle))
                    * Mat4::from_translation(-drag.pivot)
            }
            Tool::Scale => {
                Mat4::from_translation(drag.pivot)
                    * Mat4::from_mat3(
                        basis * glam::Mat3::from_diagonal(scaling) * basis.transpose(),
                    )
                    * Mat4::from_translation(-drag.pivot)
            }
            Tool::Select => Mat4::IDENTITY,
        };
        if !operation.is_finite() {
            drag.error = Some("Transform exceeds the numeric range".into());
            self.status =
                "Transform exceeds the numeric range · Adjust the value or Esc cancel".into();
            cx.notify();
            return;
        }
        drag.error = None;
        if self.edit_mode.is_component() {
            let original_mesh = drag.before.object_mesh(original.id);
            if let Some(mesh) = original_mesh.filter(|_| !drag.vertices.is_empty()) {
                let face = &drag.vertices;
                let matrix = drag.before.world_transform(original.id).unwrap();
                if let Some(edited) = self.scene.object_mesh_mut(original.id) {
                    for index in face {
                        edited.positions[*index as usize] = mesh.positions[*index as usize];
                    }
                    edited.transform_vertices(face, matrix, operation);
                }
            }
        } else {
            let ids: std::collections::BTreeSet<_> = drag.objects.iter().map(|o| o.id).collect();
            for object in &drag.objects {
                let world = drag.before.world_transform(object.id).unwrap();
                let mut parent = world * object.transform.matrix().inverse();
                let mut ancestor = object.parent;
                while let Some(id) = ancestor {
                    if ids.contains(&id) {
                        parent = operation * parent;
                        break;
                    }
                    ancestor = drag.before.object(id).and_then(|o| o.parent);
                }
                let local = parent.inverse() * operation * world;
                let (scale, rotation, translation) = local.to_scale_rotation_translation();
                let (x, y, z) = rotation.to_euler(glam::EulerRot::XYZ);
                if let Some(edited) = self.scene.object_mut(object.id) {
                    edited.transform = if operation.abs_diff_eq(Mat4::IDENTITY, 1e-7) {
                        object.transform
                    } else {
                        forma_core::Transform {
                            translation,
                            rotation: Vec3::new(x, y, z),
                            scale,
                        }
                    };
                }
            }
        }
        drag.changed = drag.history_before.is_some() || self.scene != drag.before;
        let value = match drag.tool {
            Tool::Move => format!("{:.3} m", translation.length()),
            Tool::Rotate => format!("{:.2}°", angle.to_degrees()),
            Tool::Scale => format!("{factor:.3}×"),
            Tool::Select => String::new(),
        };
        self.status = format!(
            "{:?} · {} · {}{} · Enter confirm · Esc cancel",
            drag.tool,
            drag.constraint.label(),
            if drag.numeric.is_empty() {
                value
            } else {
                drag.numeric.clone()
            },
            if snapping { " · Snap" } else { "" }
        );
        self.invalidate(true, cx);
    }

    pub(crate) fn finish_transform(&mut self, cancel: bool, cx: &mut Context<Self>) {
        if !cancel
            && self.transform_drag.as_ref().is_some_and(|d| {
                d.error.is_some()
                    || (!d.numeric.is_empty()
                        && crate::transform::numeric_value(&d.numeric).is_none())
            })
        {
            self.status = self
                .transform_drag
                .as_ref()
                .and_then(|d| d.error.clone())
                .unwrap_or_else(|| {
                    "Finish or clear the numeric expression before confirming".into()
                });
            cx.notify();
            return;
        }
        let Some(drag) = self.transform_drag.take() else {
            return;
        };
        if cancel {
            self.scene = drag.history_before.unwrap_or(drag.before);
            self.selected = drag.selection_object_before;
            self.edit_mode = drag.selection_mode_before;
            self.set_components(drag.selection_before);
            self.objects_selected = drag.objects_before;
            self.status = "Operation cancelled".into();
            self.invalidate(true, cx);
        } else if drag.changed {
            if let Err(error) = self.scene.validate() {
                self.scene = drag.history_before.unwrap_or(drag.before);
                self.selected = drag.selection_object_before;
                self.edit_mode = drag.selection_mode_before;
                self.set_components(drag.selection_before);
                self.objects_selected = drag.objects_before;
                self.status = format!("Transform cancelled: {error}");
                self.invalidate(true, cx);
            } else {
                self.history
                    .checkpoint(drag.history_before.as_ref().unwrap_or(&drag.before));
                self.dirty = true;
                self.status = drag
                    .mesh_tool
                    .map(|tool| format!("{tool:?} applied"))
                    .unwrap_or_else(|| format!("{:?} applied", drag.tool));
                cx.notify();
            }
        }
    }
}

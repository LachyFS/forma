//! Document transactions for the mesh finishing commands.
use crate::app::{Command, EditMode, Studio};
use forma_core::{ApplyTransform, MeshElement};
use gpui::Context;
use std::collections::BTreeSet;

impl Studio {
    pub(crate) fn apply_object_command(&mut self, command: Command, cx: &mut Context<Self>) {
        let label = match command {
            Command::ApplyScale => "Apply scale",
            Command::ApplyRotationScale => "Apply rotation and scale",
            Command::CenterOrigin => "Center origin",
            _ => return,
        };
        if self.edit_mode != EditMode::Object {
            self.status = "Switch to Object mode to change object transforms or origins".into();
            return;
        }
        let ids = self.selected_ids();
        if ids.is_empty() {
            self.status = "Select a mesh first".into();
            return;
        }
        let mut edited = self.scene.clone();
        let result = match command {
            Command::ApplyScale => edited.apply_object_transforms(&ids, ApplyTransform::Scale),
            Command::ApplyRotationScale => {
                edited.apply_object_transforms(&ids, ApplyTransform::RotationScale)
            }
            Command::CenterOrigin => edited.center_object_origins(&ids),
            _ => unreachable!(),
        };
        if let Err(error) = result {
            self.status = format!("{label}: {error}");
            return;
        }
        if edited == self.scene {
            self.status = format!("{label}: already up to date");
            return;
        }
        self.history.checkpoint(&self.scene);
        self.scene = edited;
        self.dirty = true;
        self.status = format!("{label} applied · Undo to restore");
        self.invalidate(true, cx);
    }

    pub(crate) fn finish_mesh(&mut self, command: Command, cx: &mut Context<Self>) {
        let label = match command {
            Command::Triangulate => "Triangulate",
            Command::FlipNormals => "Flip normals",
            Command::RecalculateNormals => "Recalculate normals",
            _ => return,
        };
        let ids = self.selected_ids();
        if ids.is_empty() {
            self.status = "Select a mesh first".into();
            return;
        }
        let selected_faces = self.selected_faces();
        if self.edit_mode.is_component()
            && command != Command::RecalculateNormals
            && selected_faces.is_empty()
        {
            self.status = "Select faces first".into();
            return;
        }
        let mut edited = self.scene.clone();
        let mut visited = BTreeSet::new();
        let mut selection = None;
        for id in ids {
            if self.edit_mode.is_component() && self.selected != Some(id) {
                continue;
            }
            let Some(mesh_id) = edited.object(id).and_then(|object| object.mesh_id()) else {
                continue;
            };
            // Linked instances share one data block; never apply an operation twice.
            if !visited.insert(mesh_id) {
                continue;
            }
            let mesh = edited.object_mesh_mut(id).unwrap();
            let faces = if self.edit_mode.is_component() {
                selected_faces.clone()
            } else {
                (0..mesh.faces.len()).collect()
            };
            let result = match command {
                Command::Triangulate => mesh.triangulate_faces(&faces).map(|faces| {
                    if self.edit_mode.is_component() {
                        selection = Some(faces.into_iter().map(MeshElement::Face).collect());
                    }
                }),
                Command::FlipNormals => mesh.flip_faces(&faces),
                Command::RecalculateNormals => mesh.recalculate_normals(),
                _ => unreachable!(),
            };
            if let Err(error) = result {
                self.status = format!("{label}: {error}");
                return;
            }
        }
        if let Err(error) = edited.validate() {
            self.status = format!("{label}: {error}");
            return;
        }
        if edited == self.scene {
            self.status = format!("{label}: mesh already up to date");
            return;
        }
        self.history.checkpoint(&self.scene);
        self.scene = edited;
        if let Some(selection) = selection {
            self.edit_mode = EditMode::Face;
            self.set_components(selection);
        } else if command == Command::Triangulate {
            self.clear_components();
        }
        self.dirty = true;
        self.status = format!("{label} applied · Undo to restore");
        self.invalidate(true, cx);
    }
}

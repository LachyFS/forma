use crate::Scene;
use std::collections::VecDeque;

/// Whole-scene transactions. Call `checkpoint` immediately before an edit, once
/// per drag or command. Both snapshot count and estimated geometry memory are
/// bounded so a large imported model does not retain dozens of copies. At least
/// the latest transaction is retained, even if that one scene exceeds the budget.
pub struct History {
    undo: VecDeque<Scene>,
    redo: VecDeque<Scene>,
    limit: usize,
    byte_limit: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            limit: 64,
            byte_limit: 256 * 1024 * 1024,
        }
    }
}

impl History {
    pub fn checkpoint(&mut self, scene: &Scene) {
        self.checkpoint_owned(scene.clone());
    }

    /// Records a snapshot taken before an edit that may fail. Take the copy
    /// first, apply the edit, and record the copy only once it succeeded, so
    /// a failed edit neither adds an undo step nor clears redo.
    pub fn checkpoint_owned(&mut self, before: Scene) {
        self.redo.clear();
        self.undo.push_back(before);
        Self::trim(&mut self.undo, self.limit, self.byte_limit);
    }

    pub fn undo(&mut self, scene: &mut Scene) -> bool {
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };
        self.redo.push_back(std::mem::replace(scene, previous));
        Self::trim(&mut self.redo, self.limit, self.byte_limit);
        true
    }

    pub fn redo(&mut self, scene: &mut Scene) -> bool {
        let Some(next) = self.redo.pop_back() else {
            return false;
        };
        self.undo.push_back(std::mem::replace(scene, next));
        Self::trim(&mut self.undo, self.limit, self.byte_limit);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn trim(stack: &mut VecDeque<Scene>, limit: usize, byte_limit: usize) {
        let mut bytes: usize = stack.iter().map(scene_bytes).sum();
        while stack.len() > limit || (stack.len() > 1 && bytes > byte_limit) {
            if let Some(scene) = stack.pop_front() {
                bytes = bytes.saturating_sub(scene_bytes(&scene));
            }
        }
    }
}

fn scene_bytes(scene: &Scene) -> usize {
    let objects = scene
        .objects
        .iter()
        .map(|object| {
            let sketch = match &object.data {
                crate::ObjectData::Sketch { sketch } => {
                    sketch.entities.len() * std::mem::size_of::<crate::SketchEntity>()
                }
                _ => 0,
            };
            std::mem::size_of_val(object) + object.name.len() + sketch
        })
        .sum::<usize>();
    let meshes = scene
        .meshes
        .iter()
        .map(|data| {
            std::mem::size_of_val(data)
                + data.name.len()
                + data.mesh.positions.len() * std::mem::size_of::<glam::Vec3>()
                + data
                    .mesh
                    .faces
                    .iter()
                    .map(|face| face.len() * 4 + 24)
                    .sum::<usize>()
        })
        .sum::<usize>();
    let materials = scene
        .materials
        .iter()
        .map(|data| {
            std::mem::size_of_val(data)
                + data.name.len()
                + data.material.custom_code.len()
                + data
                    .material
                    .textures
                    .iter()
                    .flatten()
                    .map(|image| image.rgba.len() + image.name.len())
                    .sum::<usize>()
        })
        .sum::<usize>();
    objects + meshes + materials
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Sketch, SketchCurve, SketchEntity, Vec2};

    #[test]
    fn sketch_curves_count_toward_the_history_budget() {
        let mut scene = Scene::empty();
        let id = scene.add_sketch("Sketch", Sketch::default()).unwrap();
        let empty = scene_bytes(&scene);
        let sketch = scene.object_sketch_mut(id).unwrap();
        for i in 0..1000 {
            let x = i as f32;
            sketch.entities.push(SketchEntity::from(SketchCurve::Line {
                start: Vec2::new(x, 0.0),
                end: Vec2::new(x, 1.0),
            }));
        }
        assert!(scene_bytes(&scene) >= empty + 1000 * std::mem::size_of::<SketchEntity>());
    }

    #[test]
    fn owned_checkpoints_undo_like_borrowed_ones() {
        let mut history = History::default();
        let mut scene = Scene::empty();
        let before = scene.clone();
        scene.add_sketch("Sketch", Sketch::default()).unwrap();
        history.checkpoint_owned(before.clone());
        assert!(history.undo(&mut scene));
        assert_eq!(scene, before);
        assert!(history.redo(&mut scene));
        assert_eq!(scene.objects.len(), 1);
    }
}

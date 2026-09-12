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
        self.redo.clear();
        self.undo.push_back(scene.clone());
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
    scene
        .objects
        .iter()
        .map(|object| {
            std::mem::size_of_val(object)
                + object.name.len()
                + object.mesh.positions.len() * std::mem::size_of::<glam::Vec3>()
                + object
                    .mesh
                    .faces
                    .iter()
                    .map(|face| face.len() * 4 + 24)
                    .sum::<usize>()
        })
        .sum()
}

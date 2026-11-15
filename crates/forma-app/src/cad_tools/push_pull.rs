//! Push/Pull: one tool for flat faces of B-rep parts and polygon meshes.
//!
//! Hover outlines the face under the pointer (a whole B-rep face group on a
//! part, one polygon on a mesh). Press and drag moves it along its normal
//! with a ghost and live dimension; release applies it as one undo step: a
//! `PushPullFeature` on a part (the kernel decides join or cut from the
//! sign), a region extrusion on a mesh. A click without dragging selects the
//! face and shows its arrow, so a typed distance and Return apply exactly.
//! In part edit mode, E or P pushes the selected faces modally: the distance
//! follows the pointer and a click applies.
use super::geometry::{self, Prism};
use super::{AxisDrag, DRAG_PIXELS, PICK_PIXELS, PendingGhost, Tool};
use crate::app::Studio;
use forma_core::{FaceRef, FeatureKind, PushPullFeature, Vec2, Vec3};
use gpui::{Context, KeyDownEvent, MouseDownEvent};
use std::{collections::BTreeSet, rc::Rc};

/// Smallest move applied (world units).
const MIN_DISTANCE: f32 = 1.0e-5;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FaceKind {
    /// A B-rep face group of a part, with its persistent reference.
    Part { group: u32, reference: FaceRef },
    /// A polygon of an editable mesh.
    Mesh { face: usize },
}

/// A face under the pointer or picked for pushing.
#[derive(Clone, Debug)]
pub(crate) struct FacePick {
    pub object: u64,
    pub name: String,
    pub kind: FaceKind,
    /// Outward unit normal in world space.
    pub normal: Vec3,
    /// The picked point in world space.
    pub point: Vec3,
    /// The face's cap and outline in world space.
    pub prism: Rc<Prism>,
    pub planar: bool,
    /// World length of one object-space unit along the normal.
    pub scale: f32,
}

impl FacePick {
    pub(crate) fn is_part(&self) -> bool {
        matches!(self.kind, FaceKind::Part { .. })
    }

    fn same_face(&self, other: &FacePick) -> bool {
        self.object == other.object
            && match (&self.kind, &other.kind) {
                (FaceKind::Part { group: a, .. }, FaceKind::Part { group: b, .. }) => a == b,
                (FaceKind::Mesh { face: a }, FaceKind::Mesh { face: b }) => a == b,
                _ => false,
            }
    }
}

/// Faces being pushed and how far.
#[derive(Clone, Debug)]
pub(crate) struct ActiveFaces {
    pub faces: Vec<FacePick>,
    /// Signed world distance along the first face's normal.
    pub distance: f32,
    pub numeric: String,
    pub drag: Option<AxisDrag>,
    /// Keyboard-started: the pointer drives the distance and a click applies.
    pub modal: bool,
    /// Offsets of parallel faces of the same object, for snapping.
    pub heights: Vec<f32>,
    pub snapped_face: bool,
    pub error: Option<String>,
}

impl ActiveFaces {
    pub(crate) fn typed(&self) -> Result<Option<f32>, ()> {
        let text = self.numeric.trim();
        if text.is_empty() {
            return Ok(None);
        }
        crate::transform::distance_value(text).map(Some).ok_or(())
    }

    /// The first face's centroid, where the arrow stands.
    pub(crate) fn anchor(&self) -> Vec3 {
        self.faces[0].prism.centroid()
    }
}

#[derive(Default)]
pub(crate) struct PushPullTool {
    pub hover: Option<FacePick>,
    pub active: Option<ActiveFaces>,
    pub handle_hot: bool,
}

impl PushPullTool {
    /// Screen base and tip of the active face's arrow.
    pub(crate) fn handle_screen(&self, studio: &Studio) -> Option<(Vec2, Vec2)> {
        let active = self.active.as_ref()?;
        let face = &active.faces[0];
        let base = active.anchor() + face.normal * active.distance;
        let screen = studio.world_to_screen(base)?;
        let ahead = studio.world_to_screen(base + face.normal * studio.world_per_pixel(base) * 10.0);
        let sign = if active.distance < 0.0 { -1.0 } else { 1.0 };
        let direction = ahead
            .map(|p| (p - screen) * sign)
            .and_then(|d| d.try_normalize())
            .unwrap_or(Vec2::NEG_Y);
        Some((screen, screen + direction * super::HANDLE_PIXELS))
    }
}

impl Studio {
    /// Starts the Push/Pull tool: hover a flat face, drag it, release.
    pub(crate) fn start_push_pull(&mut self, cx: &mut Context<Self>) {
        if self.cad_tool.active() {
            self.cancel_cad_tool(cx);
        }
        self.cad_tool.region_hover = None;
        let mut tool = PushPullTool::default();
        tool.hover = self.pick_face(self.last_mouse, None);
        self.cad_tool.tool = Some(Tool::PushPull(Box::new(tool)));
        self.status =
            "Push/Pull · Drag a flat face of a part or mesh · Click a face to type a distance".into();
        cx.notify();
    }

    /// The face under a window position, reusing `previous` when it is the
    /// same face (so hover does not rebuild outlines every move).
    pub(crate) fn pick_face(&self, screen: Vec2, previous: Option<&FacePick>) -> Option<FacePick> {
        let ray = self.tool_ray(screen)?;
        let hit = self.scene.pick(ray)?;
        let object = self.scene.object(hit.object_id)?;
        let instance = self.scene.mesh_instance(hit.object_id)?;
        let matrix = instance.world_transform;
        if !matrix.is_finite() || matrix.determinant().abs() < 1.0e-12 {
            return None;
        }
        let mesh = instance.mesh;
        let is_part = object.is_part();
        if !is_part && !object.data.is_editable_mesh() {
            return None;
        }
        let group = if is_part {
            mesh.face_group(hit.face)?
        } else {
            hit.face as u32
        };
        if let Some(previous) = previous
            && previous.object == hit.object_id
            && match previous.kind {
                FaceKind::Part { group: g, .. } => is_part && g == group,
                FaceKind::Mesh { face } => !is_part && face == hit.face,
            }
        {
            let mut reused = previous.clone();
            reused.point = hit.position;
            return Some(reused);
        }
        let faces = if is_part {
            geometry::group_faces(mesh, group)
        } else {
            vec![hit.face]
        };
        let plane = geometry::group_plane(mesh, &faces);
        let inverse = matrix.inverse();
        let local_point = inverse.transform_point3(hit.position);
        let local_normal = match plane {
            Some((normal, _)) => normal,
            None => mesh.face_normal(hit.face),
        };
        let kind = if is_part {
            let reference = forma_cad::face_ref_from_pick(mesh, hit.face, local_point)?;
            FaceKind::Part { group, reference }
        } else {
            FaceKind::Mesh { face: hit.face }
        };
        let world_normal = inverse
            .transpose()
            .transform_vector3(local_normal)
            .normalize_or_zero();
        let scale = matrix.transform_vector3(local_normal).length().max(1.0e-9);
        let prism = face_prism(mesh, &faces, matrix, world_normal);
        Some(FacePick {
            object: hit.object_id,
            name: object.name.clone(),
            kind,
            normal: world_normal,
            point: hit.position,
            prism: Rc::new(prism),
            planar: plane.is_some(),
            scale,
        })
    }

    /// Pushes the faces selected in part edit mode, following the pointer.
    pub(crate) fn push_pull_selected_faces(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.part_editing() else {
            return;
        };
        let faces: Vec<FacePick> = self
            .selected_edit_faces(id)
            .into_iter()
            .filter_map(|(group, reference)| self.face_pick_for_group(id, group, reference))
            .collect();
        if faces.is_empty() {
            self.start_push_pull(cx);
            return;
        }
        if let Some(face) = faces.iter().find(|f| !f.planar) {
            let _ = face;
            self.status = "Push/pull works on flat faces · Deselect curved faces".into();
            cx.notify();
            return;
        }
        if self.cad_tool.active() {
            self.cancel_cad_tool(cx);
        }
        let face = &faces[0];
        let mut drag = AxisDrag::new(self, face.prism.centroid(), face.normal, 0.0, self.last_mouse);
        drag.started = true;
        let heights = self.parallel_heights_for(face);
        let count = faces.len();
        let tool = PushPullTool {
            hover: None,
            active: Some(ActiveFaces {
                faces,
                distance: 0.0,
                numeric: String::new(),
                drag: Some(drag),
                modal: true,
                heights,
                snapped_face: false,
                error: None,
            }),
            handle_hot: false,
        };
        self.cad_tool.tool = Some(Tool::PushPull(Box::new(tool)));
        self.status = format!(
            "Push/Pull {count} face{} · Move the pointer or type a distance · Click or Return to apply · Esc cancel",
            if count == 1 { "" } else { "s" }
        );
        cx.notify();
    }

    /// A pick for a face group of a part, as if the pointer were on it.
    pub(crate) fn face_pick_for_group(&self, object: u64, group: u32, reference: FaceRef) -> Option<FacePick> {
        let instance = self.scene.mesh_instance(object)?;
        let matrix = instance.world_transform;
        let mesh = instance.mesh;
        let faces = geometry::group_faces(mesh, group);
        if faces.is_empty() {
            return None;
        }
        let plane = geometry::group_plane(mesh, &faces);
        let local_normal = plane.map(|p| p.0).unwrap_or(reference.normal);
        let world_normal = matrix
            .inverse()
            .transpose()
            .transform_vector3(local_normal)
            .normalize_or_zero();
        Some(FacePick {
            object,
            name: instance.object.name.clone(),
            kind: FaceKind::Part { group, reference },
            normal: world_normal,
            point: matrix.transform_point3(reference.point),
            prism: Rc::new(face_prism(mesh, &faces, matrix, world_normal)),
            planar: plane.is_some(),
            scale: matrix.transform_vector3(local_normal).length().max(1.0e-9),
        })
    }

    /// Offsets of the object's faces parallel to `face`, along its normal.
    fn parallel_heights_for(&self, face: &FacePick) -> Vec<f32> {
        let Some(instance) = self.scene.mesh_instance(face.object) else {
            return Vec::new();
        };
        geometry::parallel_heights(
            instance.mesh,
            instance.world_transform,
            face.prism.centroid(),
            face.normal,
        )
    }

    pub(crate) fn push_pull_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        let character = event
            .keystroke
            .key_char
            .as_deref()
            .and_then(|text| text.chars().next());
        let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_mut() else {
            return false;
        };
        match key {
            "escape" => {
                if let Some(active) = tool.active.as_mut() {
                    if !active.numeric.is_empty() && !active.modal {
                        active.numeric.clear();
                    } else if active.modal {
                        self.cancel_cad_tool(cx);
                    } else {
                        tool.active = None;
                        self.status = "Push/Pull · Drag a flat face".into();
                    }
                } else {
                    self.cancel_cad_tool(cx);
                    self.status = "Push/Pull off".into();
                }
                true
            }
            "enter" => {
                if !event.is_held {
                    self.commit_push_pull(cx);
                }
                true
            }
            "p" if !mods.shift && tool.active.as_ref().is_none_or(|a| a.numeric.is_empty()) => {
                self.cancel_cad_tool(cx);
                self.status = "Push/Pull off".into();
                true
            }
            "backspace" | "delete" => {
                if let Some(active) = tool.active.as_mut()
                    && super::erase_numeric(&mut active.numeric, key, mods.alt)
                {
                    apply_typed(active);
                }
                true
            }
            "f" if tool.active.as_ref().is_some_and(|a| a.numeric.is_empty()) => {
                let active = tool.active.as_mut().unwrap();
                active.distance = -active.distance;
                true
            }
            "g" | "r" | "s" | "e" | "i" | "b" => {
                // Other tools take over.
                self.cancel_cad_tool(cx);
                false
            }
            _ => match tool.active.as_mut() {
                Some(active) => {
                    let typed = super::type_numeric(&mut active.numeric, character);
                    if typed {
                        apply_typed(active);
                    }
                    typed
                }
                None => false,
            },
        }
    }

    pub(crate) fn push_pull_press(
        &mut self,
        position: Vec2,
        _event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_ref() else {
            return false;
        };
        if tool.active.as_ref().is_some_and(|a| a.modal) {
            self.commit_push_pull(cx);
            return true;
        }
        // The selected face's arrow.
        if let Some((base, tip)) = tool.handle_screen(self)
            && super::segment_distance(position, base, tip) <= PICK_PIXELS
        {
            let active = tool.active.as_ref().unwrap();
            let drag = AxisDrag::new(
                self,
                active.anchor(),
                active.faces[0].normal,
                active.distance,
                position,
            );
            if let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_mut()
                && let Some(active) = tool.active.as_mut()
            {
                active.drag = Some(drag);
                active.numeric.clear();
            }
            return true;
        }
        let pick = self.pick_face(position, tool.hover.as_ref());
        let Some(pick) = pick else {
            if let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_mut() {
                tool.active = None;
            }
            self.status = "Push/Pull · Drag a flat face of a part or mesh".into();
            return true;
        };
        if !pick.planar {
            self.status = "Push/pull works on flat faces · This face is curved".into();
            return true;
        }
        let drag = AxisDrag::new(self, pick.point, pick.normal, 0.0, position);
        let heights = self.parallel_heights_for(&pick);
        let label = if pick.is_part() { "part" } else { "mesh" };
        if let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_mut() {
            let same = tool
                .active
                .as_ref()
                .is_some_and(|a| a.faces.len() == 1 && a.faces[0].same_face(&pick));
            if same {
                // Pressing the selected face again drags it from its value.
                let active = tool.active.as_mut().unwrap();
                let mut drag = drag;
                drag.grab -= active.distance;
                drag.start_value = active.distance;
                active.drag = Some(drag);
            } else {
                tool.active = Some(ActiveFaces {
                    faces: vec![pick],
                    distance: 0.0,
                    numeric: String::new(),
                    drag: Some(drag),
                    modal: false,
                    heights,
                    snapped_face: false,
                    error: None,
                });
            }
        }
        self.status = format!("Push/Pull {label} face · Drag along the normal · Release to apply");
        cx.notify();
        true
    }

    pub(crate) fn push_pull_move(
        &mut self,
        position: Vec2,
        held: bool,
        free: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_ref() else {
            return false;
        };
        let modal = tool.active.as_ref().is_some_and(|a| a.modal);
        let dragging = tool.active.as_ref().and_then(|a| a.drag).filter(|_| held || modal);
        if let Some(mut drag) = dragging {
            if !drag.started {
                if drag.press.distance(position) <= DRAG_PIXELS {
                    return true;
                }
                drag.started = true;
            }
            let active = tool.active.as_ref().unwrap();
            let Some(raw) = drag.value(self, position) else {
                return true;
            };
            let anchor = active.anchor();
            let normal = active.faces[0].normal;
            let per_pixel = self.world_per_pixel(anchor);
            let along = self
                .world_to_screen(anchor)
                .zip(self.world_to_screen(anchor + normal * per_pixel * 20.0))
                .map(|(a, b)| a.distance(b) / (per_pixel * 20.0))
                .filter(|p| *p > 1.0e-3)
                .unwrap_or(1.0 / per_pixel.max(1.0e-9));
            let (value, face) = if free || !active.numeric.is_empty() {
                (raw, None)
            } else {
                geometry::snap_value(raw, geometry::snap_step(along), &active.heights, 8.0 / along)
            };
            if let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_mut()
                && let Some(active) = tool.active.as_mut()
            {
                active.drag = Some(drag);
                if active.numeric.is_empty() {
                    active.distance = value;
                    active.snapped_face = face.is_some();
                }
                let distance = active.distance;
                self.status = format!(
                    "Push/Pull {} · {} · Release to apply · Type an exact value",
                    crate::sketch::overlay::format_length(distance),
                    if distance >= 0.0 { "Pull out" } else { "Push in" }
                );
            }
            return true;
        }
        if held {
            return false;
        }
        // A release outside the window never arrived.
        if let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_mut()
            && let Some(active) = tool.active.as_mut()
            && active.drag.is_some()
            && !active.modal
        {
            active.drag = None;
        }
        let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_ref() else {
            return false;
        };
        let handle_hot = tool
            .handle_screen(self)
            .is_some_and(|(a, b)| super::segment_distance(position, a, b) <= PICK_PIXELS);
        let hover = if handle_hot {
            None
        } else {
            self.pick_face(position, tool.hover.as_ref())
        };
        if let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_mut() {
            let changed = tool.handle_hot != handle_hot
                || match (&tool.hover, &hover) {
                    (Some(a), Some(b)) => !a.same_face(b),
                    (None, None) => false,
                    _ => true,
                };
            tool.hover = hover;
            tool.handle_hot = handle_hot;
            if changed {
                cx.notify();
            }
        }
        false
    }

    pub(crate) fn push_pull_release(&mut self, _position: Vec2, cx: &mut Context<Self>) {
        let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_mut() else {
            return;
        };
        let Some(active) = tool.active.as_mut() else {
            return;
        };
        if active.modal {
            return;
        }
        let dragged = active.drag.take().is_some_and(|d| d.started);
        if dragged && active.distance.abs() >= MIN_DISTANCE {
            self.commit_push_pull(cx);
        } else if !dragged {
            self.status =
                "Push/Pull · Type a distance and press Return, or drag the arrow".into();
        }
        cx.notify();
    }

    /// Applies the active push/pull as one undo step.
    pub(crate) fn commit_push_pull(&mut self, cx: &mut Context<Self>) {
        let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_mut() else {
            return;
        };
        let Some(active) = tool.active.as_mut() else {
            self.status = "Push/Pull · Drag a flat face first".into();
            return;
        };
        if active.typed().is_err() {
            self.status = format!("Invalid distance · {}", active.numeric);
            cx.notify();
            return;
        }
        apply_typed(active);
        if active.distance.abs() < MIN_DISTANCE {
            self.status = "Push/Pull · Drag the face or type a distance".into();
            cx.notify();
            return;
        }
        let active = active.clone();
        let modal = active.modal;
        let before = self.scene.clone();
        let mut scene = self.scene.clone();
        let mut pending = Vec::new();
        let mut features = Vec::new();
        let mut mesh_record = None;
        let color = {
            let t = self.theme_colors();
            if active.distance >= 0.0 { t.accent } else { t.danger }
        };
        for face in &active.faces {
            match &face.kind {
                FaceKind::Part { reference, .. } => {
                    let Some(part) = scene.object_part_mut(face.object) else {
                        continue;
                    };
                    let distance = active.distance / face.scale;
                    let id = part.push_feature(
                        "Push/pull",
                        FeatureKind::PushPull(PushPullFeature {
                            face: *reference,
                            distance,
                        }),
                    );
                    if let Err(error) = part.validate() {
                        self.status = format!("Push/pull: {error:#}");
                        cx.notify();
                        return;
                    }
                    features.push(id);
                    pending.push(PendingGhost {
                        part: part.id,
                        stamp: part.stamp,
                        prism: Some((
                            face.prism.clone(),
                            0.0_f32.min(active.distance),
                            0.0_f32.max(active.distance),
                        )),
                        quads: Vec::new(),
                        color,
                    });
                }
                FaceKind::Mesh { face: index } => {
                    let Some(matrix) = scene.world_transform(face.object) else {
                        continue;
                    };
                    let Some(mesh) = scene.object_mesh_mut(face.object) else {
                        continue;
                    };
                    let source = mesh.clone();
                    let direction = matrix.inverse().transform_vector3(face.normal);
                    if let Err(error) = push_mesh_face(mesh, *index, direction * active.distance) {
                        self.status = format!("Push/pull: {error:#}");
                        cx.notify();
                        return;
                    }
                    mesh_record = Some((source, *index, direction));
                }
            }
        }
        let first = &active.faces[0];
        self.scene = scene;
        self.history.checkpoint_owned(before);
        self.dirty = true;
        self.cad.announce = true;
        self.cad_tool.pending.extend(pending);
        self.cad_tool.last = super::LastOperation::push_pull(
            &active,
            first,
            features,
            mesh_record,
            &self.scene,
            self.edit_mode,
        );
        if !self.part_editing().is_some_and(|id| id == first.object) {
            self.select_objects([first.object].into_iter().collect(), Some(first.object));
        }
        self.cad_tool.edit.faces.clear();
        self.status = format!(
            "{} {} · {}{}",
            if active.distance >= 0.0 {
                "Pulled"
            } else {
                "Pushed"
            },
            crate::sketch::overlay::format_length(active.distance.abs()),
            first.name,
            if first.is_part() {
                " · Regenerating…"
            } else {
                ""
            }
        );
        // Keyboard-started pushes end with their apply; hover pushes keep
        // the tool for the next face.
        if modal {
            self.cad_tool.tool = None;
        } else if let Some(Tool::PushPull(tool)) = self.cad_tool.tool.as_mut() {
            tool.active = None;
            tool.hover = None;
        }
        self.invalidate(true, cx);
    }
}

/// Applies a valid typed distance on the current side.
fn apply_typed(active: &mut ActiveFaces) {
    if let Ok(Some(value)) = active.typed() {
        active.distance = super::typed_on_side(value, active.distance);
        active.snapped_face = false;
    }
}

/// Pushes or pulls one mesh polygon by `offset` (object space). When every
/// other face at its vertices runs along the move (a box's sides), the face
/// slides and its neighbours stretch, as push/pull does in SketchUp;
/// otherwise the face is extruded as a region, growing walls (or a pocket
/// when pushed in).
pub(crate) fn push_mesh_face(
    mesh: &mut forma_core::Mesh,
    face: usize,
    offset: Vec3,
) -> anyhow::Result<()> {
    let polygon = mesh
        .faces
        .get(face)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("The face no longer exists"))?;
    let normal = offset.normalize_or_zero();
    anyhow::ensure!(normal != Vec3::ZERO, "Move the face away from where it is");
    let moving: BTreeSet<u32> = polygon.iter().copied().collect();
    let neighbours: Vec<usize> = (0..mesh.faces.len())
        .filter(|&f| f != face && mesh.faces[f].iter().any(|i| moving.contains(i)))
        .collect();
    let slides = neighbours
        .iter()
        .all(|&f| mesh.face_normal(f).dot(normal).abs() < 1.0e-4);
    if !slides {
        let faces: BTreeSet<usize> = [face].into_iter().collect();
        return mesh.extrude_region(&faces, offset);
    }
    // Stretching must not fold a neighbour over: the moved edge has to stay
    // beyond each neighbour's fixed vertices.
    let height = |p: Vec3| p.dot(normal);
    let level = height(mesh.positions[polygon[0] as usize]);
    let amount = offset.length();
    let folds = neighbours.iter().any(|&f| {
        mesh.faces[f]
            .iter()
            .filter(|i| !moving.contains(i))
            .map(|&i| height(mesh.positions[i as usize]))
            .any(|h| h > level + 1.0e-6 && h <= level + amount + 1.0e-6)
    });
    if folds {
        anyhow::bail!("Push/pull would collapse the neighbouring faces · Use a smaller distance");
    }
    let mut edited = mesh.clone();
    for &i in &moving {
        edited.positions[i as usize] += offset;
    }
    edited.validate()?;
    *mesh = edited;
    Ok(())
}

/// The cap and outline of a polygon set in world space, for ghosts and hover.
pub(crate) fn face_prism(
    mesh: &forma_core::Mesh,
    faces: &[usize],
    matrix: forma_core::Mat4,
    normal: Vec3,
) -> Prism {
    let mut prism = Prism {
        normal,
        ..Default::default()
    };
    let mut index: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut map = |vertex: u32, prism: &mut Prism| -> u32 {
        *index.entry(vertex).or_insert_with(|| {
            prism
                .points
                .push(matrix.transform_point3(mesh.positions[vertex as usize]));
            prism.points.len() as u32 - 1
        })
    };
    let flip = matrix.determinant() < 0.0;
    for &f in faces {
        let face = &mesh.faces[f];
        if face.len() < 3 {
            continue;
        }
        // Fan or ear-clip in the polygon's own plane.
        let points: Vec<Vec3> = face.iter().map(|&i| mesh.positions[i as usize]).collect();
        let n = mesh.face_normal(f);
        let u = n.any_orthonormal_vector();
        let v = n.cross(u);
        let flat: Vec<Vec2> = points.iter().map(|p| Vec2::new(p.dot(u), p.dot(v))).collect();
        let triangles = if face.len() == 3 {
            vec![[0, 1, 2]]
        } else {
            geometry::triangulate(&flat, &[])
        };
        let mapped: Vec<u32> = face.iter().map(|&i| map(i, &mut prism)).collect();
        for t in triangles {
            let mut triangle = t.map(|i| mapped[i as usize]);
            if flip {
                triangle.swap(1, 2);
            }
            prism.triangles.push(triangle);
        }
    }
    let boundary = geometry::group_boundary(mesh, faces);
    for lp in geometry::chain_loops(&boundary) {
        let mut mapped: Vec<u32> = lp.iter().map(|&i| map(i, &mut prism)).collect();
        if flip {
            mapped.reverse();
        }
        prism.loops.push(mapped);
    }
    prism
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::{Mesh, Primitive};

    #[test]
    fn face_prisms_cover_a_face_group_with_its_outline() {
        let cube = Mesh::primitive(Primitive::Cube);
        let top = (0..cube.faces.len())
            .find(|&f| cube.face_normal(f).y > 0.9)
            .unwrap();
        let prism = face_prism(&cube, &[top], forma_core::Mat4::IDENTITY, Vec3::Y);
        assert_eq!(prism.points.len(), 4);
        assert!((prism.area() - 4.0).abs() < 1.0e-5);
        assert_eq!(prism.loops.len(), 1);
        assert_eq!(prism.loops[0].len(), 4);
        // The cap winds about the outward normal.
        for t in &prism.triangles {
            let [a, b, c] = t.map(|i| prism.points[i as usize]);
            assert!((b - a).cross(c - a).y > 0.0);
        }
        assert!((prism.volume(0.0, 0.5) - 2.0).abs() < 1.0e-5);
    }

    #[test]
    fn box_faces_slide_and_other_faces_extrude() {
        let cube = Mesh::primitive(Primitive::Cube);
        let top = (0..cube.faces.len())
            .find(|&f| cube.face_normal(f).y > 0.9)
            .unwrap();
        // A box's top slides: its sides stretch, no faces are added.
        let mut pulled = cube.clone();
        push_mesh_face(&mut pulled, top, Vec3::Y * 0.5).unwrap();
        assert!((geometry::mesh_volume(&pulled) - 10.0).abs() < 1.0e-5);
        assert_eq!(pulled.faces.len(), cube.faces.len());
        let mut pushed = cube.clone();
        push_mesh_face(&mut pushed, top, Vec3::Y * -0.5).unwrap();
        assert!((geometry::mesh_volume(&pushed) - 6.0).abs() < 1.0e-5);
        // Pushing through the bottom is refused.
        assert!(push_mesh_face(&mut cube.clone(), top, Vec3::Y * -2.5).is_err());
        // A face whose neighbours are not along the move grows walls.
        let mut tilted = cube.clone();
        for p in &mut tilted.positions {
            if p.y > 0.0 {
                p.x *= 0.5;
                p.z *= 0.5;
            }
        }
        push_mesh_face(&mut tilted, top, Vec3::Y * 0.5).unwrap();
        assert_eq!(tilted.faces.len(), cube.faces.len() + 4);
    }
}

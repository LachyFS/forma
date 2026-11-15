//! Part edit mode: B-rep component editing on a part's regenerated
//! tessellation. Faces are whole face groups, edges are chains of display
//! edges between two groups; both are stored geometrically (part space), so
//! selections survive regeneration. Actions: Push/Pull faces, Chamfer
//! straight edges, Sketch on a face. B-rep components are never deleted
//! directly.
use super::geometry::{self, BrepEdge};
use super::{AxisDrag, PICK_PIXELS, PendingGhost, Tool};
use crate::app::{Command, EditMode, Studio};
use crate::sketch::DrawTarget;
use forma_core::{ChamferFeature, EdgeRef, FaceRef, FeatureKind, Mat4, Vec2, Vec3};
use gpui::{Context, KeyDownEvent, MouseDownEvent, Window};
use std::rc::Rc;

/// A selected edge by its end points (and a middle point to tell curved
/// edges with shared ends apart), in part space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EdgeSelection {
    pub start: Vec3,
    pub end: Vec3,
    pub middle: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EditHover {
    Face(u32),
    Edge(usize),
}

/// Selection and hover of part edit mode. Selections are geometric and
/// survive regeneration; the hover is an index into one tessellation's
/// topology, so it is only valid at the scene revision it was picked at.
#[derive(Clone, Debug, Default)]
pub(crate) struct EditState {
    pub object: Option<u64>,
    pub faces: Vec<FaceRef>,
    pub edges: Vec<EdgeSelection>,
    hover: Option<(u64, EditHover)>,
}

impl EditState {
    /// The hovered face group or edge, if it was picked at `revision`.
    pub(crate) fn hover(&self, revision: u64) -> Option<EditHover> {
        self.hover
            .filter(|(at, _)| *at == revision)
            .map(|(_, hover)| hover)
    }

    pub(crate) fn set_hover(&mut self, revision: u64, hover: Option<EditHover>) -> bool {
        let next = hover.map(|hover| (revision, hover));
        let changed = next != self.hover;
        self.hover = next;
        changed
    }
}

/// B-rep edges of one part tessellation, with the welded vertex of every
/// tessellation vertex (see `geometry::weld`).
#[derive(Debug)]
pub(crate) struct Topology {
    pub edges: Vec<BrepEdge>,
    pub canon: Vec<u32>,
}

impl Topology {
    pub(crate) fn new(mesh: &forma_core::Mesh) -> Self {
        let canon = geometry::weld(mesh);
        Self {
            edges: geometry::brep_edges_welded(mesh, &canon),
            canon,
        }
    }
}

impl Topology {
    fn edge_selection(&self, mesh: &forma_core::Mesh, index: usize) -> Option<EdgeSelection> {
        let edge = self.edges.get(index)?;
        let p = |i: u32| mesh.positions[i as usize];
        Some(EdgeSelection {
            start: p(edge.points[0]),
            end: p(*edge.points.last()?),
            middle: p(edge.points[edge.points.len() / 2]),
        })
    }

    /// The edge matching a stored selection (either direction).
    fn resolve(&self, mesh: &forma_core::Mesh, selection: &EdgeSelection) -> Option<usize> {
        let tolerance = selection.start.distance(selection.end).max(1.0e-3) * 1.0e-3;
        (0..self.edges.len()).find(|&i| {
            self.edge_selection(mesh, i).is_some_and(|e| {
                let forward = e.start.distance(selection.start) < tolerance
                    && e.end.distance(selection.end) < tolerance;
                let backward = e.start.distance(selection.end) < tolerance
                    && e.end.distance(selection.start) < tolerance;
                (forward || backward)
                    && e.middle.distance(selection.middle) < tolerance.max(1.0e-4) * 10.0
            })
        })
    }
}

/// A straight edge to chamfer, with the directions into its two faces.
#[derive(Clone, Debug)]
pub(crate) struct ChamferEdge {
    pub reference: EdgeRef,
    /// World end points.
    pub start: Vec3,
    pub end: Vec3,
    /// Unit directions from the edge into each adjacent face (world).
    pub into_a: Vec3,
    pub into_b: Vec3,
}

impl ChamferEdge {
    /// The chamfer face for `distance` (world).
    pub(crate) fn quad(&self, distance: f32) -> [Vec3; 4] {
        [
            self.start + self.into_a * distance,
            self.end + self.into_a * distance,
            self.end + self.into_b * distance,
            self.start + self.into_b * distance,
        ]
    }
}

pub(crate) struct ChamferTool {
    pub object: u64,
    pub edges: Vec<ChamferEdge>,
    /// World distance along each face.
    pub distance: f32,
    pub numeric: String,
    pub drag: AxisDrag,
    /// World length of one part unit (uniform object scale).
    pub scale: f32,
    pub error: Option<String>,
}

impl ChamferTool {
    pub(crate) fn typed(&self) -> Result<Option<f32>, ()> {
        let text = self.numeric.trim();
        if text.is_empty() {
            return Ok(None);
        }
        crate::transform::distance_value(text).map(Some).ok_or(())
    }

    pub(crate) fn value(&self) -> f32 {
        match self.typed() {
            Ok(Some(value)) => value.abs(),
            _ => self.distance,
        }
    }
}

impl Studio {
    /// B-rep edges of a part's current tessellation, cached per scene revision.
    pub(crate) fn part_topology(&self, object: u64) -> Option<Rc<Topology>> {
        let mesh = self.scene.object_mesh(object)?;
        let mut cache = self.cad_tool.topology.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|(revision, _)| *revision != self.scene_revision)
        {
            *cache = Some((self.scene_revision, Vec::new()));
        }
        let (_, entries) = cache.as_mut().unwrap();
        if let Some((_, topology)) = entries.iter().find(|(id, _)| *id == object) {
            return Some(topology.clone());
        }
        let topology = Rc::new(Topology::new(mesh));
        entries.push((object, topology.clone()));
        Some(topology)
    }

    /// Enters B-rep component editing on a part.
    pub(crate) fn enter_part_edit(&mut self, id: u64, mode: EditMode, cx: &mut Context<Self>) {
        self.edit_mode = mode;
        self.clear_components();
        self.select_objects_keep_edit(id);
        self.cad_tool.edit = EditState {
            object: Some(id),
            ..Default::default()
        };
        self.status = format!(
            "Edit part · {} · Click to select · P push/pull · {} chamfer · D sketch on face",
            mode.label(),
            crate::platform_shortcut("⌘B", "Ctrl+B")
        );
        self.invalidate(false, cx);
    }

    fn select_objects_keep_edit(&mut self, id: u64) {
        self.selected = Some(id);
        self.objects_selected = [id].into_iter().collect();
    }

    fn leave_part_edit(&mut self, cx: &mut Context<Self>) {
        self.edit_mode = EditMode::Object;
        self.cad_tool.edit = EditState::default();
        self.status = "Object mode".into();
        self.invalidate(false, cx);
    }

    /// The edit state for `id`, reset when the edited part changed.
    fn edit_state_for(&mut self, id: u64) -> &mut EditState {
        if self.cad_tool.edit.object != Some(id) {
            self.cad_tool.edit = EditState {
                object: Some(id),
                ..Default::default()
            };
        }
        &mut self.cad_tool.edit
    }

    /// Selected faces of the edited part resolved against its tessellation.
    /// Resolved once per scene revision and selection: the overlay and the
    /// status bar ask every frame.
    pub(crate) fn selected_edit_faces(&self, id: u64) -> Vec<(u32, FaceRef)> {
        if self.cad_tool.edit.object != Some(id) {
            return Vec::new();
        }
        let key = (self.scene_revision, id, self.cad_tool.edit.faces.clone());
        if let Some((cached, groups)) = self.cad_tool.resolved_faces.borrow().as_ref()
            && *cached == key
        {
            return groups.clone();
        }
        let Some(mesh) = self.scene.object_mesh(id) else {
            return Vec::new();
        };
        let references: Vec<(Vec3, Vec3)> = self
            .cad_tool
            .edit
            .faces
            .iter()
            .map(|r| (r.point, r.normal))
            .collect();
        let mut groups: Vec<(u32, FaceRef)> = Vec::new();
        for (group, reference) in geometry::resolve_groups(mesh, &references)
            .into_iter()
            .zip(&self.cad_tool.edit.faces)
        {
            if let Some(group) = group
                && !groups.iter().any(|(g, _)| *g == group)
            {
                groups.push((group, *reference));
            }
        }
        *self.cad_tool.resolved_faces.borrow_mut() = Some((key, groups.clone()));
        groups
    }

    /// Selected edges of the edited part as topology indices.
    pub(crate) fn selected_edit_edges(&self, id: u64) -> Vec<usize> {
        if self.cad_tool.edit.object != Some(id) {
            return Vec::new();
        }
        let (Some(mesh), Some(topology)) = (self.scene.object_mesh(id), self.part_topology(id))
        else {
            return Vec::new();
        };
        let key = (self.scene_revision, id, self.cad_tool.edit.edges.clone());
        if let Some((cached, indices)) = self.cad_tool.resolved_edges.borrow().as_ref()
            && *cached == key
        {
            return indices.clone();
        }
        let indices: Vec<usize> = self
            .cad_tool
            .edit
            .edges
            .iter()
            .filter_map(|selection| topology.resolve(mesh, selection))
            .collect();
        *self.cad_tool.resolved_edges.borrow_mut() = Some((key, indices.clone()));
        indices
    }

    /// Workspace commands while editing a part. Returns true when handled.
    pub(crate) fn part_edit_command(
        &mut self,
        id: u64,
        command: Command,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut hint = |s: &mut Self, text: &str| {
            s.status = text.into();
            cx.notify();
        };
        match command {
            Command::ToggleEdit | Command::SetEditMode(EditMode::Object) => {
                self.leave_part_edit(cx);
                true
            }
            Command::SetEditMode(EditMode::Vertex) => {
                hint(self, "Parts edit faces and edges · 2 Edge · 3 Face");
                true
            }
            Command::SetEditMode(mode) => {
                self.edit_mode = mode;
                self.edit_state_for(id).set_hover(0, None);
                self.status = format!(
                    "Edit part · {} · Click to select · Shift adds",
                    mode.label()
                );
                self.invalidate(false, cx);
                true
            }
            Command::SelectAll | Command::DeselectAll | Command::InvertSelection => {
                let select = command != Command::DeselectAll;
                let faces = self.edit_mode == EditMode::Face;
                let all_faces = self.all_face_refs(id);
                let all_edges = self.all_edge_selections(id);
                let state = self.edit_state_for(id);
                if command == Command::InvertSelection {
                    if faces {
                        let current = std::mem::take(&mut state.faces);
                        state.faces = all_faces
                            .into_iter()
                            .filter(|f| !current.iter().any(|c| same_face_ref(c, f)))
                            .collect();
                    } else {
                        let current = std::mem::take(&mut state.edges);
                        state.edges = all_edges
                            .into_iter()
                            .filter(|e| !current.contains(e))
                            .collect();
                    }
                } else if faces {
                    state.faces = if select { all_faces } else { Vec::new() };
                } else {
                    state.edges = if select { all_edges } else { Vec::new() };
                }
                self.status = self.part_edit_summary().unwrap_or_default();
                cx.notify();
                true
            }
            Command::SelectLinked | Command::BoxSelect => {
                hint(self, "Click faces or edges · Shift adds");
                true
            }
            Command::Delete => {
                hint(
                    self,
                    "B-rep faces and edges can't be deleted · Push/pull (P), chamfer or Convert to mesh",
                );
                true
            }
            Command::Duplicate => {
                hint(self, "Tab to Object mode to duplicate the part");
                true
            }
            Command::Extrude => {
                self.push_pull_selected_faces(cx);
                true
            }
            Command::Bevel => {
                self.start_chamfer(cx);
                true
            }
            Command::Inset
            | Command::Subdivide
            | Command::Triangulate
            | Command::FlipNormals
            | Command::RecalculateNormals => {
                hint(
                    self,
                    "Polygon tools apply to meshes · Convert to mesh to edit polygons",
                );
                true
            }
            _ => false,
        }
    }

    fn all_face_refs(&self, id: u64) -> Vec<FaceRef> {
        let Some(mesh) = self.scene.object_mesh(id) else {
            return Vec::new();
        };
        let mut groups: Vec<u32> = mesh.groups.clone();
        groups.sort_unstable();
        groups.dedup();
        groups
            .into_iter()
            .filter_map(|g| {
                let faces = geometry::group_faces(mesh, g);
                let first = *faces.first()?;
                let face = &mesh.faces[first];
                let centroid = face
                    .iter()
                    .map(|&i| mesh.positions[i as usize])
                    .sum::<Vec3>()
                    / face.len() as f32;
                forma_cad::face_ref_from_pick(mesh, first, centroid)
            })
            .collect()
    }

    fn all_edge_selections(&self, id: u64) -> Vec<EdgeSelection> {
        let (Some(mesh), Some(topology)) = (self.scene.object_mesh(id), self.part_topology(id))
        else {
            return Vec::new();
        };
        (0..topology.edges.len())
            .filter_map(|i| topology.edge_selection(mesh, i))
            .collect()
    }

    /// Keys while editing a part without a running tool.
    pub(crate) fn part_edit_key(
        &mut self,
        id: u64,
        key: &str,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut hint = |s: &mut Self, text: &str| {
            s.status = text.into();
            cx.notify();
        };
        match key {
            "e" | "p" if !shift => {
                self.cad_tool.last = None;
                if self.selected_edit_faces(id).is_empty() {
                    self.start_push_pull(cx);
                } else {
                    self.push_pull_selected_faces(cx);
                }
                true
            }
            "d" if !shift => {
                self.cad_tool.last = None;
                self.sketch_on_selected_face(window, cx);
                true
            }
            "g" | "r" | "s" => {
                hint(
                    self,
                    "Move faces with Push/Pull (P) · Tab to Object mode to transform the part",
                );
                true
            }
            "delete" | "backspace" => {
                hint(
                    self,
                    "B-rep faces and edges can't be deleted · Push/pull (P), chamfer or Convert to mesh",
                );
                true
            }
            "i" | "l" => {
                hint(
                    self,
                    "Polygon tools apply to meshes · Convert to mesh to edit polygons",
                );
                true
            }
            "1" => {
                hint(self, "Parts edit faces and edges · 2 Edge · 3 Face");
                true
            }
            "escape" => {
                let state = self.edit_state_for(id);
                if state.faces.is_empty() && state.edges.is_empty() {
                    self.leave_part_edit(cx);
                } else {
                    state.faces.clear();
                    state.edges.clear();
                    cx.notify();
                }
                true
            }
            _ => false,
        }
    }

    /// Hover over the edited part's faces or edges.
    pub(crate) fn part_edit_hover(&mut self, id: u64, position: Vec2, cx: &mut Context<Self>) {
        let hover = self.part_edit_pick(id, position).map(|(hover, _)| hover);
        let revision = self.scene_revision;
        if self.edit_state_for(id).set_hover(revision, hover) {
            cx.notify();
        }
    }

    /// The face group or edge of the edited part under the pointer, with a
    /// part-space face reference for faces.
    fn part_edit_pick(&self, id: u64, position: Vec2) -> Option<(EditHover, Option<FaceRef>)> {
        let instance = self.scene.mesh_instance(id)?;
        let matrix = instance.world_transform;
        match self.edit_mode {
            EditMode::Edge => {
                let topology = self.part_topology(id)?;
                let query = self.visibility_query();
                let camera = self.scene.camera;
                let mut best: Option<(f32, usize)> = None;
                for (index, edge) in topology.edges.iter().enumerate() {
                    for pair in edge.points.windows(2) {
                        let a = matrix.transform_point3(instance.mesh.positions[pair[0] as usize]);
                        let b = matrix.transform_point3(instance.mesh.positions[pair[1] as usize]);
                        let (Some(sa), Some(sb)) =
                            (self.world_to_screen(a), self.world_to_screen(b))
                        else {
                            continue;
                        };
                        let d = super::segment_distance(position, sa, sb);
                        if d <= PICK_PIXELS
                            && best.is_none_or(|(bd, _)| d < bd)
                            && crate::viewport::visible_in_query(camera, query, (a + b) * 0.5)
                        {
                            best = Some((d, index));
                        }
                    }
                }
                best.map(|(_, index)| (EditHover::Edge(index), None))
            }
            _ => {
                let ray = self.tool_ray(position)?;
                // The edited part's own index, then occlusion by everything
                // else through the shared visibility hierarchy.
                let hit = self.pick_index(Some(id)).pick(ray)?;
                if !crate::viewport::visible_in_query(
                    self.scene.camera,
                    self.visibility_query(),
                    hit.position,
                ) {
                    return None;
                }
                let group = instance.mesh.face_group(hit.face)?;
                let local = matrix.inverse().transform_point3(hit.position);
                let reference = forma_cad::face_ref_from_pick(instance.mesh, hit.face, local);
                Some((EditHover::Face(group), reference))
            }
        }
    }

    /// Presses while editing a part: select faces or edges, switch parts.
    pub(crate) fn part_edit_press(
        &mut self,
        id: u64,
        position: Vec2,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.cad_tool.last = None;
        let shift = event.modifiers.shift;
        let pick = self.part_edit_pick(id, position);
        let mesh = self.scene.object_mesh(id).cloned();
        let topology = self.part_topology(id);
        match pick {
            Some((EditHover::Face(_), Some(reference))) => {
                let state = self.edit_state_for(id);
                let existing = state
                    .faces
                    .iter()
                    .position(|f| same_face_ref(f, &reference));
                match (shift, existing) {
                    (true, Some(index)) => {
                        state.faces.remove(index);
                    }
                    (true, None) => state.faces.push(reference),
                    (false, _) => state.faces = vec![reference],
                }
                if !shift {
                    state.edges.clear();
                }
            }
            Some((EditHover::Edge(index), _)) => {
                if let (Some(mesh), Some(topology)) = (mesh, topology)
                    && let Some(selection) = topology.edge_selection(&mesh, index)
                {
                    let state = self.edit_state_for(id);
                    let existing = state.edges.iter().position(|e| *e == selection);
                    match (shift, existing) {
                        (true, Some(i)) => {
                            state.edges.remove(i);
                        }
                        (true, None) => state.edges.push(selection),
                        (false, _) => state.edges = vec![selection],
                    }
                    if !shift {
                        state.faces.clear();
                    }
                }
            }
            _ => {
                // Another part under the pointer becomes the edited part.
                let other = self
                    .tool_ray(position)
                    .and_then(|ray| self.scene.pick(ray))
                    .map(|hit| hit.object_id)
                    .filter(|&other| other != id);
                match other {
                    Some(other) if self.scene.object(other).is_some_and(|o| o.is_part()) => {
                        self.select_objects_keep_edit(other);
                        self.cad_tool.edit = EditState {
                            object: Some(other),
                            ..Default::default()
                        };
                        self.status = format!(
                            "Editing {}",
                            self.scene
                                .object(other)
                                .map(|o| o.name.clone())
                                .unwrap_or_default()
                        );
                        self.invalidate(false, cx);
                    }
                    Some(_) => {
                        self.status = "Tab to Object mode to select meshes".into();
                    }
                    None if !shift => {
                        let state = self.edit_state_for(id);
                        state.faces.clear();
                        state.edges.clear();
                    }
                    None => {}
                }
            }
        }
        if let Some(summary) = self.part_edit_summary() {
            self.status = summary;
        }
        cx.notify();
        true
    }

    /// Status bar readout of the part edit selection.
    pub(crate) fn part_edit_summary(&self) -> Option<String> {
        let id = self.part_editing()?;
        let faces = self.selected_edit_faces(id).len();
        let edges = self.selected_edit_edges(id).len();
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        Some(match (faces, edges) {
            (0, 0) => format!(
                "No {} selected",
                if self.edit_mode == EditMode::Edge {
                    "edge"
                } else {
                    "face"
                }
            ),
            (f, 0) => format!("{} selected", plural(f, "face", "faces")),
            (0, e) => format!("{} selected", plural(e, "edge", "edges")),
            (f, e) => format!(
                "{} · {} selected",
                plural(f, "face", "faces"),
                plural(e, "edge", "edges")
            ),
        })
    }

    /// Starts a sketch on the face under the pointer, else the selected face.
    pub(crate) fn sketch_on_selected_face(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let under_pointer = self
            .tool_ray(self.last_mouse)
            .and_then(|ray| crate::sketch::plane::face_plane(&self.scene, self.scene.camera, ray));
        if under_pointer.is_some() {
            self.enter_draw(DrawTarget::AtPointer, cx);
            return;
        }
        let selected = self
            .part_editing()
            .and_then(|id| Some((id, *self.selected_edit_faces(id).first()?)));
        let choice = selected.and_then(|(id, (group, _))| {
            crate::sketch::plane::group_plane_choice(
                &self.scene,
                self.scene.camera,
                id,
                group,
                None,
            )
        });
        match choice {
            Some(choice) => self.enter_draw(DrawTarget::Choice(choice), cx),
            None => self.enter_draw(DrawTarget::AtPointer, cx),
        }
    }

    /// Starts Chamfer on the selected straight edges (or the hovered one).
    pub(crate) fn start_chamfer(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.part_editing() else {
            self.status = format!(
                "Chamfer · Tab into a part, press 2 for edges and select straight edges · {}",
                crate::platform_shortcut("⌘B", "Ctrl+B")
            );
            cx.notify();
            return;
        };
        let mut indices = self.selected_edit_edges(id);
        // The hover refers to the current tessellation only (undo or a
        // regeneration renumbers edges).
        if indices.is_empty()
            && let Some(EditHover::Edge(index)) = self.cad_tool.edit.hover(self.scene_revision)
        {
            indices.push(index);
        }
        if indices.is_empty() {
            self.status = "Chamfer · Select straight edges first (2 for Edge mode)".into();
            cx.notify();
            return;
        }
        let (Some(instance), Some(topology)) =
            (self.scene.mesh_instance(id), self.part_topology(id))
        else {
            return;
        };
        let matrix = instance.world_transform;
        let mut edges = Vec::new();
        for index in indices {
            let Some(edge) = topology.edges.get(index) else {
                self.status = "Chamfer · Select straight edges first (2 for Edge mode)".into();
                cx.notify();
                return;
            };
            match chamfer_edge(instance.mesh, &topology.canon, edge, matrix) {
                Ok(edge) => edges.push(edge),
                Err(message) => {
                    self.status = format!("Chamfer · {message}");
                    cx.notify();
                    return;
                }
            }
        }
        // Chamfer distances are measured along faces in part space; a part
        // scaled unevenly would chamfer by different amounts on each side.
        let Some(scale) = uniform_scale(matrix) else {
            self.status = "Chamfer needs an evenly scaled part · Apply its scale first".into();
            cx.notify();
            return;
        };
        let first = &edges[0];
        let middle = (first.start + first.end) * 0.5;
        let bisector = (first.into_a + first.into_b).normalize_or_zero();
        let mut drag = AxisDrag::new(self, middle, bisector, 0.0, self.last_mouse);
        drag.started = true;
        let count = edges.len();
        if self.cad_tool.active() {
            self.cancel_cad_tool(cx);
        }
        self.cad_tool.tool = Some(Tool::Chamfer(Box::new(ChamferTool {
            object: id,
            edges,
            distance: 0.0,
            numeric: String::new(),
            drag,
            scale,
            error: None,
        })));
        self.status = format!(
            "Chamfer {count} edge{} · Move the pointer or type a distance · Click or Return to apply · Esc cancel",
            if count == 1 { "" } else { "s" }
        );
        cx.notify();
    }

    pub(crate) fn chamfer_move(&mut self, position: Vec2, free: bool, cx: &mut Context<Self>) {
        let Some(Tool::Chamfer(tool)) = self.cad_tool.tool.as_ref() else {
            return;
        };
        if !tool.numeric.is_empty() {
            return;
        }
        let Some(raw) = tool.drag.value(self, position) else {
            return;
        };
        let first = &tool.edges[0];
        // The chamfer face's middle moves |a + b| / 2 per unit of distance.
        let rate = ((first.into_a + first.into_b).length() * 0.5).max(1.0e-3);
        let middle = (first.start + first.end) * 0.5;
        let per_pixel = self.world_per_pixel(middle);
        let mut distance = (raw / rate).max(0.0);
        if !free {
            distance =
                geometry::snap_value(distance, geometry::snap_step(1.0 / per_pixel), &[], 0.0).0;
        }
        if let Some(Tool::Chamfer(tool)) = self.cad_tool.tool.as_mut() {
            tool.distance = distance;
            self.status = format!(
                "Chamfer {} · Click or Return to apply · Type an exact value",
                crate::sketch::overlay::format_length(distance)
            );
        }
        cx.notify();
    }

    pub(crate) fn chamfer_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        let character = event
            .keystroke
            .key_char
            .as_deref()
            .and_then(|text| text.chars().next());
        let Some(Tool::Chamfer(tool)) = self.cad_tool.tool.as_mut() else {
            return false;
        };
        match key {
            "escape" => {
                self.cancel_cad_tool(cx);
                true
            }
            "enter" => {
                if !event.is_held {
                    self.commit_chamfer(cx);
                }
                true
            }
            "backspace" | "delete" => {
                super::erase_numeric(&mut tool.numeric, key, mods.alt);
                true
            }
            _ => super::type_numeric(&mut tool.numeric, character),
        }
    }

    /// Adds the chamfer feature as one undo step.
    pub(crate) fn commit_chamfer(&mut self, cx: &mut Context<Self>) {
        let Some(Tool::Chamfer(tool)) = self.cad_tool.tool.as_ref() else {
            return;
        };
        if tool.typed().is_err() {
            self.status = format!("Invalid distance · {}", tool.numeric);
            cx.notify();
            return;
        }
        let distance = tool.value();
        if distance < 1.0e-5 {
            self.status = "Chamfer · Move the pointer or type a distance".into();
            cx.notify();
            return;
        }
        let object = tool.object;
        let local = distance / tool.scale;
        let references: Vec<EdgeRef> = tool.edges.iter().map(|e| e.reference).collect();
        let quads: Vec<[Vec3; 4]> = tool.edges.iter().map(|e| e.quad(distance)).collect();
        let edges = tool.edges.clone();
        let scale = tool.scale;
        let before = self.scene.clone();
        let Some(part) = self.scene.object_part_mut(object) else {
            self.cancel_cad_tool(cx);
            return;
        };
        let feature = super::insert_feature(
            part,
            "Chamfer",
            FeatureKind::Chamfer(ChamferFeature {
                edges: references,
                distance: local,
            }),
            None,
        );
        if let Err(error) = part.validate() {
            self.scene = before;
            self.status = format!("Chamfer: {error:#}");
            cx.notify();
            return;
        }
        let (part_id, stamp) = (part.id, part.stamp);
        self.history.checkpoint_owned(before);
        self.cad_tool.tool = None;
        self.dirty = true;
        self.cad.announce = true;
        let color = self.theme_colors().accent;
        self.prune_pending_ghosts();
        self.cad_tool.pending.push(PendingGhost {
            part: part_id,
            stamp,
            prism: None,
            quads,
            color,
        });
        self.cad_tool.edit.edges.clear();
        self.reveal_feature(object, feature);
        self.cad_tool.last = Some(super::LastOperation::chamfer(
            object,
            part_id,
            feature,
            stamp,
            distance,
            scale,
            edges,
            &self.scene,
            self.edit_mode,
            self.history_version(),
        ));
        self.status = format!(
            "Chamfered {} edge{} by {} · Regenerating…",
            self.cad_tool.last.as_ref().map_or(0, |l| l.edge_count()),
            if self.cad_tool.last.as_ref().map_or(0, |l| l.edge_count()) == 1 {
                ""
            } else {
                "s"
            },
            crate::sketch::overlay::format_length(distance)
        );
        self.invalidate(true, cx);
    }
}

/// The scale of a transform that scales all axes equally (rotations and
/// mirroring allowed), or `None` for uneven or skewing scales.
pub(crate) fn uniform_scale(matrix: Mat4) -> Option<f32> {
    let axes = [matrix.x_axis, matrix.y_axis, matrix.z_axis].map(|a| a.truncate());
    let lengths = axes.map(|a| a.length());
    let scale = lengths[0];
    let even = lengths.iter().all(|l| (l - scale).abs() <= scale * 1.0e-4)
        && (0..3).all(|i| axes[i].dot(axes[(i + 1) % 3]).abs() <= scale * scale * 1.0e-4);
    (even && scale > 1.0e-9 && scale.is_finite()).then_some(scale)
}

fn same_face_ref(a: &FaceRef, b: &FaceRef) -> bool {
    a.normal.dot(b.normal) > 0.9999
        && (a.point - b.point).dot(a.normal).abs() < 1.0e-5
        && a.point.distance(b.point) < 1.0e-5
}

/// The chamfer geometry of a straight B-rep edge between two flat faces.
pub(crate) fn chamfer_edge(
    mesh: &forma_core::Mesh,
    canon: &[u32],
    edge: &BrepEdge,
    matrix: Mat4,
) -> Result<ChamferEdge, &'static str> {
    if !edge.straight {
        return Err("only straight edges can be chamfered");
    }
    if edge.groups.0 == edge.groups.1 {
        return Err("this edge has a face on one side only");
    }
    let p = |i: u32| mesh.positions[i as usize];
    let (a, b) = (p(edge.points[0]), p(*edge.points.last().unwrap()));
    let (s0, s1) = (edge.points[0], edge.points[1]);
    let inverse_transpose = matrix.inverse().transpose();
    // For each side: the polygon of that group using the first segment.
    let into = |group: u32| -> Option<(Vec3, Vec3)> {
        let face = (0..mesh.faces.len()).find(|&f| {
            mesh.face_group(f) == Some(group) && {
                let face = &mesh.faces[f];
                (0..face.len()).any(|k| {
                    let (x, y) = (
                        canon[face[k] as usize],
                        canon[face[(k + 1) % face.len()] as usize],
                    );
                    (x == s0 && y == s1) || (x == s1 && y == s0)
                })
            }
        })?;
        let faces = geometry::group_faces(mesh, group);
        let (normal, _) = geometry::group_plane(mesh, &faces)?;
        let polygon = &mesh.faces[face];
        let centroid = polygon.iter().map(|&i| p(i)).sum::<Vec3>() / polygon.len() as f32;
        let mut direction = normal.cross(b - a).normalize_or_zero();
        if direction.dot(centroid - a) < 0.0 {
            direction = -direction;
        }
        Some((direction, normal))
    };
    let ((into_a, normal_a), (into_b, normal_b)) = (
        into(edge.groups.0).ok_or("chamfers need flat faces on both sides")?,
        into(edge.groups.1).ok_or("chamfers need flat faces on both sides")?,
    );
    if (normal_a + normal_b).dot(into_a + into_b) >= -1.0e-4 {
        return Err("chamfers need convex edges");
    }
    let world = |v: Vec3| matrix.transform_vector3(v).normalize_or_zero();
    let _ = inverse_transpose;
    Ok(ChamferEdge {
        reference: EdgeRef { start: a, end: b },
        start: matrix.transform_point3(a),
        end: matrix.transform_point3(b),
        into_a: world(into_a),
        into_b: world(into_b),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::{Mesh, Primitive};

    fn grouped_cube() -> Mesh {
        let cube = Mesh::primitive(Primitive::Cube);
        Mesh {
            groups: (0..cube.faces.len() as u32).collect(),
            ..cube
        }
    }

    #[test]
    fn convex_box_edges_chamfer_into_both_faces() {
        let mesh = grouped_cube();
        let edges = geometry::brep_edges(&mesh);
        let edge = &edges[0];
        let canon = geometry::weld(&mesh);
        let chamfer = chamfer_edge(&mesh, &canon, edge, Mat4::IDENTITY).unwrap();
        assert!(
            chamfer.into_a.dot(chamfer.into_b).abs() < 1.0e-5,
            "perpendicular faces"
        );
        // Both directions point back toward the cube's centre.
        let middle = (chamfer.start + chamfer.end) * 0.5;
        assert!((chamfer.into_a + chamfer.into_b).dot(-middle) > 0.0);
        let quad = chamfer.quad(0.1);
        assert!((quad[0].distance(chamfer.start) - 0.1).abs() < 1.0e-5);
        // Curved edges are refused.
        let curved = BrepEdge {
            straight: false,
            ..edge.clone()
        };
        assert!(chamfer_edge(&mesh, &canon, &curved, Mat4::IDENTITY).is_err());
    }

    /// Part tessellations repeat edge points per face group; edges are
    /// found by welding positions, as on the real bracket.
    #[test]
    fn unwelded_part_tessellations_still_give_shared_edges() {
        let scene = crate::cad::demo_scene();
        let part = scene.parts.iter().find(|p| p.name == "Bracket").unwrap();
        let result = forma_cad::regenerate(
            &forma_cad::RegenRequest {
                part_id: part.id,
                part: part.clone(),
                skip: Vec::new(),
            },
            &mut forma_cad::RegenCache::default(),
            &mut |_| {},
        );
        let mesh = result.mesh.unwrap();
        let topology = Topology::new(&mesh);
        let p = |i: u32| mesh.positions[i as usize];
        // The plate's top front edge: straight, between two different faces.
        let index = topology
            .edges
            .iter()
            .position(|e| {
                let (a, b) = (p(e.points[0]), p(*e.points.last().unwrap()));
                (a.y - 0.3).abs() < 1.0e-3 && (b.y - 0.3).abs() < 1.0e-3 && a.z > 0.74 && b.z > 0.74
            })
            .expect("top front edge");
        let edge = &topology.edges[index];
        assert!(edge.straight && edge.groups.0 != edge.groups.1, "{edge:?}");
        let chamfer = chamfer_edge(&mesh, &topology.canon, edge, Mat4::IDENTITY).unwrap();
        assert!(chamfer.into_a.dot(chamfer.into_b).abs() < 1.0e-3);
        // Hole rims are single curved edges.
        assert!(
            topology
                .edges
                .iter()
                .any(|e| !e.straight && e.points.len() > 8)
        );
    }

    /// Regression (review): chamfers assumed uniform scale.
    #[test]
    fn chamfers_require_even_scale() {
        use glam::{Quat, Vec3};
        assert_eq!(uniform_scale(Mat4::IDENTITY), Some(1.0));
        let even = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_y(0.7),
            Vec3::new(1.0, 2.0, 3.0),
        );
        assert!((uniform_scale(even).unwrap() - 2.0).abs() < 1.0e-5);
        assert!(uniform_scale(Mat4::from_scale(Vec3::new(1.0, 2.0, 1.0))).is_none());
        assert!(uniform_scale(Mat4::from_scale(Vec3::ZERO)).is_none());
    }

    /// Regression: Ctrl+B after Undo used an edge index hovered on the
    /// pre-undo tessellation (15 edges) against the new one (12) and panicked.
    #[test]
    fn hovers_expire_with_the_tessellation_they_index() {
        let mut state = EditState::default();
        assert!(state.set_hover(7, Some(EditHover::Edge(14))));
        assert_eq!(state.hover(7), Some(EditHover::Edge(14)));
        assert_eq!(state.hover(8), None, "a new scene revision drops the index");
        assert!(!state.set_hover(7, Some(EditHover::Edge(14))));
        assert!(state.set_hover(8, None));
        assert_eq!(state.hover(8), None);
    }

    #[test]
    fn edge_selections_resolve_in_either_direction() {
        let mesh = grouped_cube();
        let topology = Topology::new(&mesh);
        let selection = topology.edge_selection(&mesh, 3).unwrap();
        assert_eq!(topology.resolve(&mesh, &selection), Some(3));
        let reversed = EdgeSelection {
            start: selection.end,
            end: selection.start,
            middle: selection.middle,
        };
        assert_eq!(topology.resolve(&mesh, &reversed), Some(3));
        let moved = EdgeSelection {
            start: selection.start + Vec3::X * 3.0,
            ..selection
        };
        assert_eq!(topology.resolve(&mesh, &moved), None);
    }
}

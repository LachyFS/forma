//! Interactive direct-modelling tools that work across B-rep parts and
//! polygon meshes: Extrude and Revolve sketch regions, Push/Pull faces of
//! parts and meshes, B-rep component editing with Chamfer, and the "adjust
//! last operation" panel.
//!
//! Every tool previews instantly with an in-app ghost (no kernel call while
//! the pointer moves), then hands one feature to the kernel worker. Sessions
//! that stay open after a drag (Extrude, Revolve) apply their candidate
//! feature to a live copy of the scene on release, so the real regenerated
//! result appears before committing; `Session::before` restores the scene
//! on cancel and is the single undo checkpoint on commit.
//!
//! Hooks other code calls:
//! - `Studio::cad_tool_key` / `cad_tool_mouse_down|move|up` from the viewport
//!   and window input handlers, before the generic ones;
//! - `Studio::cad_tool_before_command` from `Studio::execute`;
//! - `Studio::cad_tool_overlay` / `overlay::paint` from the viewport canvas;
//! - `Studio::cad_tool_hud` / `adjust_panel` from the reserved UI slots;
//! - `Studio::cad_tool_hints` / `cad_tool_summary` from the status bar.
mod adjust;
pub(crate) mod demo;
mod edit;
pub(crate) mod geometry;
mod hud;
pub(crate) mod overlay;
mod push_pull;
mod sketch_tool;

pub(crate) use adjust::{AdjustParam, LastOperation};
pub(crate) use hud::operation_icon as hud_operation_icon;
pub(crate) use edit::{ChamferTool, EditState};
pub(crate) use push_pull::PushPullTool;
pub(crate) use sketch_tool::{Operation, Regions, SketchTool, SketchToolKind};

use crate::app::{Command, EditMode, Studio};
use forma_core::{Ray, Scene};
use geometry::Prism;
use glam::{Vec2, Vec3};
use gpui::{Context, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Window};
use std::{cell::RefCell, rc::Rc};

/// Pointer travel that turns a press into a drag.
pub(crate) const DRAG_PIXELS: f32 = 4.0;
/// Screen length of a tool's arrow handle.
pub(crate) const HANDLE_PIXELS: f32 = 56.0;
/// Pixels within which the pointer grabs a handle or picks an edge.
pub(crate) const PICK_PIXELS: f32 = 9.0;

/// Commands the tools expose to menus, the palette, the HUD and the panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToolCommand {
    /// Extrude regions of the selected sketch (or the one under the pointer).
    Extrude,
    /// Revolve regions of the selected sketch around a line or axis.
    Revolve,
    /// Push or pull faces of parts and meshes.
    PushPull,
    /// Chamfer the selected straight B-rep edges.
    Chamfer,
    /// Start a sketch on the selected part face.
    SketchOnFace,
    /// Commit the active tool.
    Done,
    /// Cancel the active tool without changing the scene.
    Cancel,
    SetOperation(Operation),
    ToggleSymmetric,
    Flip,
    /// Revolve around the sketch's x (false) or y (true) axis.
    SketchAxis(bool),
    /// Amend the last operation's boolean.
    AdjustOperation(Operation),
    /// Amend the last extrusion's symmetric extent.
    AdjustSymmetric,
    /// Reverse the last operation's direction.
    AdjustFlip,
}

/// The active tool, if any. At most one runs at a time.
pub(crate) enum Tool {
    Sketch(Box<SketchTool>),
    PushPull(Box<PushPullTool>),
    Chamfer(Box<ChamferTool>),
}

/// A scene edit in progress that previews through the kernel before it is
/// committed: the scene before the tool started, and the candidate applied
/// to the live scene (if any).
pub(crate) struct Session {
    pub before: Scene,
    pub preview: Option<Preview>,
}

impl Session {
    pub(crate) fn new(scene: &Scene) -> Self {
        Self {
            before: scene.clone(),
            preview: None,
        }
    }
}

/// The candidate feature currently applied to the live scene.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Preview {
    /// Parameters the preview was built from; unchanged parameters reuse it.
    pub key: String,
    pub object: u64,
    pub part: u64,
    pub feature: u64,
}

/// A committed operation whose kernel result has not arrived yet: its ghost
/// keeps showing so the geometry never visibly snaps back while waiting.
#[derive(Clone, Debug)]
pub(crate) struct PendingGhost {
    pub part: u64,
    pub stamp: u64,
    pub prism: Option<(Rc<Prism>, f32, f32)>,
    pub quads: Vec<[Vec3; 4]>,
    pub color: u32,
}

/// A visible sketch region under the pointer in Object mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RegionHover {
    pub object: u64,
    pub profile: usize,
}

type Cached<K, T> = RefCell<Option<(K, Rc<T>)>>;

#[derive(Default)]
pub(crate) struct CadToolState {
    pub tool: Option<Tool>,
    /// Sketch region under the pointer while no tool runs.
    pub region_hover: Option<RegionHover>,
    pub edit: EditState,
    pub last: Option<LastOperation>,
    pub pending: Vec<PendingGhost>,
    /// Bumped by every change that should cancel a scheduled kernel preview.
    pub preview_token: u64,
    /// Regions of every visible sketch, per scene and sketch revision.
    regions: Cached<(u64, u64), Vec<Rc<Regions>>>,
    /// B-rep edges and face data of part tessellations, per scene revision.
    pub(crate) topology: RefCell<Option<(u64, Vec<(u64, Rc<edit::Topology>)>)>>,
    /// Pointer position of the last hover update, to skip redundant picks.
    last_hover: Option<Vec2>,
}

impl CadToolState {
    pub(crate) fn active(&self) -> bool {
        self.tool.is_some()
    }
}

/// Fill colour of a ghost for an operation: neutral for a new body, accent
/// for join, danger for cut, warning for intersect.
pub(crate) fn operation_color(t: &crate::theme::Colors, operation: Operation) -> u32 {
    match operation {
        Operation::NewPart => t.text,
        Operation::Join => t.accent,
        Operation::Cut => t.danger,
        Operation::Intersect => t.warning,
    }
}

impl Studio {
    /// Whether a modelling tool is running.
    pub(crate) fn cad_tool_active(&self) -> bool {
        self.cad_tool.active()
    }

    /// The part being edited as B-rep components: component mode with a part
    /// as the active object.
    pub(crate) fn part_editing(&self) -> Option<u64> {
        if !self.edit_mode.is_component() {
            return None;
        }
        self.selected
            .filter(|id| self.scene.object(*id).is_some_and(|o| o.is_part()))
    }

    pub(crate) fn execute_cad_tool(
        &mut self,
        command: ToolCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            ToolCommand::Extrude | ToolCommand::Revolve => {
                let kind = if command == ToolCommand::Extrude {
                    SketchToolKind::Extrude
                } else {
                    SketchToolKind::Revolve
                };
                if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() {
                    tool.kind = kind;
                    self.sketch_tool_changed(true, cx);
                } else {
                    self.start_sketch_tool(kind, None, cx);
                }
            }
            ToolCommand::PushPull => {
                if self.part_editing().is_some() && !self.cad_tool.edit.faces.is_empty() {
                    self.push_pull_selected_faces(cx);
                } else if matches!(self.cad_tool.tool, Some(Tool::PushPull(_))) {
                    self.cancel_cad_tool(cx);
                } else {
                    self.start_push_pull(cx);
                }
            }
            ToolCommand::Chamfer => self.start_chamfer(cx),
            ToolCommand::SketchOnFace => self.sketch_on_selected_face(window, cx),
            ToolCommand::Done => self.commit_cad_tool(cx),
            ToolCommand::Cancel => self.cancel_cad_tool(cx),
            ToolCommand::SetOperation(operation) => {
                if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() {
                    if tool.target.is_none() && operation != Operation::NewPart {
                        self.status =
                            "Join, Cut and Intersect need a part · Sketch on a part's face".into();
                    } else {
                        tool.manual_operation = Some(operation);
                        self.sketch_tool_changed(true, cx);
                    }
                }
            }
            ToolCommand::ToggleSymmetric => {
                if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() {
                    tool.symmetric = !tool.symmetric;
                    self.sketch_tool_changed(true, cx);
                }
            }
            ToolCommand::Flip => self.flip_tool(cx),
            ToolCommand::SketchAxis(y) => {
                if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() {
                    tool.set_sketch_axis(y);
                    tool.kind = SketchToolKind::Revolve;
                    self.sketch_tool_changed(true, cx);
                }
            }
            ToolCommand::AdjustOperation(operation) => self.adjust_operation(operation, cx),
            ToolCommand::AdjustSymmetric => self.adjust_symmetric(cx),
            ToolCommand::AdjustFlip => self.adjust_flip(cx),
        }
        cx.notify();
    }

    fn flip_tool(&mut self, cx: &mut Context<Self>) {
        match self.cad_tool.tool.as_mut() {
            Some(Tool::Sketch(tool)) => {
                if tool.kind == SketchToolKind::Revolve {
                    tool.angle = -tool.angle;
                } else {
                    tool.distance = -tool.distance;
                }
                tool.numeric.clear();
                self.sketch_tool_changed(true, cx);
            }
            Some(Tool::PushPull(tool)) => {
                if let Some(active) = tool.active.as_mut() {
                    active.distance = -active.distance;
                    active.numeric.clear();
                }
            }
            _ => {}
        }
    }

    /// Commits the active tool as one undo step.
    pub(crate) fn commit_cad_tool(&mut self, cx: &mut Context<Self>) {
        match self.cad_tool.tool.as_ref() {
            Some(Tool::Sketch(_)) => self.commit_sketch_tool(cx),
            Some(Tool::PushPull(_)) => self.commit_push_pull(cx),
            Some(Tool::Chamfer(_)) => self.commit_chamfer(cx),
            None => {}
        }
    }

    /// Leaves the active tool, restoring the scene it started from.
    pub(crate) fn cancel_cad_tool(&mut self, cx: &mut Context<Self>) {
        let Some(tool) = self.cad_tool.tool.take() else {
            return;
        };
        self.cad_tool.preview_token += 1;
        // Push/Pull and Chamfer preview with ghosts only; Extrude and
        // Revolve may have applied a kernel preview to the live scene.
        if let Tool::Sketch(tool) = tool
            && tool.session.preview.is_some()
        {
            self.scene = tool.session.before;
            self.invalidate(true, cx);
        }
        self.status = "Cancelled".into();
        cx.notify();
    }

    /// Prepares the tools for a workspace command. Returns true when the
    /// command was fully handled here.
    pub(crate) fn cad_tool_before_command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let view_only = matches!(
            command,
            Command::ViewFront
                | Command::ViewBack
                | Command::ViewRight
                | Command::ViewLeft
                | Command::ViewTop
                | Command::ViewBottom
                | Command::ViewPerspective
                | Command::ToggleProjection
                | Command::FrameAll
                | Command::FrameSelected
                | Command::SetMode(_)
                | Command::ToggleGrid
                | Command::ToggleMeasurements
                | Command::TogglePalette
                | Command::ToggleHelp
                | Command::ToggleTheme
                | Command::TogglePreviewSettings
                | Command::SetPreviewStudio(_)
                | Command::TogglePreviewWorld
                | Command::TogglePreviewAo
                | Command::ResetPreview
                | Command::SetTool(_)
        );
        if let Command::CadTool(tool_command) = command {
            if !matches!(
                tool_command,
                ToolCommand::AdjustOperation(_)
                    | ToolCommand::AdjustSymmetric
                    | ToolCommand::AdjustFlip
            ) {
                self.cad_tool.last = None;
            }
            self.execute_cad_tool(tool_command, window, cx);
            return true;
        }
        // The previous interactive "Extrude sketch" (always 1 m) now opens
        // the Extrude tool.
        if command == Command::Cad(crate::cad::CadCommand::ExtrudeSketch) {
            self.cad_tool.last = None;
            if self.cad_tool.active() {
                self.cancel_cad_tool(cx);
            }
            self.start_sketch_tool(SketchToolKind::Extrude, None, cx);
            return true;
        }
        if !view_only {
            self.cad_tool.last = None;
        }
        if self.cad_tool.active() && !view_only {
            let undo = matches!(command, Command::Undo | Command::Redo);
            self.cancel_cad_tool(cx);
            if undo {
                // Undo while a tool runs cancels the tool, nothing more.
                return true;
            }
        }
        if let Some(id) = self.part_editing()
            && self.part_edit_command(id, command, cx)
        {
            return true;
        }
        if matches!(command, Command::ToggleEdit | Command::SetEditMode(_))
            && self.edit_mode == EditMode::Object
            && let Some(id) = self.selected
            && self.scene.object(id).is_some_and(|o| o.is_part())
        {
            let mode = match command {
                Command::SetEditMode(EditMode::Object) => return false,
                Command::SetEditMode(EditMode::Edge) => EditMode::Edge,
                _ => EditMode::Face,
            };
            self.enter_part_edit(id, mode, cx);
            return true;
        }
        false
    }

    /// Keyboard input for the tools and their entry keys. Returns true when
    /// the key was consumed.
    pub(crate) fn cad_tool_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_drawing() || self.transform_drag.is_some() {
            return false;
        }
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        if self.cad_tool.active() {
            return self.tool_key(event, window, cx);
        }
        if crate::command_modifier(mods) {
            if key == "b" && self.part_editing().is_some() {
                if !event.is_held {
                    self.cad_tool.last = None;
                    self.start_chamfer(cx);
                }
                return true;
            }
            return false;
        }
        if mods.control || mods.platform || mods.alt || event.is_held {
            return false;
        }
        if let Some(id) = self.part_editing() {
            return self.part_edit_key(id, key, mods.shift, window, cx);
        }
        if self.edit_mode != EditMode::Object {
            return false;
        }
        match key {
            "e" if !mods.shift && self.sketch_context().is_some() => {
                self.cad_tool.last = None;
                self.start_sketch_tool(SketchToolKind::Extrude, None, cx);
                true
            }
            "r" if mods.shift && self.sketch_context().is_some() => {
                self.cad_tool.last = None;
                self.start_sketch_tool(SketchToolKind::Revolve, None, cx);
                true
            }
            "p" if !mods.shift => {
                self.cad_tool.last = None;
                self.start_push_pull(cx);
                true
            }
            _ => false,
        }
    }

    fn tool_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        // Undo, save and the palette go through `execute`, which cancels or
        // keeps the tool as appropriate.
        if mods.control || mods.platform {
            return false;
        }
        let consumed = match self.cad_tool.tool.as_ref() {
            Some(Tool::Sketch(_)) => self.sketch_tool_key(event, window, cx),
            Some(Tool::PushPull(_)) => self.push_pull_key(event, cx),
            Some(Tool::Chamfer(_)) => self.chamfer_key(event, cx),
            None => false,
        };
        if consumed {
            cx.notify();
            return true;
        }
        // View and shading keys keep working while a tool runs.
        if matches!(
            key,
            "home" | "5" | "0" | "z" | "x" | "c" | "v" | "h" | "?" | "space" | "`" | "."
        ) {
            return false;
        }
        // A persistent Push/Pull lets other keys act normally (they end it
        // through `execute` when they change the scene).
        if matches!(self.cad_tool.tool, Some(Tool::PushPull(ref tool)) if tool.active.is_none()) {
            return false;
        }
        self.consume_unused_key(key)
    }

    /// Keys a running tool does not use: mutating shortcuts would act on a
    /// half-finished edit, so they are swallowed with a hint.
    fn consume_unused_key(&mut self, key: &str) -> bool {
        if matches!(key, "shift" | "control" | "alt" | "platform" | "function") {
            return false;
        }
        self.status = "Finish the tool first · Return to apply · Esc to cancel".into();
        true
    }

    /// The sketch an Extrude or Revolve would use now: the region under the
    /// pointer, else the selected sketch.
    pub(crate) fn sketch_context(&self) -> Option<(u64, Option<usize>)> {
        if let Some(hit) = self.sketch_region_at(self.last_mouse) {
            return Some((hit.object, Some(hit.profile)));
        }
        self.selected
            .into_iter()
            .chain(self.selected_ids())
            .find(|id| self.scene.object_sketch(*id).is_some())
            .map(|id| (id, None))
    }

    /// Regions of every visible sketch object, cached per scene and sketch revision.
    pub(crate) fn region_sets(&self) -> Rc<Vec<Rc<Regions>>> {
        let key = (self.scene_revision, self.sketch.revision);
        let mut cache = self.cad_tool.regions.borrow_mut();
        if let Some((cached, sets)) = cache.as_ref()
            && *cached == key
        {
            return sets.clone();
        }
        let previous = cache.take().map(|(_, sets)| sets);
        let mut sets = Vec::new();
        for object in &self.scene.objects {
            let forma_core::ObjectData::Sketch { sketch } = &object.data else {
                continue;
            };
            if !self.scene.is_effectively_visible(object.id) || !object.selectable {
                continue;
            }
            let Some(matrix) = self.scene.world_transform(object.id) else {
                continue;
            };
            if let Some(reused) = previous.as_ref().and_then(|p| {
                p.iter()
                    .find(|r| r.object == object.id && r.matrix == matrix && r.sketch == *sketch)
            }) {
                sets.push(reused.clone());
                continue;
            }
            if let Some(regions) = Regions::build(object.id, &object.name, sketch, matrix) {
                sets.push(Rc::new(regions));
            }
        }
        let sets = Rc::new(sets);
        *cache = Some((key, sets.clone()));
        sets
    }

    /// The visible sketch region under a window position, nearest first.
    pub(crate) fn sketch_region_at(&self, screen: Vec2) -> Option<RegionHover> {
        let ray = self.pointer_ray(screen)?;
        let sets = self.region_sets();
        let mut best: Option<(f32, RegionHover, Vec3)> = None;
        for regions in sets.iter() {
            let Some((point, world, distance)) = regions.ray_point(ray) else {
                continue;
            };
            let Some(profile) = regions.region_at(point) else {
                continue;
            };
            if best.as_ref().is_none_or(|(d, _, _)| distance < *d) {
                best = Some((
                    distance,
                    RegionHover {
                        object: regions.object,
                        profile,
                    },
                    world,
                ));
            }
        }
        let (_, hit, world) = best?;
        crate::viewport::visible_in_query(self.scene.camera, self.visibility_query(), world)
            .then_some(hit)
    }

    /// Viewport presses for the tools and part editing. Returns true when
    /// consumed (navigation presses fall through).
    pub(crate) fn cad_tool_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_drawing() || self.transform_drag.is_some() || self.box_select.is_some() {
            return false;
        }
        let position = crate::viewport::mouse_point(event.position);
        if event.button == MouseButton::Right {
            // Right presses orbit, except during a modal drag where they cancel.
            return match self.cad_tool.tool.as_ref() {
                Some(Tool::Chamfer(_)) => {
                    self.cancel_cad_tool(cx);
                    true
                }
                Some(Tool::PushPull(tool)) if tool.active.as_ref().is_some_and(|a| a.modal) => {
                    self.cancel_cad_tool(cx);
                    true
                }
                Some(Tool::Sketch(tool)) if tool.drag.as_ref().is_some_and(|d| d.started()) => {
                    self.cancel_sketch_drag(cx);
                    true
                }
                _ => false,
            };
        }
        if event.button != MouseButton::Left || event.modifiers.alt {
            return false;
        }
        let consumed = match self.cad_tool.tool.as_ref() {
            Some(Tool::Sketch(_)) => self.sketch_tool_press(position, event, cx),
            Some(Tool::PushPull(_)) => self.push_pull_press(position, event, cx),
            Some(Tool::Chamfer(_)) => {
                self.commit_chamfer(cx);
                true
            }
            None => {
                if let Some(id) = self.part_editing() {
                    self.part_edit_press(id, position, event, window, cx)
                } else {
                    // A selection click starts a new action.
                    self.cad_tool.last = None;
                    false
                }
            }
        };
        if consumed {
            cx.notify();
        }
        consumed
    }

    /// Pointer motion for the tools, part editing and region hover. Returns
    /// true when a tool drag consumed it.
    pub(crate) fn cad_tool_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_drawing()
            || self.transform_drag.is_some()
            || self.box_select.is_some()
            || self.navigation.is_some()
            || matches!(
                event.pressed_button,
                Some(MouseButton::Middle) | Some(MouseButton::Right)
            )
        {
            return false;
        }
        let position = crate::viewport::mouse_point(event.position);
        let free = event.modifiers.control || event.modifiers.platform;
        let held = event.pressed_button == Some(MouseButton::Left);
        let consumed = match self.cad_tool.tool.as_ref() {
            Some(Tool::Sketch(_)) => self.sketch_tool_move(position, held, free, cx),
            Some(Tool::PushPull(_)) => self.push_pull_move(position, held, free, cx),
            Some(Tool::Chamfer(_)) => {
                self.chamfer_move(position, free, cx);
                true
            }
            None => {
                if self.cad_tool.last_hover == Some(position) {
                    return false;
                }
                self.cad_tool.last_hover = Some(position);
                if let Some(id) = self.part_editing() {
                    self.part_edit_hover(id, position, cx);
                } else if self.edit_mode == EditMode::Object && !held {
                    let hover = self.sketch_region_at(position);
                    if hover != self.cad_tool.region_hover {
                        self.cad_tool.region_hover = hover;
                        cx.notify();
                    }
                }
                false
            }
        };
        if consumed {
            cx.notify();
        }
        consumed
    }

    /// Left button release anywhere in the window.
    pub(crate) fn cad_tool_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if event.button != MouseButton::Left {
            return;
        }
        let position = crate::viewport::mouse_point(event.position);
        match self.cad_tool.tool.as_ref() {
            Some(Tool::Sketch(_)) => self.sketch_tool_release(position, cx),
            Some(Tool::PushPull(_)) => self.push_pull_release(position, cx),
            _ => {}
        }
    }

    /// The pointer left the viewport for a tool's HUD: hovers step aside.
    pub(crate) fn cad_tool_pointer_in_hud(&mut self, cx: &mut Context<Self>) {
        let mut changed = self.cad_tool.region_hover.take().is_some();
        match self.cad_tool.tool.as_mut() {
            Some(Tool::Sketch(tool)) => {
                changed |= tool.hover.take().is_some() | tool.hover_line.take().is_some();
                changed |= std::mem::take(&mut tool.handle_hot);
            }
            Some(Tool::PushPull(tool)) => changed |= tool.hover.take().is_some(),
            _ => {}
        }
        if changed {
            cx.notify();
        }
    }

    /// Applies the session's candidate to the live scene after a short pause,
    /// so the kernel runs once typing or toggling settles.
    pub(crate) fn schedule_kernel_preview(&mut self, cx: &mut Context<Self>) {
        self.cad_tool.preview_token += 1;
        let token = self.cad_tool.preview_token;
        cx.spawn(async move |this, cx| {
            gpui::Timer::after(std::time::Duration::from_millis(220)).await;
            let _ = this.update(cx, |s, cx| {
                if s.cad_tool.preview_token == token {
                    s.apply_kernel_preview(cx);
                }
            });
        })
        .detach();
    }

    /// Applies the active session's candidate now.
    pub(crate) fn apply_kernel_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(Tool::Sketch(_)) = self.cad_tool.tool.as_ref() {
            self.apply_sketch_preview(cx);
        }
    }

    /// Removes pending ghosts whose part has been regenerated or changed.
    pub(crate) fn prune_pending_ghosts(&mut self) {
        let scene = &self.scene;
        self.cad_tool.pending.retain(|ghost| {
            scene
                .part(ghost.part)
                .is_some_and(|part| part.stamp == ghost.stamp && !part.is_evaluated())
        });
    }

    /// Screen position of a world point.
    pub(crate) fn world_to_screen(&self, world: Vec3) -> Option<Vec2> {
        crate::viewport::project(world, self.view_projection(), self.bounds.get())
            .map(crate::viewport::mouse_point)
    }

    /// World units per screen pixel at `world` (for snapping and fallbacks).
    pub(crate) fn world_per_pixel(&self, world: Vec3) -> f32 {
        let camera = self.scene.camera;
        let height = f32::from(self.bounds.get().size.height).max(1.0);
        let depth = if camera.orthographic {
            camera.distance
        } else {
            (-camera.view_matrix().transform_point3(world).z).max(1.0e-4)
        };
        depth * (camera.fov_y * 0.5).tan() * 2.0 / height
    }

    /// The pointer ray at a window position.
    pub(crate) fn tool_ray(&self, screen: Vec2) -> Option<Ray> {
        self.pointer_ray(screen)
    }
}

/// A drag along a world axis, from a press on a handle, region or face.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AxisDrag {
    pub origin: Vec3,
    /// Unit direction in world space.
    pub direction: Vec3,
    /// Axis parameter under the pointer at the press, minus the value then.
    pub grab: f32,
    pub press: Vec2,
    pub started: bool,
    /// Value when the drag started, restored by a right-click cancel.
    pub start_value: f32,
    /// Screen direction and world units per pixel, for axes seen end on.
    pub fallback: Option<(Vec2, f32)>,
}

impl AxisDrag {
    /// Starts a drag of `value` along the axis from a press at `screen`.
    pub(crate) fn new(studio: &Studio, origin: Vec3, direction: Vec3, value: f32, screen: Vec2) -> Self {
        let direction = direction.normalize_or_zero();
        let at_press = studio
            .tool_ray(screen)
            .and_then(|ray| geometry::axis_parameter(origin, direction, ray));
        let fallback = match at_press {
            Some(_) => None,
            None => {
                // Looking along the axis: up the screen means toward the viewer.
                let toward = studio
                    .world_to_screen(origin)
                    .zip(studio.world_to_screen(origin + direction * 1.0e-3))
                    .map(|(a, b)| (b - a).normalize_or_zero())
                    .filter(|d| *d != Vec2::ZERO)
                    .unwrap_or(Vec2::NEG_Y);
                Some((toward, studio.world_per_pixel(origin)))
            }
        };
        Self {
            origin,
            direction,
            grab: at_press.unwrap_or(0.0) - value,
            press: screen,
            started: false,
            start_value: value,
            fallback,
        }
    }

    /// The dragged value at pointer position `screen`.
    pub(crate) fn value(&self, studio: &Studio, screen: Vec2) -> Option<f32> {
        if let Some((toward, per_pixel)) = self.fallback {
            return Some(self.start_value + (screen - self.press).dot(toward) * per_pixel);
        }
        let ray = studio.tool_ray(screen)?;
        geometry::axis_parameter(self.origin, self.direction, ray).map(|t| t - self.grab)
    }
}

/// Distance of `p` to the segment `a`–`b`.
pub(crate) fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let d = b - a;
    let t = ((p - a).dot(d) / d.length_squared().max(1.0e-9)).clamp(0.0, 1.0);
    p.distance(a + d * t)
}

/// Appends typed characters to a tool's numeric entry, following Draw's
/// rule: a value must start with a digit, sign, point or parenthesis; unit
/// letters only continue one. Returns true when the text changed.
pub(crate) fn type_numeric(text: &mut String, character: Option<char>) -> bool {
    let Some(c) = character else {
        return false;
    };
    let accepted = if text.is_empty() {
        c.is_ascii_digit() || ".-(".contains(c)
    } else {
        crate::transform::numeric_input_char(c)
    };
    if accepted && text.len() < 128 {
        text.push(c);
        return true;
    }
    false
}

/// Erases from a tool's numeric entry. Returns true when it changed.
pub(crate) fn erase_numeric(text: &mut String, key: &str, alt: bool) -> bool {
    if text.is_empty() {
        return false;
    }
    let erase = if key == "delete" {
        crate::text_entry::Erase::Delete
    } else if alt {
        crate::text_entry::Erase::PreviousWord
    } else {
        crate::text_entry::Erase::Backspace
    };
    let mut replace = false;
    crate::text_entry::erase(text, &mut replace, erase)
}

/// The typed magnitude applied on the current side, as Draw's locked values
/// follow the pointer: `0.2` while pushing in gives −0.2; a typed sign
/// reverses it.
pub(crate) fn typed_on_side(typed: f32, current: f32) -> f32 {
    if current < 0.0 { -typed } else { typed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_entry_starts_with_numbers_and_continues_with_units() {
        let mut text = String::new();
        assert!(!type_numeric(&mut text, Some('m')));
        assert!(type_numeric(&mut text, Some('2')));
        assert!(type_numeric(&mut text, Some('5')));
        assert!(type_numeric(&mut text, Some('m')));
        assert!(type_numeric(&mut text, Some('m')));
        assert_eq!(text, "25mm");
        assert!((crate::transform::distance_value(&text).unwrap() - 0.025).abs() < 1.0e-6);
        assert!(erase_numeric(&mut text, "backspace", false));
        assert_eq!(text, "25m");
        assert_eq!(typed_on_side(0.2, -0.5), -0.2);
        assert_eq!(typed_on_side(-0.2, -0.5), 0.2);
        assert_eq!(typed_on_side(0.2, 0.0), 0.2);
    }

    #[test]
    fn segment_distances_clamp_to_the_ends() {
        let (a, b) = (Vec2::ZERO, Vec2::new(10.0, 0.0));
        assert_eq!(segment_distance(Vec2::new(5.0, 3.0), a, b), 3.0);
        assert_eq!(segment_distance(Vec2::new(-4.0, 3.0), a, b), 5.0);
    }
}

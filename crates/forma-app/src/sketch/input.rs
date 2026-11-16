//! Draw mode input: keyboard, pointer and command handling.
use super::analysis;
use super::overlay::{ActiveOverlay, AngleArc, DimLabel, format_angle, format_length};
use super::tools::{self, Step};
use super::{DragTarget, DrawCommand, DrawTarget, DrawTool, Hover};
use crate::app::{Command, EditMode, Studio};
use crate::viewport::mouse_point;
use forma_core::{SKETCH_TOLERANCE, Sketch, SketchCurve, SketchEntity, Vec2};
use gpui::{
    Context, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Window,
};
use std::{collections::BTreeSet, rc::Rc};

/// Pixels within which the pointer picks a curve.
const PICK_PIXELS: f32 = 7.0;
/// Pointer travel that turns a press into a drag.
const DRAG_PIXELS: f32 = 4.0;

fn suppress_snapping(modifiers: gpui::Modifiers) -> bool {
    modifiers.control || modifiers.platform
}

impl Studio {
    fn session_mut(&mut self) -> Option<&mut super::DrawSession> {
        self.sketch.session.as_mut()
    }

    /// Clears selection and hover that refer to a previous version of the
    /// sketch after undo, redo or loading.
    fn sync_draw_session(&mut self) {
        let revision = self.scene_revision;
        let count = self.session_entities_len();
        if let Some(session) = self.sketch.session.as_mut() {
            if session.selection_revision != revision {
                session.selection_revision = revision;
                session.selection.clear();
                session.hover = Hover::default();
                session.drag = None;
            }
            session.selection.retain(|&i| i < count);
        }
    }

    fn session_entities_len(&self) -> usize {
        self.session_object()
            .and_then(|id| self.scene.object_sketch(id))
            .map_or(0, |sketch| sketch.entities.len())
    }

    fn sketch_entities(&self) -> Vec<SketchEntity> {
        self.session_object()
            .and_then(|id| self.scene.object_sketch(id))
            .map(|sketch| sketch.entities.clone())
            .unwrap_or_default()
    }

    /// Re-evaluates the pointer, hover and inference memory.
    fn update_draw_pointer(&mut self, screen: Vec2, suppress: bool) {
        self.sync_draw_session();
        let pointer = self.evaluate_pointer(screen, suppress);
        let analysis = self.sketch_analysis();
        let Some(session) = self.sketch.session.as_ref() else {
            return;
        };
        let tool = session.tool;
        let step_active = session.step.is_active();
        let mut hover = Hover::default();
        let mut memory = session.memory.clone();
        if let (Some(pointer), Some(analysis)) = (pointer.as_ref(), analysis.as_ref()) {
            let radius = PICK_PIXELS / pointer.pixels_per_unit;
            hover.entity = analysis.entity_at(pointer.raw, radius);
            if hover.entity.is_none() && !step_active && tool != DrawTool::Trim {
                hover.profile = analysis.profile_at(pointer.raw);
            }
            if tool == DrawTool::Trim
                && let Some(entity) = hover.entity
            {
                let sketch = Sketch {
                    entities: analysis.entities.clone(),
                    ..Default::default()
                };
                hover.trim = sketch.trim_piece(entity, pointer.raw, SKETCH_TOLERANCE);
            }
            if pointer.snap.kind.is_point() {
                memory.track(pointer.snap.point);
            }
            // Remember the last straight curve hovered for parallel and
            // perpendicular inference.
            let hovered_line = pointer.snap.entity.or(hover.entity).and_then(|index| {
                analysis
                    .targets
                    .curves
                    .iter()
                    .find(|(i, _)| *i == index)
                    .map(|(_, curve)| *curve)
            });
            if let Some(SketchCurve::Line { start, end }) = hovered_line
                && session.step.anchor().is_none_or(|anchor| {
                    !tools::same_point(anchor, start) && !tools::same_point(anchor, end)
                })
            {
                memory.reference = Some(end - start);
            }
        }
        let session = self.sketch.session.as_mut().unwrap();
        session.pointer = pointer;
        session.hover = hover;
        session.memory = memory;
    }

    pub(crate) fn execute_draw(&mut self, command: DrawCommand, cx: &mut Context<Self>) {
        match command {
            DrawCommand::NewSketch => self.enter_draw(DrawTarget::AtPointer, cx),
            DrawCommand::NewGroundSketch => self.enter_draw(
                DrawTarget::Plane(forma_core::SketchPlane::ground(), "Ground plane".into()),
                cx,
            ),
            DrawCommand::EditSketch => {
                let target = self
                    .selected
                    .into_iter()
                    .chain(self.selected_ids())
                    .find(|id| self.scene.object_sketch(*id).is_some());
                match target {
                    Some(id) => self.enter_draw(DrawTarget::Object(id), cx),
                    None => self.status = "Select a sketch to edit".into(),
                }
            }
            DrawCommand::Finish => self.finish_draw(cx),
            DrawCommand::Tool(tool) => self.set_draw_tool(tool),
            DrawCommand::ToggleConstruction => self.toggle_construction(cx),
            DrawCommand::ToggleRectangleMode => {
                if let Some(session) = self.session_mut() {
                    session.options.rect_center = !session.options.rect_center;
                    if session.tool != DrawTool::Rectangle {
                        session.tool = DrawTool::Rectangle;
                        session.step = Step::Idle;
                        session.numeric.clear();
                    }
                }
            }
            DrawCommand::PolygonSides(delta) => {
                if let Some(session) = self.session_mut() {
                    let sides = session.options.polygon_sides as i32 + delta;
                    session.options.polygon_sides = sides.clamp(3, 64) as u32;
                    self.status = format!(
                        "Polygon · {} sides",
                        self.sketch.session.as_ref().unwrap().options.polygon_sides
                    );
                }
            }
        }
        cx.notify();
    }

    fn set_draw_tool(&mut self, tool: DrawTool) {
        let Some(session) = self.session_mut() else {
            return;
        };
        if session.tool != tool {
            session.step = Step::Idle;
            session.numeric.clear();
        }
        session.tool = tool;
        session.hover = Hover::default();
        session.drag = None;
        session.marquee = None;
        if tool != DrawTool::Select {
            session.selection.clear();
        }
        self.status = self.draw_hint();
    }

    /// One line of guidance for the current tool and step, short enough
    /// for the status bar: the toolbar already names the tool and the key
    /// hints list the rest.
    pub(crate) fn draw_hint(&self) -> String {
        let Some(session) = self.sketch.session.as_ref() else {
            return String::new();
        };
        let center = session.options.rect_center;
        match (session.tool, session.step) {
            (DrawTool::Select, _) => {
                if session.selection.is_empty() {
                    "Click or drag a box to select · Drag to move".into()
                } else {
                    format!(
                        "{} selected · Drag to move · Delete removes",
                        session.selection.len()
                    )
                }
            }
            (DrawTool::Trim, _) => "Click a piece of a curve to remove it".into(),
            (DrawTool::Line, Step::Line { arc: true, .. }) => {
                "Click the arc's end · type a radius · L for lines".into()
            }
            (DrawTool::Line, Step::Line { .. }) => {
                "Click next point · type a length · Tab for angle".into()
            }
            (DrawTool::Line, _) => "Click to start a line".into(),
            (DrawTool::Rectangle, Step::Rectangle { .. }) => {
                "Click the opposite corner · type a width".into()
            }
            (DrawTool::Rectangle, _) if center => "Click the centre · R for corners".into(),
            (DrawTool::Rectangle, _) => "Click the first corner · R for centre".into(),
            (DrawTool::Circle, Step::Circle { .. }) => {
                "Click to set the radius · or type it".into()
            }
            (DrawTool::Circle, _) => "Click the centre".into(),
            (DrawTool::Arc, Step::Arc { end: None, .. }) => "Click the end point".into(),
            (DrawTool::Arc, Step::Arc { .. }) => "Click a point on the arc · type a radius".into(),
            (DrawTool::Arc, _) => "Click the start point".into(),
            (DrawTool::Polygon, Step::Polygon { .. }) => {
                "Click a corner · type a radius · + − sides".into()
            }
            (DrawTool::Polygon, _) => "Click the centre · + − sides".into(),
        }
    }

    fn toggle_construction(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.sketch.session.as_ref() else {
            return;
        };
        let selection = session.selection.clone();
        if selection.is_empty() {
            let session = self.session_mut().unwrap();
            session.construction = !session.construction;
            self.status = if session.construction {
                "Construction on · New curves guide drawing and never form profiles"
            } else {
                "Construction off"
            }
            .into();
            return;
        }
        let make = !selection.iter().all(|&i| {
            self.sketch_entities()
                .get(i)
                .is_some_and(|e| e.construction)
        });
        self.edit_active_sketch(
            "Construction",
            |sketch| {
                for &i in &selection {
                    if let Some(entity) = sketch.entities.get_mut(i) {
                        entity.construction = make;
                    }
                }
                true
            },
            cx,
        );
        self.status = format!(
            "{} curve{} {} construction",
            selection.len(),
            if selection.len() == 1 { "" } else { "s" },
            if make { "set to" } else { "removed from" }
        );
    }

    /// Prepares Draw mode for a workspace command. Returns true when the
    /// command was handled entirely by Draw mode.
    pub(crate) fn draw_before_command(&mut self, command: Command, cx: &mut Context<Self>) -> bool {
        if let Command::Draw(command) = command {
            self.execute_draw(command, cx);
            return true;
        }
        if !self.is_drawing() {
            return false;
        }
        match command {
            Command::Undo | Command::Redo => {
                self.cancel_draw_step();
                if let Some(session) = self.session_mut() {
                    session.selection.clear();
                }
                false
            }
            Command::Delete => {
                self.delete_selected_curves(cx);
                true
            }
            Command::SelectAll => {
                self.select_all_curves();
                true
            }
            Command::DeselectAll => {
                if let Some(session) = self.session_mut() {
                    session.selection.clear();
                }
                true
            }
            Command::ToggleVisible(_)
            | Command::Save
            | Command::SaveAs
            | Command::ExportImage
            | Command::ExportObj
            | Command::ExportStl
            | Command::ViewFront
            | Command::ViewBack
            | Command::ViewRight
            | Command::ViewLeft
            | Command::ViewTop
            | Command::ViewBottom
            | Command::ViewPerspective
            | Command::ToggleProjection
            | Command::FrameAll
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
            | Command::LoadPreviewHdri
            | Command::ResetPreview
            | Command::ToggleViewportDenoise
            | Command::ToggleRenderDenoise
            | Command::SetDenoiseQuality(_) => false,
            Command::FrameSelected => {
                self.frame_sketch(cx);
                true
            }
            _ => {
                self.finish_draw(cx);
                false
            }
        }
    }

    fn cancel_draw_step(&mut self) {
        let drag_before = self
            .sketch
            .session
            .as_ref()
            .and_then(|s| s.drag.as_ref())
            .filter(|d| d.started)
            .map(|d| d.before.clone());
        if let Some(before) = drag_before {
            self.set_sketch_entities_live(before);
        }
        if let Some(session) = self.session_mut() {
            session.step = Step::Idle;
            session.numeric.clear();
            session.drag = None;
            session.marquee = None;
            session.press = None;
        }
    }

    fn select_all_curves(&mut self) {
        let count = self.session_entities_len();
        if let Some(session) = self.session_mut() {
            session.tool = DrawTool::Select;
            session.step = Step::Idle;
            session.selection = (0..count).collect();
        }
        self.status = format!("{count} curves selected");
    }

    fn delete_selected_curves(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self
            .sketch
            .session
            .as_ref()
            .map(|s| s.selection.clone())
            .filter(|s| !s.is_empty())
        else {
            self.status = "Select curves to delete".into();
            return;
        };
        let deleted = self.edit_active_sketch(
            "Delete",
            |sketch| {
                for &i in selection.iter().rev() {
                    if i < sketch.entities.len() {
                        sketch.entities.remove(i);
                    }
                }
                true
            },
            cx,
        );
        if deleted {
            if let Some(session) = self.session_mut() {
                session.selection.clear();
                session.hover = Hover::default();
            }
            self.status = format!(
                "{} curve{} deleted",
                selection.len(),
                if selection.len() == 1 { "" } else { "s" }
            );
        }
    }

    /// Fits the sketch (or its reference face) in the view.
    fn frame_sketch(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self.sketch_frame() else {
            return;
        };
        let session = self.sketch.session.as_ref().unwrap();
        let mut points: Vec<Vec2> = Vec::new();
        for entity in self.sketch_entities() {
            let (a, b) = entity.curve.bounds();
            points.extend([a, b]);
        }
        if points.is_empty() {
            for curve in &session.reference {
                let (a, b) = curve.bounds();
                points.extend([a, b]);
            }
        }
        let Some((min, max)) = points
            .iter()
            .map(|p| (*p, *p))
            .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
        else {
            self.status = "Nothing to frame yet".into();
            return;
        };
        let center = frame.to_world((min + max) * 0.5);
        let radius = frame.to_world(min).distance(frame.to_world(max)) * 0.5;
        let size = self.bounds.get().size;
        let aspect = f32::from(size.width) / f32::from(size.height).max(1.0);
        self.scene
            .camera
            .frame_in_viewport(center, radius.max(0.01), aspect);
        self.invalidate(false, cx);
    }

    /// Keyboard dispatch while drawing, plus the Object mode entry keys.
    /// Returns true when the key was consumed.
    pub(crate) fn draw_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        if !self.is_drawing() {
            if mods.platform || mods.control || mods.alt || self.transform_drag.is_some() {
                return false;
            }
            return match key {
                "d" if !mods.shift => {
                    if !event.is_held {
                        self.execute(Command::Draw(DrawCommand::NewSketch), window, cx);
                    }
                    true
                }
                "enter"
                    if self.edit_mode == EditMode::Object
                        && self
                            .selected
                            .is_some_and(|id| self.scene.object_sketch(id).is_some()) =>
                {
                    self.execute(Command::Draw(DrawCommand::EditSketch), window, cx);
                    true
                }
                _ => false,
            };
        }
        self.sync_draw_session();
        if crate::command_modifier(mods) {
            return match key {
                "a" => {
                    self.select_all_curves();
                    cx.notify();
                    true
                }
                // Mesh duplicate, bevel and invert have no meaning here.
                "d" | "b" | "i" => true,
                _ => false,
            };
        }
        if mods.control || mods.platform {
            return false;
        }
        let session = self.sketch.session.as_ref().unwrap();
        let step = session.step;
        let tool = session.tool;
        let numeric_empty = session.numeric.is_empty();
        let character = event
            .keystroke
            .key_char
            .as_deref()
            .and_then(|text| text.chars().next());
        let handled = match key {
            "escape" => {
                self.draw_escape(cx);
                true
            }
            "enter" => {
                if !event.is_held {
                    self.draw_enter(cx);
                }
                true
            }
            "tab" => {
                let count = tools::dimensions(&step).len();
                let session = self.session_mut().unwrap();
                session.numeric.advance(count, mods.shift);
                true
            }
            "backspace" | "delete" => {
                if !numeric_empty {
                    let erase = if key == "delete" {
                        crate::text_entry::Erase::Delete
                    } else if mods.alt {
                        crate::text_entry::Erase::PreviousWord
                    } else {
                        crate::text_entry::Erase::Backspace
                    };
                    self.session_mut().unwrap().numeric.erase(erase);
                } else {
                    self.delete_selected_curves(cx);
                }
                true
            }
            "+" | "=" if tool == DrawTool::Polygon && numeric_empty => {
                self.execute_draw(DrawCommand::PolygonSides(1), cx);
                true
            }
            "-" if tool == DrawTool::Polygon && numeric_empty => {
                self.execute_draw(DrawCommand::PolygonSides(-1), cx);
                true
            }
            _ => {
                let typing = step.is_active()
                    && character.is_some_and(|c| {
                        if numeric_empty {
                            super::numeric::starts_number(c)
                        } else {
                            crate::transform::numeric_input_char(c)
                        }
                    });
                if typing {
                    let text = character.unwrap().to_string();
                    self.session_mut().unwrap().numeric.push(&text);
                    true
                } else if mods.alt {
                    false
                } else {
                    self.draw_letter(key, mods.shift, window, cx)
                }
            }
        };
        if handled {
            self.refresh_draw_status();
            cx.notify();
        }
        handled
    }

    fn refresh_draw_status(&mut self) {
        let hint = self.draw_hint();
        if let Some(session) = self.sketch.session.as_ref() {
            let numeric = session.numeric.active_text();
            let dimensions = tools::dimensions(&session.step);
            self.status = match dimensions.get(session.numeric.active) {
                Some(dimension) if !numeric.is_empty() => {
                    format!(
                        "{} {numeric} · Return to apply · Tab next value",
                        dimension.label()
                    )
                }
                _ => hint,
            };
        }
    }

    fn draw_letter(
        &mut self,
        key: &str,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if shift {
            return false;
        }
        let session = self.sketch.session.as_ref().unwrap();
        let (tool, step) = (session.tool, session.step);
        match key {
            "l" => {
                if let Step::Line { arc: true, .. } = step {
                    let session = self.session_mut().unwrap();
                    if let Step::Line { arc, .. } = &mut session.step {
                        *arc = false;
                    }
                    session.numeric.clear();
                } else {
                    self.set_draw_tool(DrawTool::Line);
                }
                true
            }
            "a" => {
                if let Step::Line {
                    tangent: Some(_),
                    arc,
                    ..
                } = step
                {
                    let session = self.session_mut().unwrap();
                    if let Step::Line { arc: mode, .. } = &mut session.step {
                        *mode = !arc;
                    }
                    session.numeric.clear();
                } else {
                    self.set_draw_tool(DrawTool::Arc);
                }
                true
            }
            "r" if tool == DrawTool::Rectangle => {
                self.execute_draw(DrawCommand::ToggleRectangleMode, cx);
                true
            }
            "x" => {
                self.toggle_construction(cx);
                true
            }
            "d" => {
                self.finish_draw(cx);
                true
            }
            "f" => {
                self.frame_sketch(cx);
                true
            }
            // Mesh transforms and tools do not apply to sketches.
            "g" | "s" | "e" | "i" | "b" => true,
            _ => match DrawTool::from_key(key) {
                Some(tool) => {
                    self.set_draw_tool(tool);
                    let _ = window;
                    true
                }
                None => false,
            },
        }
    }

    fn draw_escape(&mut self, cx: &mut Context<Self>) {
        let session = self.sketch.session.as_ref().unwrap();
        if session.drag.as_ref().is_some_and(|d| d.started) {
            self.finish_sketch_drag(true, cx);
            self.status = "Move cancelled".into();
        } else if session.marquee.is_some() || session.step.is_active() || session.drag.is_some() {
            let was_line = matches!(session.step, Step::Line { segments: 1.., .. });
            self.cancel_draw_step();
            self.status = if was_line {
                "Line ended".into()
            } else {
                "Step cancelled".into()
            };
        } else if !session.selection.is_empty() {
            self.session_mut().unwrap().selection.clear();
        } else {
            self.finish_draw(cx);
        }
    }

    fn draw_enter(&mut self, cx: &mut Context<Self>) {
        let session = self.sketch.session.as_ref().unwrap();
        let step = session.step;
        if !step.is_active() {
            self.finish_draw(cx);
            return;
        }
        let dimensions = tools::dimensions(&step);
        let (_, valid) = session.numeric.locks(dimensions);
        if !valid {
            self.status = format!("Invalid value · {}", session.numeric.active_text());
            return;
        }
        if session.numeric.is_empty() && matches!(step, Step::Line { .. }) {
            self.cancel_draw_step();
            self.status = "Line ended".into();
            return;
        }
        self.tool_click(1, cx);
    }

    /// The pointer position for placing points (see `tools::placement`).
    fn placement_cursor(&self) -> Option<Vec2> {
        let session = self.sketch.session.as_ref()?;
        let (locks, _) = session.numeric.locks(tools::dimensions(&session.step));
        tools::placement(
            &session.step,
            session.pointer.as_ref().map(|p| p.snap.point),
            locks,
        )
    }

    /// Places the next point of the active drawing tool.
    fn tool_click(&mut self, click_count: usize, cx: &mut Context<Self>) {
        let Some(cursor) = self.placement_cursor() else {
            self.status = "Point the cursor at the sketch plane".into();
            return;
        };
        let session = self.sketch.session.as_ref().unwrap();
        let (tool, step, options) = (session.tool, session.step, session.options);
        let dimensions = tools::dimensions(&step);
        let (locks, _) = session.numeric.locks(dimensions);
        let resolved = tools::resolve(&step, cursor, locks);
        let preview = tools::preview(&step, cursor, locks, options);
        let next = match (tool, step) {
            (DrawTool::Line, Step::Idle) => Some(Step::Line {
                start: cursor,
                chain_start: cursor,
                segments: 0,
                tangent: None,
                arc: false,
            }),
            (
                DrawTool::Line,
                Step::Line {
                    start,
                    chain_start,
                    segments,
                    arc,
                    ..
                },
            ) => {
                if click_count >= 2 || tools::same_point(start, resolved) {
                    // A double-click (or a click on the last point) ends the chain.
                    self.cancel_draw_step();
                    self.status = "Line ended".into();
                    return;
                }
                if preview.is_empty() {
                    self.status = "Too small to draw · Move further or type a larger value".into();
                    return;
                }
                let curve = preview[0];
                if !self.add_sketch_curves(if arc { "Arc" } else { "Line" }, vec![curve], cx) {
                    return;
                }
                let end = curve.end();
                if segments >= 1 && tools::same_point(end, chain_start) {
                    self.status = "Closed loop".into();
                    Some(Step::Idle)
                } else {
                    Some(Step::Line {
                        start: end,
                        chain_start,
                        segments: segments + 1,
                        tangent: Some(curve.tangent_at(1.0)),
                        arc: false,
                    })
                }
            }
            (DrawTool::Rectangle, Step::Idle) => Some(Step::Rectangle { first: cursor }),
            (DrawTool::Circle, Step::Idle) => Some(Step::Circle { center: cursor }),
            (DrawTool::Polygon, Step::Idle) => Some(Step::Polygon { center: cursor }),
            (DrawTool::Arc, Step::Idle) => Some(Step::Arc {
                start: cursor,
                end: None,
            }),
            (DrawTool::Arc, Step::Arc { start, end: None }) => {
                if tools::same_point(start, resolved) {
                    return;
                }
                Some(Step::Arc {
                    start,
                    end: Some(resolved),
                })
            }
            (DrawTool::Rectangle, Step::Rectangle { .. })
            | (DrawTool::Circle, Step::Circle { .. })
            | (DrawTool::Polygon, Step::Polygon { .. })
            | (DrawTool::Arc, Step::Arc { end: Some(_), .. }) => {
                if preview.is_empty() {
                    self.status = "Too small to draw · Move further or type a larger value".into();
                    return;
                }
                if !self.add_sketch_curves(tool.label(), preview, cx) {
                    return;
                }
                Some(Step::Idle)
            }
            _ => None,
        };
        if let Some(next) = next
            && let Some(session) = self.session_mut()
        {
            session.step = next;
            session.numeric.clear();
        }
        cx.notify();
    }

    /// Fills in the active tool's preview, placed points and live dimensions.
    pub(crate) fn describe_tool_preview(&self, overlay: &mut ActiveOverlay) {
        let Some(session) = self.sketch.session.as_ref() else {
            return;
        };
        let step = session.step;
        match step {
            Step::Line { start, .. } => overlay.placed.push(start),
            Step::Rectangle { first } => overlay.placed.push(first),
            Step::Circle { center } | Step::Polygon { center } => overlay.placed.push(center),
            Step::Arc { start, end } => {
                overlay.placed.push(start);
                overlay.placed.extend(end);
            }
            _ => {}
        }
        let Some(pointer) = overlay.pointer.as_ref() else {
            return;
        };
        if !step.is_active() {
            return;
        }
        let cursor = pointer.snap.point;
        let scale = pointer.pixels_per_unit.max(1.0e-9);
        let dimensions = tools::dimensions(&step);
        let (locks, _) = session.numeric.locks(dimensions);
        let options = session.options;
        let preview = tools::preview(&step, cursor, locks, options);
        let values = tools::measure(&step, cursor, locks, options);
        let resolved = tools::resolve(&step, cursor, locks);
        let label =
            |index: usize, at: Vec2, away: Vec2, span: Option<(Vec2, Vec2)>| -> Option<DimLabel> {
                let dimension = *dimensions.get(index)?;
                let typed = session.numeric.fields[index].trim().to_owned();
                let parsed = session.numeric.value(index, dimension);
                let text = if typed.is_empty() {
                    let value = values[index]?;
                    if dimension.is_angle() {
                        format_angle(value)
                    } else {
                        format_length(value)
                    }
                } else {
                    typed.clone()
                };
                Some(DimLabel {
                    at,
                    away,
                    span,
                    radial: false,
                    text,
                    locked: !typed.is_empty(),
                    active: session.numeric.active == index,
                    invalid: parsed.is_err(),
                })
            };
        match step {
            Step::Line {
                start, arc: false, ..
            }
            | Step::Arc { start, end: None } => {
                if matches!(step, Step::Arc { .. }) {
                    overlay.preview_guides = preview.clone();
                } else {
                    overlay.preview = preview.clone();
                }
                let offset = resolved - start;
                let length = offset.length();
                if length * scale > 4.0 {
                    let direction = offset / length;
                    let mut away = direction.perp();
                    if away.y < 0.0 || (away.y == 0.0 && away.x > 0.0) {
                        away = -away;
                    }
                    overlay.dims.extend(label(
                        0,
                        (start + resolved) * 0.5,
                        away,
                        Some((start, resolved)),
                    ));
                    if matches!(step, Step::Line { .. }) {
                        overlay.segment = Some((start, resolved, away));
                    }
                    let angle = offset.y.atan2(offset.x);
                    let radius = 38.0 / scale;
                    let axis_aligned = (angle / std::f32::consts::FRAC_PI_2).fract().abs() < 1.0e-4;
                    if length > radius * 1.2 && (!axis_aligned || session.numeric.active == 1) {
                        overlay.arcs.push(AngleArc {
                            center: start,
                            radius,
                            from: 0.0,
                            to: angle,
                        });
                        let middle = Vec2::from_angle(angle * 0.5);
                        overlay
                            .dims
                            .extend(label(1, start + middle * radius * 1.6, middle, None));
                    } else if session.numeric.active == 1 {
                        overlay.dims.extend(label(
                            1,
                            start - direction * 24.0 / scale,
                            -direction,
                            None,
                        ));
                    }
                }
            }
            Step::Line { arc: true, .. } | Step::Arc { end: Some(_), .. } => {
                overlay.preview = preview.clone();
                if let Some(arc @ SketchCurve::Arc { center, .. }) = preview.first() {
                    let middle = arc.midpoint();
                    let away = (middle - *center).normalize_or(Vec2::Y);
                    overlay.dims.extend(label(0, middle, away, None));
                    if dimensions.len() > 1 {
                        overlay.dims.extend(label(1, *center, -away, None));
                    }
                }
            }
            Step::Rectangle { .. } => {
                overlay.preview = preview.clone();
                let (min, max) = preview
                    .iter()
                    .map(SketchCurve::bounds)
                    .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
                    .unwrap_or((resolved, resolved));
                if (max - min).min_element() * scale > 2.0 {
                    overlay.dims.extend(label(
                        0,
                        Vec2::new((min.x + max.x) * 0.5, min.y),
                        Vec2::NEG_Y,
                        Some((min, Vec2::new(max.x, min.y))),
                    ));
                    overlay.dims.extend(label(
                        1,
                        Vec2::new(max.x, (min.y + max.y) * 0.5),
                        Vec2::X,
                        Some((Vec2::new(max.x, min.y), max)),
                    ));
                }
            }
            Step::Circle { center } | Step::Polygon { center } => {
                overlay.preview = preview.clone();
                if resolved.distance(center) * scale > 4.0 {
                    let direction = (resolved - center).normalize();
                    let mut away = direction.perp();
                    if away.y < 0.0 {
                        away = -away;
                    }
                    // A radial dimension: the radius itself is the dimension
                    // line, arrowed at the rim.
                    overlay.dims.extend(
                        label(0, (center + resolved) * 0.5, away, Some((center, resolved))).map(
                            |dim| DimLabel {
                                radial: true,
                                ..dim
                            },
                        ),
                    );
                    overlay.segment = Some((center, resolved, away));
                    if matches!(step, Step::Polygon { .. }) && session.numeric.active == 1 {
                        overlay.dims.extend(label(
                            1,
                            center - direction * 24.0 / scale,
                            -direction,
                            None,
                        ));
                    }
                }
            }
            Step::Idle => {}
        }
    }

    /// Viewport left-button handling while drawing. Returns true when Draw
    /// mode consumed the press (navigation presses fall through).
    pub(crate) fn draw_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_drawing() || event.button != MouseButton::Left || event.modifiers.alt {
            return false;
        }
        let position = mouse_point(event.position);
        self.update_draw_pointer(position, suppress_snapping(event.modifiers));
        let session = self.sketch.session.as_mut().unwrap();
        session.pointer_in_hud = false;
        let step_was_idle = !session.step.is_active();
        session.press = Some(position);
        match session.tool {
            DrawTool::Select => self.select_press(position, event, cx),
            DrawTool::Trim => self.trim_at_pointer(cx),
            _ => {
                self.tool_click(event.click_count, cx);
                // Only presses that start a step can finish it on release.
                if let Some(session) = self.session_mut()
                    && !(step_was_idle && session.step.is_active())
                {
                    session.press = None;
                }
            }
        }
        self.refresh_draw_status();
        cx.notify();
        true
    }

    fn trim_at_pointer(&mut self, cx: &mut Context<Self>) {
        let Some((entity, at)) = self.sketch.session.as_ref().and_then(|s| {
            let pointer = s.pointer.as_ref()?;
            Some((s.hover.entity?, pointer.raw))
        }) else {
            self.status = "Nothing to trim here".into();
            return;
        };
        if self.edit_active_sketch(
            "Trim",
            |sketch| sketch.trim(entity, at, SKETCH_TOLERANCE),
            cx,
        ) {
            if let Some(session) = self.session_mut() {
                session.hover = Hover::default();
                session.selection.clear();
            }
            self.status = "Trimmed".into();
            self.update_draw_pointer(at_screen(self), false);
        }
    }

    fn select_press(&mut self, position: Vec2, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(analysis) = self.sketch_analysis() else {
            return;
        };
        let session = self.sketch.session.as_ref().unwrap();
        let Some(pointer) = session.pointer.clone() else {
            return;
        };
        let radius = PICK_PIXELS / pointer.pixels_per_unit;
        let hit = session.hover.entity;
        let shift = event.modifiers.shift;
        if event.click_count >= 2 {
            let chosen: BTreeSet<usize> = match hit {
                Some(entity) => connected_chain(&analysis.entities, entity),
                None => analysis
                    .profile_at(pointer.raw)
                    .map(|p| {
                        let profile = &analysis.profiles[p];
                        std::iter::once(&profile.outer)
                            .chain(&profile.holes)
                            .flat_map(|l| l.entities.iter().copied())
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            let session = self.session_mut().unwrap();
            session.drag = None;
            session.marquee = None;
            if shift {
                session.selection.extend(chosen);
            } else {
                session.selection = chosen;
            }
            return;
        }
        // Grabbing an endpoint (or circle centre) moves every curve there.
        let grab = analysis
            .entities
            .iter()
            .flat_map(|e| match e.curve {
                SketchCurve::Circle { center, .. } => vec![center],
                SketchCurve::Arc { center, .. } => vec![e.curve.start(), e.curve.end(), center],
                _ => vec![e.curve.start(), e.curve.end()],
            })
            .filter(|p| p.distance(pointer.raw) <= radius * 1.2)
            .min_by(|a, b| a.distance(pointer.raw).total_cmp(&b.distance(pointer.raw)));
        let session = self.sketch.session.as_mut().unwrap();
        let before = analysis.entities.clone();
        if let Some(point) = grab {
            let moving: Vec<usize> = before
                .iter()
                .enumerate()
                .filter(|(_, e)| touches(&e.curve, point))
                .map(|(i, _)| i)
                .collect();
            if !shift && !moving.iter().any(|i| session.selection.contains(i)) {
                session.selection = moving.iter().copied().collect();
            }
            session.drag = Some(super::Drag {
                press_screen: position,
                press_point: point,
                target: DragTarget::Point(point),
                targets: Rc::new(analysis::snap_targets(&before, &moving, &session.reference)),
                before,
                moving,
                started: false,
            });
            return;
        }
        if let Some(entity) = hit {
            if shift {
                if !session.selection.remove(&entity) {
                    session.selection.insert(entity);
                }
                return;
            }
            if !session.selection.contains(&entity) {
                session.selection = [entity].into_iter().collect();
            }
            let curve = before[entity].curve;
            let (target, moving) =
                if matches!(curve, SketchCurve::Circle { .. }) && session.selection.len() == 1 {
                    (DragTarget::Radius(entity), vec![entity])
                } else {
                    (
                        DragTarget::Entities,
                        session.selection.iter().copied().collect(),
                    )
                };
            session.drag = Some(super::Drag {
                press_screen: position,
                press_point: pointer.snap.point,
                target,
                targets: Rc::new(analysis::snap_targets(&before, &moving, &session.reference)),
                before,
                moving,
                started: false,
            });
            return;
        }
        if !shift {
            session.selection.clear();
        }
        session.marquee = Some((position, position));
        let _ = cx;
    }

    /// Pointer motion while drawing. Returns true when Draw mode handled it;
    /// navigation drags fall through to the viewport.
    pub(crate) fn draw_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_drawing() || self.navigation.is_some() {
            return false;
        }
        if matches!(
            event.pressed_button,
            Some(MouseButton::Middle) | Some(MouseButton::Right)
        ) {
            return false;
        }
        let position = mouse_point(event.position);
        let suppress = suppress_snapping(event.modifiers);
        let session = self.sketch.session.as_mut().unwrap();
        session.pointer_in_hud = false;
        if event.pressed_button != Some(MouseButton::Left) {
            // A release outside the window never arrived.
            session.marquee = None;
            if session.drag.is_some() {
                self.finish_sketch_drag(false, cx);
            }
        }
        let session = self.sketch.session.as_mut().unwrap();
        if let Some((_, end)) = session.marquee.as_mut() {
            *end = position;
            cx.notify();
            return true;
        }
        if let Some(drag) = session.drag.as_mut() {
            if !drag.started && drag.press_screen.distance(position) > DRAG_PIXELS {
                drag.started = true;
            }
            if drag.started {
                self.apply_sketch_drag(position, suppress);
                cx.notify();
                return true;
            }
        }
        self.update_draw_pointer(position, suppress);
        cx.notify();
        true
    }

    fn apply_sketch_drag(&mut self, position: Vec2, suppress: bool) {
        let Some(drag) = self.sketch.session.as_ref().and_then(|s| s.drag.clone()) else {
            return;
        };
        let Some(pointer) = self.evaluate_pointer(position, suppress) else {
            return;
        };
        let target = pointer.snap.point;
        let mut entities = drag.before.clone();
        match drag.target {
            DragTarget::Point(at) => {
                for entity in &mut entities {
                    entity.curve = move_point(&entity.curve, at, target);
                }
            }
            DragTarget::Radius(index) => {
                if let SketchCurve::Circle { center, .. } = entities[index].curve {
                    let radius = center.distance(target);
                    if radius > SKETCH_TOLERANCE * 10.0 {
                        entities[index].curve = SketchCurve::Circle { center, radius };
                    }
                }
            }
            DragTarget::Entities => {
                let delta = target - drag.press_point;
                let moving: BTreeSet<usize> = drag.moving.iter().copied().collect();
                let ends: Vec<Vec2> = moving
                    .iter()
                    .filter_map(|&i| drag.before.get(i))
                    .filter(|e| !e.curve.is_closed())
                    .flat_map(|e| [e.curve.start(), e.curve.end()])
                    .collect();
                for (index, entity) in entities.iter_mut().enumerate() {
                    if moving.contains(&index) {
                        entity.curve = entity.curve.translated(delta);
                    } else {
                        // Connected curves stretch to stay attached.
                        for end in &ends {
                            entity.curve = move_point(&entity.curve, *end, *end + delta);
                        }
                    }
                }
            }
        }
        for (entity, before) in entities.iter_mut().zip(&drag.before) {
            if !entity.curve.is_valid() {
                *entity = *before;
            }
        }
        self.set_sketch_entities_live(entities);
        if let Some(session) = self.session_mut() {
            session.pointer = Some(pointer);
            session.hover = Hover::default();
        }
    }

    pub(crate) fn finish_sketch_drag(&mut self, cancel: bool, cx: &mut Context<Self>) {
        let Some(drag) = self.session_mut().and_then(|s| s.drag.take()) else {
            return;
        };
        if !drag.started {
            return;
        }
        let after = self.sketch_entities();
        if cancel || after == drag.before {
            self.set_sketch_entities_live(drag.before);
            cx.notify();
            return;
        }
        // One undo step for the whole drag.
        self.set_sketch_entities_live(drag.before);
        self.history.checkpoint(&self.scene);
        self.set_sketch_entities_live(after);
        self.dirty = true;
        self.status = "Moved".into();
        if let Some(id) = self.session_object() {
            self.sync_sketch_links(id, cx);
        }
        cx.notify();
    }

    pub(crate) fn draw_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if !self.is_drawing() || event.button != MouseButton::Left {
            return;
        }
        let position = mouse_point(event.position);
        let session = self.sketch.session.as_mut().unwrap();
        let press = session.press.take();
        if let Some((start, end)) = session.marquee.take() {
            self.finish_marquee(start, end);
        } else if session.drag.is_some() {
            self.finish_sketch_drag(false, cx);
        } else if let Some(press) = press
            && press.distance(position) > DRAG_PIXELS * 2.0
            && session.step.is_active()
            && !matches!(session.tool, DrawTool::Select | DrawTool::Trim)
        {
            // Press, drag and release draws in one gesture.
            self.update_draw_pointer(position, suppress_snapping(event.modifiers));
            self.tool_click(1, cx);
        }
        self.refresh_draw_status();
        cx.notify();
    }

    fn finish_marquee(&mut self, start: Vec2, end: Vec2) {
        let Some(frame) = self.sketch_frame() else {
            return;
        };
        let Some(analysis) = self.sketch_analysis() else {
            return;
        };
        let (low, high) = (start.min(end), start.max(end));
        if (high - low).max_element() < DRAG_PIXELS {
            return;
        }
        let crossing = end.x < start.x;
        let inside = |p: Vec2| p.cmpge(low).all() && p.cmple(high).all();
        let mut chosen = Vec::new();
        for (index, polyline) in analysis.polylines.iter().enumerate() {
            let points: Vec<Option<Vec2>> = polyline
                .iter()
                .map(|p| self.plane_to_screen(&frame, *p))
                .collect();
            let selected = if crossing {
                points.iter().flatten().any(|p| inside(*p))
                    || points
                        .windows(2)
                        .any(|w| matches!((w[0], w[1]), (Some(a), Some(b)) if segment_hits_box(a, b, low, high)))
            } else {
                points.iter().all(|p| p.is_some_and(inside))
            };
            if selected {
                chosen.push(index);
            }
        }
        if let Some(session) = self.session_mut() {
            session.selection.extend(chosen);
        }
    }

    /// Object mode: clicking near a sketch's curves selects it; a double
    /// click edits it. Returns true when a sketch was hit.
    pub(crate) fn sketch_object_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_drawing()
            || self.edit_mode != EditMode::Object
            || event.button != MouseButton::Left
        {
            return false;
        }
        let Some(id) = self.sketch_object_at(mouse_point(event.position), 6.0) else {
            return false;
        };
        if event.click_count >= 2 {
            self.select_objects([id].into_iter().collect(), Some(id));
            self.execute(Command::Draw(DrawCommand::EditSketch), window, cx);
            return true;
        }
        let mut ids = if event.modifiers.shift {
            self.selected_ids()
        } else {
            Default::default()
        };
        if !ids.insert(id) {
            ids.remove(&id);
        }
        self.select_objects(ids, Some(id));
        let name = self
            .scene
            .object(id)
            .map(|o| o.name.clone())
            .unwrap_or_default();
        self.status = format!("{name} · Double-click or Return to edit");
        self.invalidate(false, cx);
        true
    }

    /// Pointer leaves the viewport for the Draw HUD.
    pub(crate) fn draw_pointer_in_hud(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut()
            && !session.pointer_in_hud
        {
            session.pointer_in_hud = true;
            session.hover = Hover::default();
            cx.notify();
        }
    }
}

fn at_screen(studio: &Studio) -> Vec2 {
    studio.last_mouse
}

/// Whether a curve has an endpoint (or a circle/arc centre) at `point`.
fn touches(curve: &SketchCurve, point: Vec2) -> bool {
    match *curve {
        SketchCurve::Circle { center, .. } => tools::same_point(center, point),
        SketchCurve::Arc { center, .. } => {
            tools::same_point(center, point)
                || tools::same_point(curve.start(), point)
                || tools::same_point(curve.end(), point)
        }
        SketchCurve::Line { start, end } => {
            tools::same_point(start, point) || tools::same_point(end, point)
        }
    }
}

/// Moves a curve's endpoint (or circle/arc centre) at `from` to `to`.
/// Arcs keep passing through their midpoint when an end moves.
fn move_point(curve: &SketchCurve, from: Vec2, to: Vec2) -> SketchCurve {
    match *curve {
        SketchCurve::Line { start, end } => SketchCurve::Line {
            start: if tools::same_point(start, from) {
                to
            } else {
                start
            },
            end: if tools::same_point(end, from) {
                to
            } else {
                end
            },
        },
        SketchCurve::Circle { center, radius } if tools::same_point(center, from) => {
            SketchCurve::Circle { center: to, radius }
        }
        SketchCurve::Arc { center, .. } if tools::same_point(center, from) => {
            curve.translated(to - from)
        }
        SketchCurve::Arc { .. } => {
            let (start, middle, end) = (curve.start(), curve.midpoint(), curve.end());
            let start_moves = tools::same_point(start, from);
            let end_moves = tools::same_point(end, from);
            if !start_moves && !end_moves {
                return *curve;
            }
            SketchCurve::arc_through(
                if start_moves { to } else { start },
                middle,
                if end_moves { to } else { end },
            )
            .unwrap_or(*curve)
        }
        other => other,
    }
}

/// Entities connected to `start` through shared endpoints.
fn connected_chain(entities: &[SketchEntity], start: usize) -> BTreeSet<usize> {
    let mut chosen: BTreeSet<usize> = [start].into_iter().collect();
    let mut frontier = vec![start];
    while let Some(current) = frontier.pop() {
        let curve = entities[current].curve;
        if curve.is_closed() {
            continue;
        }
        for (index, entity) in entities.iter().enumerate() {
            if chosen.contains(&index) || entity.curve.is_closed() {
                continue;
            }
            let shared = [curve.start(), curve.end()].into_iter().any(|p| {
                tools::same_point(p, entity.curve.start())
                    || tools::same_point(p, entity.curve.end())
            });
            if shared {
                chosen.insert(index);
                frontier.push(index);
            }
        }
    }
    chosen
}

fn segment_hits_box(a: Vec2, b: Vec2, low: Vec2, high: Vec2) -> bool {
    // Liang–Barsky clipping of the segment against the box.
    let d = b - a;
    let mut t0: f32 = 0.0;
    let mut t1: f32 = 1.0;
    for (p, q) in [
        (-d.x, a.x - low.x),
        (d.x, high.x - a.x),
        (-d.y, a.y - low.y),
        (d.y, high.y - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

impl Studio {
    /// Status bar keycap hints while drawing.
    pub(crate) fn draw_input_hints(&self) -> Option<Vec<(&'static str, &'static str)>> {
        let session = self.sketch.session.as_ref()?;
        let tools = ("L R C A P T", "Tools");
        let mut hints = match (session.tool, session.step) {
            (DrawTool::Select, _) => vec![
                ("Click", "Select"),
                ("Drag", "Move or box"),
                ("Delete", "Remove"),
                ("X", "Construction"),
                tools,
                ("Return", "Finish"),
            ],
            (DrawTool::Trim, _) => vec![("Click", "Trim piece"), tools, ("Return", "Finish")],
            (_, Step::Line { arc, .. }) => vec![
                ("Click", "Next point"),
                ("0–9", "Length"),
                ("Tab", "Angle"),
                if arc {
                    ("L", "Straight")
                } else {
                    ("A", "Tangent arc")
                },
                ("Return", "End line"),
                ("Ctrl", "Free"),
            ],
            (_, step) if step.is_active() => vec![
                ("Click", "Place"),
                ("0–9", "Exact value"),
                ("Tab", "Next value"),
                ("Esc", "Cancel"),
                ("Ctrl", "Free"),
            ],
            _ => vec![
                ("Click", "Start"),
                ("Ctrl", "Free"),
                tools,
                ("X", "Construction"),
                ("Return", "Finish"),
            ],
        };
        match session.tool {
            DrawTool::Polygon => hints.insert(1, ("+ −", "Sides")),
            DrawTool::Rectangle if !session.step.is_active() => hints.insert(1, ("R", "Centre")),
            _ => {}
        }
        Some(hints)
    }

    /// Status bar selection readout while drawing.
    pub(crate) fn draw_summary(&self) -> Option<String> {
        let session = self.sketch.session.as_ref()?;
        if !session.selection.is_empty() {
            let count = session.selection.len();
            return Some(format!(
                "{count} curve{} selected",
                if count == 1 { "" } else { "s" }
            ));
        }
        let analysis = self.sketch_analysis()?;
        let (curves, profiles) = (analysis.entities.len(), analysis.profiles.len());
        Some(format!(
            "{curves} curve{} · {profiles} profile{}",
            if curves == 1 { "" } else { "s" },
            if profiles == 1 { "" } else { "s" }
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(a: (f32, f32), b: (f32, f32)) -> SketchEntity {
        SketchCurve::Line {
            start: Vec2::new(a.0, a.1),
            end: Vec2::new(b.0, b.1),
        }
        .into()
    }

    #[test]
    fn chains_follow_shared_endpoints() {
        let entities = vec![
            line((0.0, 0.0), (1.0, 0.0)),
            line((1.0, 0.0), (1.0, 1.0)),
            line((5.0, 5.0), (6.0, 5.0)),
            line((1.0, 1.0), (0.0, 0.0)),
        ];
        assert_eq!(
            connected_chain(&entities, 0),
            [0, 1, 3].into_iter().collect()
        );
    }

    #[test]
    fn moving_points_keeps_curves_attached() {
        let moved = move_point(
            &line((0.0, 0.0), (1.0, 0.0)).curve,
            Vec2::new(1.0, 0.0),
            Vec2::new(2.0, 1.0),
        );
        assert_eq!(moved.end(), Vec2::new(2.0, 1.0));
        let arc = SketchCurve::arc_through(
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 0.0),
        )
        .unwrap();
        let moved = move_point(&arc, Vec2::new(1.0, 0.0), Vec2::new(2.0, 0.0));
        assert!(moved.end().distance(Vec2::new(2.0, 0.0)) < 1.0e-4);
        assert!(moved.closest(Vec2::new(0.0, 1.0)).1 < 1.0e-4);
    }

    #[test]
    fn marquee_segments_clip_against_boxes() {
        let (low, high) = (Vec2::ZERO, Vec2::splat(10.0));
        assert!(segment_hits_box(
            Vec2::new(-5.0, 5.0),
            Vec2::new(15.0, 5.0),
            low,
            high
        ));
        assert!(!segment_hits_box(
            Vec2::new(-5.0, -5.0),
            Vec2::new(-1.0, 15.0),
            low,
            high
        ));
    }
}

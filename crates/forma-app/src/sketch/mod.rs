//! Draw mode: precise 2D sketching on planes and planar faces.
//!
//! The scene is the single source of truth for sketch geometry: a session
//! holds only the edited object's id, its plane, and transient tool state.
//! Each completed operation (a segment, a rectangle, a trim, a drag) is one
//! history checkpoint, so undo works inside Draw mode and after it. A new
//! sketch object is created with its first curve, so entering and leaving
//! Draw without drawing leaves the scene untouched.
//!
//! Entry points for other code:
//! - [`Studio::enter_draw`] / [`Studio::finish_draw`] start and end a session.
//! - [`Studio::is_drawing`] and [`Studio::active_sketch`] read its state.
//! - `Command::Draw(DrawCommand::…)` drives it from menus and the palette.
mod analysis;
pub(crate) mod demo;
pub(crate) mod hud;
mod input;
mod numeric;
pub(crate) mod overlay;
pub(crate) mod plane;
pub(crate) mod snap;
pub(crate) mod tools;

use crate::app::{EditMode, Studio, Tool};
use analysis::Analysis;
use forma_core::{Camera, Mat4, Ray, Sketch, SketchCurve, SketchEntity, SketchPlane, Vec2, Vec3};
use gpui::Context;
use numeric::Numeric;
use plane::CameraAnimation;
use snap::{Snap, SnapContext};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    rc::Rc,
    time::{Duration, Instant},
};
pub use tools::DrawTool;
use tools::{Options, Step};

/// Commands that drive Draw mode from menus, the palette and the HUD.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawCommand {
    /// Start a sketch on the face or view plane under the pointer.
    NewSketch,
    /// Edit the selected sketch object.
    EditSketch,
    /// Commit and leave Draw mode.
    Finish,
    Tool(DrawTool),
    /// Construction mode for new curves, or toggle the selected curves.
    ToggleConstruction,
    /// Toggle corner / centre rectangles.
    ToggleRectangleMode,
    /// Add or remove polygon sides.
    PolygonSides(i32),
}

/// A value derived from scene state, rebuilt when its key changes.
type Cached<K, T> = RefCell<Option<(K, Rc<T>)>>;

/// Entity indices at or above this refer to reference geometry, such as the
/// outline of the face a sketch was started on.
pub(crate) const REFERENCE_BASE: usize = 1 << 30;

/// Where a new session draws.
#[derive(Clone, Debug)]
pub(crate) enum DrawTarget {
    /// The face or view plane under the pointer, else the ground.
    AtPointer,
    /// An existing sketch object.
    Object(u64),
    /// An explicit world plane.
    Plane(SketchPlane, String),
}

/// What other code can read about the sketch being drawn.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ActiveSketch {
    /// The scene object, once the first curve has been committed.
    pub object: Option<u64>,
    /// The sketch with its plane in the object's local space.
    pub sketch: Sketch,
    /// Plane-to-world transform of the object.
    pub matrix: Mat4,
}

/// The pointer's last evaluation against the sketch plane.
#[derive(Clone, Debug)]
pub(crate) struct Pointer {
    pub raw: Vec2,
    pub snap: Snap,
    pub pixels_per_unit: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Hover {
    pub entity: Option<usize>,
    pub profile: Option<usize>,
    pub trim: Option<forma_core::TrimPiece>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DragTarget {
    /// Every endpoint coincident with this point moves together.
    Point(Vec2),
    /// A circle or arc's radius.
    Radius(usize),
    /// The selected entities move; connected endpoints stretch.
    Entities,
}

#[derive(Clone, Debug)]
pub(crate) struct Drag {
    pub press_screen: Vec2,
    pub press_point: Vec2,
    pub target: DragTarget,
    pub before: Vec<SketchEntity>,
    pub moving: Vec<usize>,
    pub started: bool,
    /// What the dragged geometry may snap to: everything that is not moving,
    /// as it was when the drag began. Built once per drag.
    pub targets: Rc<analysis::SnapTargets>,
}

/// Points and lines the pointer recently hovered, for SketchUp-style
/// alignment and parallel/perpendicular inference.
#[derive(Clone, Debug, Default)]
pub(crate) struct Memory {
    pub tracked: Vec<Vec2>,
    pub reference: Option<Vec2>,
}

impl Memory {
    fn track(&mut self, point: Vec2) {
        if self.tracked.iter().any(|p| tools::same_point(*p, point)) {
            return;
        }
        self.tracked.push(point);
        if self.tracked.len() > 3 {
            self.tracked.remove(0);
        }
    }
}

pub(crate) struct DrawSession {
    /// The scene object; `None` until the first curve creates it.
    pub object: Option<u64>,
    /// Plane for a sketch that does not exist yet, in world space.
    pub plane: SketchPlane,
    pub label: String,
    pub reference: Vec<SketchCurve>,
    pub tool: DrawTool,
    pub options: Options,
    pub construction: bool,
    pub step: Step,
    pub numeric: Numeric,
    pub pointer: Option<Pointer>,
    pub hover: Hover,
    pub selection: BTreeSet<usize>,
    pub drag: Option<Drag>,
    /// Screen-space marquee start and end.
    pub marquee: Option<(Vec2, Vec2)>,
    /// Left button press position, for press-drag-release drawing.
    pub press: Option<Vec2>,
    pub memory: Memory,
    pub pointer_in_hud: bool,
    /// Scene revision the selection refers to.
    pub selection_revision: u64,
    previous_tool: Tool,
    /// The renderer's ground grid, hidden while the sketch grid is shown.
    previous_grid: bool,
    /// Camera before entering and after the entry move, for restoring the
    /// view on finish when the user did not navigate.
    camera_restore: Option<(Camera, Camera)>,
    analysis: Cached<(u64, u64, bool), Analysis>,
}

#[derive(Default)]
pub(crate) struct SketchState {
    pub session: Option<DrawSession>,
    /// Bumped on every sketch edit, so caches notice changes without the
    /// renderer's scene snapshot being rebuilt for overlay-only geometry.
    pub revision: u64,
    pub animation: Option<CameraAnimation>,
    animation_running: bool,
    display: Cached<(u64, u64, Option<u64>), Vec<Rc<overlay::SketchDisplay>>>,
    visibility: Cached<(u64, u64, Camera, gpui::Size<gpui::Pixels>), overlay::Visibility>,
}

impl SketchState {
    pub(crate) fn active(&self) -> bool {
        self.session.is_some()
    }

    pub(crate) fn animating(&self) -> bool {
        self.animation.is_some()
    }
}

/// Plane, transform and inverse of the edited sketch.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    pub plane: SketchPlane,
    pub matrix: Mat4,
    pub inverse: Mat4,
}

impl Frame {
    pub(crate) fn to_world(self, point: Vec2) -> Vec3 {
        self.matrix.transform_point3(self.plane.to_world(point))
    }

    pub(crate) fn ray_point(&self, ray: Ray) -> Option<Vec2> {
        self.plane.intersect(Ray {
            origin: self.inverse.transform_point3(ray.origin),
            direction: self.inverse.transform_vector3(ray.direction),
        })
    }
}

impl Studio {
    /// Whether Draw mode is active.
    pub(crate) fn is_drawing(&self) -> bool {
        self.sketch.active()
    }

    /// The sketch being drawn, read from the scene (or the empty sketch
    /// waiting for its first curve).
    pub(crate) fn active_sketch(&self) -> Option<ActiveSketch> {
        let session = self.sketch.session.as_ref()?;
        let object = self.session_object();
        let sketch = object
            .and_then(|id| self.scene.object_sketch(id).cloned())
            .unwrap_or_else(|| Sketch::new(session.plane));
        let matrix = object
            .and_then(|id| self.scene.world_transform(id))
            .unwrap_or(Mat4::IDENTITY);
        Some(ActiveSketch {
            object,
            sketch,
            matrix,
        })
    }

    /// The edited object, if it still exists (undo may have removed it).
    pub(crate) fn session_object(&self) -> Option<u64> {
        let session = self.sketch.session.as_ref()?;
        session
            .object
            .filter(|id| self.scene.object_sketch(*id).is_some())
    }

    pub(crate) fn sketch_frame(&self) -> Option<Frame> {
        let session = self.sketch.session.as_ref()?;
        let object = self.session_object();
        let plane = object
            .and_then(|id| self.scene.object_sketch(id))
            .map(|sketch| sketch.plane)
            .unwrap_or(session.plane);
        let matrix = object
            .and_then(|id| self.scene.world_transform(id))
            .filter(|m| m.is_finite() && m.determinant().abs() > 1.0e-12)
            .unwrap_or(Mat4::IDENTITY);
        Some(Frame {
            plane,
            matrix,
            inverse: matrix.inverse(),
        })
    }

    fn session_entities(&self) -> &[SketchEntity] {
        self.session_object()
            .and_then(|id| self.scene.object_sketch(id))
            .map(|sketch| sketch.entities.as_slice())
            .unwrap_or(&[])
    }

    /// Profiles, snap points and polylines of the edited sketch, rebuilt only
    /// when the scene or a sketch changes.
    pub(crate) fn sketch_analysis(&self) -> Option<Rc<Analysis>> {
        let session = self.sketch.session.as_ref()?;
        // While a drag moves curves, only what the display needs is rebuilt
        // per move; the full analysis follows when the drag ends.
        let dragging = session.drag.as_ref().is_some_and(|drag| drag.started);
        let key = (self.scene_revision, self.sketch.revision, dragging);
        let mut cache = session.analysis.borrow_mut();
        if let Some((cached, analysis)) = cache.as_ref()
            && *cached == key
        {
            return Some(analysis.clone());
        }
        let analysis = Rc::new(if dragging {
            analysis::analyze_for_drag(self.session_entities())
        } else {
            analysis::analyze(self.session_entities(), &session.reference)
        });
        *cache = Some((key, analysis.clone()));
        Some(analysis)
    }

    /// Starts a Draw session. A session already running is finished first,
    /// unless it already edits the requested object.
    pub(crate) fn enter_draw(&mut self, target: DrawTarget, cx: &mut Context<Self>) {
        if let (DrawTarget::Object(id), Some(current)) = (&target, self.session_object())
            && *id == current
        {
            return;
        }
        if self.is_drawing() {
            self.finish_draw(cx);
        }
        let camera = self.scene.camera;
        let (object, plane, label, reference, focus, world_plane) = match target {
            DrawTarget::Object(id) => {
                let Some(sketch) = self.scene.object_sketch(id) else {
                    self.status = "Select a sketch to edit".into();
                    return;
                };
                let matrix = self.scene.world_transform(id).unwrap_or(Mat4::IDENTITY);
                let world = sketch.plane.transformed(matrix).unwrap_or(sketch.plane);
                let name = self
                    .scene
                    .object(id)
                    .map(|o| o.name.clone())
                    .unwrap_or_default();
                let (min, max) = sketch_bounds(sketch).unwrap_or((Vec2::ZERO, Vec2::ZERO));
                let focus = matrix.transform_point3(sketch.plane.to_world((min + max) * 0.5));
                (Some(id), sketch.plane, name, Vec::new(), focus, world)
            }
            DrawTarget::Plane(plane, label) => {
                let focus = plane.to_world(plane.to_local(camera.target));
                (None, plane, label, Vec::new(), focus, plane)
            }
            DrawTarget::AtPointer => {
                let ray = self.pointer_ray(self.last_mouse);
                let choice = plane::choose(&self.scene, camera, ray);
                // Centre a picked face rather than the exact point clicked.
                let focus = choice
                    .reference
                    .iter()
                    .map(SketchCurve::bounds)
                    .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
                    .map_or(choice.focus, |(min, max)| {
                        choice.plane.to_world((min + max) * 0.5)
                    });
                (
                    None,
                    choice.plane,
                    choice.label,
                    choice.reference,
                    focus,
                    choice.plane,
                )
            }
        };
        if self.edit_mode != EditMode::Object {
            self.edit_mode = EditMode::Object;
            self.clear_components();
        }
        self.navigation = None;
        self.box_select = None;
        let previous_tool = std::mem::replace(&mut self.tool, Tool::Select);
        let previous_grid = std::mem::replace(&mut self.settings.show_grid, false);
        // Turn to face the plane unless the view is already close to it.
        let mut target_camera = plane::look_at(camera, &world_plane, focus);
        // Bring a small face up to a comfortable drawing size.
        if let Some((min, max)) = reference
            .iter()
            .map(SketchCurve::bounds)
            .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
        {
            let size = self.bounds.get().size;
            let aspect = f32::from(size.width) / f32::from(size.height).max(1.0);
            let mut framed = target_camera;
            framed.frame_in_viewport(focus, (max - min).length() * 0.5 * 1.5, aspect);
            if framed.distance < target_camera.distance {
                target_camera.distance = framed.distance;
            }
        }
        let reorient = plane::view_angle(camera, &world_plane) > 10f32.to_radians()
            || target_camera.distance < camera.distance * 0.9;
        let camera_restore = if reorient {
            self.animate_camera(target_camera, cx);
            Some((camera, target_camera))
        } else {
            None
        };
        let editing = object.is_some();
        self.sketch.session = Some(DrawSession {
            object,
            plane,
            label: label.clone(),
            reference,
            tool: if editing {
                DrawTool::Select
            } else {
                DrawTool::Line
            },
            options: Options::default(),
            construction: false,
            step: Step::Idle,
            numeric: Numeric::default(),
            pointer: None,
            hover: Hover::default(),
            selection: BTreeSet::new(),
            drag: None,
            marquee: None,
            press: None,
            memory: Memory::default(),
            pointer_in_hud: false,
            selection_revision: self.scene_revision,
            previous_tool,
            previous_grid,
            camera_restore,
            analysis: RefCell::new(None),
        });
        self.status = format!(
            "Draw · {label} · L line · R rectangle · C circle · A arc · P polygon · Return to finish"
        );
        // The ground grid change renders through the GPU frame.
        self.invalidate(false, cx);
    }

    /// Commits and leaves Draw mode. Drawing is committed as it happens, so
    /// finishing never discards work; an in-progress tool step is dropped.
    pub(crate) fn finish_draw(&mut self, cx: &mut Context<Self>) {
        if self
            .sketch
            .session
            .as_ref()
            .is_some_and(|s| s.drag.is_some())
        {
            self.finish_sketch_drag(false, cx);
        }
        let Some(session) = self.sketch.session.take() else {
            return;
        };
        self.restore_after_draw(&session);
        let object = session
            .object
            .filter(|id| self.scene.object_sketch(*id).is_some());
        let mut summary = String::from("Draw finished");
        if let Some(id) = object {
            let sketch = self.scene.object_sketch(id).unwrap();
            if sketch.entities.is_empty() {
                self.history.checkpoint(&self.scene);
                self.scene.remove(id);
                self.sketch.revision += 1;
                self.dirty = true;
                summary = "Draw finished · Removed the empty sketch".into();
                if self.selected == Some(id) {
                    self.select_objects(Default::default(), None);
                }
            } else {
                let curves = sketch.entities.len();
                let profiles = sketch.profiles(forma_core::SKETCH_TOLERANCE).len();
                let name = self
                    .scene
                    .object(id)
                    .map(|o| o.name.clone())
                    .unwrap_or_default();
                summary = format!(
                    "{name} · {curves} curve{} · {profiles} profile{}",
                    if curves == 1 { "" } else { "s" },
                    if profiles == 1 { "" } else { "s" }
                );
                self.select_objects([id].into_iter().collect(), Some(id));
            }
        }
        // Return to the previous view if the user kept the one Draw chose.
        if let Some((before, after)) = session.camera_restore {
            let untouched = match self.sketch.animation {
                Some(animation) => animation.last == self.scene.camera && animation.to == after,
                None => self.scene.camera == after,
            };
            if untouched {
                self.animate_camera(before, cx);
            }
        }
        self.status = summary;
        // Selection highlights render through the GPU frame.
        self.invalidate(false, cx);
    }

    /// Jumps a running camera move to its end.
    pub(crate) fn complete_camera_animation(&mut self, cx: &mut Context<Self>) {
        if let Some(animation) = self.sketch.animation.take() {
            self.scene.camera = animation.to;
            self.invalidate(false, cx);
        }
    }

    /// Leaves Draw mode without any finishing behaviour (selection, view
    /// restore). Drawing is already committed as it happens.
    pub(crate) fn abandon_draw(&mut self) {
        if let Some(session) = self.sketch.session.take() {
            self.restore_after_draw(&session);
        }
        self.sketch.animation = None;
    }

    fn restore_after_draw(&mut self, session: &DrawSession) {
        self.tool = session.previous_tool;
        if !self.settings.show_grid {
            self.settings.show_grid = session.previous_grid;
        }
    }

    /// Eases the camera to `to`. Navigating during the move cancels it.
    pub(crate) fn animate_camera(&mut self, to: Camera, cx: &mut Context<Self>) {
        let from = self.scene.camera;
        if from == to {
            self.sketch.animation = None;
            return;
        }
        self.sketch.animation = Some(CameraAnimation::new(from, to));
        if self.sketch.animation_running {
            return;
        }
        self.sketch.animation_running = true;
        cx.spawn(async move |this, cx| {
            loop {
                gpui::Timer::after(Duration::from_millis(12)).await;
                let running = this
                    .update(cx, |studio, cx| studio.step_camera_animation(cx))
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
        })
        .detach();
    }

    fn step_camera_animation(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(animation) = self.sketch.animation.as_mut() else {
            self.sketch.animation_running = false;
            return false;
        };
        if self.scene.camera != animation.last {
            // The user navigated: their view wins.
            self.sketch.animation = None;
            self.sketch.animation_running = false;
            return false;
        }
        let (camera, done) = animation.sample(Instant::now());
        animation.last = camera;
        self.scene.camera = camera;
        if done {
            self.sketch.animation = None;
            self.sketch.animation_running = false;
        }
        self.invalidate(false, cx);
        !done
    }

    /// A world ray through a window position inside the viewport.
    pub(crate) fn pointer_ray(&self, screen: Vec2) -> Option<Ray> {
        let bounds = self.bounds.get();
        let size = Vec2::new(bounds.size.width.into(), bounds.size.height.into());
        if size.x < 1.0 || size.y < 1.0 {
            return None;
        }
        let uv = (screen - crate::viewport::mouse_point(bounds.origin)) / size;
        if !(0.0..=1.0).contains(&uv.x) || !(0.0..=1.0).contains(&uv.y) {
            return None;
        }
        Some(self.scene.camera.ray(uv, size.x / size.y))
    }

    pub(crate) fn view_projection(&self) -> Mat4 {
        let bounds = self.bounds.get();
        let aspect = f32::from(bounds.size.width) / f32::from(bounds.size.height).max(1.0);
        self.scene.camera.projection_matrix(aspect) * self.scene.camera.view_matrix()
    }

    /// Window position of a plane point.
    pub(crate) fn plane_to_screen(&self, frame: &Frame, point: Vec2) -> Option<Vec2> {
        crate::viewport::project(
            frame.to_world(point),
            self.view_projection(),
            self.bounds.get(),
        )
        .map(crate::viewport::mouse_point)
    }

    /// Screen pixels per plane unit around `point`.
    pub(crate) fn pixels_per_unit(&self, frame: &Frame, point: Vec2) -> f32 {
        let step = (self.scene.camera.distance * 1.0e-3).max(1.0e-6);
        let Some(origin) = self.plane_to_screen(frame, point) else {
            return 1.0;
        };
        let mut total = 0.0;
        let mut count = 0.0;
        for offset in [Vec2::X, Vec2::Y] {
            if let Some(p) = self.plane_to_screen(frame, point + offset * step) {
                total += p.distance(origin) / step;
                count += 1.0;
            }
        }
        if count > 0.0 && total > 0.0 {
            total / count
        } else {
            1.0
        }
    }

    /// The plane point under the centre of the viewport, which sets the
    /// grid's scale and fade centre.
    pub(crate) fn plane_focus(&self, frame: &Frame) -> Vec2 {
        let bounds = self.bounds.get();
        let center = crate::viewport::mouse_point(bounds.center());
        self.pointer_ray(center)
            .and_then(|ray| frame.ray_point(ray))
            .unwrap_or_else(|| {
                frame
                    .plane
                    .to_local(frame.inverse.transform_point3(self.scene.camera.target))
            })
    }

    pub(crate) fn grid_steps(&self, frame: &Frame) -> (f32, f32) {
        snap::grid_steps(self.pixels_per_unit(frame, self.plane_focus(frame)))
    }

    /// Evaluates a window position against the plane: raw point, snapped
    /// point and scale. During a drag the pointer snaps only to geometry
    /// that is not moving.
    pub(crate) fn evaluate_pointer(&self, screen: Vec2, suppress: bool) -> Option<Pointer> {
        let session = self.sketch.session.as_ref()?;
        let frame = self.sketch_frame()?;
        let raw = frame.ray_point(self.pointer_ray(screen)?)?;
        if !raw.is_finite() || raw.abs().max_element() > forma_core::MAX_SKETCH_COORDINATE {
            return None;
        }
        let pixels_per_unit = self.pixels_per_unit(&frame, raw).max(1.0e-9);
        let targets = match session.drag.as_ref() {
            Some(drag) => drag.targets.clone(),
            None => self.sketch_analysis()?.targets.clone(),
        };
        let (grid, _) = self.grid_steps(&frame);
        let mut references: Vec<Vec2> = session.memory.reference.into_iter().collect();
        if let Some(tangent) = match session.step {
            Step::Line { tangent, .. } => tangent,
            _ => None,
        } {
            references.push(tangent);
        }
        let anchor = match session.drag.as_ref().map(|d| d.target) {
            Some(DragTarget::Point(_))
            | Some(DragTarget::Radius(_))
            | Some(DragTarget::Entities) => None,
            None => session.step.anchor(),
        };
        let cx = SnapContext {
            points: &targets.points,
            curves: &targets.curves,
            exclude: &[],
            anchor,
            references: &references,
            tracked: &session.memory.tracked,
            grid,
            radius: 11.0 / pixels_per_unit,
            enabled: !suppress,
        };
        Some(Pointer {
            raw,
            snap: snap::snap(raw, &cx),
            pixels_per_unit,
        })
    }

    /// Applies an edit to the active sketch as one undoable step, creating
    /// the sketch object on first use. Invalid results are rejected whole.
    pub(crate) fn edit_active_sketch(
        &mut self,
        label: &str,
        edit: impl FnOnce(&mut Sketch) -> bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(session) = self.sketch.session.as_ref() else {
            return false;
        };
        let object = self.session_object();
        let mut candidate = object
            .and_then(|id| self.scene.object_sketch(id).cloned())
            .unwrap_or_else(|| Sketch::new(session.plane));
        if !edit(&mut candidate) {
            return false;
        }
        if let Err(error) = candidate.validate() {
            self.status = format!("{label}: {error:#}");
            cx.notify();
            return false;
        }
        match object {
            Some(id) => {
                self.history.checkpoint(&self.scene);
                *self.scene.object_sketch_mut(id).unwrap() = candidate;
            }
            None => {
                // Creation can fail (object limit); record history only
                // once it succeeded, so failure leaves undo and redo intact.
                let before = self.scene.clone();
                match self.scene.add_sketch("Sketch", candidate) {
                    Ok(id) => {
                        self.history.checkpoint_owned(before);
                        if let Some(session) = self.sketch.session.as_mut() {
                            session.object = Some(id);
                        }
                    }
                    Err(error) => {
                        self.status = format!("{label}: {error:#}");
                        cx.notify();
                        return false;
                    }
                }
            }
        }
        self.sketch.revision += 1;
        self.dirty = true;
        cx.notify();
        true
    }

    /// Adds curves to the active sketch as one step.
    pub(crate) fn add_sketch_curves(
        &mut self,
        label: &str,
        curves: Vec<SketchCurve>,
        cx: &mut Context<Self>,
    ) -> bool {
        let construction = self.sketch.session.as_ref().is_some_and(|s| s.construction);
        let curves: Vec<_> = curves.into_iter().filter(SketchCurve::is_valid).collect();
        if curves.is_empty() {
            self.status = format!("{label} too small to draw · Curves must be longer than 0.01 mm");
            cx.notify();
            return false;
        }
        let added = self.edit_active_sketch(
            label,
            |sketch| {
                sketch
                    .entities
                    .extend(curves.into_iter().map(|curve| SketchEntity {
                        curve,
                        construction,
                    }));
                true
            },
            cx,
        );
        if added {
            self.status = format!("{label} added");
        }
        added
    }

    /// Replaces the edited sketch's curves without recording history, for
    /// live drags. The drag's end records one step.
    pub(crate) fn set_sketch_entities_live(&mut self, entities: Vec<SketchEntity>) {
        if let Some(id) = self.session_object()
            && let Some(sketch) = self.scene.object_sketch_mut(id)
        {
            sketch.entities = entities;
            self.sketch.revision += 1;
        }
    }
}

pub(crate) fn sketch_bounds(sketch: &Sketch) -> Option<(Vec2, Vec2)> {
    sketch
        .entities
        .iter()
        .map(|entity| entity.curve.bounds())
        .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
}

//! Sketch overlays painted over the viewport frame in every render mode.
//!
//! World-space polylines and profile polygons are cached per sketch edit;
//! each paint only projects them through the current camera.
use super::analysis::{Analysis, display_tolerance};
use super::snap::{GuideKind, SnapKind};
use super::{Frame, Hover, Pointer};
use crate::app::Studio;
use crate::theme::Colors;
use forma_core::{ObjectData, SKETCH_TOLERANCE, SketchCurve, Vec2, Vec3};
use glam::{Mat4, Vec4};
use gpui::{
    App, Bounds, Hsla, PathBuilder, Pixels, Point, Rgba, TextRun, Window, fill, point, px, size,
};
use std::{collections::BTreeSet, rc::Rc};

/// A visible sketch object other than the one being edited.
pub(crate) struct SketchDisplay {
    pub id: u64,
    /// The sketch and transform this was built from, so unchanged sketches
    /// are reused when another one is edited.
    source: (forma_core::Sketch, Mat4),
    pub curves: Vec<(Vec<Vec3>, bool)>,
    /// Each profile's outer loop and holes.
    pub fills: Vec<Vec<Vec<Vec3>>>,
}

/// A live dimension next to the geometry being drawn.
#[derive(Clone, Debug)]
pub(crate) struct DimLabel {
    pub at: Vec2,
    /// Plane direction the label is pushed away from the geometry.
    pub away: Vec2,
    /// The measured span for a linear dimension: drawn offset along `away`
    /// with extension lines and arrowheads. `None` places the value alone.
    pub span: Option<(Vec2, Vec2)>,
    /// The span runs from a centre to a rim: drawn as the radius itself,
    /// arrowed at the rim, with the value beside it.
    pub radial: bool,
    pub text: String,
    /// Typed values lock the dimension.
    pub locked: bool,
    /// The field receiving keystrokes.
    pub active: bool,
    pub invalid: bool,
}

/// An angle indicator: centre, radius in plane units, from and to angles.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AngleArc {
    pub center: Vec2,
    pub radius: f32,
    pub from: f32,
    pub to: f32,
}

pub(crate) struct ActiveOverlay {
    pub frame: Frame,
    pub analysis: Rc<Analysis>,
    pub selection: BTreeSet<usize>,
    pub hover: Hover,
    pub preview: Vec<SketchCurve>,
    pub preview_construction: bool,
    /// Guide-only preview, such as a three point arc's chord.
    pub preview_guides: Vec<SketchCurve>,
    pub placed: Vec<Vec2>,
    pub pointer: Option<Pointer>,
    pub drawing: bool,
    pub dims: Vec<DimLabel>,
    pub arcs: Vec<AngleArc>,
    pub grid: (f32, f32),
    pub focus: Vec2,
    pub reference: Vec<SketchCurve>,
    pub marquee: Option<(Vec2, Vec2)>,
    /// The straight segment being drawn and the side its dimension sits on:
    /// direction inferences attach to it as constraint chips.
    pub segment: Option<(Vec2, Vec2, Vec2)>,
    /// Where scene geometry stands in front of the plane, so the grid stops
    /// at objects instead of running across them.
    pub mask: Option<Rc<PlaneMask>>,
}

/// Where geometry hides the sketch plane, sampled on a coarse screen grid:
/// the plane point under each sample is tested against the scene, and grid
/// lines fade out where it is covered. Sampled at the corners of
/// `MASK_CELL` pixel cells and read bilinearly, so the grid fades over a few
/// pixels at an object's outline rather than stepping.
pub(crate) struct PlaneMask {
    origin: Vec2,
    columns: usize,
    rows: usize,
    /// 1 where the plane point under the sample is hidden.
    hidden: Vec<f32>,
}

const MASK_CELL: f32 = 14.0;

impl PlaneMask {
    /// How hidden the plane is at window position `p`, from 0 to 1.
    fn hidden_at(&self, p: Vec2) -> f32 {
        let g = (p - self.origin) / MASK_CELL;
        let (x0, y0) = (g.x.floor(), g.y.floor());
        let (fx, fy) = (g.x - x0, g.y - y0);
        let at = |x: f32, y: f32| {
            let x = (x.max(0.0) as usize).min(self.columns - 1);
            let y = (y.max(0.0) as usize).min(self.rows - 1);
            self.hidden[y * self.columns + x]
        };
        let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
        let bottom = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// The plane mask and what it was sampled for: scene revision, camera,
/// viewport and plane.
type MaskKey = (
    u64,
    forma_core::Camera,
    Bounds<Pixels>,
    Mat4,
    forma_core::SketchPlane,
);

thread_local! {
    static PLANE_MASK: std::cell::RefCell<Option<(MaskKey, Rc<PlaneMask>)>> =
        const { std::cell::RefCell::new(None) };
}

/// Inks for Draw mode. Curves pair a core that contrasts with the theme's
/// viewport backdrop with a halo that separates them from any shaded
/// surface, so they read over dark space and light faces alike.
///
/// Draw's colour rule, the same for every tool and both themes:
/// - committed curves, open or closed, are the two-tone ink (`core` over
///   `halo`); a closed outline is not recoloured;
/// - the curve being drawn (the tool's preview) is the accent;
/// - a closed profile is marked by a light accent fill (`PROFILE_FILL`),
///   never by recolouring its curves;
/// - an open end, which keeps a region from closing, carries an amber ring
///   (the theme's `warning`) around its point, the colour of the panel's
///   "Open ends" warning, so the canvas and the panel name the same
///   problem. Amber means an open end in Draw and nothing else;
/// - the plane's axes, and the horizontal and vertical inferences along
///   them, take the colour of the world axis each one runs along (see
///   [`plane_axis_colors`]), as the view gizmo labels them;
/// - selection is the accent and hover a lighter accent: interaction
///   states, layered over the rule rather than part of it.
#[derive(Clone, Copy)]
pub(crate) struct SketchInk {
    pub core: Rgba,
    pub halo: Rgba,
    /// Endpoint fill and its thin ring.
    pub point: Rgba,
    pub point_ring: Rgba,
    /// Dimension and extension lines.
    pub annotation: Rgba,
    /// Laid over the rendered frame so the plane and sketch stand out while
    /// the scene stays visible: about two thirds of the viewport backdrop,
    /// so bodies keep about a third of their contrast. A face being
    /// sketched on is cut out of it and stays at full strength.
    pub scrim: Rgba,
}

impl SketchInk {
    pub(crate) fn new(c: Colors) -> Self {
        if c.dark {
            Self {
                core: ink(c.text, 1.0),
                halo: ink(0x07080a, 0.72),
                point: ink(0xffffff, 1.0),
                point_ring: ink(0x07080a, 0.9),
                annotation: ink(c.text, 0.82),
                scrim: ink(0x1c2027, 0.66),
            }
        } else {
            Self {
                core: ink(c.text, 1.0),
                halo: ink(0xffffff, 0.82),
                point: ink(0xffffff, 1.0),
                point_ring: ink(c.text, 0.92),
                annotation: ink(c.text, 0.78),
                scrim: ink(0xe3e5e8, 0.62),
            }
        }
    }
}

/// Per sketch, per curve, per point: whether geometry hides the point.
pub(crate) type Visibility = Vec<Vec<Vec<bool>>>;

/// Hidden-line flags and what they were computed for. Holding the displays
/// keeps their identities from being reused while cached.
pub(crate) struct VisibilityCache {
    scene_revision: u64,
    camera: forma_core::Camera,
    size: gpui::Size<Pixels>,
    displays: Vec<Rc<SketchDisplay>>,
    value: Rc<Visibility>,
}

pub(crate) struct SketchOverlay {
    pub colors: Colors,
    pub others: Rc<Vec<Rc<SketchDisplay>>>,
    pub visibility: Option<Rc<Visibility>>,
    pub selected: BTreeSet<u64>,
    pub active_id: Option<u64>,
    pub active: Option<ActiveOverlay>,
}

pub(crate) fn ink(color: u32, alpha: f32) -> Rgba {
    gpui::rgba((color << 8) | (alpha.clamp(0.0, 1.0) * 255.0).round() as u32)
}

pub(crate) fn format_length(value: f32) -> String {
    format!("{} m", crate::transform::format_precision_value(value))
}

pub(crate) fn format_angle(radians: f32) -> String {
    let degrees = super::tools::display_degrees(radians);
    let text = format!("{degrees:.1}");
    format!("{}°", text.trim_end_matches('0').trim_end_matches('.'))
}

impl Studio {
    /// World-space display data for every visible sketch, cached per edit.
    pub(crate) fn sketch_displays(&self) -> Rc<Vec<Rc<SketchDisplay>>> {
        // The sketch being edited is painted from its own live analysis.
        let editing = self.session_object();
        let key = (self.scene_revision, self.sketch.revision, editing);
        let mut cache = self.sketch.display.borrow_mut();
        if let Some((cached, displays)) = cache.as_ref()
            && *cached == key
        {
            return displays.clone();
        }
        let previous = cache.take().map(|(_, displays)| displays);
        let mut displays = Vec::new();
        for object in &self.scene.objects {
            let ObjectData::Sketch { sketch } = &object.data else {
                continue;
            };
            if Some(object.id) == editing || !self.scene.is_effectively_visible(object.id) {
                continue;
            }
            let Some(matrix) = self.scene.world_transform(object.id) else {
                continue;
            };
            if let Some(reused) = previous.as_ref().and_then(|previous| {
                previous
                    .iter()
                    .find(|d| d.id == object.id && d.source.1 == matrix && d.source.0 == *sketch)
            }) {
                displays.push(reused.clone());
                continue;
            }
            let world = |p: Vec2| matrix.transform_point3(sketch.plane.to_world(p));
            let curves = sketch
                .entities
                .iter()
                .filter(|e| e.curve.is_valid())
                .map(|e| {
                    (
                        e.curve
                            .polyline(display_tolerance(&e.curve))
                            .into_iter()
                            .map(world)
                            .collect(),
                        e.construction,
                    )
                })
                .collect();
            let fills = sketch
                .profiles(SKETCH_TOLERANCE)
                .iter()
                .map(|profile| {
                    std::iter::once(&profile.outer)
                        .chain(&profile.holes)
                        .map(|profile_loop| {
                            let (min, max) = profile_loop.bounds();
                            profile_loop
                                .polygon(((max - min).max_element() * 1.0e-3).max(1.0e-7))
                                .into_iter()
                                .map(world)
                                .collect()
                        })
                        .collect()
                })
                .collect();
            displays.push(Rc::new(SketchDisplay {
                id: object.id,
                source: (sketch.clone(), matrix),
                curves,
                fills,
            }));
        }
        let displays = Rc::new(displays);
        *cache = Some((key, displays.clone()));
        displays
    }

    /// Which points of each inactive sketch are hidden behind geometry, so
    /// overlays read like hidden lines. Rays are cast every few pixels along
    /// each curve and cached until the camera or the scene changes.
    fn sketch_visibility(&self, displays: &Rc<Vec<Rc<SketchDisplay>>>) -> Option<Rc<Visibility>> {
        if displays.is_empty() || self.scene.mesh_instances().next().is_none() {
            return None;
        }
        // Keyed by the inactive sketches themselves (unchanged ones keep their
        // display), so editing another sketch does not recast every ray.
        let (revision, camera, size) = (
            self.scene_revision,
            self.scene.camera,
            self.bounds.get().size,
        );
        let mut cache = self.sketch.visibility.borrow_mut();
        if let Some(cached) = cache.as_ref()
            && cached.scene_revision == revision
            && cached.camera == camera
            && cached.size == size
            && cached.displays.len() == displays.len()
            && cached
                .displays
                .iter()
                .zip(displays.iter())
                .all(|(a, b)| Rc::ptr_eq(a, b))
        {
            return Some(cached.value.clone());
        }
        let query = self.visibility_query();
        let camera = self.scene.camera;
        let projector = Projector {
            matrix: self.view_projection(),
            bounds: self.bounds.get(),
        };
        let visible = |p: Vec3| crate::viewport::visible_in_query(camera, query, p);
        let visibility: Visibility = displays
            .iter()
            .map(|display| {
                display
                    .curves
                    .iter()
                    .map(|(points, _)| {
                        let mut flags = Vec::with_capacity(points.len());
                        let mut last: Option<(Vec2, bool)> = None;
                        for (index, world) in points.iter().enumerate() {
                            let screen = projector.point(*world).map(to_vec);
                            let reuse = match (last, screen) {
                                (Some((at, flag)), Some(p))
                                    if p.distance(at) < 8.0 && index + 1 < points.len() =>
                                {
                                    Some(flag)
                                }
                                _ => None,
                            };
                            let flag = reuse.unwrap_or_else(|| {
                                let flag = visible(*world);
                                if let Some(p) = screen {
                                    last = Some((p, flag));
                                }
                                flag
                            });
                            flags.push(flag);
                        }
                        flags
                    })
                    .collect()
            })
            .collect();
        let visibility = Rc::new(visibility);
        *cache = Some(VisibilityCache {
            scene_revision: revision,
            camera,
            size,
            displays: displays.iter().cloned().collect(),
            value: visibility.clone(),
        });
        Some(visibility)
    }

    /// Screen-space hit test against visible sketch objects' curves, for
    /// selecting sketches in Object mode.
    pub(crate) fn sketch_object_at(&self, screen: Vec2, radius: f32) -> Option<u64> {
        let displays = self.sketch_displays();
        let bounds = self.bounds.get();
        let projector = Projector {
            matrix: self.view_projection(),
            bounds,
        };
        // Every curve passing near the pointer, with the world point there.
        let mut candidates: Vec<(f32, u64, Vec3)> = Vec::new();
        for display in displays.iter() {
            if !self.scene.object(display.id).is_some_and(|o| o.selectable) {
                continue;
            }
            for (points, _) in &display.curves {
                for pair in points.windows(2) {
                    let (Some(a), Some(b)) = (projector.point(pair[0]), projector.point(pair[1]))
                    else {
                        continue;
                    };
                    let (a, b) = (to_vec(a), to_vec(b));
                    let d = b - a;
                    let t = ((screen - a).dot(d) / d.length_squared().max(1.0e-6)).clamp(0.0, 1.0);
                    let distance = screen.distance(a + d * t);
                    if distance <= radius {
                        candidates.push((distance, display.id, pair[0].lerp(pair[1], t)));
                    }
                }
            }
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Curves hidden behind geometry are not clickable: the surface in
        // front of them is what the pointer is on.
        let camera = self.scene.camera;
        let query = self.visibility_query();
        candidates
            .into_iter()
            .find(|(_, _, world)| crate::viewport::visible_in_query(camera, query, *world))
            .map(|(_, id, _)| id)
    }

    /// Everything the viewport needs to paint sketches this frame.
    pub(crate) fn sketch_overlay(&self) -> Option<SketchOverlay> {
        let others = self.sketch_displays();
        let session = self.sketch.session.as_ref();
        if others.is_empty() && session.is_none() {
            return None;
        }
        let colors = self.theme_colors();
        let active = session.and_then(|session| {
            let frame = self.sketch_frame()?;
            let analysis = self.sketch_analysis()?;
            let focus = self.plane_focus(&frame);
            let grid = self.grid_steps(&frame);
            let mut overlay = ActiveOverlay {
                frame,
                analysis,
                selection: session.selection.clone(),
                hover: session.hover.clone(),
                preview: Vec::new(),
                preview_construction: session.construction,
                preview_guides: Vec::new(),
                placed: Vec::new(),
                pointer: session.pointer.clone().filter(|_| !session.pointer_in_hud),
                drawing: !matches!(
                    session.tool,
                    super::DrawTool::Select | super::DrawTool::Trim
                ),
                dims: Vec::new(),
                arcs: Vec::new(),
                grid,
                focus,
                reference: session.reference.clone(),
                marquee: session.marquee,
                segment: None,
                mask: self.plane_mask(&frame),
            };
            self.describe_tool_preview(&mut overlay);
            Some(overlay)
        });
        let visibility = self.sketch_visibility(&others);
        Some(SketchOverlay {
            colors,
            visibility,
            others,
            selected: self.selected_ids(),
            active_id: self.session_object(),
            active,
        })
    }
}

impl Studio {
    /// Where scene geometry hides the sketch plane, sampled once per camera
    /// and scene revision. While the view moves the grid goes unmasked, as
    /// resampling every frame would stall navigation.
    fn plane_mask(&self, frame: &Frame) -> Option<Rc<PlaneMask>> {
        if self.navigation.is_some() || self.sketch.animating() {
            return None;
        }
        self.scene.mesh_instances().next()?;
        let bounds = self.bounds.get();
        let key: MaskKey = (
            self.scene_revision,
            self.scene.camera,
            bounds,
            frame.matrix,
            frame.plane,
        );
        if let Some(mask) = PLANE_MASK.with(|cache| {
            cache
                .borrow()
                .as_ref()
                .filter(|(cached, _)| *cached == key)
                .map(|(_, mask)| mask.clone())
        }) {
            return Some(mask);
        }
        let origin = crate::viewport::mouse_point(bounds.origin);
        let size = Vec2::new(bounds.size.width.into(), bounds.size.height.into());
        if size.x < 1.0 || size.y < 1.0 {
            return None;
        }
        let columns = (size.x / MASK_CELL).ceil() as usize + 1;
        let rows = (size.y / MASK_CELL).ceil() as usize + 1;
        let camera = self.scene.camera;
        let query = self.visibility_query();
        let mut hidden = Vec::with_capacity(columns * rows);
        for row in 0..rows {
            for column in 0..columns {
                // Edge samples sit just inside the viewport, where rays exist.
                let at = origin
                    + Vec2::new(column as f32 * MASK_CELL, row as f32 * MASK_CELL)
                        .min(size - Vec2::splat(0.5));
                let covered = self
                    .pointer_ray(at)
                    .and_then(|ray| frame.ray_point(ray))
                    .is_some_and(|p| {
                        !crate::viewport::visible_in_query(camera, query, frame.to_world(p))
                    });
                hidden.push(if covered { 1.0 } else { 0.0 });
            }
        }
        let mask = Rc::new(PlaneMask {
            origin,
            columns,
            rows,
            hidden,
        });
        PLANE_MASK.with(|cache| *cache.borrow_mut() = Some((key, mask.clone())));
        Some(mask)
    }
}

/// Splits a polyline into alternating visible and hidden pieces that share
/// their transition points.
fn split_by_visibility(points: &[Vec3], flags: Option<&Vec<bool>>) -> Vec<(bool, Vec<Vec3>)> {
    let Some(flags) = flags.filter(|f| f.len() == points.len()) else {
        return vec![(true, points.to_vec())];
    };
    let mut pieces: Vec<(bool, Vec<Vec3>)> = Vec::new();
    for (point, &visible) in points.iter().zip(flags) {
        match pieces.last_mut() {
            Some((state, piece)) if *state == visible => piece.push(*point),
            Some((_, piece)) => {
                let last = *piece.last().unwrap();
                pieces.push((visible, vec![last, *point]));
            }
            None => pieces.push((visible, vec![*point])),
        }
    }
    pieces
}

pub(crate) fn to_vec(p: Point<Pixels>) -> Vec2 {
    Vec2::new(p.x.into(), p.y.into())
}

pub(crate) fn to_point(v: Vec2) -> Point<Pixels> {
    point(px(v.x), px(v.y))
}

/// Projects world geometry to window pixels, clipping at the near plane.
pub(crate) struct Projector {
    pub matrix: Mat4,
    pub bounds: Bounds<Pixels>,
}

const NEAR_W: f32 = 1.0e-5;

impl Projector {
    fn screen(&self, clip: Vec4) -> Point<Pixels> {
        let ndc = clip.truncate() / clip.w;
        let x = f32::from(self.bounds.origin.x)
            + f32::from(self.bounds.size.width) * (ndc.x * 0.5 + 0.5);
        let y = f32::from(self.bounds.origin.y)
            + f32::from(self.bounds.size.height) * (0.5 - ndc.y * 0.5);
        point(px(x.clamp(-1.0e5, 1.0e5)), px(y.clamp(-1.0e5, 1.0e5)))
    }

    pub(crate) fn point(&self, world: Vec3) -> Option<Point<Pixels>> {
        let clip = self.matrix * world.extend(1.0);
        (clip.w > NEAR_W).then(|| self.screen(clip))
    }

    /// Visible screen runs of a world polyline.
    pub(crate) fn polyline(&self, points: &[Vec3]) -> Vec<Vec<Point<Pixels>>> {
        let mut runs = Vec::new();
        let mut run: Vec<Point<Pixels>> = Vec::new();
        let mut previous: Option<Vec4> = None;
        for world in points {
            let clip = self.matrix * world.extend(1.0);
            if let Some(last) = previous {
                match (last.w > NEAR_W, clip.w > NEAR_W) {
                    (true, true) => run.push(self.screen(clip)),
                    (true, false) => {
                        let t = (last.w - NEAR_W) / (last.w - clip.w);
                        run.push(self.screen(last.lerp(clip, t)));
                        runs.push(std::mem::take(&mut run));
                    }
                    (false, true) => {
                        let t = (last.w - NEAR_W) / (last.w - clip.w);
                        run.push(self.screen(last.lerp(clip, t)));
                        run.push(self.screen(clip));
                    }
                    (false, false) => {}
                }
            } else if clip.w > NEAR_W {
                run.push(self.screen(clip));
            }
            previous = Some(clip);
        }
        if run.len() > 1 {
            runs.push(run);
        }
        runs.retain(|r| r.len() > 1);
        runs
    }
}

/// Strokes many runs with as few paths as the vertex budget allows.
pub(crate) fn stroke_runs(
    window: &mut Window,
    runs: &[Vec<Point<Pixels>>],
    width: f32,
    color: Rgba,
) {
    let mut path = PathBuilder::stroke(px(width));
    let mut points = 0;
    let flush = |path: PathBuilder, window: &mut Window| {
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    };
    for run in runs {
        if run.len() < 2 {
            continue;
        }
        if points + run.len() > 4000 {
            flush(
                std::mem::replace(&mut path, PathBuilder::stroke(px(width))),
                window,
            );
            points = 0;
        }
        path.move_to(run[0]);
        for p in &run[1..] {
            path.line_to(*p);
        }
        points += run.len();
    }
    if points > 0 {
        flush(path, window);
    }
}

pub(crate) fn stroke_dashed(
    window: &mut Window,
    run: &[Point<Pixels>],
    width: f32,
    color: Rgba,
    dash: [f32; 2],
) {
    if run.len() < 2 {
        return;
    }
    let mut path = PathBuilder::stroke(px(width)).dash_array(&[px(dash[0]), px(dash[1])]);
    path.move_to(run[0]);
    for p in &run[1..] {
        path.line_to(*p);
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

pub(crate) fn fill_polygons(window: &mut Window, polygons: &[Vec<Point<Pixels>>], color: Rgba) {
    let mut path = PathBuilder::fill();
    let mut any = false;
    for polygon in polygons.iter().filter(|p| p.len() > 2) {
        path.move_to(polygon[0]);
        for p in &polygon[1..] {
            path.line_to(*p);
        }
        path.close();
        any = true;
    }
    if any && let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

pub(crate) fn dot(window: &mut Window, center: Point<Pixels>, diameter: f32, color: Rgba) {
    let radius = diameter * 0.5;
    window.paint_quad(
        fill(
            Bounds::new(
                center - point(px(radius), px(radius)),
                size(px(diameter), px(diameter)),
            ),
            color,
        )
        .corner_radii(px(radius)),
    );
}

pub(crate) fn ring(
    window: &mut Window,
    center: Point<Pixels>,
    radius: f32,
    width: f32,
    color: Rgba,
) {
    let points: Vec<_> = (0..=24)
        .map(|i| {
            let a = i as f32 / 24.0 * std::f32::consts::TAU;
            center + point(px(a.cos() * radius), px(a.sin() * radius))
        })
        .collect();
    stroke_runs(window, &[points], width, color);
}

/// A rounded text label centred on `center`. Returns its bounds.
#[allow(clippy::too_many_arguments)]
pub(crate) fn label(
    window: &mut Window,
    cx: &mut App,
    text: &str,
    center: Point<Pixels>,
    text_color: Rgba,
    background: Rgba,
    border: Option<Rgba>,
    font_size: f32,
) -> Bounds<Pixels> {
    let run = TextRun {
        len: text.len(),
        font: window.text_style().font(),
        color: Hsla::from(text_color),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped =
        window
            .text_system()
            .shape_line(text.to_owned().into(), px(font_size), &[run], None);
    let height = font_size + 9.0;
    let width = f32::from(shaped.width) + 12.0;
    let bounds = Bounds::new(
        center - point(px(width * 0.5), px(height * 0.5)),
        size(px(width), px(height)),
    );
    let mut quad = fill(bounds, background).corner_radii(px(4.0));
    if let Some(border) = border {
        quad = quad.border_widths(px(1.0)).border_color(Hsla::from(border));
    }
    window.paint_quad(quad);
    let _ = shaped.paint(
        bounds.origin + point(px(6.0), px(4.5)),
        px(font_size),
        window,
        cx,
    );
    bounds
}

pub(crate) fn paint(
    overlay: &SketchOverlay,
    matrix: Mat4,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let projector = Projector { matrix, bounds };
    let c = overlay.colors;
    window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
        if let Some(active) = &overlay.active {
            paint_scrim(active, &projector, c, window);
            paint_grid(active, &projector, c, window);
        }
        for (index, display) in overlay.others.iter().enumerate() {
            if Some(display.id) == overlay.active_id {
                continue;
            }
            let selected = overlay.selected.contains(&display.id);
            let drawing = overlay.active.is_some();
            for fills in &display.fills {
                let polygons: Vec<Vec<Point<Pixels>>> = fills
                    .iter()
                    .filter_map(|polygon| polygon.iter().map(|p| projector.point(*p)).collect())
                    .collect();
                fill_polygons(
                    window,
                    &polygons,
                    ink(c.accent, if selected { 0.12 } else { 0.05 }),
                );
            }
            let mut solid = Vec::new();
            let mut hidden = Vec::new();
            let flags = overlay.visibility.as_ref().and_then(|v| v.get(index));
            for (curve, (points, construction)) in display.curves.iter().enumerate() {
                let curve_flags = flags.and_then(|f| f.get(curve));
                for (visible, piece) in split_by_visibility(points, curve_flags) {
                    let runs = projector.polyline(&piece);
                    if !visible {
                        hidden.extend(runs);
                    } else if *construction {
                        for run in &runs {
                            stroke_dashed(window, run, 1.0, ink(c.text_muted, 0.7), [5.0, 4.0]);
                        }
                    } else {
                        solid.extend(runs);
                    }
                }
            }
            // Other sketches recede while one is being edited. Visible
            // curves use Draw's two-tone ink, so they read on any surface.
            let alpha = if drawing { 0.45 } else { 1.0 };
            let inks = SketchInk::new(c);
            stroke_runs(
                window,
                &hidden,
                1.0,
                ink(if selected { c.accent } else { c.text }, 0.28 * alpha),
            );
            stroke_runs(window, &solid, 3.8, ink_alpha(inks.halo, 0.8 * alpha));
            stroke_runs(
                window,
                &solid,
                if selected { 2.0 } else { 1.6 },
                if selected {
                    ink(c.accent, alpha)
                } else {
                    ink_alpha(inks.core, 0.92 * alpha)
                },
            );
        }
        if let Some(active) = &overlay.active {
            paint_active(active, &projector, c, window, cx);
        }
    });
}

/// The scene recedes behind the plane being drawn on. Sketching on a face,
/// the face itself (its outline, less its holes) is left out of the scrim,
/// so the surface being drawn on keeps its full shading.
fn paint_scrim(active: &ActiveOverlay, projector: &Projector, c: Colors, window: &mut Window) {
    let bounds = projector.bounds;
    let scrim = SketchInk::new(c).scrim;
    let mut polygons = vec![vec![
        bounds.origin,
        bounds.top_right(),
        bounds.bottom_right(),
        bounds.bottom_left(),
    ]];
    for outline in reference_loops(&active.reference) {
        let runs = plane_runs(&active.frame, projector, &outline);
        // A face crossing behind the camera cannot be cut out cleanly.
        if runs.len() != 1 {
            polygons.truncate(1);
            break;
        }
        polygons.extend(runs);
    }
    // One even-odd fill: the viewport, less the face, plus its holes.
    fill_polygons(window, &polygons, scrim);
}

/// Closed loops (in plane coordinates) of a face outline given as separate
/// curves, chained end to end. Curves that do not close a loop are left out.
fn reference_loops(reference: &[SketchCurve]) -> Vec<Vec<Vec2>> {
    let mut open: Vec<Vec<Vec2>> = Vec::new();
    let mut loops = Vec::new();
    let mut extent = 0.0_f32;
    for curve in reference.iter().filter(|c| c.is_valid()) {
        let polyline = curve.polyline(display_tolerance(curve));
        if polyline.len() < 2 {
            continue;
        }
        let (min, max) = curve.bounds();
        extent = extent.max((max - min).max_element());
        open.push(polyline);
    }
    let near = (extent * 1.0e-4).max(SKETCH_TOLERANCE * 4.0);
    let closes = |l: &[Vec2]| l.len() > 2 && l[0].distance(l[l.len() - 1]) <= near;
    while let Some(mut chain) = open.pop() {
        while !closes(&chain) {
            let end = chain[chain.len() - 1];
            let Some((index, reverse)) = open.iter().enumerate().find_map(|(i, l)| {
                if l[0].distance(end) <= near {
                    Some((i, false))
                } else if l[l.len() - 1].distance(end) <= near {
                    Some((i, true))
                } else {
                    None
                }
            }) else {
                break;
            };
            let mut next = open.swap_remove(index);
            if reverse {
                next.reverse();
            }
            chain.extend(next.into_iter().skip(1));
        }
        if closes(&chain) {
            chain.pop();
            loops.push(chain);
        }
    }
    loops
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn paint_grid(active: &ActiveOverlay, projector: &Projector, c: Colors, window: &mut Window) {
    let (minor, major) = active.grid;
    if !minor.is_finite() || minor <= 0.0 {
        return;
    }
    let frame = active.frame;
    let focus = active.focus;
    let axes = plane_axis_colors(&frame, c);
    // Reach of the grid: as far as the viewport corners land on the plane.
    let corners = [
        projector.bounds.origin,
        projector.bounds.top_right(),
        projector.bounds.bottom_left(),
        projector.bounds.bottom_right(),
    ];
    let inverse = projector.matrix.inverse();
    let mut reach: f32 = 0.0;
    for corner in corners {
        let ndc = Vec2::new(
            (f32::from(corner.x) - f32::from(projector.bounds.origin.x))
                / f32::from(projector.bounds.size.width).max(1.0)
                * 2.0
                - 1.0,
            1.0 - (f32::from(corner.y) - f32::from(projector.bounds.origin.y))
                / f32::from(projector.bounds.size.height).max(1.0)
                * 2.0,
        );
        let near = inverse.project_point3(ndc.extend(0.0));
        let far = inverse.project_point3(ndc.extend(1.0));
        let ray = forma_core::Ray {
            origin: near,
            direction: (far - near).normalize_or_zero(),
        };
        reach = reach.max(match frame.ray_point(ray) {
            Some(p) => p.distance(focus),
            None => major * 40.0,
        });
    }
    let reach = (reach * 1.08).min(minor * 260.0).max(minor * 4.0);
    let view = (
        to_vec(projector.bounds.origin),
        to_vec(projector.bounds.bottom_right()),
    );
    // Minor lines fade in as zooming spreads them apart.
    let minor_pixels = projector
        .point(frame.to_world(focus))
        .zip(projector.point(frame.to_world(focus + Vec2::X * minor)))
        .map(|(a, b)| to_vec(a).distance(to_vec(b)))
        .unwrap_or(20.0);
    let minor_fade = 0.55 + 0.45 * smoothstep(14.0, 30.0, minor_pixels);
    const LEVELS: usize = GRID_LEVELS;
    // Buckets of runs by (style, fade level).
    let mut buckets: Vec<Vec<Vec<Point<Pixels>>>> = vec![Vec::new(); 4 * LEVELS];
    let fade = |p: Vec2| 1.0 - smoothstep(reach * 0.45, reach, p.distance(focus));
    for axis in 0..2 {
        let start = ((focus[axis] - reach) / minor).floor() as i64;
        let end = ((focus[axis] + reach) / minor).ceil() as i64;
        for index in start..=end {
            let value = index as f32 * minor;
            let offset = value - focus[axis];
            let half = (reach * reach - offset * offset).max(0.0).sqrt();
            if half <= 0.0 {
                continue;
            }
            let style = if index == 0 {
                2 + axis
            } else if ((value / major).round() * major - value).abs() < minor * 1.0e-3 {
                1
            } else {
                0
            };
            const PIECES: usize = 10;
            let other = 1 - axis;
            for piece in 0..PIECES {
                let a = focus[other] - half + 2.0 * half * piece as f32 / PIECES as f32;
                let b = focus[other] - half + 2.0 * half * (piece + 1) as f32 / PIECES as f32;
                let mut p0 = Vec2::ZERO;
                let mut p1 = Vec2::ZERO;
                p0[axis] = value;
                p1[axis] = value;
                p0[other] = a;
                p1[other] = b;
                let strength = fade((p0 + p1) * 0.5);
                if (strength * LEVELS as f32).round() < 1.0 {
                    continue;
                }
                let world = [frame.to_world(p0), frame.to_world(p1)];
                for run in projector.polyline(&world) {
                    for (level, piece) in masked_runs(&run, strength, active.mask.as_deref(), view)
                    {
                        buckets[style * LEVELS + level - 1].push(piece);
                    }
                }
            }
        }
    }
    // Every line is stroked twice, dark under light, so the grid reads over
    // both bright and dark surfaces of the rendered frame.
    let base = [
        (c.text, c.canvas, 0.14 * minor_fade, 1.0),
        (c.text, c.canvas, 0.22, 1.0),
        // Index 0 lines are the sketch axes, coloured by the world axis
        // each runs along: the line x = 0 is the plane's y axis, the line
        // y = 0 its x axis.
        (axes[1], c.canvas, 0.6, 1.3),
        (axes[0], c.canvas, 0.6, 1.3),
    ];
    for (style, (light, dark, alpha, width)) in base.into_iter().enumerate() {
        for level in 0..LEVELS {
            let runs = &buckets[style * LEVELS + level];
            if !runs.is_empty() {
                let strength = (level + 1) as f32 / LEVELS as f32;
                stroke_runs(window, runs, width, ink(dark, alpha * strength * 0.8));
                stroke_runs(window, runs, width, ink(light, alpha * strength));
            }
        }
    }
}

/// Fade levels of the grid (see `paint_grid`).
const GRID_LEVELS: usize = 6;

/// A grid run cut into pieces by fade level: `strength` from the distance
/// fade, lowered where `mask` says geometry covers the plane. Pieces with
/// no strength left are dropped. Without a mask the run stays whole; with
/// one it is first clipped to `view` (its window corners), as only the part
/// on screen needs sampling.
fn masked_runs(
    run: &[Point<Pixels>],
    strength: f32,
    mask: Option<&PlaneMask>,
    view: (Vec2, Vec2),
) -> Vec<(usize, Vec<Point<Pixels>>)> {
    let level_of = |s: f32| ((s * GRID_LEVELS as f32).round() as usize).min(GRID_LEVELS);
    let Some(mask) = mask else {
        let level = level_of(strength);
        return if level == 0 {
            Vec::new()
        } else {
            vec![(level, run.to_vec())]
        };
    };
    let mut pieces = Vec::new();
    for pair in run.windows(2) {
        let Some((a, b)) = clip_segment(to_vec(pair[0]), to_vec(pair[1]), view) else {
            continue;
        };
        let steps = ((b - a).length() / 5.0).ceil().max(1.0) as usize;
        let level_at = |t: f32| level_of(strength * (1.0 - mask.hidden_at(a.lerp(b, t))));
        // Pieces change level halfway between the samples that differ.
        let mut start = 0.0;
        let mut level = level_at(0.0);
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let next = level_at(t);
            let end = if next != level {
                t - 0.5 / steps as f32
            } else if step == steps {
                1.0
            } else {
                continue;
            };
            if level > 0 && end > start {
                pieces.push((
                    level,
                    vec![to_point(a.lerp(b, start)), to_point(a.lerp(b, end))],
                ));
            }
            start = end;
            level = next;
            if step == steps && end < 1.0 && level > 0 {
                pieces.push((level, vec![to_point(a.lerp(b, end)), to_point(b)]));
            }
        }
    }
    pieces
}

/// The part of segment `a`–`b` inside the rectangle `low`–`high`.
fn clip_segment(a: Vec2, b: Vec2, (low, high): (Vec2, Vec2)) -> Option<(Vec2, Vec2)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for axis in 0..2 {
        if d[axis].abs() < 1.0e-9 {
            if a[axis] < low[axis] || a[axis] > high[axis] {
                return None;
            }
            continue;
        }
        let (mut near, mut far) = (
            (low[axis] - a[axis]) / d[axis],
            (high[axis] - a[axis]) / d[axis],
        );
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    Some((a + d * t0, a + d * t1))
}

fn plane_runs(frame: &Frame, projector: &Projector, points: &[Vec2]) -> Vec<Vec<Point<Pixels>>> {
    let world: Vec<Vec3> = points.iter().map(|p| frame.to_world(*p)).collect();
    projector.polyline(&world)
}

fn curve_runs(
    frame: &Frame,
    projector: &Projector,
    curve: &SketchCurve,
) -> Vec<Vec<Point<Pixels>>> {
    plane_runs(frame, projector, &curve.polyline(display_tolerance(curve)))
}

fn paint_active(
    active: &ActiveOverlay,
    projector: &Projector,
    c: Colors,
    window: &mut Window,
    cx: &mut App,
) {
    let frame = &active.frame;
    let analysis = &active.analysis;
    let inks = SketchInk::new(c);
    let screen = |p: Vec2| projector.point(frame.to_world(p));
    // Profile regions: what CAD features will extrude, marked by a light
    // tint alone (see `SketchInk` for the colour rule); a hovered region
    // adds an accent outline.
    let profiles: Vec<(bool, Vec<Vec<Point<Pixels>>>)> = analysis
        .profile_polygons
        .iter()
        .enumerate()
        .map(|(index, polygons)| {
            let projected = polygons
                .iter()
                .filter_map(|polygon| polygon.iter().map(|p| screen(*p)).collect())
                .collect();
            (active.hover.profile == Some(index), projected)
        })
        .collect();
    for (hovered, projected) in &profiles {
        fill_polygons(
            window,
            projected,
            ink(
                c.accent,
                if *hovered {
                    PROFILE_FILL_HOVERED
                } else {
                    PROFILE_FILL
                },
            ),
        );
    }
    for curve in &active.reference {
        for run in curve_runs(frame, projector, curve) {
            stroke_dashed(window, &run, 1.2, ink(c.text_muted, 0.75), [3.0, 3.0]);
        }
    }
    let mut normal = Vec::new();
    let mut selected = Vec::new();
    let mut hovered = Vec::new();
    let mut construction = Vec::new();
    for (index, entity) in analysis.entities.iter().enumerate() {
        let Some(polyline) = analysis.polylines.get(index) else {
            continue;
        };
        let runs = plane_runs(frame, projector, polyline);
        let is_selected = active.selection.contains(&index);
        let is_hovered = active.hover.entity == Some(index) && active.hover.trim.is_none();
        if entity.construction {
            let color = if is_selected {
                ink(c.accent, 1.0)
            } else if is_hovered {
                ink(c.text, 0.95)
            } else {
                ink(c.text_muted, 0.9)
            };
            construction.push((runs, color, is_selected));
        } else if is_selected {
            selected.extend(runs);
        } else if is_hovered {
            hovered.extend(runs);
        } else {
            normal.extend(runs);
        }
    }
    // Construction guides sit under real curves: thin, dashed and quiet.
    for (runs, color, is_selected) in &construction {
        for run in runs {
            stroke_dashed(window, run, 3.2, ink_alpha(inks.halo, 0.45), [7.0, 5.0]);
            stroke_dashed(
                window,
                run,
                if *is_selected { 1.8 } else { 1.3 },
                *color,
                [7.0, 5.0],
            );
        }
    }
    // Curves: a halo under every stroke keeps a 2 px core legible on any
    // surface; selection and hover add a soft accent glow.
    let all: Vec<_> = normal
        .iter()
        .chain(&hovered)
        .chain(&selected)
        .cloned()
        .collect();
    stroke_runs(window, &all, 4.6, inks.halo);
    stroke_runs(window, &selected, 8.0, ink(c.accent, 0.3));
    stroke_runs(window, &hovered, 7.0, ink(c.accent, 0.2));
    stroke_runs(window, &normal, 2.0, inks.core);
    stroke_runs(window, &hovered, 2.2, ink(c.accent, 0.85));
    stroke_runs(window, &selected, 2.4, ink(c.accent, 1.0));
    // The hovered region: an accent outline over its curves says which
    // region a click or an extrude would take.
    for (_, projected) in profiles.iter().filter(|(hovered, _)| *hovered) {
        let outlines: Vec<Vec<Point<Pixels>>> = projected
            .iter()
            .map(|polygon| {
                let mut closed = polygon.clone();
                closed.extend(polygon.first().copied());
                closed
            })
            .collect();
        stroke_runs(window, &outlines, 2.2, ink(c.accent, 1.0));
    }
    // One ink scheme for Draw: committed curves in the two-tone ink, the
    // curve being drawn in the accent, and every vertex, committed or just
    // placed, a 7 px light point with a thin dark ring. Open ends, which
    // keep a region from closing, add an amber ring, as the panel's "Open
    // ends" warning: the place to continue or close the outline.
    for (point, count) in &analysis.endpoints {
        if let Some(p) = screen(*point) {
            match count {
                0 => point_marker(window, p, 4.0, ink(c.text_muted, 1.0), inks.halo),
                1 => {
                    ring(window, p, OPEN_RING, 3.4, ink_alpha(inks.halo, 0.6));
                    ring(window, p, OPEN_RING, 1.6, ink(c.warning, 1.0));
                    point_marker(window, p, 7.0, inks.point, inks.point_ring);
                }
                _ => point_marker(window, p, 7.0, inks.point, inks.point_ring),
            }
        }
    }
    if let Some(trim) = &active.hover.trim {
        for run in curve_runs(frame, projector, &trim.removed) {
            stroke_runs(window, std::slice::from_ref(&run), 5.0, inks.halo);
            stroke_dashed(window, &run, 2.6, ink(c.danger, 1.0), [6.0, 3.0]);
        }
    }
    // Inference guides run under the preview they shape.
    if let Some(pointer) = &active.pointer {
        for guide in &pointer.snap.guides {
            let direction = (guide.to - guide.from).normalize_or_zero();
            let extend = 36.0 / pointer.pixels_per_unit.max(1.0e-6);
            let ends = [guide.from, guide.to + direction * extend];
            for run in plane_runs(frame, projector, &ends) {
                stroke_dashed(window, &run, 3.0, ink_alpha(inks.halo, 0.45), [5.0, 4.0]);
                stroke_dashed(
                    window,
                    &run,
                    1.2,
                    guide_color(guide.kind, c, plane_axis_colors(frame, c)),
                    [5.0, 4.0],
                );
            }
        }
    }
    // Tool preview.
    for curve in &active.preview_guides {
        for run in curve_runs(frame, projector, curve) {
            stroke_dashed(window, &run, 1.2, ink(c.accent, 0.85), [4.0, 4.0]);
        }
    }
    let preview: Vec<_> = active
        .preview
        .iter()
        .flat_map(|curve| curve_runs(frame, projector, curve))
        .collect();
    if active.preview_construction {
        for run in &preview {
            stroke_dashed(window, run, 3.4, ink_alpha(inks.halo, 0.5), [7.0, 5.0]);
            stroke_dashed(window, run, 1.8, ink(c.accent, 1.0), [7.0, 5.0]);
        }
    } else {
        stroke_runs(window, &preview, 4.8, inks.halo);
        stroke_runs(window, &preview, 2.2, ink(c.accent, 1.0));
    }
    for arc in &active.arcs {
        let steps = 32;
        let points: Vec<Vec2> = (0..=steps)
            .map(|i| {
                let a = arc.from + (arc.to - arc.from) * i as f32 / steps as f32;
                arc.center + Vec2::from_angle(a) * arc.radius
            })
            .collect();
        for run in plane_runs(frame, projector, &points) {
            let run = std::slice::from_ref(&run);
            stroke_runs(window, run, 3.0, ink_alpha(inks.halo, 0.6));
            stroke_runs(window, run, 1.0, inks.annotation);
        }
        let reference = [arc.center, arc.center + Vec2::X * arc.radius * 1.35];
        for run in plane_runs(frame, projector, &reference) {
            stroke_dashed(window, &run, 1.0, inks.annotation, [3.0, 3.0]);
        }
    }
    for point in &active.placed {
        if let Some(p) = screen(*point) {
            point_marker(window, p, 7.0, inks.point, inks.point_ring);
        }
    }
    // The snap marker and what the snap means.
    if let Some(pointer) = &active.pointer {
        let snap = &pointer.snap;
        // Direction inferences describe the segment being drawn, so they
        // attach to it as constraint chips at its midpoint.
        let mut chips: Vec<GuideKind> = Vec::new();
        if active.segment.is_some() {
            for kind in snap
                .guides
                .iter()
                .map(|g| g.kind)
                .chain(guide_kind(snap.kind))
            {
                if kind != GuideKind::Aligned && !chips.contains(&kind) {
                    chips.push(kind);
                }
            }
        }
        if let Some((a, b, away)) = active.segment
            && !chips.is_empty()
            && let (Some(pa), Some(pb)) = (screen(a), screen(b))
        {
            let (pa, pb) = (to_vec(pa), to_vec(pb));
            let along = (pb - pa).normalize_or_zero();
            let side = screen((a + b) * 0.5 + away * 1.0e-3)
                .map(|p| to_vec(p) - (pa + pb) * 0.5)
                .unwrap_or(Vec2::ZERO);
            let mut side_normal = (side - along * side.dot(along)).normalize_or_zero();
            if side_normal == Vec2::ZERO {
                side_normal = along.perp();
            }
            // Opposite the dimension, stacked outward from the segment.
            let mut gap = 8.0;
            for kind in chips {
                let size = constraint_chip_size(window, kind);
                let reach =
                    (side_normal.x * size.x).abs() * 0.5 + (side_normal.y * size.y).abs() * 0.5;
                let center = (pa + pb) * 0.5 - side_normal * (gap + reach);
                paint_constraint_chip(
                    window,
                    cx,
                    kind,
                    to_point(center),
                    c,
                    plane_axis_colors(frame, c),
                );
                gap += reach * 2.0 + 4.0;
            }
        }
        if let Some(p) = screen(snap.point) {
            paint_snap_marker(window, p, snap.kind, active.drawing, c);
            let attached = active.segment.is_some() && guide_kind(snap.kind).is_some();
            let text = snap.kind.label();
            if !text.is_empty() && !attached {
                // Point snaps name the point, tucked against its marker.
                let width = shaped_width(window, text, 10.0) + 12.0;
                let center = p + point(px(10.0 + width * 0.5), px(17.0));
                label(
                    window,
                    cx,
                    text,
                    center,
                    ink(c.text_muted, 1.0),
                    ink(c.overlay, 0.96),
                    Some(gpui::rgba(c.hud_border)),
                    10.0,
                );
            }
        }
    }
    // Live dimensions.
    for dim in &active.dims {
        paint_dimension(dim, frame, projector, &inks, c, window, cx);
    }
    if let Some((start, end)) = active.marquee {
        let (low, high) = (start.min(end), start.max(end));
        let bounds = Bounds::new(to_point(low), size(px(high.x - low.x), px(high.y - low.y)));
        let crossing = end.x < start.x;
        window.paint_quad(fill(
            bounds,
            ink(c.accent, if crossing { 0.05 } else { 0.08 }),
        ));
        let outline = [
            low,
            Vec2::new(high.x, low.y),
            high,
            Vec2::new(low.x, high.y),
            low,
        ]
        .map(to_point);
        if crossing {
            stroke_dashed(window, &outline, 1.0, ink(c.accent, 0.9), [4.0, 3.0]);
        } else {
            stroke_runs(window, &[outline.to_vec()], 1.0, ink(c.accent, 0.9));
        }
    }
}

fn ink_alpha(color: Rgba, alpha: f32) -> Rgba {
    Rgba {
        a: color.a * alpha,
        ..color
    }
}

/// Radius of the amber ring around an open end.
const OPEN_RING: f32 = 7.0;
/// Accent fill of a closed profile, and of the one under the pointer.
const PROFILE_FILL: f32 = 0.15;
const PROFILE_FILL_HOVERED: f32 = 0.3;

/// A filled point with a thin ring around it.
fn point_marker(window: &mut Window, center: Point<Pixels>, diameter: f32, fill: Rgba, ring: Rgba) {
    dot(window, center, diameter + 2.0, ring);
    dot(window, center, diameter, fill);
}

/// The colours of the plane's x and y axes: the world axis each runs
/// along, so a ground sketch's x is red (world X) and its y blue (world
/// -Z), as the view gizmo shows them. An axis along no world axis (a
/// sketch on a slanted face) is neutral.
pub(crate) fn plane_axis_colors(frame: &Frame, c: Colors) -> [u32; 2] {
    let origin = frame.to_world(Vec2::ZERO);
    [Vec2::X, Vec2::Y].map(|direction| {
        let along = (frame.to_world(direction) - origin).normalize_or_zero();
        (0..3)
            .find(|&axis| along[axis].abs() > 0.999)
            .map_or(c.text_muted, |axis| c.axis[axis])
    })
}

fn guide_color(kind: GuideKind, c: Colors, axes: [u32; 2]) -> Rgba {
    match kind {
        GuideKind::Horizontal => ink(axes[0], 0.95),
        GuideKind::Vertical => ink(axes[1], 0.95),
        GuideKind::Parallel | GuideKind::Perpendicular => ink(c.accent, 0.95),
        GuideKind::Aligned => ink(c.text_muted, 0.9),
    }
}

/// The direction inference a snap kind stands for.
fn guide_kind(kind: SnapKind) -> Option<GuideKind> {
    match kind {
        SnapKind::Horizontal => Some(GuideKind::Horizontal),
        SnapKind::Vertical => Some(GuideKind::Vertical),
        SnapKind::Parallel => Some(GuideKind::Parallel),
        SnapKind::Perpendicular => Some(GuideKind::Perpendicular),
        _ => None,
    }
}

fn constraint_label(kind: GuideKind) -> &'static str {
    match kind {
        GuideKind::Horizontal => "Horizontal",
        GuideKind::Vertical => "Vertical",
        GuideKind::Parallel => "Parallel",
        GuideKind::Perpendicular => "Perpendicular",
        GuideKind::Aligned => "Aligned",
    }
}

const CHIP_TEXT: f32 = 10.0;
const CHIP_HEIGHT: f32 = 18.0;
const CHIP_GLYPH: f32 = 12.0;

fn constraint_chip_size(window: &mut Window, kind: GuideKind) -> Vec2 {
    let text = shaped_width(window, constraint_label(kind), CHIP_TEXT);
    Vec2::new(6.0 + CHIP_GLYPH + 4.0 + text + 7.0, CHIP_HEIGHT)
}

/// A small chip naming a constraint the segment being drawn will have,
/// with its glyph in the inference line's colour.
fn paint_constraint_chip(
    window: &mut Window,
    cx: &mut App,
    kind: GuideKind,
    center: Point<Pixels>,
    c: Colors,
    axes: [u32; 2],
) {
    let size_v = constraint_chip_size(window, kind);
    let origin = to_vec(center) - size_v * 0.5;
    let bounds = Bounds::new(to_point(origin), size(px(size_v.x), px(size_v.y)));
    window.paint_quad(
        fill(bounds, ink(c.overlay, 0.97))
            .corner_radii(px(5.0))
            .border_widths(px(1.0))
            .border_color(Hsla::from(gpui::rgba(c.hud_border))),
    );
    let color = guide_color(kind, c, axes);
    let g = origin + Vec2::new(6.0 + CHIP_GLYPH * 0.5, CHIP_HEIGHT * 0.5);
    let at = |x: f32, y: f32| to_point(g + Vec2::new(x, y));
    let h = CHIP_GLYPH * 0.5 - 1.0;
    let strokes: Vec<Vec<Point<Pixels>>> = match kind {
        GuideKind::Horizontal => vec![
            vec![at(-h, 0.0), at(h, 0.0)],
            vec![at(-h, -2.5), at(-h, 2.5)],
            vec![at(h, -2.5), at(h, 2.5)],
        ],
        GuideKind::Vertical => vec![
            vec![at(0.0, -h), at(0.0, h)],
            vec![at(-2.5, -h), at(2.5, -h)],
            vec![at(-2.5, h), at(2.5, h)],
        ],
        GuideKind::Parallel => vec![
            vec![at(-h + 1.0, h), at(-1.0, -h)],
            vec![at(1.0, h), at(h - 1.0, -h)],
        ],
        GuideKind::Perpendicular => vec![vec![at(-h, h), at(h, h)], vec![at(0.0, h), at(0.0, -h)]],
        GuideKind::Aligned => vec![vec![at(-h, 0.0), at(h, 0.0)]],
    };
    stroke_runs(window, &strokes, 1.6, color);
    let text = constraint_label(kind);
    let shaped = shape(window, text, CHIP_TEXT, ink(c.text, 0.92), false);
    let _ = shaped.paint(
        to_point(
            origin
                + Vec2::new(
                    6.0 + CHIP_GLYPH + 4.0,
                    (CHIP_HEIGHT - CHIP_TEXT) * 0.5 - 1.5,
                ),
        ),
        px(CHIP_TEXT + 3.0),
        window,
        cx,
    );
}

fn shape(
    window: &mut Window,
    text: &str,
    font_size: f32,
    color: Rgba,
    tabular: bool,
) -> gpui::ShapedLine {
    let mut font = window.text_style().font();
    if tabular {
        font.features = gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]));
    }
    let run = TextRun {
        len: text.len(),
        font,
        color: Hsla::from(color),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text.to_owned().into(), px(font_size), &[run], None)
}

pub(crate) fn shaped_width(window: &mut Window, text: &str, font_size: f32) -> f32 {
    f32::from(shape(window, text, font_size, ink(0, 1.0), false).width)
}

/// Offset of a linear dimension line from the measured geometry, and how
/// far extension lines start from it and run past the dimension line.
const DIM_OFFSET: f32 = 18.0;
const DIM_GAP: f32 = 3.0;
const DIM_OVERSHOOT: f32 = 4.0;
const DIM_ARROW: f32 = 7.0;
const FIELD_TEXT: f32 = 12.0;
const FIELD_HEIGHT: f32 = 22.0;

fn arrowhead(window: &mut Window, tip: Vec2, direction: Vec2, color: Rgba) {
    let side = direction.perp();
    let back = tip - direction * DIM_ARROW;
    let triangle = [tip, back + side * 2.8, back - side * 2.8, tip].map(to_point);
    fill_polygons(window, &[triangle.to_vec()], color);
}

/// A live dimension: for a span, extension lines and an arrowed dimension
/// line offset from the geometry; the value sits in an input-style field,
/// accented while it takes keystrokes.
fn paint_dimension(
    dim: &DimLabel,
    frame: &Frame,
    projector: &Projector,
    inks: &SketchInk,
    c: Colors,
    window: &mut Window,
    cx: &mut App,
) {
    let screen = |p: Vec2| projector.point(frame.to_world(p)).map(to_vec);
    let Some(anchor) = screen(dim.at) else {
        return;
    };
    let away = screen(dim.at + dim.away * 1.0e-3)
        .map(|p| (p - anchor).normalize_or_zero())
        .filter(|v| *v != Vec2::ZERO)
        .unwrap_or(Vec2::NEG_Y);
    let typed = dim.locked;
    let (text_color, background, border, border_width) = if dim.invalid {
        (
            ink(c.danger, 1.0),
            ink(c.overlay, 0.98),
            ink(c.danger, 1.0),
            1.5,
        )
    } else if dim.active {
        (
            ink(c.text, 1.0),
            ink(c.overlay, 0.98),
            ink(c.accent, 1.0),
            1.5,
        )
    } else if typed {
        (
            ink(c.accent_text, 1.0),
            ink(c.accent_soft, 0.98),
            ink(c.accent_border, 1.0),
            1.0,
        )
    } else {
        (
            ink(c.text, 1.0),
            ink(c.overlay, 0.98),
            ink(c.border_strong, 1.0),
            1.0,
        )
    };
    let shaped = shape(window, &dim.text, FIELD_TEXT, text_color, true);
    let text_width = f32::from(shaped.width);
    let caret_width = if dim.active && typed { 3.0 } else { 0.0 };
    let field = Vec2::new((text_width + caret_width + 18.0).max(44.0), FIELD_HEIGHT);
    let reach = |normal: Vec2| (normal.x * field.x).abs() * 0.5 + (normal.y * field.y).abs() * 0.5;
    let span = dim
        .span
        .and_then(|(a, b)| Some((screen(a)?, screen(b)?)))
        .filter(|(a, b)| a.distance(*b) > 2.0);
    let halo = ink_alpha(inks.halo, 0.55);
    let center = match span {
        Some((a, b)) if dim.radial => {
            // The radius is the dimension line, arrowed where it meets the rim.
            let along = (b - a).normalize();
            let mut normal = (away - along * away.dot(along)).normalize_or_zero();
            if normal == Vec2::ZERO {
                normal = along.perp();
            }
            let line = [vec![to_point(a), to_point(b)]];
            stroke_runs(window, &line, 3.0, halo);
            stroke_runs(window, &line, 1.0, inks.annotation);
            arrowhead(window, b + along * 0.5, along, halo);
            arrowhead(window, b, along, inks.annotation);
            (a + b) * 0.5 + normal * (reach(normal) + 6.0)
        }
        Some((a, b)) => {
            let along = (b - a).normalize();
            let mut normal = (away - along * away.dot(along)).normalize_or_zero();
            if normal == Vec2::ZERO {
                normal = along.perp();
            }
            // Far enough out that the value, centred on the dimension line,
            // clears the geometry it measures.
            let offset = DIM_OFFSET.max(reach(normal) + 7.0);
            let (qa, qb) = (a + normal * offset, b + normal * offset);
            let extensions = [
                vec![a + normal * DIM_GAP, a + normal * (offset + DIM_OVERSHOOT)],
                vec![b + normal * DIM_GAP, b + normal * (offset + DIM_OVERSHOOT)],
            ]
            .map(|run| run.into_iter().map(to_point).collect::<Vec<_>>());
            // The value sits in a break of the dimension line when it fits
            // between the arrows; otherwise the arrows point in from outside
            // and the value moves clear of the line.
            let length = qa.distance(qb);
            let along_reach = (along.x * field.x).abs() * 0.5 + (along.y * field.y).abs() * 0.5;
            let inside = length >= along_reach * 2.0 + DIM_ARROW * 2.0 + 8.0;
            let line: Vec<Vec<Point<Pixels>>> = if inside {
                let middle = (qa + qb) * 0.5;
                vec![
                    vec![to_point(qa), to_point(middle - along * (along_reach + 4.0))],
                    vec![to_point(middle + along * (along_reach + 4.0)), to_point(qb)],
                ]
            } else {
                vec![
                    vec![to_point(qa - along * (DIM_ARROW + 6.0)), to_point(qa)],
                    vec![to_point(qa), to_point(qb)],
                    vec![to_point(qb), to_point(qb + along * (DIM_ARROW + 6.0))],
                ]
            };
            stroke_runs(window, &extensions, 3.0, halo);
            stroke_runs(window, &line, 3.0, halo);
            stroke_runs(window, &extensions, 1.0, inks.annotation);
            stroke_runs(window, &line, 1.0, inks.annotation);
            let (da, db) = if inside {
                (-along, along)
            } else {
                (along, -along)
            };
            for (tip, direction) in [(qa, da), (qb, db)] {
                arrowhead(window, tip + direction * 0.5, direction, halo);
                arrowhead(window, tip, direction, inks.annotation);
            }
            if inside {
                (qa + qb) * 0.5
            } else {
                (qa + qb) * 0.5 + normal * (reach(normal) + 5.0)
            }
        }
        None => anchor + away * (DIM_OFFSET + reach(away)),
    };
    let origin = center - field * 0.5;
    let bounds = Bounds::new(to_point(origin), size(px(field.x), px(field.y)));
    window.paint_quad(
        fill(bounds, background)
            .corner_radii(px(R_FIELD))
            .border_widths(px(border_width))
            .border_color(Hsla::from(border)),
    );
    let text_origin = origin
        + Vec2::new(
            (field.x - text_width - caret_width) * 0.5,
            (FIELD_HEIGHT - FIELD_TEXT) * 0.5 - 2.0,
        );
    if dim.active && !typed && !dim.invalid {
        // The live value reads as selected: typing replaces it.
        let highlight = Bounds::new(
            to_point(text_origin + Vec2::new(-2.0, 0.0)),
            size(px(text_width + 4.0), px(FIELD_TEXT + 4.0)),
        );
        window.paint_quad(fill(highlight, ink(c.accent, 0.22)).corner_radii(px(2.0)));
    }
    let _ = shaped.paint(to_point(text_origin), px(FIELD_TEXT + 4.0), window, cx);
    if dim.active && typed {
        let x = text_origin.x + text_width + 1.5;
        let caret = Bounds::new(
            to_point(Vec2::new(x, origin.y + 4.0)),
            size(px(1.0), px(FIELD_HEIGHT - 8.0)),
        );
        window.paint_quad(fill(caret, ink(c.accent, 1.0)));
    }
}

const R_FIELD: f32 = 4.0;

fn paint_snap_marker(
    window: &mut Window,
    p: Point<Pixels>,
    kind: SnapKind,
    drawing: bool,
    c: Colors,
) {
    let accent = ink(c.accent, 1.0);
    let halo = ink(c.surface, 0.85);
    let at = |x: f32, y: f32| p + point(px(x), px(y));
    match kind {
        SnapKind::Endpoint => {
            let square =
                |side: f32| Bounds::new(at(-side * 0.5, -side * 0.5), size(px(side), px(side)));
            window.paint_quad(fill(square(11.0), halo).corner_radii(px(2.0)));
            window.paint_quad(fill(square(8.0), accent).corner_radii(px(1.5)));
        }
        SnapKind::Midpoint => {
            let triangle = [at(0.0, -6.0), at(5.5, 4.0), at(-5.5, 4.0), at(0.0, -6.0)].to_vec();
            stroke_runs(window, std::slice::from_ref(&triangle), 4.0, halo);
            fill_polygons(window, std::slice::from_ref(&triangle), accent);
        }
        SnapKind::Center => {
            ring(window, p, 5.5, 3.5, halo);
            ring(window, p, 5.5, 1.6, accent);
            dot(window, p, 3.0, accent);
        }
        SnapKind::Quadrant => {
            let diamond = [
                at(0.0, -6.0),
                at(6.0, 0.0),
                at(0.0, 6.0),
                at(-6.0, 0.0),
                at(0.0, -6.0),
            ]
            .to_vec();
            stroke_runs(window, std::slice::from_ref(&diamond), 3.5, halo);
            stroke_runs(window, std::slice::from_ref(&diamond), 1.6, accent);
        }
        SnapKind::Intersection => {
            let a = vec![at(-5.0, -5.0), at(5.0, 5.0)];
            let b = vec![at(-5.0, 5.0), at(5.0, -5.0)];
            stroke_runs(window, &[a.clone(), b.clone()], 4.5, halo);
            stroke_runs(window, &[a, b], 2.0, accent);
        }
        SnapKind::OnCurve => {
            let hourglass = vec![
                at(-4.5, -5.0),
                at(4.5, -5.0),
                at(-4.5, 5.0),
                at(4.5, 5.0),
                at(-4.5, -5.0),
            ];
            stroke_runs(window, std::slice::from_ref(&hourglass), 3.5, halo);
            stroke_runs(window, std::slice::from_ref(&hourglass), 1.4, accent);
        }
        SnapKind::Origin => {
            ring(window, p, 6.5, 3.5, halo);
            ring(window, p, 6.5, 1.5, accent);
            let cross = [
                vec![at(-9.0, 0.0), at(9.0, 0.0)],
                vec![at(0.0, -9.0), at(0.0, 9.0)],
            ];
            stroke_runs(window, &cross, 1.2, accent);
        }
        SnapKind::Horizontal
        | SnapKind::Vertical
        | SnapKind::Parallel
        | SnapKind::Perpendicular
        | SnapKind::Aligned => {
            dot(window, p, 8.0, halo);
            dot(window, p, 5.0, accent);
        }
        SnapKind::Grid | SnapKind::Free => {
            if drawing {
                let color = ink(c.text, 0.9);
                let cross = [
                    vec![at(-5.0, 0.0), at(5.0, 0.0)],
                    vec![at(0.0, -5.0), at(0.0, 5.0)],
                ];
                stroke_runs(window, &cross, 3.0, halo);
                stroke_runs(window, &cross, 1.2, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_runs_clip_to_the_view() {
        let view = (Vec2::ZERO, Vec2::new(100.0, 50.0));
        let (a, b) = clip_segment(Vec2::new(-50.0, 25.0), Vec2::new(150.0, 25.0), view).unwrap();
        assert!(a.distance(Vec2::new(0.0, 25.0)) < 1.0e-4);
        assert!(b.distance(Vec2::new(100.0, 25.0)) < 1.0e-4);
        assert!(clip_segment(Vec2::new(-5.0, 60.0), Vec2::new(200.0, 70.0), view).is_none());
        let inside = clip_segment(Vec2::new(10.0, 10.0), Vec2::new(20.0, 30.0), view).unwrap();
        assert_eq!(inside, (Vec2::new(10.0, 10.0), Vec2::new(20.0, 30.0)));
    }

    /// Grid lines stop where geometry covers the plane and keep their
    /// strength elsewhere.
    #[test]
    fn grid_runs_fade_out_under_geometry() {
        // A 7 by 3 sample mask (98 by 28 pixels) covered in its middle columns.
        let columns = 8;
        let hidden = (0..3)
            .flat_map(|_| (0..columns).map(|c| if (3..=4).contains(&c) { 1.0 } else { 0.0 }))
            .collect();
        let mask = PlaneMask {
            origin: Vec2::ZERO,
            columns,
            rows: 3,
            hidden,
        };
        let run = vec![
            to_point(Vec2::new(0.0, 14.0)),
            to_point(Vec2::new(98.0, 14.0)),
        ];
        let view = (Vec2::ZERO, Vec2::new(98.0, 28.0));
        let pieces = masked_runs(&run, 1.0, Some(&mask), view);
        let full: Vec<_> = pieces
            .iter()
            .filter(|(level, _)| *level == GRID_LEVELS)
            .collect();
        assert_eq!(full.len(), 2, "{} full-strength pieces", full.len());
        // Nothing is drawn over the covered columns (42 to 56 pixels).
        for (_, piece) in &pieces {
            let (a, b) = (to_vec(piece[0]).x, to_vec(piece[1]).x);
            assert!(b <= 42.0 + 1.0e-3 || a >= 56.0 - 1.0e-3, "{a}..{b}");
        }
        // Unmasked runs stay whole.
        assert_eq!(masked_runs(&run, 1.0, None, view).len(), 1);
        assert!(masked_runs(&run, 0.01, None, view).is_empty());
    }
}

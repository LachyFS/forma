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
}

/// Per sketch, per curve, per point: whether geometry hides the point.
pub(crate) type Visibility = Vec<Vec<Vec<bool>>>;

pub(crate) struct SketchOverlay {
    pub colors: Colors,
    pub others: Rc<Vec<SketchDisplay>>,
    pub visibility: Option<Rc<Visibility>>,
    pub selected: BTreeSet<u64>,
    pub active_id: Option<u64>,
    pub active: Option<ActiveOverlay>,
}

fn ink(color: u32, alpha: f32) -> Rgba {
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
    pub(crate) fn sketch_displays(&self) -> Rc<Vec<SketchDisplay>> {
        let key = (self.scene_revision, self.sketch.revision);
        let mut cache = self.sketch.display.borrow_mut();
        if let Some((cached, displays)) = cache.as_ref()
            && *cached == key
        {
            return displays.clone();
        }
        let mut displays = Vec::new();
        for object in &self.scene.objects {
            let ObjectData::Sketch { sketch } = &object.data else {
                continue;
            };
            if !self.scene.is_effectively_visible(object.id) {
                continue;
            }
            let Some(matrix) = self.scene.world_transform(object.id) else {
                continue;
            };
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
            displays.push(SketchDisplay {
                id: object.id,
                curves,
                fills,
            });
        }
        let displays = Rc::new(displays);
        *cache = Some((key, displays.clone()));
        displays
    }

    /// Which points of each inactive sketch are hidden behind geometry, so
    /// overlays read like hidden lines. Rays are cast every few pixels along
    /// each curve and cached until the camera or the scene changes.
    fn sketch_visibility(&self, displays: &Rc<Vec<SketchDisplay>>) -> Option<Rc<Visibility>> {
        if displays.is_empty() || self.scene.mesh_instances().next().is_none() {
            return None;
        }
        let key = (
            self.scene_revision,
            self.sketch.revision,
            self.scene.camera,
            self.bounds.get().size,
        );
        let mut cache = self.sketch.visibility.borrow_mut();
        if let Some((cached, visibility)) = cache.as_ref()
            && *cached == key
        {
            return Some(visibility.clone());
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
        *cache = Some((key, visibility.clone()));
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
        let mut best: Option<(f32, u64)> = None;
        for display in displays.iter() {
            if !self.scene.object(display.id).is_some_and(|o| o.selectable) {
                continue;
            }
            for (points, _) in &display.curves {
                for run in projector.polyline(points) {
                    for pair in run.windows(2) {
                        let (a, b) = (to_vec(pair[0]), to_vec(pair[1]));
                        let distance = segment_distance(screen, a, b);
                        if distance <= radius && best.is_none_or(|(d, _)| distance < d) {
                            best = Some((distance, display.id));
                        }
                    }
                }
            }
        }
        best.map(|(_, id)| id)
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

fn to_vec(p: Point<Pixels>) -> Vec2 {
    Vec2::new(p.x.into(), p.y.into())
}

fn to_point(v: Vec2) -> Point<Pixels> {
    point(px(v.x), px(v.y))
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let d = b - a;
    let t = ((p - a).dot(d) / d.length_squared().max(1.0e-6)).clamp(0.0, 1.0);
    p.distance(a + d * t)
}

/// Projects world geometry to window pixels, clipping at the near plane.
struct Projector {
    matrix: Mat4,
    bounds: Bounds<Pixels>,
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

    fn point(&self, world: Vec3) -> Option<Point<Pixels>> {
        let clip = self.matrix * world.extend(1.0);
        (clip.w > NEAR_W).then(|| self.screen(clip))
    }

    /// Visible screen runs of a world polyline.
    fn polyline(&self, points: &[Vec3]) -> Vec<Vec<Point<Pixels>>> {
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
fn stroke_runs(window: &mut Window, runs: &[Vec<Point<Pixels>>], width: f32, color: Rgba) {
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

fn stroke_dashed(
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

fn fill_polygons(window: &mut Window, polygons: &[Vec<Point<Pixels>>], color: Rgba) {
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

fn dot(window: &mut Window, center: Point<Pixels>, diameter: f32, color: Rgba) {
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

fn ring(window: &mut Window, center: Point<Pixels>, radius: f32, width: f32, color: Rgba) {
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
fn label(
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
            // Other sketches recede while one is being edited.
            let alpha = if drawing { 0.45 } else { 1.0 };
            stroke_runs(
                window,
                &hidden,
                1.0,
                ink(if selected { c.accent } else { c.text }, 0.28 * alpha),
            );
            stroke_runs(window, &solid, 3.5, ink(c.surface, 0.55 * alpha));
            stroke_runs(
                window,
                &solid,
                if selected { 2.0 } else { 1.4 },
                ink(if selected { c.accent } else { c.text }, 0.9 * alpha),
            );
        }
        if let Some(active) = &overlay.active {
            paint_active(active, &projector, c, window, cx);
        }
    });
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
    // Minor lines fade in as zooming spreads them apart.
    let minor_pixels = projector
        .point(frame.to_world(focus))
        .zip(projector.point(frame.to_world(focus + Vec2::X * minor)))
        .map(|(a, b)| to_vec(a).distance(to_vec(b)))
        .unwrap_or(20.0);
    let minor_fade = 0.55 + 0.45 * smoothstep(14.0, 30.0, minor_pixels);
    const LEVELS: usize = 6;
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
                let level = (fade((p0 + p1) * 0.5) * LEVELS as f32).round() as usize;
                if level == 0 {
                    continue;
                }
                let world = [frame.to_world(p0), frame.to_world(p1)];
                for run in projector.polyline(&world) {
                    buckets[style * LEVELS + level.min(LEVELS) - 1].push(run);
                }
            }
        }
    }
    // Every line is stroked twice, dark under light, so the grid reads over
    // both bright and dark surfaces of the rendered frame.
    let base = [
        (c.text, c.canvas, 0.14 * minor_fade, 1.0),
        (c.text, c.canvas, 0.22, 1.0),
        // Index 0 lines are the sketch axes: x is horizontal, y vertical.
        (c.axis[1], c.canvas, 0.6, 1.3),
        (c.axis[0], c.canvas, 0.6, 1.3),
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
    let screen = |p: Vec2| projector.point(frame.to_world(p));
    // Profile regions: what CAD features will extrude.
    for (index, polygons) in analysis.profile_polygons.iter().enumerate() {
        let projected: Vec<Vec<Point<Pixels>>> = polygons
            .iter()
            .filter_map(|polygon| polygon.iter().map(|p| screen(*p)).collect())
            .collect();
        let hovered = active.hover.profile == Some(index);
        fill_polygons(
            window,
            &projected,
            ink(c.accent, if hovered { 0.34 } else { 0.14 }),
        );
    }
    for curve in &active.reference {
        for run in curve_runs(frame, projector, curve) {
            stroke_dashed(window, &run, 1.0, ink(c.text_muted, 0.55), [3.0, 3.0]);
        }
    }
    // Curves: a soft halo under every stroke keeps them legible on any surface.
    let mut normal = Vec::new();
    let mut selected = Vec::new();
    let mut hovered = Vec::new();
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
                ink(c.text, 0.9)
            } else {
                ink(c.text_muted, 0.85)
            };
            for run in &runs {
                stroke_dashed(window, run, 3.0, ink(c.surface, 0.35), [6.0, 4.0]);
                stroke_dashed(
                    window,
                    run,
                    if is_selected { 1.8 } else { 1.2 },
                    color,
                    [6.0, 4.0],
                );
            }
        } else if is_selected {
            selected.extend(runs);
        } else if is_hovered {
            hovered.extend(runs);
        } else {
            normal.extend(runs);
        }
    }
    let all: Vec<_> = normal
        .iter()
        .chain(&hovered)
        .chain(&selected)
        .cloned()
        .collect();
    stroke_runs(window, &all, 3.6, ink(c.canvas, 0.5));
    stroke_runs(window, &normal, 1.6, ink(c.text, 0.95));
    stroke_runs(window, &hovered, 2.2, ink(c.accent, 0.75));
    stroke_runs(window, &selected, 2.4, ink(c.accent, 1.0));
    // Endpoints: connected points are solid, open ends are rings.
    for (point, count) in &analysis.endpoints {
        if let Some(p) = screen(*point) {
            if *count > 1 {
                dot(window, p, 4.0, ink(c.text, 0.9));
            } else {
                dot(window, p, 7.0, ink(c.surface, 0.8));
                ring(window, p, 3.0, 1.3, ink(c.warning, 0.95));
            }
        }
    }
    if let Some(trim) = &active.hover.trim {
        for run in curve_runs(frame, projector, &trim.removed) {
            stroke_runs(window, std::slice::from_ref(&run), 5.0, ink(c.surface, 0.7));
            stroke_dashed(window, &run, 2.6, ink(c.danger, 1.0), [6.0, 3.0]);
        }
    }
    // Tool preview.
    for curve in &active.preview_guides {
        for run in curve_runs(frame, projector, curve) {
            stroke_dashed(window, &run, 1.2, ink(c.accent, 0.8), [4.0, 4.0]);
        }
    }
    let preview: Vec<_> = active
        .preview
        .iter()
        .flat_map(|curve| curve_runs(frame, projector, curve))
        .collect();
    if active.preview_construction {
        for run in &preview {
            stroke_dashed(window, run, 2.0, ink(c.accent, 1.0), [6.0, 4.0]);
        }
    } else {
        stroke_runs(window, &preview, 4.5, ink(c.surface, 0.6));
        stroke_runs(window, &preview, 2.0, ink(c.accent, 1.0));
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
            stroke_runs(
                window,
                std::slice::from_ref(&run),
                1.0,
                ink(c.text_muted, 0.9),
            );
        }
        let reference = [arc.center, arc.center + Vec2::X * arc.radius * 1.35];
        for run in plane_runs(frame, projector, &reference) {
            stroke_dashed(window, &run, 1.0, ink(c.text_muted, 0.7), [3.0, 3.0]);
        }
    }
    for point in &active.placed {
        if let Some(p) = screen(*point) {
            dot(window, p, 7.0, ink(c.surface, 0.9));
            dot(window, p, 5.0, ink(c.accent, 1.0));
        }
    }
    // Inference guides and the snap marker.
    if let Some(pointer) = &active.pointer {
        let snap = &pointer.snap;
        for guide in &snap.guides {
            let color = match guide.kind {
                GuideKind::Horizontal => ink(c.axis[0], 0.95),
                GuideKind::Vertical => ink(c.axis[1], 0.95),
                GuideKind::Parallel | GuideKind::Perpendicular => ink(c.accent, 0.95),
                GuideKind::Aligned => ink(c.text_muted, 0.9),
            };
            let direction = (guide.to - guide.from).normalize_or_zero();
            let extend = 36.0 / pointer.pixels_per_unit.max(1.0e-6);
            let ends = [guide.from, guide.to + direction * extend];
            for run in plane_runs(frame, projector, &ends) {
                stroke_dashed(window, &run, 1.2, color, [5.0, 4.0]);
            }
        }
        if let Some(p) = screen(snap.point) {
            paint_snap_marker(window, p, snap.kind, active.drawing, c);
            let text = snap.kind.label();
            if !text.is_empty() {
                let at = p + point(px(16.0), px(20.0));
                let run_width = text.len() as f32 * 6.2 + 12.0;
                let center = at + point(px(run_width * 0.5), px(0.0));
                label(
                    window,
                    cx,
                    text,
                    center,
                    ink(c.text_muted, 1.0),
                    ink(c.overlay, 1.0),
                    Some(gpui::rgba(c.hud_border)),
                    10.0,
                );
            }
        }
    }
    // Live dimensions.
    for dim in &active.dims {
        let Some(anchor) = screen(dim.at) else {
            continue;
        };
        let away = screen(dim.at + dim.away * 1.0e-3)
            .map(|p| (to_vec(p) - to_vec(anchor)).normalize_or_zero())
            .unwrap_or(Vec2::NEG_Y);
        let center = to_point(to_vec(anchor) + away * 20.0);
        let text = if dim.active && dim.locked {
            format!("{}▏", dim.text)
        } else {
            dim.text.clone()
        };
        let (text_color, background, border) = if dim.invalid {
            (
                ink(c.danger, 1.0),
                ink(c.overlay, 0.95),
                Some(ink(c.danger, 1.0)),
            )
        } else if dim.locked {
            (
                ink(c.accent_text, 1.0),
                ink(c.accent_soft, 0.97),
                Some(ink(c.accent_border, 1.0)),
            )
        } else if dim.active {
            (
                ink(c.text, 1.0),
                ink(c.overlay, 1.0),
                Some(ink(c.accent_border, 1.0)),
            )
        } else {
            (
                ink(c.text, 1.0),
                ink(c.overlay, 1.0),
                Some(gpui::rgba(c.hud_border)),
            )
        };
        label(
            window, cx, &text, center, text_color, background, border, 11.0,
        );
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

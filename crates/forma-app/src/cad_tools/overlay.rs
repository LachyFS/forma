//! Viewport overlays for the tools: region and face highlights, ghost
//! prisms and lathes coloured by operation, chamfer faces, arrow handles and
//! live dimensions. Built once per frame from cached world geometry and
//! projected in the canvas paint, so drags never wait on the kernel.
use super::edit::EditHover;
use super::geometry::{Lathe, Prism, rotate_about};
use super::sketch_tool::{SketchToolKind, Target};
use super::{Tool, operation_color};
use crate::app::Studio;
use crate::sketch::overlay::{
    Projector, dot, fill_polygons, ink, label, ring, stroke_dashed, stroke_runs, to_point, to_vec,
};
use crate::theme::Colors;
use forma_core::{Mat4, Vec2, Vec3};
use gpui::{App, Bounds, FillOptions, FillRule, PathBuilder, PathStyle, Pixels, Point, Window, px};
use std::rc::Rc;

/// A highlighted region or face: polygons (outer loops and holes) in world space.
pub(crate) struct AreaPaint {
    pub polygons: Vec<Vec<Vec3>>,
    pub color: u32,
    pub fill: f32,
    pub outline: f32,
}

/// A ghost sweep of a cached prism between two offsets.
pub(crate) struct PrismPaint {
    pub prism: Rc<Prism>,
    pub start: f32,
    pub end: f32,
    pub color: u32,
    /// The moving cap (`end`, or `start` when pushing) is emphasised.
    pub emphasis_end: bool,
    /// Fill strength: full while previewing, faint once the kernel result shows.
    pub fill: f32,
}

pub(crate) struct LathePaint {
    pub lathe: Rc<Lathe>,
    pub angle: f32,
    pub color: u32,
    pub fill: f32,
}

pub(crate) struct LinePaint {
    pub points: Vec<Vec3>,
    pub color: u32,
    pub alpha: f32,
    pub width: f32,
    pub dashed: bool,
}

/// How a dimension label reads.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LabelStyle {
    Measured,
    Typed,
    Invalid,
    /// Snapped to a face: the label says so.
    Snapped,
}

pub(crate) struct LabelPaint {
    pub at: Vec2,
    pub text: String,
    pub style: LabelStyle,
}

pub(crate) struct ArrowPaint {
    pub base: Vec2,
    pub tip: Vec2,
    pub color: u32,
    pub hot: bool,
}

pub(crate) struct ToolOverlay {
    pub colors: Colors,
    /// Camera position (perspective) or backward view direction (orthographic).
    pub eye: Vec3,
    pub orthographic: bool,
    pub areas: Vec<AreaPaint>,
    pub prisms: Vec<PrismPaint>,
    pub lathes: Vec<LathePaint>,
    pub quads: Vec<([Vec3; 4], u32)>,
    pub lines: Vec<LinePaint>,
    pub arrows: Vec<ArrowPaint>,
    pub knobs: Vec<(Vec2, u32, bool)>,
    pub labels: Vec<LabelPaint>,
}

impl ToolOverlay {
    fn facing(&self, normal: Vec3, point: Vec3) -> bool {
        if self.orthographic {
            normal.dot(self.eye) > 0.0
        } else {
            normal.dot(self.eye - point) > 0.0
        }
    }

    fn is_empty(&self) -> bool {
        self.areas.is_empty()
            && self.prisms.is_empty()
            && self.lathes.is_empty()
            && self.quads.is_empty()
            && self.lines.is_empty()
            && self.arrows.is_empty()
            && self.knobs.is_empty()
            && self.labels.is_empty()
    }
}

fn prism_polygons(prism: &Prism, offset: f32) -> Vec<Vec<Vec3>> {
    prism
        .loops
        .iter()
        .map(|l| {
            l.iter()
                .map(|&i| prism.points[i as usize] + prism.normal * offset)
                .collect()
        })
        .collect()
}

impl Studio {
    /// Everything the tools paint over the viewport this frame.
    pub(crate) fn cad_tool_overlay(&self) -> Option<ToolOverlay> {
        let t = self.theme_colors();
        let camera = self.scene.camera;
        let backward = camera.view_matrix().inverse().z_axis.truncate();
        let mut overlay = ToolOverlay {
            colors: t,
            eye: if camera.orthographic {
                backward
            } else {
                camera.position()
            },
            orthographic: camera.orthographic,
            areas: Vec::new(),
            prisms: Vec::new(),
            lathes: Vec::new(),
            quads: Vec::new(),
            lines: Vec::new(),
            arrows: Vec::new(),
            knobs: Vec::new(),
            labels: Vec::new(),
        };
        // Committed operations waiting for their kernel result.
        for ghost in &self.cad_tool.pending {
            if !self
                .scene
                .part(ghost.part)
                .is_some_and(|p| p.stamp == ghost.stamp && !p.is_evaluated())
            {
                continue;
            }
            if let Some((prism, start, end)) = &ghost.prism {
                overlay.prisms.push(PrismPaint {
                    prism: prism.clone(),
                    start: *start,
                    end: *end,
                    color: ghost.color,
                    emphasis_end: true,
                    fill: 0.7,
                });
            }
            overlay
                .quads
                .extend(ghost.quads.iter().map(|q| (*q, ghost.color)));
        }
        match self.cad_tool.tool.as_ref() {
            Some(Tool::Sketch(tool)) => self.sketch_tool_overlay(tool, &mut overlay),
            Some(Tool::PushPull(tool)) => self.push_pull_overlay(tool, &mut overlay),
            Some(Tool::Chamfer(tool)) => {
                let value = tool.value();
                for edge in &tool.edges {
                    overlay.lines.push(LinePaint {
                        points: vec![edge.start, edge.end],
                        color: t.accent,
                        alpha: 1.0,
                        width: 3.0,
                        dashed: false,
                    });
                    if value > 0.0 {
                        overlay.quads.push((edge.quad(value), t.accent));
                        for corner in [edge.start, edge.end] {
                            let wedge = vec![
                                corner + edge.into_a * value,
                                corner,
                                corner + edge.into_b * value,
                            ];
                            overlay.lines.push(LinePaint {
                                points: wedge,
                                color: t.danger,
                                alpha: 0.9,
                                width: 1.2,
                                dashed: true,
                            });
                        }
                    }
                }
                if let Some(edge) = tool.edges.first()
                    && let Some(at) =
                        self.world_to_screen((edge.start + edge.end) * 0.5 + (edge.into_a + edge.into_b) * value * 0.5)
                {
                    overlay.labels.push(LabelPaint {
                        at: at + Vec2::new(0.0, -26.0),
                        text: numeric_text(&tool.numeric, value, false),
                        style: label_style(&tool.numeric, tool.typed().is_err(), false),
                    });
                }
            }
            None => {
                if let Some(id) = self.part_editing() {
                    self.part_edit_overlay(id, &mut overlay);
                } else if let Some(hover) = self.cad_tool.region_hover
                    && let Some(regions) = self.region_sets().iter().find(|r| r.object == hover.object)
                    && let Some(polygons) = regions.world.get(hover.profile)
                {
                    overlay.areas.push(AreaPaint {
                        polygons: polygons.clone(),
                        color: t.accent,
                        fill: 0.22,
                        outline: 1.5,
                    });
                }
            }
        }
        (!overlay.is_empty()).then_some(overlay)
    }

    fn sketch_tool_overlay(&self, tool: &super::SketchTool, overlay: &mut ToolOverlay) {
        let t = overlay.colors;
        let regions = &tool.regions;
        // Every region of the sketch, so picking is discoverable.
        for (index, polygons) in regions.world.iter().enumerate() {
            let selected = tool.selected.contains(&index);
            let hovered = tool.hover == Some(index);
            overlay.areas.push(AreaPaint {
                polygons: polygons.clone(),
                color: t.accent,
                fill: match (selected, hovered) {
                    (true, true) => 0.34,
                    (true, false) => 0.24,
                    (false, true) => 0.18,
                    _ => 0.06,
                },
                outline: if selected || hovered { 1.4 } else { 0.0 },
            });
        }
        let operation = tool.operation();
        let color = operation_color(&t, operation);
        let dragging = tool.drag.as_ref().is_some_and(|d| d.started());
        // Once the kernel's real result is on screen, the ghost recedes to an outline.
        let real = !dragging
            && tool.session.preview.as_ref().is_some_and(|preview| {
                self.scene
                    .part(preview.part)
                    .is_some_and(|part| part.is_evaluated() && part.issues().is_empty())
            });
        let fill = if real { 0.0 } else { 1.0 };
        match tool.kind {
            SketchToolKind::Extrude => {
                let (start, end) = tool.offsets();
                if end - start > 1.0e-6 {
                    overlay.prisms.push(PrismPaint {
                        prism: tool.prism.clone(),
                        start,
                        end,
                        color,
                        emphasis_end: tool.distance >= 0.0 || tool.symmetric,
                        fill,
                    });
                }
                if let Some((base, tip)) = tool.handle_screen(self) {
                    overlay.arrows.push(ArrowPaint {
                        base,
                        tip,
                        color,
                        hot: tool.handle_hot || dragging,
                    });
                }
                self.distance_label(
                    tool.anchor(),
                    regions.normal(),
                    tool.handle_offset(),
                    &tool.numeric,
                    tool.typed().is_err(),
                    tool.snapped_face,
                    tool.target.as_ref(),
                    overlay,
                );
            }
            SketchToolKind::Revolve => {
                if let Some(line) = tool.hover_line
                    && let Some(&(_, a, b)) = regions.lines.iter().find(|l| l.0 == line)
                {
                    overlay.lines.push(LinePaint {
                        points: vec![regions.to_world(a), regions.to_world(b)],
                        color: t.accent,
                        alpha: 1.0,
                        width: 3.0,
                        dashed: false,
                    });
                }
                let Some((origin, direction)) = tool.world_axis() else {
                    return;
                };
                // The axis, extended well past the profile.
                let reach = self.world_per_pixel(origin) * 1600.0;
                overlay.lines.push(LinePaint {
                    points: vec![origin - direction * reach, origin + direction * reach],
                    color: t.accent,
                    alpha: 0.95,
                    width: 1.4,
                    dashed: true,
                });
                let problem = tool.revolve_problem().is_some();
                if let Some(lathe) = &tool.lathe
                    && !problem
                {
                    overlay.lathes.push(LathePaint {
                        lathe: lathe.clone(),
                        angle: tool.angle,
                        color,
                        fill,
                    });
                }
                // The sweep arc through the profile's centroid, with the handle.
                let anchor = tool.anchor();
                let foot = origin + direction * (anchor - origin).dot(direction);
                let steps = 48;
                let arc: Vec<Vec3> = (0..=steps)
                    .map(|i| {
                        rotate_about(anchor, origin, direction, tool.angle * i as f32 / steps as f32)
                    })
                    .collect();
                overlay.lines.push(LinePaint {
                    points: arc,
                    color: if problem { t.danger } else { color },
                    alpha: 0.95,
                    width: 1.6,
                    dashed: false,
                });
                overlay.lines.push(LinePaint {
                    points: vec![foot, anchor],
                    color: t.text_muted,
                    alpha: 0.8,
                    width: 1.0,
                    dashed: true,
                });
                if let Some(handle) = tool.angle_handle_screen(self) {
                    overlay.knobs.push((handle, color, tool.handle_hot || dragging));
                    let half = rotate_about(anchor, origin, direction, tool.angle * 0.5);
                    if let Some(at) = self.world_to_screen(half) {
                        let away = self
                            .world_to_screen(foot)
                            .map(|f| (at - f).normalize_or(Vec2::NEG_Y))
                            .unwrap_or(Vec2::NEG_Y);
                        overlay.labels.push(LabelPaint {
                            at: at + away * 24.0,
                            text: if tool.numeric.is_empty() {
                                crate::sketch::overlay::format_angle(tool.angle)
                            } else {
                                format!("{}▏", tool.numeric)
                            },
                            style: label_style(&tool.numeric, tool.typed().is_err(), false),
                        });
                    }
                }
            }
        }
    }

    /// A dimension from the sketch plane (or face) to the moving face.
    #[allow(clippy::too_many_arguments)]
    fn distance_label(
        &self,
        anchor: Vec3,
        normal: Vec3,
        offset: f32,
        numeric: &str,
        invalid: bool,
        snapped: bool,
        target: Option<&Target>,
        overlay: &mut ToolOverlay,
    ) {
        let t = overlay.colors;
        let tip = anchor + normal * offset;
        if offset.abs() > 1.0e-6 {
            overlay.lines.push(LinePaint {
                points: vec![anchor, tip],
                color: t.text,
                alpha: 0.75,
                width: 1.0,
                dashed: true,
            });
        }
        let (Some(a), Some(b)) = (self.world_to_screen(anchor), self.world_to_screen(tip)) else {
            return;
        };
        let along = (b - a).try_normalize().unwrap_or(Vec2::NEG_Y);
        let side = Vec2::new(-along.y, along.x);
        let side = if side.x < 0.0 { -side } else { side };
        let text = numeric_text(numeric, offset, snapped);
        let _ = target;
        overlay.labels.push(LabelPaint {
            at: (a + b) * 0.5 + side * 34.0,
            text,
            style: label_style(numeric, invalid, snapped),
        });
    }

    fn push_pull_overlay(&self, tool: &super::PushPullTool, overlay: &mut ToolOverlay) {
        let t = overlay.colors;
        if let Some(hover) = &tool.hover
            && tool
                .active
                .as_ref()
                .is_none_or(|a| a.drag.is_none_or(|d| !d.started) && !a.modal)
        {
            overlay.areas.push(AreaPaint {
                polygons: prism_polygons(&hover.prism, 0.0),
                color: if hover.planar { t.accent } else { t.text_muted },
                fill: if hover.planar { 0.16 } else { 0.08 },
                outline: 1.6,
            });
        }
        let Some(active) = &tool.active else {
            return;
        };
        let distance = match active.typed() {
            Ok(Some(value)) => super::typed_on_side(value, active.distance),
            _ => active.distance,
        };
        let color = if distance >= 0.0 { t.accent } else { t.danger };
        for face in &active.faces {
            overlay.areas.push(AreaPaint {
                polygons: prism_polygons(&face.prism, 0.0),
                color: t.accent,
                fill: 0.1,
                outline: 1.6,
            });
            if distance.abs() > 1.0e-6 {
                overlay.prisms.push(PrismPaint {
                    prism: face.prism.clone(),
                    start: 0.0_f32.min(distance),
                    end: 0.0_f32.max(distance),
                    color,
                    emphasis_end: distance >= 0.0,
                    fill: 1.0,
                });
            }
        }
        if let Some((base, tip)) = tool.handle_screen(self) {
            overlay.arrows.push(ArrowPaint {
                base,
                tip,
                color,
                hot: tool.handle_hot || active.drag.is_some_and(|d| d.started),
            });
        }
        let first = &active.faces[0];
        self.distance_label(
            active.anchor(),
            first.normal,
            distance,
            &active.numeric,
            active.typed().is_err(),
            active.snapped_face,
            None,
            overlay,
        );
    }

    fn part_edit_overlay(&self, id: u64, overlay: &mut ToolOverlay) {
        let t = overlay.colors;
        let Some(instance) = self.scene.mesh_instance(id) else {
            return;
        };
        let matrix = instance.world_transform;
        let face_area = |group: u32, fill: f32, outline: f32| -> AreaPaint {
            let faces = super::geometry::group_faces(instance.mesh, group);
            let prism = super::push_pull::face_prism(instance.mesh, &faces, matrix, Vec3::ZERO);
            // Triangles as separate polygons: groups may be curved.
            let mut polygons: Vec<Vec<Vec3>> = Vec::new();
            for triangle in &prism.triangles {
                polygons.push(triangle.iter().map(|&i| prism.points[i as usize]).collect());
            }
            let _ = outline;
            AreaPaint {
                polygons,
                color: t.accent,
                fill,
                outline: 0.0,
            }
        };
        let outline = |group: u32| -> Vec<Vec<Vec3>> {
            let faces = super::geometry::group_faces(instance.mesh, group);
            let boundary = super::geometry::group_boundary(instance.mesh, &faces);
            boundary
                .iter()
                .map(|e| {
                    e.iter()
                        .map(|&i| matrix.transform_point3(instance.mesh.positions[i as usize]))
                        .collect()
                })
                .collect()
        };
        let selected_faces = self.selected_edit_faces(id);
        for (group, _) in &selected_faces {
            overlay.areas.push(face_area(*group, 0.3, 0.0));
            for segment in outline(*group) {
                overlay.lines.push(LinePaint {
                    points: segment,
                    color: t.accent,
                    alpha: 1.0,
                    width: 2.2,
                    dashed: false,
                });
            }
        }
        let topology = self.part_topology(id);
        let edge_points = |index: usize| -> Option<Vec<Vec3>> {
            let edge = topology.as_ref()?.edges.get(index)?;
            Some(
                edge.points
                    .iter()
                    .map(|&i| matrix.transform_point3(instance.mesh.positions[i as usize]))
                    .collect(),
            )
        };
        for index in self.selected_edit_edges(id) {
            if let Some(points) = edge_points(index) {
                overlay.lines.push(LinePaint {
                    points,
                    color: t.accent,
                    alpha: 1.0,
                    width: 3.2,
                    dashed: false,
                });
            }
        }
        match self.cad_tool.edit.hover {
            Some(EditHover::Face(group)) if !selected_faces.iter().any(|(g, _)| *g == group) => {
                overlay.areas.push(face_area(group, 0.14, 0.0));
                for segment in outline(group) {
                    overlay.lines.push(LinePaint {
                        points: segment,
                        color: t.accent,
                        alpha: 0.85,
                        width: 1.6,
                        dashed: false,
                    });
                }
            }
            Some(EditHover::Edge(index)) => {
                if let Some(points) = edge_points(index) {
                    let straight = topology
                        .as_ref()
                        .and_then(|t| t.edges.get(index))
                        .is_some_and(|e| e.straight);
                    overlay.lines.push(LinePaint {
                        points,
                        color: if straight { t.accent } else { t.text },
                        alpha: 0.85,
                        width: 2.6,
                        dashed: false,
                    });
                }
            }
            _ => {}
        }
        let _ = Mat4::IDENTITY;
    }
}

fn numeric_text(numeric: &str, value: f32, snapped: bool) -> String {
    if numeric.is_empty() {
        let text = crate::sketch::overlay::format_length(value);
        if snapped { format!("{text} · face") } else { text }
    } else {
        format!("{numeric}▏")
    }
}

fn label_style(numeric: &str, invalid: bool, snapped: bool) -> LabelStyle {
    if invalid {
        LabelStyle::Invalid
    } else if !numeric.is_empty() {
        LabelStyle::Typed
    } else if snapped {
        LabelStyle::Snapped
    } else {
        LabelStyle::Measured
    }
}

/// Fills polygons as one non-zero path: front faces of a closed ghost share
/// one orientation on screen, so overlaps tint once, never twice.
fn fill_union(window: &mut Window, polygons: &[Vec<Point<Pixels>>], color: gpui::Rgba) {
    let mut path = PathBuilder::fill()
        .with_style(PathStyle::Fill(FillOptions::default().with_fill_rule(FillRule::NonZero)));
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

pub(crate) fn paint(
    overlay: &ToolOverlay,
    matrix: Mat4,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let projector = Projector { matrix, bounds };
    let c = overlay.colors;
    window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
        for area in &overlay.areas {
            let polygons: Vec<Vec<Point<Pixels>>> = area
                .polygons
                .iter()
                .filter_map(|polygon| polygon.iter().map(|p| projector.point(*p)).collect())
                .collect();
            if area.fill > 0.0 {
                fill_polygons(window, &polygons, ink(area.color, area.fill));
            }
            if area.outline > 0.0 {
                let runs: Vec<Vec<Point<Pixels>>> = area
                    .polygons
                    .iter()
                    .flat_map(|polygon| {
                        let mut closed = polygon.clone();
                        if let Some(first) = polygon.first() {
                            closed.push(*first);
                        }
                        projector.polyline(&closed)
                    })
                    .collect();
                stroke_runs(window, &runs, area.outline + 2.0, ink(c.canvas, 0.35));
                stroke_runs(window, &runs, area.outline, ink(area.color, 0.95));
            }
        }
        for prism in &overlay.prisms {
            paint_prism(overlay, prism, &projector, window);
        }
        for lathe in &overlay.lathes {
            paint_lathe(overlay, lathe, &projector, window);
        }
        for (quad, color) in &overlay.quads {
            if let Some(points) = quad.iter().map(|p| projector.point(*p)).collect::<Option<Vec<_>>>() {
                fill_polygons(window, std::slice::from_ref(&points), ink(*color, 0.32));
                let mut closed = points.clone();
                closed.push(points[0]);
                stroke_runs(window, std::slice::from_ref(&closed), 3.0, ink(c.canvas, 0.4));
                stroke_runs(window, &[closed], 1.4, ink(*color, 1.0));
            }
        }
        for line in &overlay.lines {
            let runs = projector.polyline(&line.points);
            if line.dashed {
                for run in &runs {
                    stroke_dashed(window, run, line.width, ink(line.color, line.alpha), [5.0, 4.0]);
                }
            } else {
                stroke_runs(window, &runs, line.width + 2.2, ink(c.canvas, 0.4));
                stroke_runs(window, &runs, line.width, ink(line.color, line.alpha));
            }
        }
        for arrow in &overlay.arrows {
            paint_arrow(arrow, c, window);
        }
        for (at, color, hot) in &overlay.knobs {
            let p = to_point(*at);
            dot(window, p, if *hot { 17.0 } else { 15.0 }, ink(c.canvas, 0.5));
            dot(window, p, if *hot { 13.0 } else { 11.0 }, ink(*color, 1.0));
            ring(window, p, if *hot { 6.5 } else { 5.5 }, 1.4, ink(c.overlay, 0.95));
        }
        for paint_label in &overlay.labels {
            let (text, background, border) = match paint_label.style {
                LabelStyle::Invalid => (ink(c.danger, 1.0), ink(c.overlay, 0.97), Some(ink(c.danger, 1.0))),
                LabelStyle::Typed => (
                    ink(c.accent_text, 1.0),
                    ink(c.accent_soft, 0.97),
                    Some(ink(c.accent_border, 1.0)),
                ),
                LabelStyle::Snapped => (
                    ink(c.accent_text, 1.0),
                    ink(c.overlay, 0.97),
                    Some(ink(c.accent_border, 1.0)),
                ),
                LabelStyle::Measured => (ink(c.text, 1.0), ink(c.overlay, 0.97), Some(gpui::rgba(c.hud_border))),
            };
            label(window, cx, &paint_label.text, to_point(paint_label.at), text, background, border, 11.5);
        }
    });
}

fn paint_prism(overlay: &ToolOverlay, paint: &PrismPaint, projector: &Projector, window: &mut Window) {
    let prism = &*paint.prism;
    let n = prism.normal;
    let (lo, hi) = (paint.start.min(paint.end), paint.start.max(paint.end));
    let at = |i: u32, h: f32| prism.points[i as usize] + n * h;
    let screen = |p: Vec3| projector.point(p);
    let mut front: Vec<Vec<Point<Pixels>>> = Vec::new();
    let mut moving: Vec<Vec<Point<Pixels>>> = Vec::new();
    for (cap, height) in [(1.0_f32, hi), (-1.0, lo)] {
        let normal = n * cap;
        let emphasised = (cap > 0.0) == paint.emphasis_end;
        for triangle in &prism.triangles {
            let mut points = triangle.map(|i| at(i, height));
            if cap < 0.0 {
                points.swap(1, 2);
            }
            let centroid = (points[0] + points[1] + points[2]) / 3.0;
            if !overlay.facing(normal, centroid) {
                continue;
            }
            if let Some(projected) = points.iter().map(|p| screen(*p)).collect::<Option<Vec<_>>>() {
                if emphasised {
                    moving.push(projected.clone());
                }
                front.push(projected);
            }
        }
    }
    // Walls, with each corner's visibility for silhouette edges.
    let mut edges: Vec<Vec<Vec3>> = Vec::new();
    for lp in &prism.loops {
        let count = lp.len();
        let mut visible = Vec::with_capacity(count);
        for k in 0..count {
            let (a, b) = (lp[k], lp[(k + 1) % count]);
            let (pa, pb) = (prism.points[a as usize], prism.points[b as usize]);
            let normal = (pb - pa).cross(n).normalize_or_zero();
            let quad = [at(a, lo), at(b, lo), at(b, hi), at(a, hi)];
            let center = (quad[0] + quad[2]) * 0.5;
            let facing = overlay.facing(normal, center);
            visible.push(facing);
            if facing
                && let Some(projected) = quad.iter().map(|p| screen(*p)).collect::<Option<Vec<_>>>()
            {
                front.push(projected);
            }
        }
        // Cap outlines.
        let top: Vec<Vec3> = lp.iter().chain(lp.first()).map(|&i| at(i, hi)).collect();
        let bottom: Vec<Vec3> = lp.iter().chain(lp.first()).map(|&i| at(i, lo)).collect();
        edges.push(top);
        edges.push(bottom);
        // Vertical edges at creases and silhouettes.
        for k in 0..count {
            let previous = visible[(k + count - 1) % count];
            if prism.sharp_corner(lp, k) || previous != visible[k] {
                edges.push(vec![at(lp[k], lo), at(lp[k], hi)]);
            }
        }
    }
    let c = overlay.colors;
    if paint.fill > 0.0 {
        fill_union(window, &front, ink(paint.color, 0.16 * paint.fill));
        fill_union(window, &moving, ink(paint.color, 0.14 * paint.fill));
    }
    let runs: Vec<Vec<Point<Pixels>>> = edges.iter().flat_map(|e| projector.polyline(e)).collect();
    let strength = 0.45 + 0.55 * paint.fill;
    stroke_runs(window, &runs, 3.2, ink(c.canvas, 0.35 * strength));
    stroke_runs(window, &runs, 1.4, ink(paint.color, 0.95 * strength));
}

fn paint_lathe(overlay: &ToolOverlay, paint: &LathePaint, projector: &Projector, window: &mut Window) {
    let lathe = &*paint.lathe;
    let (origin, axis) = (lathe.axis_origin, lathe.axis_direction);
    let angle = paint.angle;
    let steps = ((angle.abs() / 7.5f32.to_radians()).ceil() as usize).clamp(4, 48);
    let rotate = |p: Vec3, step: usize| rotate_about(p, origin, axis, angle * step as f32 / steps as f32);
    let mut front: Vec<Vec<Point<Pixels>>> = Vec::new();
    let mut rings: Vec<Vec<Vec3>> = Vec::new();
    // Sparse loops keep the fill light on dense curves.
    for lp in &lathe.loops {
        let stride = (lp.len() / 48).max(1);
        let sampled: Vec<Vec3> = lp.iter().step_by(stride).copied().collect();
        let count = sampled.len();
        for k in 0..count {
            let (a, b) = (sampled[k], sampled[(k + 1) % count]);
            for step in 0..steps {
                let quad = [rotate(a, step), rotate(b, step), rotate(b, step + 1), rotate(a, step + 1)];
                let normal = (quad[1] - quad[0]).cross(quad[3] - quad[0]);
                let center = (quad[0] + quad[2]) * 0.5;
                // Either side may face the viewer: the surface is thin.
                let _ = normal;
                if let Some(projected) = quad.iter().map(|p| projector.point(*p)).collect::<Option<Vec<_>>>() {
                    let facing = overlay.facing(normal, center);
                    let mut projected = projected;
                    if !facing {
                        projected.reverse();
                    }
                    front.push(projected);
                }
            }
        }
        // Rings traced by the profile's corners.
        let corners: Vec<Vec3> = (0..lp.len())
            .filter(|&k| {
                let n = lp.len();
                let (a, b, c) = (lp[(k + n - 1) % n], lp[k], lp[(k + 1) % n]);
                (b - a).normalize_or_zero().dot((c - b).normalize_or_zero()) < 25f32.to_radians().cos()
            })
            .map(|k| lp[k])
            .collect();
        for corner in corners.into_iter().take(16) {
            rings.push((0..=steps).map(|s| rotate(corner, s)).collect());
        }
        let start: Vec<Vec3> = lp.iter().chain(lp.first()).copied().collect();
        let end: Vec<Vec3> = start.iter().map(|p| rotate(*p, steps)).collect();
        rings.push(start);
        rings.push(end);
    }
    let c = overlay.colors;
    if paint.fill > 0.0 {
        fill_union(window, &front, ink(paint.color, 0.13 * paint.fill));
    }
    let runs: Vec<Vec<Point<Pixels>>> = rings.iter().flat_map(|r| projector.polyline(r)).collect();
    stroke_runs(window, &runs, 3.0, ink(c.canvas, 0.3));
    stroke_runs(window, &runs, 1.3, ink(paint.color, 0.9));
    // The profile at its end position is the moving face.
    let ends: Vec<Vec<Point<Pixels>>> = lathe
        .prism
        .triangles
        .iter()
        .filter_map(|t| {
            t.iter()
                .map(|&i| projector.point(rotate(lathe.prism.points[i as usize], steps)))
                .collect()
        })
        .collect();
    fill_polygons(window, &ends, ink(paint.color, 0.22 * paint.fill.max(0.4)));
}

fn paint_arrow(arrow: &ArrowPaint, c: Colors, window: &mut Window) {
    let direction = (arrow.tip - arrow.base).normalize_or(Vec2::NEG_Y);
    let side = Vec2::new(-direction.y, direction.x);
    let head = 13.0;
    let neck = arrow.tip - direction * head;
    let shaft = [to_point(arrow.base), to_point(neck)];
    let width = if arrow.hot { 3.0 } else { 2.4 };
    stroke_runs(window, &[shaft.to_vec()], width + 3.0, ink(c.canvas, 0.45));
    stroke_runs(window, &[shaft.to_vec()], width, ink(arrow.color, 1.0));
    let triangle = vec![
        to_point(arrow.tip),
        to_point(neck + side * 7.0),
        to_point(neck - side * 7.0),
        to_point(arrow.tip),
    ];
    stroke_runs(window, std::slice::from_ref(&triangle), 3.0, ink(c.canvas, 0.45));
    fill_polygons(window, std::slice::from_ref(&triangle), ink(arrow.color, 1.0));
    // The grip where the arrow meets the face.
    let grip = to_point(arrow.base);
    dot(window, grip, if arrow.hot { 13.0 } else { 11.0 }, ink(c.canvas, 0.45));
    dot(window, grip, if arrow.hot { 9.0 } else { 7.0 }, ink(arrow.color, 1.0));
    dot(window, grip, 3.0, ink(c.overlay, 1.0));
    let _ = to_vec;
}

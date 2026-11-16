//! Viewport overlays for the tools: region and face highlights, ghost
//! prisms and lathes coloured by operation, chamfer faces, arrow handles and
//! live dimensions. Built once per frame from cached world geometry and
//! projected in the canvas paint, so drags never wait on the kernel.
use super::edit::EditHover;
use super::geometry::{Lathe, Prism, rotate_about};
use super::sketch_tool::SketchToolKind;
use super::{Tool, operation_color};
use crate::app::Studio;
use crate::sketch::overlay::{
    Projector, dot, fill_polygons, ink, label, ring, shaped_width, stroke_dashed, stroke_runs,
    to_point,
};
use crate::theme::Colors;
use forma_core::{Mat4, Vec2, Vec3};
use gpui::{App, Bounds, FillOptions, FillRule, PathBuilder, PathStyle, Pixels, Point, Window};
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
    /// A tool body (the selected feature's ghost) rather than a preview:
    /// `fill` is the alpha of one faint fill with no cap emphasis, and its
    /// outline is drawn as lines by the caller, which knows what in the
    /// scene hides them (see [`prism_edges`]).
    pub tool_body: bool,
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

/// A value chip. With a leader it hangs off `at` (an arrow tip, the middle
/// of a sweep) by a short line in `leader`'s screen direction, so it reads
/// as attached to the handle it measures; without one it is centred on `at`.
pub(crate) struct LabelPaint {
    pub at: Vec2,
    pub leader: Option<Vec2>,
    pub text: String,
    pub style: LabelStyle,
}

/// The material an operation removes, as the faces of a closed solid, each
/// with the direction it faces out of the solid: its front faces are tinted
/// faintly, so the cut reads as a ghost inside the part's old silhouette.
pub(crate) struct WedgePaint {
    pub faces: Vec<(Vec<Vec3>, Vec3)>,
    pub color: u32,
    pub fill: f32,
}

pub(crate) struct ArrowPaint {
    pub base: Vec2,
    pub tip: Vec2,
    pub color: u32,
    pub hot: bool,
}

/// A highlighted face group: fill triangles and outline segments.
#[derive(Default)]
pub(crate) struct FaceHighlight {
    pub fill: Vec<Vec<Vec3>>,
    pub visible: Vec<Vec<Vec3>>,
    pub hidden: Vec<Vec<Vec3>>,
}

pub(crate) struct ToolOverlay {
    pub colors: Colors,
    /// Camera position (perspective) or backward view direction (orthographic).
    pub eye: Vec3,
    pub orthographic: bool,
    pub wedges: Vec<WedgePaint>,
    pub areas: Vec<AreaPaint>,
    pub prisms: Vec<PrismPaint>,
    pub lathes: Vec<LathePaint>,
    pub quads: Vec<([Vec3; 4], u32)>,
    pub lines: Vec<LinePaint>,
    pub arrows: Vec<ArrowPaint>,
    pub knobs: Vec<(Vec2, u32, bool)>,
    /// Accent end points of selected edges.
    pub dots: Vec<Vec3>,
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
            && self.wedges.is_empty()
            && self.prisms.is_empty()
            && self.lathes.is_empty()
            && self.quads.is_empty()
            && self.lines.is_empty()
            && self.arrows.is_empty()
            && self.knobs.is_empty()
            && self.dots.is_empty()
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
            wedges: Vec::new(),
            areas: Vec::new(),
            prisms: Vec::new(),
            lathes: Vec::new(),
            quads: Vec::new(),
            lines: Vec::new(),
            arrows: Vec::new(),
            knobs: Vec::new(),
            dots: Vec::new(),
            labels: Vec::new(),
        };
        // Committed operations waiting for their kernel result.
        for ghost in &self.cad_tool.pending {
            if !self
                .scene
                .part(ghost.part)
                .is_some_and(|p| p.stamp == ghost.stamp && p.is_pending())
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
                    tool_body: false,
                });
            }
            // Chamfer faces keep the preview's look until the result lands.
            for quad in &ghost.quads {
                let normal = newell(quad);
                let up = if normal.y < 0.0 { -normal } else { normal };
                overlay.areas.push(AreaPaint {
                    polygons: vec![quad.clone()],
                    color: surface_tone(up.normalize_or_zero()),
                    fill: 0.94,
                    outline: 0.0,
                });
                overlay.areas.push(AreaPaint {
                    polygons: vec![quad.clone()],
                    color: ghost.color,
                    fill: CHAMFER_TINT,
                    outline: CHAMFER_OUTLINE,
                });
            }
        }
        match self.cad_tool.tool.as_ref() {
            Some(Tool::Sketch(tool)) => self.sketch_tool_overlay(tool, &mut overlay),
            Some(Tool::PushPull(tool)) => self.push_pull_overlay(tool, &mut overlay),
            Some(Tool::Chamfer(tool)) => self.chamfer_overlay(tool, &mut overlay),
            None => {
                if let Some(id) = self.part_editing() {
                    self.part_edit_overlay(id, &mut overlay);
                    return (!overlay.is_empty()).then_some(overlay);
                }
                self.feature_context_overlay(&mut overlay);
                if let Some(hover) = self
                    .cad_tool
                    .region_hover
                    .filter(|hover| hover.revision == self.scene_revision)
                    && let Some(regions) =
                        self.region_sets().iter().find(|r| r.object == hover.object)
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

    /// The chamfer as its result. Each new face in view is painted as a
    /// lit surface under a light accent tint ([`CHAMFER_TINT`]) with a
    /// crisp accent outline, so it covers the corner it cuts, the old edge
    /// disappears and the bevel still reads as shaded material rather than
    /// a sticker. A new face hidden from view (an edge behind the part)
    /// stays see-through, with the wedge it removes as a faint red ghost.
    /// An arrow on the first edge drags the distance, with the value
    /// hanging off its tip.
    fn chamfer_overlay(&self, tool: &super::ChamferTool, overlay: &mut ToolOverlay) {
        let t = overlay.colors;
        let value = tool.value();
        let invalid = tool.typed().is_err();
        let camera = self.scene.camera;
        let query = self.visibility_query();
        let faces = super::edit::chamfer_faces(&tool.edges, value);
        for (edge, face) in tool.edges.iter().zip(faces) {
            if value <= 1.0e-6 || invalid {
                // Nothing cut yet: the edges about to be chamfered.
                overlay.lines.push(LinePaint {
                    points: vec![edge.start, edge.end],
                    color: t.accent,
                    alpha: 1.0,
                    width: 3.0,
                    dashed: false,
                });
                continue;
            }
            // An edge behind other geometry keeps a see-through face: an
            // opaque one would float in front of what hides it.
            let outward = edge.outward();
            let lift = self.world_per_pixel((edge.start + edge.end) * 0.5) * 1.5;
            let seen = (1..=5)
                .filter(|&k| {
                    let p = edge.start.lerp(edge.end, k as f32 / 6.0) + outward * lift;
                    crate::viewport::visible_in_query(camera, query, p)
                })
                .count();
            let quad = face;
            if seen >= 3 {
                // An end cap of the removed wedge that faces away (its end
                // lies on a corner face turned from the viewer) is part of
                // the wedge's outline on screen that the new face does not
                // cover: the old corner showed past the preview there as a
                // light sliver. Painted in the same tone it reads as the
                // removed material it is, without an outline of its own.
                let caps: Vec<Vec<Vec3>> = edge.wedge(value)[2..]
                    .iter()
                    .filter(|(cap, out)| {
                        let center = cap.iter().copied().sum::<Vec3>() / cap.len() as f32;
                        !overlay.facing(*out, center)
                    })
                    .map(|(cap, _)| cap.clone())
                    .collect();
                overlay.areas.push(AreaPaint {
                    polygons: std::iter::once(quad.clone()).chain(caps.clone()).collect(),
                    color: surface_tone(outward),
                    fill: 0.94,
                    outline: 0.0,
                });
                if !caps.is_empty() {
                    overlay.areas.push(AreaPaint {
                        polygons: caps,
                        color: t.accent,
                        fill: CHAMFER_TINT,
                        outline: 0.0,
                    });
                }
            } else {
                // The new face is hidden: the material it removes is the cue.
                // A face in view says it alone; a ghost beside it would only
                // show where a mitred end stops short of the old corner, as
                // a tinted sliver past the part's outline.
                overlay.wedges.push(WedgePaint {
                    faces: edge.wedge(value).to_vec(),
                    color: t.danger,
                    fill: 0.18,
                });
            }
            overlay.areas.push(AreaPaint {
                polygons: vec![quad],
                color: t.accent,
                fill: CHAMFER_TINT,
                outline: CHAMFER_OUTLINE,
            });
        }
        if let Some((base, tip)) = tool.handle_screen(self) {
            let dragging = tool.handle_drag.is_some_and(|(d, _)| d.started);
            let hot = tool.handle_hot || dragging;
            overlay.arrows.push(ArrowPaint {
                base,
                tip,
                color: t.accent,
                hot,
            });
            // The chip shows the live value; a handle at rest at zero (no
            // pointer driving it, nothing typed) stays a bare arrow.
            if hot || tool.modal || !tool.numeric.is_empty() || value > 1.0e-6 {
                overlay.labels.push(LabelPaint {
                    at: tip,
                    leader: (tip - base).try_normalize(),
                    text: numeric_text(&tool.numeric, value, false),
                    style: label_style(&tool.numeric, invalid, false),
                });
            }
        }
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
            && tool.preview.as_ref().is_some_and(|preview| {
                preview.key == tool.preview_key()
                    && preview.outcome.as_ref().is_some_and(|outcome| {
                        !outcome.failed && outcome.issues.iter().all(|issue| issue.warning)
                    })
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
                        tool_body: false,
                    });
                }
                let arrow = tool.handle_screen(self);
                if let Some((base, tip)) = arrow {
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
                    arrow,
                    &tool.numeric,
                    tool.typed().is_err(),
                    tool.snapped_face,
                    tool.handle_hot || dragging,
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
                // The axis, a little past the profile at both ends.
                let along: Vec<f32> = tool
                    .prism
                    .points
                    .iter()
                    .map(|p| (*p - origin).dot(direction))
                    .collect();
                let (low, high) = along
                    .iter()
                    .fold((0.0_f32, 0.0_f32), |(lo, hi), t| (lo.min(*t), hi.max(*t)));
                let margin = (high - low).max(self.world_per_pixel(origin) * 40.0) * 0.3;
                overlay.lines.push(LinePaint {
                    points: vec![
                        origin + direction * (low - margin),
                        origin + direction * (high + margin),
                    ],
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
                        rotate_about(
                            anchor,
                            origin,
                            direction,
                            tool.angle * i as f32 / steps as f32,
                        )
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
                    overlay
                        .knobs
                        .push((handle, color, tool.handle_hot || dragging));
                    let half = rotate_about(anchor, origin, direction, tool.angle * 0.5);
                    if let Some(at) = self.world_to_screen(half) {
                        let away = self
                            .world_to_screen(foot)
                            .map(|f| (at - f).normalize_or(Vec2::NEG_Y))
                            .unwrap_or(Vec2::NEG_Y);
                        overlay.labels.push(LabelPaint {
                            at,
                            leader: Some(away),
                            text: if tool.numeric.is_empty() {
                                super::format_sweep(tool.angle)
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

    /// A dimension from the sketch plane (or face) to the moving face. The
    /// value hangs off the arrow's tip (`arrow` is its screen base and tip),
    /// else off the middle of the dimension line. A handle at rest (`live`
    /// false: not hovered, dragged, driven by the pointer or typed into) at
    /// zero shows no chip: "0 m" before anything moved only reads as noise.
    #[allow(clippy::too_many_arguments)]
    fn distance_label(
        &self,
        anchor: Vec3,
        normal: Vec3,
        offset: f32,
        arrow: Option<(Vec2, Vec2)>,
        numeric: &str,
        invalid: bool,
        snapped: bool,
        live: bool,
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
        } else if !live && numeric.is_empty() {
            return;
        }
        let (at, leader) = match arrow {
            Some((base, tip)) => (tip, (tip - base).try_normalize()),
            None => {
                let (Some(a), Some(b)) = (self.world_to_screen(anchor), self.world_to_screen(tip))
                else {
                    return;
                };
                let along = (b - a).try_normalize().unwrap_or(Vec2::NEG_Y);
                let side = Vec2::new(-along.y, along.x);
                let side = if side.x < 0.0 { -side } else { side };
                ((a + b) * 0.5, Some(side))
            }
        };
        overlay.labels.push(LabelPaint {
            at,
            leader,
            text: numeric_text(numeric, offset, snapped),
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
            Ok(Some(value)) => value * active.side,
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
                    tool_body: false,
                });
            }
        }
        let arrow = tool.handle_screen(self);
        let hot = tool.handle_hot || active.drag.is_some_and(|d| d.started);
        if let Some((base, tip)) = arrow {
            overlay.arrows.push(ArrowPaint {
                base,
                tip,
                color,
                hot,
            });
        }
        let first = &active.faces[0];
        self.distance_label(
            active.anchor(),
            first.normal,
            distance,
            arrow,
            &active.numeric,
            active.typed().is_err(),
            active.snapped_face,
            hot || active.modal,
            overlay,
        );
    }

    /// Part edit mode: selected faces as a light accent tint under a full
    /// accent outline (the face's shading reads through), selected edges in
    /// full accent, hovered ones clearly lighter, and on the active flat
    /// face an arrow to push or pull it, with the distance hanging off the
    /// tip.
    fn part_edit_overlay(&self, id: u64, overlay: &mut ToolOverlay) {
        let t = overlay.colors;
        let Some(instance) = self.scene.mesh_instance(id) else {
            return;
        };
        let matrix = instance.world_transform;
        let paint_face =
            |group: u32, fill: f32, width: f32, alpha: f32, overlay: &mut ToolOverlay| {
                let face = self.face_highlight(id, group);
                overlay.areas.push(AreaPaint {
                    polygons: face.fill.clone(),
                    color: t.accent,
                    fill,
                    outline: 0.0,
                });
                for segment in &face.visible {
                    overlay.lines.push(LinePaint {
                        points: segment.clone(),
                        color: t.accent,
                        alpha,
                        width,
                        dashed: false,
                    });
                }
                // Occluded boundaries stay hidden: dashes through solid walls
                // read as x-ray noise once the face tint carries the selection.
            };
        let selected_faces = self.selected_edit_faces(id);
        let hover = self.cad_tool.edit.hover(self.scene_revision);
        for (group, _) in &selected_faces {
            let hovered = hover == Some(EditHover::Face(*group));
            paint_face(
                *group,
                if hovered {
                    FACE_TINT_HOVERED
                } else {
                    FACE_TINT
                },
                FACE_OUTLINE,
                1.0,
                overlay,
            );
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
                overlay.dots.extend([points[0], *points.last().unwrap()]);
                overlay.lines.push(LinePaint {
                    points,
                    color: t.accent,
                    alpha: 1.0,
                    width: 4.0,
                    dashed: false,
                });
            }
        }
        match hover {
            Some(EditHover::Face(group)) if !selected_faces.iter().any(|(g, _)| *g == group) => {
                // A light tint and a thinner, softer outline: plainly a
                // pre-highlight, not a selection.
                paint_face(group, FACE_HOVER_TINT, FACE_HOVER_OUTLINE, 0.75, overlay);
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
                        alpha: 0.55,
                        width: 2.0,
                        dashed: false,
                    });
                }
            }
            _ => {}
        }
        if let Some(arrow) = self.face_arrow(id) {
            overlay.arrows.push(ArrowPaint {
                base: arrow.base,
                tip: arrow.tip,
                color: t.accent,
                hot: self.cad_tool.edit.arrow_hot,
            });
            // At rest the arrow alone says the face can be pushed: a "0 m"
            // chip before anything moved read as noise. Under the pointer
            // the chip reads what dragging changes, the push/pull distance
            // (the running tool then counts from here). The face's area
            // lives in the panel.
            if self.cad_tool.edit.arrow_hot {
                overlay.labels.push(LabelPaint {
                    at: arrow.tip,
                    leader: (arrow.tip - arrow.base).try_normalize(),
                    text: crate::sketch::overlay::format_length(0.0),
                    style: LabelStyle::Measured,
                });
            }
        }
    }

    /// Keeps the unoccluded part of a triangle: wholly visible or hidden
    /// triangles (corners and centre agree) are decided at once; only those
    /// crossing an occlusion boundary are split, down to a few pixels. So a
    /// large planar facet partly behind other geometry is tinted only where
    /// it shows, for a bounded number of visibility rays.
    fn visible_pieces(
        &self,
        [a, b, c]: [Vec3; 3],
        depth: u32,
        visible: &impl Fn(Vec3) -> bool,
        budget: &mut usize,
        out: &mut Vec<Vec<Vec3>>,
    ) {
        let (span, area) = match (
            self.world_to_screen(a),
            self.world_to_screen(b),
            self.world_to_screen(c),
        ) {
            (Some(p), Some(q), Some(r)) => (
                p.distance(q).max(q.distance(r)).max(r.distance(p)),
                (q - p).perp_dot(r - p).abs() * 0.5,
            ),
            _ => (0.0, 0.0),
        };
        let centre = (a + b + c) / 3.0;
        // Corners, edge midpoints and centre: a long sliver crossing an
        // occluder shows the crossing at its midpoints.
        // Samples sit slightly inside the piece: points exactly on the face's
        // own boundary (a hole's rim, the foot of a boss) graze neighbouring
        // faces and would read as occluded, refining every boundary.
        let inset = |p: Vec3| p + (centre - p) * 0.08;
        let samples = [
            centre,
            inset(a),
            inset(b),
            inset(c),
            inset((a + b) * 0.5),
            inset((b + c) * 0.5),
            inset((c + a) * 0.5),
        ];
        let flags: Vec<bool> = samples.iter().map(|p| visible(*p)).collect();
        let mixed = flags.iter().any(|v| *v) && !flags.iter().all(|v| *v);
        // Very large and very long pieces are split regardless, since an
        // occluder can sit wholly inside them or across a sliver between its
        // samples (a long tessellation edge from hole to hole passed behind
        // the bracket's boss and was tinted across it); otherwise only pieces
        // crossing a boundary.
        let split = area > 4000.0 || span > 64.0 || (mixed && span > 5.0);
        if !split || depth >= 10 || *budget == 0 {
            // Most samples decide, not the centre alone: one stray ray (a
            // grazing hit, a hairline gap in an occluder's tessellation)
            // should not keep a piece that is otherwise hidden.
            if flags.iter().filter(|v| **v).count() * 2 > flags.len() {
                out.push(vec![a, b, c]);
            }
            return;
        }
        {
            *budget -= 1;
            let (ab, bc, ca) = ((a + b) * 0.5, (b + c) * 0.5, (c + a) * 0.5);
            for piece in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
                self.visible_pieces(piece, depth + 1, visible, budget, out);
            }
        }
    }

    /// The visible part of a face group, for highlighting: front-facing,
    /// unoccluded triangles and its outline split into visible and hidden
    /// segments. Cached per scene revision and camera.
    fn face_highlight(&self, object: u64, group: u32) -> Rc<FaceHighlight> {
        let camera = self.scene.camera;
        let key = (object, group);
        // While the view moves, a cheap pass (one ray per triangle) keeps
        // navigation smooth; the refined mask follows when it settles.
        let refine = self.navigation.is_none();
        if let Some((revision, cached_camera, refined, face)) =
            self.cad_tool.faces.borrow().get(&key)
            && *revision == self.scene_revision
            && *cached_camera == camera
            && (*refined || !refine)
        {
            return face.clone();
        }
        let mut face = FaceHighlight::default();
        if let Some(instance) = self.scene.mesh_instance(object) {
            let matrix = instance.world_transform;
            let mesh = instance.mesh;
            let query = self.visibility_query();
            let eye = camera.position();
            let backward = camera.view_matrix().inverse().z_axis.truncate();
            let facing = |normal: Vec3, point: Vec3| {
                if camera.orthographic {
                    normal.dot(backward) > 0.0
                } else {
                    normal.dot(eye - point) > 0.0
                }
            };
            let visible = |p: Vec3| crate::viewport::visible_in_query(camera, query, p);
            let faces = super::geometry::group_faces(mesh, group);
            let prism = super::push_pull::face_prism(mesh, &faces, matrix, Vec3::ZERO);
            let mut budget = 4000;
            for triangle in &prism.triangles {
                let [a, b, c] = triangle.map(|i| prism.points[i as usize]);
                let centre = (a + b + c) / 3.0;
                let normal = (b - a).cross(c - a);
                // A triangle with no width (as between two copies of a point
                // the tessellation repeats a hair apart) fills nothing but an
                // antialiased line across the face.
                let longest = (b - a)
                    .length_squared()
                    .max((c - b).length_squared())
                    .max((a - c).length_squared());
                if normal.length() <= longest * 1.0e-4 || !facing(normal, centre) {
                    continue;
                }
                if refine {
                    self.visible_pieces([a, b, c], 0, &visible, &mut budget, &mut face.fill);
                } else if visible(centre) {
                    face.fill.push(vec![a, b, c]);
                }
            }
            // Only the outer loop carries the selection's outline: a flat
            // face's holes and the feet of bosses standing on it are inner
            // loops, and an accent ring there reads as a second selected
            // face (the bracket's unselected boss looked picked). A curved
            // face (a cylinder wall) keeps every loop: each one bounds it.
            let boundary = super::geometry::group_boundary(mesh, &faces);
            let loops = super::geometry::chain_loops(&boundary);
            let world = |i: u32| matrix.transform_point3(mesh.positions[i as usize]);
            let plane = super::geometry::group_plane(mesh, &faces);
            let outer = plane.and_then(|(normal, _)| {
                (0..loops.len()).max_by(|&x, &y| {
                    loop_area(&loops[x], &world, normal)
                        .total_cmp(&loop_area(&loops[y], &world, normal))
                })
            });
            for (index, lp) in loops.iter().enumerate() {
                if outer.is_some_and(|outer| outer != index) {
                    continue;
                }
                // Consecutive visible segments join into one run, so the
                // outline's corners meet cleanly instead of as butt ends.
                let count = lp.len();
                let flags: Vec<bool> = (0..count)
                    .map(|k| visible((world(lp[k]) + world(lp[(k + 1) % count])) * 0.5))
                    .collect();
                let start = (0..count).find(|&k| !flags[k]).unwrap_or(0);
                let mut run: Vec<Vec3> = Vec::new();
                for step in 0..count {
                    let k = (start + step) % count;
                    let (a, b) = (world(lp[k]), world(lp[(k + 1) % count]));
                    if flags[k] {
                        if run.is_empty() {
                            run.push(a);
                        }
                        run.push(b);
                    } else {
                        if run.len() > 1 {
                            face.visible.push(std::mem::take(&mut run));
                        }
                        run.clear();
                        face.hidden.push(vec![a, b]);
                    }
                }
                if run.len() > 1 {
                    face.visible.push(run);
                }
            }
        }
        let face = Rc::new(face);
        let mut cache = self.cad_tool.faces.borrow_mut();
        cache.retain(|_, (revision, cached_camera, _, _)| {
            *revision == self.scene_revision && *cached_camera == camera
        });
        cache.insert(key, (self.scene_revision, camera, refine, face.clone()));
        face
    }
}

/// Newell's normal of a polygon (its area vector, unnormalised).
fn newell(polygon: &[Vec3]) -> Vec3 {
    let mut normal = Vec3::ZERO;
    for (k, a) in polygon.iter().enumerate() {
        normal += a.cross(polygon[(k + 1) % polygon.len()]);
    }
    normal
}

/// The area of a closed loop of mesh vertices, projected on a plane with
/// unit `normal` (sign ignored).
fn loop_area(lp: &[u32], world: &impl Fn(u32) -> Vec3, normal: Vec3) -> f32 {
    let mut area = Vec3::ZERO;
    for k in 0..lp.len() {
        area += world(lp[k]).cross(world(lp[(k + 1) % lp.len()]));
    }
    area.dot(normal).abs()
}

/// Accent tint of a selected part face: strong enough to tell a selected
/// white top from an unselected one at a glance, light enough that the
/// face's shading still reads through on dark walls. A selected face under
/// the pointer goes a step stronger.
const FACE_TINT: f32 = 0.24;
const FACE_TINT_HOVERED: f32 = 0.3;
/// The outline of a selected face's outer loop.
const FACE_OUTLINE: f32 = 2.0;
/// Pre-highlight of an unselected face under the pointer: plainly lighter
/// than a selection, with a thinner, softer outline.
const FACE_HOVER_TINT: f32 = 0.1;
const FACE_HOVER_OUTLINE: f32 = 1.5;

/// Accent tint over a previewed chamfer face's lit grey: enough to mark
/// the new face, light enough that its shading still reads.
const CHAMFER_TINT: f32 = 0.22;
/// Width of a previewed chamfer face's outline.
const CHAMFER_OUTLINE: f32 = 1.5;

/// A plain lit grey for a new face with outward `normal`, as the viewport's
/// overhead studio light shades parts: faces turned up read light, walls
/// dark. Previewed faces painted in it cover the geometry they replace.
fn surface_tone(normal: Vec3) -> u32 {
    let (dark, light) = (Vec3::new(52.0, 59.0, 70.0), Vec3::new(226.0, 229.0, 233.0));
    let lit = (0.3 + 0.6 * normal.y.max(0.0)).clamp(0.0, 1.0);
    let c = dark.lerp(light, lit).round();
    ((c.x as u32) << 16) | ((c.y as u32) << 8) | c.z as u32
}

fn numeric_text(numeric: &str, value: f32, snapped: bool) -> String {
    if numeric.is_empty() {
        let text = crate::sketch::overlay::format_length(value);
        if snapped {
            format!("{text} · face")
        } else {
            text
        }
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
pub(crate) fn fill_union(window: &mut Window, polygons: &[Vec<Point<Pixels>>], color: gpui::Rgba) {
    let mut path = PathBuilder::fill().with_style(PathStyle::Fill(
        FillOptions::default().with_fill_rule(FillRule::NonZero),
    ));
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
        for wedge in &overlay.wedges {
            paint_wedge(overlay, wedge, &projector, window);
        }
        for area in &overlay.areas {
            let polygons: Vec<Vec<Point<Pixels>>> = area
                .polygons
                .iter()
                .filter_map(|polygon| polygon.iter().map(|p| projector.point(*p)).collect())
                .collect();
            if area.fill > 0.0 {
                // A face highlight is thousands of small front-facing
                // triangles. Filled even-odd as one path, a sweep-line slip
                // between their shared edges flipped a whole scanline across
                // the gap an occluder leaves (a tinted streak across the
                // bracket's boss); non-zero fills them as one union. Region
                // outlines with holes keep even-odd, whatever their winding.
                if area.polygons.iter().all(|p| p.len() == 3) {
                    fill_union(window, &polygons, ink(area.color, area.fill));
                } else {
                    fill_polygons(window, &polygons, ink(area.color, area.fill));
                }
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
                // A narrow dark halo under a full-strength core keeps the
                // boundary crisp on light and dark surfaces alike.
                stroke_runs(window, &runs, area.outline + 1.5, ink(c.canvas, 0.3));
                stroke_runs(window, &runs, area.outline, ink(area.color, 1.0));
            }
        }
        for prism in &overlay.prisms {
            paint_prism(overlay, prism, &projector, window);
        }
        for lathe in &overlay.lathes {
            paint_lathe(overlay, lathe, &projector, window);
        }
        for (quad, color) in &overlay.quads {
            if let Some(points) = quad
                .iter()
                .map(|p| projector.point(*p))
                .collect::<Option<Vec<_>>>()
            {
                fill_polygons(window, std::slice::from_ref(&points), ink(*color, 0.32));
                let mut closed = points.clone();
                closed.push(points[0]);
                stroke_runs(
                    window,
                    std::slice::from_ref(&closed),
                    3.0,
                    ink(c.canvas, 0.4),
                );
                stroke_runs(window, &[closed], 1.4, ink(*color, 1.0));
            }
        }
        for line in &overlay.lines {
            let runs = projector.polyline(&line.points);
            if line.dashed {
                for run in &runs {
                    stroke_dashed(
                        window,
                        run,
                        line.width,
                        ink(line.color, line.alpha),
                        [5.0, 4.0],
                    );
                }
            } else {
                stroke_runs(window, &runs, line.width + 2.2, ink(c.canvas, 0.4));
                stroke_runs(window, &runs, line.width, ink(line.color, line.alpha));
            }
        }
        for point in &overlay.dots {
            if let Some(p) = projector.point(*point) {
                dot(window, p, 9.0, ink(c.canvas, 0.6));
                dot(window, p, 6.0, ink(c.accent, 1.0));
            }
        }
        for arrow in &overlay.arrows {
            paint_arrow(arrow, c, window);
        }
        for (at, color, hot) in &overlay.knobs {
            let p = to_point(*at);
            dot(
                window,
                p,
                if *hot { 17.0 } else { 15.0 },
                ink(c.canvas, 0.5),
            );
            dot(window, p, if *hot { 13.0 } else { 11.0 }, ink(*color, 1.0));
            ring(
                window,
                p,
                if *hot { 6.5 } else { 5.5 },
                1.4,
                ink(c.overlay, 0.95),
            );
        }
        for paint_label in &overlay.labels {
            let (text, background, border) = match paint_label.style {
                LabelStyle::Invalid => (
                    ink(c.danger, 1.0),
                    ink(c.overlay, 0.97),
                    Some(ink(c.danger, 1.0)),
                ),
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
                LabelStyle::Measured => (
                    ink(c.text, 1.0),
                    ink(c.overlay, 0.97),
                    Some(gpui::rgba(c.hud_border)),
                ),
            };
            let center = match paint_label.leader {
                Some(direction) => {
                    // The chip's edge meets the leader's end.
                    let size = Vec2::new(
                        shaped_width(window, &paint_label.text, LABEL_TEXT) + 12.0,
                        LABEL_TEXT + 9.0,
                    );
                    let exit = (size.x * 0.5 / direction.x.abs().max(1.0e-4))
                        .min(size.y * 0.5 / direction.y.abs().max(1.0e-4));
                    let end = paint_label.at + direction * LEADER;
                    let leader = [vec![to_point(paint_label.at), to_point(end)]];
                    stroke_runs(window, &leader, 3.0, ink(c.canvas, 0.4));
                    stroke_runs(window, &leader, 1.2, border.unwrap_or(text));
                    end + direction * exit
                }
                None => paint_label.at,
            };
            label(
                window,
                cx,
                &paint_label.text,
                to_point(center),
                text,
                background,
                border,
                LABEL_TEXT,
            );
        }
    });
}

/// Font size of value chips, and the length of the leader that attaches
/// one to its handle.
const LABEL_TEXT: f32 = 11.5;
const LEADER: f32 = 10.0;

/// Tints the front faces of a removed solid once, as one non-zero fill.
fn paint_wedge(
    overlay: &ToolOverlay,
    wedge: &WedgePaint,
    projector: &Projector,
    window: &mut Window,
) {
    let mut front: Vec<Vec<Point<Pixels>>> = Vec::new();
    for (face, outward) in &wedge.faces {
        if face.len() < 3 {
            continue;
        }
        // Newell's normal, turned the way the face looks out of the solid.
        let mut normal = Vec3::ZERO;
        for (k, a) in face.iter().enumerate() {
            normal += a.cross(face[(k + 1) % face.len()]);
        }
        let center = face.iter().copied().sum::<Vec3>() / face.len() as f32;
        let mut points = face.clone();
        if normal.dot(*outward) < 0.0 {
            normal = -normal;
            points.reverse();
        }
        if normal.length_squared() < 1.0e-12 || !overlay.facing(normal, center) {
            continue;
        }
        if let Some(projected) = points
            .iter()
            .map(|p| projector.point(*p))
            .collect::<Option<Vec<_>>>()
        {
            front.push(projected);
        }
    }
    fill_union(window, &front, ink(wedge.color, wedge.fill));
}

/// The outline of `prism` swept from `lo` to `hi`, as the viewer sees it:
/// edges beside a face toward the viewer (cap rims, creases and the
/// silhouette) and the hidden ones behind the body itself.
pub(crate) fn prism_edges(
    overlay: &ToolOverlay,
    prism: &Prism,
    lo: f32,
    hi: f32,
) -> (Vec<Vec<Vec3>>, Vec<Vec<Vec3>>) {
    let n = prism.normal;
    let at = |i: u32, h: f32| prism.points[i as usize] + n * h;
    let centroid = prism.centroid();
    let top_facing = overlay.facing(n, centroid + n * hi);
    let bottom_facing = overlay.facing(-n, centroid + n * lo);
    let mut edges: Vec<Vec<Vec3>> = Vec::new();
    let mut hidden: Vec<Vec<Vec3>> = Vec::new();
    for lp in &prism.loops {
        let count = lp.len();
        let visible: Vec<bool> = (0..count)
            .map(|k| {
                let (a, b) = (lp[k], lp[(k + 1) % count]);
                let (pa, pb) = (prism.points[a as usize], prism.points[b as usize]);
                let normal = (pb - pa).cross(n).normalize_or_zero();
                overlay.facing(normal, (at(a, lo) + at(b, hi)) * 0.5)
            })
            .collect();
        // Cap outlines, segment by segment.
        for k in 0..count {
            let (a, b) = (lp[k], lp[(k + 1) % count]);
            for (height, cap) in [(hi, top_facing), (lo, bottom_facing)] {
                let segment = vec![at(a, height), at(b, height)];
                if cap || visible[k] {
                    edges.push(segment);
                } else {
                    hidden.push(segment);
                }
            }
        }
        // Vertical edges at creases and silhouettes.
        for k in 0..count {
            let previous = visible[(k + count - 1) % count];
            let segment = vec![at(lp[k], lo), at(lp[k], hi)];
            if previous != visible[k] {
                edges.push(segment);
            } else if prism.sharp_corner(lp, k) {
                if visible[k] {
                    edges.push(segment);
                } else {
                    hidden.push(segment);
                }
            }
        }
    }
    (edges, hidden)
}

fn paint_prism(
    overlay: &ToolOverlay,
    paint: &PrismPaint,
    projector: &Projector,
    window: &mut Window,
) {
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
            if let Some(projected) = points
                .iter()
                .map(|p| screen(*p))
                .collect::<Option<Vec<_>>>()
            {
                if emphasised {
                    moving.push(projected.clone());
                }
                front.push(projected);
            }
        }
    }
    // Walls toward the viewer.
    for lp in &prism.loops {
        let count = lp.len();
        for k in 0..count {
            let (a, b) = (lp[k], lp[(k + 1) % count]);
            let (pa, pb) = (prism.points[a as usize], prism.points[b as usize]);
            let normal = (pb - pa).cross(n).normalize_or_zero();
            let quad = [at(a, lo), at(b, lo), at(b, hi), at(a, hi)];
            let center = (quad[0] + quad[2]) * 0.5;
            if overlay.facing(normal, center)
                && let Some(projected) = quad.iter().map(|p| screen(*p)).collect::<Option<Vec<_>>>()
            {
                front.push(projected);
            }
        }
    }
    if paint.tool_body {
        fill_union(window, &front, ink(paint.color, paint.fill));
        return;
    }
    let (edges, hidden) = prism_edges(overlay, prism, lo, hi);
    let c = overlay.colors;
    if paint.fill > 0.0 {
        fill_union(window, &front, ink(paint.color, 0.2 * paint.fill));
        fill_union(window, &moving, ink(paint.color, 0.16 * paint.fill));
    }
    let strength = 0.45 + 0.55 * paint.fill;
    for segment in &hidden {
        for run in projector.polyline(segment) {
            stroke_dashed(
                window,
                &run,
                1.0,
                ink(paint.color, 0.4 * strength),
                [3.0, 3.0],
            );
        }
    }
    let runs: Vec<Vec<Point<Pixels>>> = edges.iter().flat_map(|e| projector.polyline(e)).collect();
    stroke_runs(window, &runs, 3.2, ink(c.canvas, 0.35 * strength));
    stroke_runs(window, &runs, 1.5, ink(paint.color, 0.95 * strength));
}

fn paint_lathe(
    overlay: &ToolOverlay,
    paint: &LathePaint,
    projector: &Projector,
    window: &mut Window,
) {
    let lathe = &*paint.lathe;
    let (origin, axis) = (lathe.axis_origin, lathe.axis_direction);
    let angle = paint.angle;
    let steps = ((angle.abs() / 7.5f32.to_radians()).ceil() as usize).clamp(4, 48);
    let rotate =
        |p: Vec3, step: usize| rotate_about(p, origin, axis, angle * step as f32 / steps as f32);
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
                let quad = [
                    rotate(a, step),
                    rotate(b, step),
                    rotate(b, step + 1),
                    rotate(a, step + 1),
                ];
                let normal = (quad[1] - quad[0]).cross(quad[3] - quad[0]);
                let center = (quad[0] + quad[2]) * 0.5;
                // Either side may face the viewer: the surface is thin.
                if let Some(projected) = quad
                    .iter()
                    .map(|p| projector.point(*p))
                    .collect::<Option<Vec<_>>>()
                {
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
                (b - a).normalize_or_zero().dot((c - b).normalize_or_zero())
                    < 25f32.to_radians().cos()
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

/// A screen-space arrow handle, one style for every tool: an origin ring
/// where the handle meets the geometry, a 2 px shaft and a filled 14 px
/// cone. Under the pointer (or while dragged) the cone and ring grow a step
/// and the shaft thickens, so the handle answers the hover before it is
/// pressed. A narrow dark halo keeps it crisp on light and dark faces.
fn paint_arrow(arrow: &ArrowPaint, c: Colors, window: &mut Window) {
    let direction = (arrow.tip - arrow.base).normalize_or(Vec2::NEG_Y);
    let side = Vec2::new(-direction.y, direction.x);
    let (head, half, radius, width) = if arrow.hot {
        (16.0, 7.0, 5.5, 2.5)
    } else {
        (14.0, 6.0, 4.5, 2.0)
    };
    let color = ink(arrow.color, 1.0);
    let halo = ink(c.canvas, 0.45);
    let neck = arrow.tip - direction * head;
    // The shaft starts at the ring's edge, so the ring stays open.
    let shaft = vec![
        to_point(arrow.base + direction * radius),
        to_point(neck + direction * 0.5),
    ];
    let cone = vec![
        to_point(arrow.tip),
        to_point(neck + side * half),
        to_point(neck - side * half),
        to_point(arrow.tip),
    ];
    let grip = to_point(arrow.base);
    // Halos first, so no part of the handle is drawn over by another's.
    stroke_runs(window, std::slice::from_ref(&shaft), width + 2.0, halo);
    stroke_runs(window, std::slice::from_ref(&cone), 2.0, halo);
    ring(window, grip, radius, 1.5 + 2.0, halo);
    stroke_runs(window, &[shaft], width, color);
    fill_polygons(window, std::slice::from_ref(&cone), color);
    // The ring's middle is filled with the overlay surface, so it reads
    // as a hollow origin over any shading.
    dot(window, grip, radius * 2.0 - 1.0, ink(c.overlay, 0.9));
    ring(window, grip, radius, 1.5, color);
}

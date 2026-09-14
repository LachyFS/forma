use crate::app::{EditMode, Studio, Tool};
use forma_render::RenderMode;
use glam::{Mat4, Vec2, Vec3};
use gpui::{
    AnyElement, Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, PathBuilder, Pixels,
    Point, ScrollWheelEvent, Size, Window, canvas, div, fill, point, prelude::*, px, rgb, size,
};

#[derive(Clone, Copy)]
pub(crate) enum NavigationMode {
    Orbit,
    Pan,
    Zoom,
}

impl NavigationMode {
    fn from_modifiers(modifiers: gpui::Modifiers) -> Self {
        if modifiers.control {
            Self::Zoom
        } else if modifiers.shift {
            Self::Pan
        } else {
            Self::Orbit
        }
    }

    fn apply(self, camera: &mut forma_core::Camera, delta: Vec2, height: f32) {
        match self {
            Self::Orbit => camera.orbit(delta),
            Self::Pan => camera.pan_in_viewport(delta, height),
            Self::Zoom => camera.zoom(-delta.y),
        }
    }
}

/// AppKit's momentum updates arrive after Ended without a new Started phase.
/// Keep their navigation mode when a modifier is released after lifting fingers.
#[derive(Default)]
pub(crate) struct TrackpadGesture {
    modifiers: gpui::Modifiers,
    momentum: bool,
}

impl TrackpadGesture {
    fn event(&mut self, event: &ScrollWheelEvent) -> ScrollWheelEvent {
        let mut event = event.clone();
        if event.delta.precise() {
            match event.touch_phase {
                gpui::TouchPhase::Started => {
                    self.momentum = false;
                    self.modifiers = event.modifiers;
                }
                gpui::TouchPhase::Ended => self.momentum = true,
                gpui::TouchPhase::Moved if !self.momentum => self.modifiers = event.modifiers,
                _ => {}
            }
            event.modifiers = self.modifiers;
        }
        event
    }
}

pub(crate) fn mouse_point(point: Point<Pixels>) -> Vec2 {
    Vec2::new(point.x.into(), point.y.into())
}

/// Modelling modes match the display's backing pixels. Keep the path tracer's
/// logical-pixel budget, and fit oversized viewports uniformly to avoid stretching
/// geometry. Align only after scaling because the NV12 planes need even dimensions.
pub(crate) fn render_dimensions(
    logical: Size<Pixels>,
    backing_scale: f32,
    mode: RenderMode,
) -> (u32, u32) {
    let scale = if !mode.progressive() && backing_scale.is_finite() && backing_scale > 0. {
        backing_scale as f64
    } else {
        1.
    };
    let width = f32::from(logical.width).max(0.) as f64 * scale;
    let height = f32::from(logical.height).max(0.) as f64 * scale;
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
        return (2, 2);
    }
    let fit = (2560. / width).min(1600. / height).min(1.);
    let even =
        |value: f64, limit| (((value * fit).ceil() as u32).max(2).div_ceil(2) * 2).min(limit);
    (even(width, 2560), even(height, 1600))
}

pub(crate) fn project(world: Vec3, matrix: Mat4, bounds: Bounds<Pixels>) -> Option<Point<Pixels>> {
    let clip = matrix * world.extend(1.);
    if clip.w <= 0.00001 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if !(0. ..=1.).contains(&ndc.z) {
        return None;
    }
    Some(point(
        bounds.origin.x + bounds.size.width * (ndc.x * 0.5 + 0.5),
        bounds.origin.y + bounds.size.height * (0.5 - ndc.y * 0.5),
    ))
}

fn line(window: &mut Window, points: &[Point<Pixels>], color: u32, width: f32) {
    if points.len() < 2 {
        return;
    }
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(points[0]);
    for p in &points[1..] {
        path.line_to(*p);
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, rgb(color));
    }
}

fn rotation_ring(
    center: Vec3,
    basis: glam::Mat3,
    axis: usize,
    radius: f32,
    matrix: Mat4,
    bounds: Bounds<Pixels>,
) -> Vec<Point<Pixels>> {
    (0..=64)
        .filter_map(|i| {
            let angle = i as f32 / 64. * std::f32::consts::TAU;
            project(
                center
                    + radius
                        * (basis.col((axis + 1) % 3) * angle.cos()
                            + basis.col((axis + 2) % 3) * angle.sin()),
                matrix,
                bounds,
            )
        })
        .collect()
}

/// Test the surface depth at a projected point, including orthographic cameras.
#[cfg(test)]
fn visible_point(scene: &forma_core::Scene, point: Vec3) -> bool {
    visible_in_query(scene.camera, &forma_core::SurfaceQuery::new(scene), point)
}

fn visible_in_query(
    camera: forma_core::Camera,
    query: &forma_core::SurfaceQuery,
    point: Vec3,
) -> bool {
    let view_point = camera.view_matrix().transform_point3(point);
    if view_point.z >= 0. {
        return false;
    }
    let origin = if camera.orthographic {
        point + camera.view_matrix().inverse().z_axis.truncate() * (-view_point.z)
    } else {
        camera.position()
    };
    let distance = point.distance(origin);
    !query.occludes(
        forma_core::Ray {
            origin,
            direction: (point - origin).normalize_or_zero(),
        },
        distance - (distance * 0.0001).max(0.0001),
    )
}

fn pick_component(
    scene: &forma_core::Scene,
    mode: EditMode,
    pointer: Vec2,
    bounds: Bounds<Pixels>,
) -> Option<(u64, [u32; 2])> {
    let aspect = f32::from(bounds.size.width) / f32::from(bounds.size.height).max(1.);
    let matrix = scene.camera.projection_matrix(aspect) * scene.camera.view_matrix();
    let query = forma_core::SurfaceQuery::new(scene);
    let mut closest = 10.0;
    let mut picked = None;
    for object in scene
        .mesh_instances()
        .filter(|o| scene.is_effectively_visible(o.id) && o.selectable)
    {
        let candidates: Box<dyn Iterator<Item = [u32; 2]>> = if mode == EditMode::Vertex {
            Box::new((0..object.mesh.positions.len() as u32).map(|i| [i, i]))
        } else {
            Box::new(object.mesh.edges().into_iter())
        };
        for indices in candidates {
            let a = object
                .world_transform
                .transform_point3(object.mesh.positions[indices[0] as usize]);
            let b = object
                .world_transform
                .transform_point3(object.mesh.positions[*indices.last().unwrap() as usize]);
            let (Some(pa), Some(pb)) = (project(a, matrix, bounds), project(b, matrix, bounds))
            else {
                continue;
            };
            let pa = mouse_point(pa);
            let pb = mouse_point(pb);
            let t =
                ((pointer - pa).dot(pb - pa) / (pb - pa).length_squared().max(1e-10)).clamp(0., 1.);
            let distance = pointer.distance(pa.lerp(pb, t));
            let wa = (matrix * a.extend(1.)).w;
            let wb = (matrix * b.extend(1.)).w;
            let world_t = t * wa / ((1. - t) * wb + t * wa);
            if distance < closest && visible_in_query(scene.camera, &query, a.lerp(b, world_t)) {
                closest = distance;
                picked = Some((object.id, indices));
            }
        }
    }
    picked
}

#[derive(Default, Clone, Copy)]
pub(crate) struct BoxSelection {
    pub start: Option<Vec2>,
    pub current: Vec2,
    pub extend: bool,
    pub subtract: bool,
}

impl Studio {
    pub(crate) fn finish_box_select(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.box_select.take() else {
            return;
        };
        let Some(start) = selection.start else {
            return;
        };
        let bounds = self.bounds.get();
        let aspect = f32::from(bounds.size.width) / f32::from(bounds.size.height).max(1.);
        let matrix = self.scene.camera.projection_matrix(aspect) * self.scene.camera.view_matrix();
        let low = start.min(selection.current);
        let high = start.max(selection.current);
        if self.edit_mode == EditMode::Object {
            let mut ids = if selection.extend || selection.subtract {
                self.selected_ids()
            } else {
                Default::default()
            };
            for object in self
                .scene
                .mesh_instances()
                .filter(|o| o.selectable && self.scene.is_effectively_visible(o.id))
            {
                if let Some(p) = project(
                    object.world_transform.transform_point3(Vec3::ZERO),
                    matrix,
                    bounds,
                )
                .map(mouse_point)
                    && p.cmpge(low).all()
                    && p.cmple(high).all()
                {
                    if selection.subtract {
                        ids.remove(&object.id);
                    } else {
                        ids.insert(object.id);
                    }
                }
            }
            self.select_objects(ids, self.selected);
            self.status = format!("{} objects selected", self.selected_ids().len());
            self.invalidate(false, cx);
            return;
        }
        let query = forma_core::SurfaceQuery::new(&self.scene);
        let candidates = self.all_components();
        let mut elements = if selection.extend || selection.subtract {
            self.component_elements()
        } else {
            Default::default()
        };
        if let Some(object) = self.selected_object() {
            for element in candidates {
                let vertices = element.vertex_indices(object.mesh);
                let center = vertices
                    .iter()
                    .map(|i| {
                        object
                            .world_transform
                            .transform_point3(object.mesh.positions[*i as usize])
                    })
                    .sum::<Vec3>()
                    / vertices.len() as f32;
                if let Some(p) = project(center, matrix, bounds).map(mouse_point)
                    && p.cmpge(low).all()
                    && p.cmple(high).all()
                    && visible_in_query(self.scene.camera, &query, center)
                {
                    if selection.subtract {
                        elements.remove(&element);
                    } else {
                        elements.insert(element);
                    }
                }
            }
        }
        self.set_components(elements);
        self.status = format!("{} components selected", self.component_elements().len());
        cx.notify();
    }

    pub(crate) fn selection_frame(&self) -> Option<(Vec3, f32)> {
        let object = self.selected_object().filter(|object| object.visible)?;
        if self.edit_mode.is_component() {
            let face = self.component_vertices();
            if face.is_empty() {
                return None;
            }
            let points = face.iter().map(|index| {
                object
                    .world_transform
                    .transform_point3(object.mesh.positions[*index as usize])
            });
            let center = points.clone().sum::<Vec3>() / face.len() as f32;
            let radius = points
                .map(|point| point.distance(center))
                .fold(0.0_f32, f32::max);
            Some((center, radius))
        } else {
            let bounds: Vec<_> = self
                .selected_ids()
                .iter()
                .filter_map(|id| self.scene.bounds(*id))
                .collect();
            if bounds.is_empty() {
                return None;
            }
            let low = bounds
                .iter()
                .fold(Vec3::splat(f32::INFINITY), |p, (center, radius)| {
                    p.min(*center - Vec3::splat(*radius))
                });
            let high = bounds
                .iter()
                .fold(Vec3::splat(f32::NEG_INFINITY), |p, (center, radius)| {
                    p.max(*center + Vec3::splat(*radius))
                });
            if bounds.len() == 1 {
                Some(bounds[0])
            } else {
                Some(((low + high) * 0.5, (high - low).length() * 0.5))
            }
        }
    }

    pub(crate) fn viewport(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme_colors();
        let geometry = self.selection_center().filter(|_| {
            self.selected
                .is_some_and(|id| self.scene.is_effectively_visible(id))
        });
        let query = (!self.component_elements().is_empty() || self.selected_ids().len() > 1)
            .then(|| forma_core::SurfaceQuery::new(&self.scene));
        let mut highlights = self
            .selected_object()
            .map(|object| {
                self.component_elements()
                    .into_iter()
                    .filter_map(|element| {
                        let points = element
                            .vertex_indices(object.mesh)
                            .iter()
                            .map(|i| {
                                object
                                    .world_transform
                                    .transform_point3(object.mesh.positions[*i as usize])
                            })
                            .collect::<Vec<_>>();
                        let center = points.iter().sum::<Vec3>() / points.len() as f32;
                        query
                            .as_ref()
                            .is_some_and(|q| visible_in_query(self.scene.camera, q, center))
                            .then_some(points)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if self.edit_mode == EditMode::Object {
            for id in self
                .selected_ids()
                .into_iter()
                .filter(|id| Some(*id) != self.selected)
            {
                if let Some(object) = self.scene.mesh_instance(id) {
                    for edge in object.mesh.edges() {
                        let points = edge.map(|i| {
                            object
                                .world_transform
                                .transform_point3(object.mesh.positions[i as usize])
                        });
                        if query.as_ref().is_some_and(|q| {
                            visible_in_query(self.scene.camera, q, (points[0] + points[1]) * 0.5)
                        }) {
                            highlights.push(points.to_vec());
                        }
                    }
                }
            }
        }
        let box_selection = self.box_select;
        #[cfg(target_os = "macos")]
        let surface = self
            .frame
            .as_ref()
            .and_then(|frame| frame.native_surface())
            .cloned();
        let frame_image = self.frame_image.clone();
        let view = self.scene.camera.view_matrix();
        let camera = self.scene.camera;
        let bounds_cell = self.bounds.clone();
        let studio = cx.weak_entity();
        let presentation_studio = cx.weak_entity();
        let edit_mode = self.edit_mode;
        let tool = self.tool;
        let constraint = self
            .transform_drag
            .as_ref()
            .map(|d| d.constraint)
            .unwrap_or_default();
        let axis = constraint.axis;
        let gizmo_basis = self
            .transform_drag
            .as_ref()
            .filter(|d| d.constraint.orientation == crate::transform::Orientation::Local)
            .map(|d| d.basis)
            .unwrap_or(glam::Mat3::IDENTITY);
        let transforming = self.transform_drag.is_some();
        let canvas = canvas(
            move |bounds, _, cx| {
                if bounds_cell.replace(bounds).size != bounds.size {
                    // Layout runs while the view is borrowed. Defer its resize
                    // request until that borrow ends, without an idle poll loop.
                    cx.defer(move |cx| {
                        let _ = studio.update(cx, |s, cx| s.resize_viewport(cx));
                    });
                }
            },
            move |bounds, _, window, cx| {
                #[cfg(target_os = "macos")]
                if let Some(surface) = surface {
                    window.paint_surface(bounds, surface);
                }
                if let Some(image) = frame_image
                    && let Err(error) =
                        window.paint_image(bounds, Default::default(), image, 0, false)
                {
                    let error = format!("Viewport presentation failed: {error:#}");
                    cx.defer(move |cx| {
                        let _ = presentation_studio.update(cx, |s, cx| {
                            if s.render_error.as_ref() != Some(&error) {
                                s.status = error.clone();
                                s.render_error = Some(error);
                                cx.notify();
                            }
                        });
                    });
                }
                let aspect = f32::from(bounds.size.width) / f32::from(bounds.size.height).max(1.);
                let matrix = camera.projection_matrix(aspect) * view;
                if let Some(selection) = box_selection
                    && let Some(start) = selection.start
                {
                    let a = start.min(selection.current);
                    let b = start.max(selection.current);
                    let points = [a, Vec2::new(b.x, a.y), b, Vec2::new(a.x, b.y), a]
                        .map(|p| point(px(p.x), px(p.y)));
                    line(window, &points, 0x83bde8, 1.);
                }
                for face in &highlights {
                    let mut points: Vec<_> = face
                        .iter()
                        .filter_map(|v| project(*v, matrix, bounds))
                        .collect();
                    if points.len() == face.len() && !points.is_empty() {
                        if edit_mode == EditMode::Face {
                            points.push(points[0]);
                        }
                        line(window, &points, 0xf4c27a, 2.);
                        for p in points.iter().filter(|_| edit_mode.is_component()) {
                            window.paint_quad(fill(
                                Bounds::new(*p - point(px(3.), px(3.)), size(px(6.), px(6.))),
                                rgb(0xffdcaa),
                            ));
                        }
                    }
                }
                if let Some(center) = &geometry
                    && tool != Tool::Select
                {
                    let depth = if camera.orthographic {
                        camera.distance
                    } else {
                        -view.transform_point3(*center).z
                    };
                    let length = depth.max(0.001) * 0.105;
                    if let Some(origin) = project(*center, matrix, bounds) {
                        for (i, color) in crate::ui::AXIS.into_iter().enumerate() {
                            let active = axis
                                .is_some_and(|a| if constraint.plane { a != i } else { a == i });
                            if transforming && active {
                                let a = project(
                                    *center - gizmo_basis.col(i) * camera.distance * 100.,
                                    matrix,
                                    bounds,
                                );
                                let b = project(
                                    *center + gizmo_basis.col(i) * camera.distance * 100.,
                                    matrix,
                                    bounds,
                                );
                                if let (Some(a), Some(b)) = (a, b) {
                                    line(window, &[a, b], color, 1.);
                                }
                            }
                            if tool == Tool::Rotate {
                                let ring =
                                    rotation_ring(*center, gizmo_basis, i, length, matrix, bounds);
                                line(
                                    window,
                                    &ring,
                                    if active { 0xffffff } else { color },
                                    if active { 2.8 } else { 1.7 },
                                );
                            } else if let Some(end) =
                                project(*center + gizmo_basis.col(i) * length, matrix, bounds)
                            {
                                line(
                                    window,
                                    &[origin, end],
                                    if active { 0xffffff } else { color },
                                    2.3,
                                );
                                if tool == Tool::Move {
                                    let direction = (mouse_point(end) - mouse_point(origin))
                                        .normalize_or_zero();
                                    let side = Vec2::new(-direction.y, direction.x);
                                    let tip = mouse_point(end);
                                    let arrow = [
                                        tip - direction * 9. + side * 4.,
                                        tip,
                                        tip - direction * 9. - side * 4.,
                                    ]
                                    .map(|p| point(px(p.x), px(p.y)));
                                    line(window, &arrow, color, 2.3);
                                } else {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            end - point(px(3.), px(3.)),
                                            size(px(6.), px(6.)),
                                        ),
                                        rgb(color),
                                    ));
                                }
                            }
                        }
                        window.paint_quad(fill(
                            Bounds::new(origin - point(px(3.), px(3.)), size(px(6.), px(6.))),
                            rgb(0xe6eeee),
                        ));
                    }
                }
                // Compact world-space axis orientation indicator.
                let origin = point(bounds.right() - px(46.), bounds.origin.y + px(94.));
                for (vector, color) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().zip(crate::ui::AXIS)
                {
                    let screen = view.transform_vector3(vector);
                    let end = origin + point(px(screen.x * 23.), px(-screen.y * 23.));
                    line(window, &[origin, end], color, 2.);
                    window.paint_quad(fill(
                        Bounds::new(end - point(px(2.5), px(2.5)), size(px(5.), px(5.))),
                        rgb(color),
                    ));
                }
            },
        )
        .size_full();
        let view_label = if camera.orthographic {
            "User orthographic"
        } else {
            "User perspective"
        };
        let selection_label = self
            .selected_object()
            .map(|o| o.name.clone())
            .unwrap_or_else(|| "No selection".into());
        let mut viewport = div()
            .id("model-viewport")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(t.well))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|s, _, _, _| s.navigation = None),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|s, _, _, _| s.navigation = None),
            )
            .on_scroll_wheel(cx.listener(|s, event: &ScrollWheelEvent, _, cx| {
                s.scroll_wheel(event, cx);
            }))
            .child(canvas)
            .child(
                div()
                    .absolute()
                    .top(px(14.))
                    .left(px(16.))
                    .px(px(9.))
                    .py(px(7.))
                    .rounded(px(6.))
                    .bg(rgb(t.panel))
                    .text_size(px(11.))
                    .text_color(rgb(t.muted))
                    .child(view_label)
                    .child(
                        div()
                            .mt(px(4.))
                            .text_color(rgb(t.text))
                            .child(selection_label),
                    ),
            );
        if self.frame.is_none() {
            viewport = viewport.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(24.))
                    .text_center()
                    .text_size(px(12.))
                    .text_color(rgb(if self.render_error.is_some() {
                        t.alert
                    } else {
                        t.muted
                    }))
                    .child(
                        self.render_error
                            .clone()
                            .unwrap_or_else(|| "Preparing the GPU viewport…".into()),
                    ),
            );
        }
        if let Some(drag) = &self.transform_drag {
            let text = format!(
                "{:?}  {}  {}    ·    Click / Return to apply    Esc to cancel",
                drag.tool,
                drag.constraint
                    .axis
                    .map(|axis| ["X", "Y", "Z"][axis])
                    .unwrap_or("Free"),
                drag.numeric
            );
            viewport = viewport.child(
                div()
                    .absolute()
                    .bottom(px(16.))
                    .left(px(16.))
                    .px(px(11.))
                    .py(px(7.))
                    .rounded(px(7.))
                    .bg(rgb(t.active))
                    .border_1()
                    .border_color(rgb(t.accent_line))
                    .text_color(rgb(t.accent))
                    .text_size(px(11.))
                    .child(text),
            );
        }
        viewport.into_any_element()
    }

    pub(crate) fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        if self.navigation_blocked() {
            self.trackpad_gesture = TrackpadGesture::default();
            return;
        }
        let event = self.trackpad_gesture.event(event);
        if navigate_scroll(
            &mut self.scene.camera,
            &event,
            f32::from(self.bounds.get().size.height),
        ) {
            self.invalidate(false, cx);
        }
        cx.stop_propagation();
    }

    pub(crate) fn magnify(
        &mut self,
        position: Point<Pixels>,
        magnification: f32,
        cx: &mut Context<Self>,
    ) {
        if self.navigation_blocked() || !self.bounds.get().contains(&position) {
            return;
        }
        let before = self.scene.camera;
        // Spreading fingers zooms in; exponential scaling composes smoothly
        // across small updates without snapping to wheel-sized steps.
        self.scene.camera.zoom(magnification * 100.);
        if self.scene.camera != before {
            self.invalidate(false, cx);
        }
    }

    pub(crate) fn navigation_blocked(&self) -> bool {
        self.transform_drag.is_some()
            || self.shading_pie.is_some()
            || self.preview_open
            || self.help_open
            || self.palette_open
            || self.theme_picker.is_some()
    }

    pub(crate) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.theme_picker.is_some() {
            self.cancel_theme(cx);
            cx.stop_propagation();
            return;
        }
        if self.preview_open {
            self.preview_open = false;
            self.active_field = None;
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if let Some(pie) = self.shading_pie.as_mut() {
            match event.button {
                MouseButton::Left => {
                    pie.pointer_moved(mouse_point(event.position));
                    if let Some(mode) = pie.hovered {
                        self.execute(crate::app::Command::SetMode(mode), window, cx);
                    } else {
                        self.close_shading_pie(cx);
                    }
                }
                MouseButton::Right => self.close_shading_pie(cx),
                _ => {}
            }
            cx.stop_propagation();
            return;
        }
        window.focus(&self.focus);
        self.active_field = None;
        self.last_mouse = mouse_point(event.position);
        if let Some(selection) = self.box_select.as_mut() {
            if event.button == MouseButton::Left {
                selection.start = Some(self.last_mouse);
                selection.current = self.last_mouse;
                selection.extend = event.modifiers.shift;
                selection.subtract = event.modifiers.control;
            } else {
                self.box_select = None;
            }
            cx.notify();
            return;
        }
        if self.transform_drag.is_some() {
            self.finish_transform(event.button == MouseButton::Right, cx);
            return;
        }
        if event.button != MouseButton::Left || event.modifiers.alt {
            self.navigation = Some((
                event.button,
                NavigationMode::from_modifiers(event.modifiers),
            ));
            return;
        }
        let bounds = self.bounds.get();
        let aspect = f32::from(bounds.size.width) / f32::from(bounds.size.height).max(1.);
        let matrix = self.scene.camera.projection_matrix(aspect) * self.scene.camera.view_matrix();
        if self.tool != Tool::Select
            && !event.modifiers.shift
            && let Some(center) = self.selection_center()
            && let Some(origin) = project(center, matrix, bounds)
        {
            let depth = if self.scene.camera.orthographic {
                self.scene.camera.distance
            } else {
                -self.scene.camera.view_matrix().transform_point3(center).z
            };
            let length = depth.max(0.001) * 0.105;
            let axis = (0..3)
                .filter_map(|axis| {
                    let distance = if self.tool == Tool::Rotate {
                        rotation_ring(center, glam::Mat3::IDENTITY, axis, length, matrix, bounds)
                            .windows(2)
                            .map(|p| {
                                segment_distance(
                                    self.last_mouse,
                                    mouse_point(p[0]),
                                    mouse_point(p[1]),
                                )
                            })
                            .fold(f32::INFINITY, f32::min)
                    } else {
                        project(
                            center + glam::Mat3::IDENTITY.col(axis) * length,
                            matrix,
                            bounds,
                        )
                        .map(|end| {
                            segment_distance(self.last_mouse, mouse_point(origin), mouse_point(end))
                        })
                        .unwrap_or(f32::INFINITY)
                    };
                    (distance < 8.).then_some((axis, distance))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(axis, _)| axis);
            if axis.is_some() {
                self.start_transform(self.tool, axis, false, cx);
                return;
            }
        }
        let uv = (self.last_mouse - mouse_point(bounds.origin))
            / Vec2::new(bounds.size.width.into(), bounds.size.height.into());
        let previous = (self.selected, self.component_vertices());
        let component = if matches!(self.edit_mode, EditMode::Edge | EditMode::Vertex) {
            pick_component(&self.scene, self.edit_mode, self.last_mouse, bounds).map(
                |(id, edge)| {
                    (
                        id,
                        if self.edit_mode == EditMode::Vertex {
                            forma_core::MeshElement::Vertex(edge[0])
                        } else {
                            forma_core::MeshElement::Edge(edge)
                        },
                    )
                },
            )
        } else {
            self.scene
                .pick_surface(self.scene.camera.ray(uv, aspect))
                .filter(|hit| {
                    self.scene
                        .object(hit.object_id)
                        .is_some_and(|o| o.selectable)
                })
                .map(|hit| (hit.object_id, forma_core::MeshElement::Face(hit.face)))
        };
        if let Some((id, element)) = component {
            if self.edit_mode.is_component() {
                let mut elements = if event.modifiers.shift && self.selected == Some(id) {
                    self.component_elements()
                } else {
                    Default::default()
                };
                if !elements.insert(element) {
                    elements.remove(&element);
                }
                self.selected = Some(id);
                self.set_components(elements);
                self.status = format!(
                    "{} components selected · G / R / S transform",
                    self.component_elements().len()
                );
            } else {
                let mut ids = if event.modifiers.shift {
                    self.selected_ids()
                } else {
                    Default::default()
                };
                if !ids.insert(id) {
                    ids.remove(&id);
                }
                self.select_objects(ids, Some(id));
                self.status = format!("{} objects selected", self.selected_ids().len());
            }
        } else if !event.modifiers.shift {
            self.clear_components();
            if self.edit_mode == EditMode::Object {
                self.select_objects(Default::default(), None);
            }
        }
        if !event.modifiers.shift
            && previous == (self.selected, self.component_vertices())
            && self.tool != Tool::Select
        {
            self.start_transform(self.tool, None, false, cx);
        }
        self.invalidate(false, cx);
    }

    pub(crate) fn mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = mouse_point(event.position);
        if self.preview_open {
            cx.stop_propagation();
            return;
        }
        if let Some(pie) = self.shading_pie.as_mut() {
            if pie.pointer_moved(position) {
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        if let Some(selection) = self.box_select.as_mut() {
            selection.current = position;
            self.last_mouse = position;
            cx.notify();
            return;
        }
        if self.transform_drag.is_some() {
            self.update_transform(position, event.modifiers.shift, event.modifiers.control, cx);
        } else if let Some((button, mode)) = self.navigation {
            if event.pressed_button != Some(button) {
                self.navigation = None;
            } else {
                let delta = position - self.last_mouse;
                mode.apply(
                    &mut self.scene.camera,
                    delta,
                    f32::from(self.bounds.get().size.height),
                );
                self.invalidate(false, cx);
            }
        }
        self.last_mouse = position;
    }
}

/// Preserve subpixel trackpad motion; wheel notches keep their existing zoom speed.
fn navigate_scroll(camera: &mut forma_core::Camera, event: &ScrollWheelEvent, height: f32) -> bool {
    let delta = mouse_point(event.delta.pixel_delta(px(18.)));
    if !delta.is_finite() || delta == Vec2::ZERO {
        return false;
    }
    let before = *camera;
    if event.modifiers.control || event.modifiers.platform {
        camera.zoom(delta.y * 0.24);
    } else if event.modifiers.shift {
        camera.pan_in_viewport(delta, height);
    } else if event.delta.precise() || event.modifiers.alt {
        camera.orbit(delta * if event.delta.precise() { 0.6 } else { 1. });
    } else {
        camera.zoom(delta.y * 0.24);
    }
    *camera != before
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let segment = b - a;
    let t = ((p - a).dot(segment) / segment.length_squared().max(0.001)).clamp(0., 1.);
    p.distance(a + segment * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_picking_targets_vertices_and_polygon_edges() {
        let mut scene = forma_core::Scene::empty();
        let id = scene.add(forma_core::Primitive::Cube);
        scene.camera.target = Vec3::ZERO;
        scene.camera.yaw = 0.;
        scene.camera.pitch = 0.;
        scene.camera.distance = 6.;
        let bounds = Bounds::new(point(px(50.), px(30.)), size(px(800.), px(600.)));
        for orthographic in [false, true] {
            scene.camera.orthographic = orthographic;
            let matrix = scene.camera.projection_matrix(800. / 600.) * scene.camera.view_matrix();
            let point_at = |p| mouse_point(project(p, matrix, bounds).unwrap());
            let vertex = pick_component(
                &scene,
                EditMode::Vertex,
                point_at(Vec3::new(1., 1., 1.)),
                bounds,
            )
            .unwrap();
            assert_eq!(vertex.0, id);
            assert_eq!(
                scene.object_mesh(id).unwrap().positions[vertex.1[0] as usize],
                Vec3::ONE
            );
            let edge = pick_component(
                &scene,
                EditMode::Edge,
                point_at(Vec3::new(0., 1., 1.)),
                bounds,
            )
            .unwrap();
            assert_eq!(edge.0, id);
            assert!(edge.1.iter().all(|i| {
                let p = scene.object_mesh(id).unwrap().positions[*i as usize];
                p.y == 1. && p.z == 1.
            }));
            assert!(
                pick_component(
                    &scene,
                    EditMode::Edge,
                    point_at(Vec3::new(0., 0., 1.)),
                    bounds
                )
                .is_none(),
                "triangulation diagonal must not be selectable"
            );
            assert!(pick_component(&scene, EditMode::Vertex, Vec2::ZERO, bounds).is_none());
        }
        scene.object_mut(id).unwrap().selectable = false;
        assert!(pick_component(&scene, EditMode::Vertex, Vec2::new(450., 330.), bounds).is_none());
    }

    #[test]
    fn component_visibility_handles_occlusion_and_locked_objects_in_both_projections() {
        let mut scene = forma_core::Scene::empty();
        let cube = scene.add(forma_core::Primitive::Cube);
        scene.camera.target = Vec3::ZERO;
        scene.camera.yaw = 0.;
        scene.camera.pitch = 0.;
        scene.camera.distance = 6.;
        scene.object_mut(cube).unwrap().selectable = false;
        for orthographic in [false, true] {
            scene.camera.orthographic = orthographic;
            assert!(visible_point(&scene, Vec3::new(0., 0., 1.)));
            assert!(!visible_point(&scene, Vec3::new(0., 0., -1.)));
            assert!(!visible_point(&scene, Vec3::new(0., 0., 8.)));
        }
        scene.object_mut(cube).unwrap().visible = false;
        assert!(visible_point(&scene, Vec3::new(0., 0., -1.)));
    }

    fn scroll(delta: gpui::ScrollDelta, modifiers: gpui::Modifiers) -> ScrollWheelEvent {
        ScrollWheelEvent {
            delta,
            modifiers,
            ..Default::default()
        }
    }

    #[test]
    fn control_drag_zooms_and_shift_drag_pans_without_changing_orientation() {
        let before = forma_core::Camera::default();
        let mut zoomed = before;
        NavigationMode::from_modifiers(gpui::Modifiers {
            control: true,
            ..Default::default()
        })
        .apply(&mut zoomed, Vec2::new(0., -40.), 600.);
        assert!(zoomed.distance < before.distance);
        assert_eq!(zoomed.target, before.target);
        assert_eq!((zoomed.yaw, zoomed.pitch), (before.yaw, before.pitch));
        let mut panned = before;
        NavigationMode::from_modifiers(gpui::Modifiers {
            shift: true,
            ..Default::default()
        })
        .apply(&mut panned, Vec2::new(20., 40.), 600.);
        assert_ne!(panned.target, before.target);
        assert_eq!(panned.distance, before.distance);
        assert_eq!((panned.yaw, panned.pitch), (before.yaw, before.pitch));
    }

    #[test]
    fn trackpad_orbits_both_axes_and_preserves_fractional_motion() {
        let mut camera = forma_core::Camera::default();
        let before = camera;
        let event = scroll(
            gpui::ScrollDelta::Pixels(point(px(0.25), px(0.5))),
            Default::default(),
        );
        assert!(navigate_scroll(&mut camera, &event, 600.));
        assert!(camera.yaw < before.yaw && camera.pitch > before.pitch);
        assert_eq!(camera.distance, before.distance);
        assert_eq!(camera.target, before.target);
        let mut batched = before;
        let mut incremental = before;
        for _ in 0..100 {
            navigate_scroll(&mut incremental, &event, 600.);
        }
        navigate_scroll(
            &mut batched,
            &scroll(
                gpui::ScrollDelta::Pixels(point(px(25.), px(50.))),
                Default::default(),
            ),
            600.,
        );
        assert!((incremental.yaw - batched.yaw).abs() < 0.00001);
        assert!((incremental.pitch - batched.pitch).abs() < 0.00001);
    }

    #[test]
    fn modifiers_pan_and_zoom_without_orbiting_and_wheels_still_zoom() {
        let before = forma_core::Camera::default();
        let pixels = gpui::ScrollDelta::Pixels(point(px(4.), px(8.)));
        let mut camera = before;
        navigate_scroll(
            &mut camera,
            &scroll(
                pixels,
                gpui::Modifiers {
                    shift: true,
                    ..Default::default()
                },
            ),
            600.,
        );
        assert_ne!(camera.target, before.target);
        assert_eq!(camera.yaw, before.yaw);
        assert_eq!(camera.distance, before.distance);
        for (delta, modifiers) in [
            (
                pixels,
                gpui::Modifiers {
                    control: true,
                    ..Default::default()
                },
            ),
            (
                pixels,
                gpui::Modifiers {
                    platform: true,
                    ..Default::default()
                },
            ),
            (
                gpui::ScrollDelta::Lines(point(0., 1.)),
                gpui::Modifiers::default(),
            ),
        ] {
            let mut camera = before;
            navigate_scroll(&mut camera, &scroll(delta, modifiers), 600.);
            assert!(camera.distance < before.distance);
            assert_eq!(camera.target, before.target);
            assert_eq!(camera.yaw, before.yaw);
        }
        let mut camera = before;
        for delta in [Vec2::ZERO, Vec2::new(f32::NAN, 1.)] {
            assert!(!navigate_scroll(
                &mut camera,
                &scroll(
                    gpui::ScrollDelta::Pixels(point(px(delta.x), px(delta.y))),
                    Default::default()
                ),
                600.
            ));
            assert_eq!(camera, before);
        }
    }

    #[test]
    fn momentum_keeps_pan_after_shift_release_and_next_gesture_resets() {
        let mut gesture = TrackpadGesture::default();
        let mut event = scroll(
            gpui::ScrollDelta::Pixels(point(px(1.), px(2.))),
            gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        event.touch_phase = gpui::TouchPhase::Started;
        assert!(gesture.event(&event).modifiers.shift);
        event.touch_phase = gpui::TouchPhase::Ended;
        event.modifiers.shift = false;
        assert!(gesture.event(&event).modifiers.shift);
        event.touch_phase = gpui::TouchPhase::Moved;
        assert!(gesture.event(&event).modifiers.shift);
        event.touch_phase = gpui::TouchPhase::Started;
        assert!(!gesture.event(&event).modifiers.shift);
        // Modifiers can still change mode while fingers remain on the pad.
        event.touch_phase = gpui::TouchPhase::Moved;
        event.modifiers.control = true;
        assert!(gesture.event(&event).modifiers.control);
    }

    #[test]
    fn modelling_modes_use_backing_pixels_and_keep_path_tracing_budget() {
        let viewport = size(px(1010.), px(696.));
        for mode in [
            RenderMode::Wireframe,
            RenderMode::Solid,
            RenderMode::MaterialPreview,
        ] {
            assert_eq!(render_dimensions(viewport, 1., mode), (1010, 696));
            assert_eq!(render_dimensions(viewport, 2., mode), (2020, 1392));
        }
        assert_eq!(
            render_dimensions(viewport, 2., RenderMode::Rendered),
            (1010, 696)
        );
    }

    #[test]
    fn oversized_viewports_keep_their_aspect_with_even_bounded_dimensions() {
        for (width, height) in [(3000., 500.), (800., 2000.), (4000., 3000.)] {
            let (w, h) =
                render_dimensions(size(px(width), px(height)), 2., RenderMode::MaterialPreview);
            assert!(w <= 2560 && h <= 1600 && w % 2 == 0 && h % 2 == 0);
            let scale = (2560. / (width * 2.)).min(1600. / (height * 2.));
            assert!((w as f32 - width * 2. * scale).abs() < 2.01);
            assert!((h as f32 - height * 2. * scale).abs() < 2.01);
        }
    }

    #[test]
    fn fractional_backing_sizes_cover_the_viewport_and_align_nv12() {
        assert_eq!(
            render_dimensions(size(px(1001.), px(751.)), 1., RenderMode::Solid),
            (1002, 752)
        );
        assert_eq!(
            render_dimensions(size(px(501.25), px(375.75)), 2., RenderMode::Solid),
            (1004, 752)
        );
        assert_eq!(
            render_dimensions(size(px(500.), px(300.)), f32::NAN, RenderMode::Solid),
            (500, 300)
        );
        assert_eq!(
            render_dimensions(size(px(0.), px(0.)), 2., RenderMode::Solid),
            (2, 2)
        );
    }

    #[test]
    fn projection_rejects_behind_camera_and_matches_ray() {
        let camera = forma_core::Camera::default();
        let bounds = Bounds::new(point(px(200.), px(100.)), size(px(800.), px(600.)));
        let matrix = camera.projection_matrix(4. / 3.) * camera.view_matrix();
        let ray = camera.ray(Vec2::new(0.3, 0.7), 4. / 3.);
        let p = project(ray.origin + ray.direction * 5., matrix, bounds).unwrap();
        assert!((f32::from(p.x) - 440.).abs() < 0.01);
        assert!((f32::from(p.y) - 520.).abs() < 0.01);
        assert!(project(ray.origin - ray.direction, matrix, bounds).is_none());
    }
}

use crate::app::{Studio, Tool, TransformDrag};
use forma_render::RenderMode;
use glam::{Mat4, Quat, Vec2, Vec3};
use gpui::{
    AnyElement, Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, PathBuilder, Pixels,
    Point, ScrollWheelEvent, Size, Window, canvas, div, fill, point, prelude::*, px, rgb, rgba,
    size,
};

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

fn mouse_point(point: Point<Pixels>) -> Vec2 {
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

fn project(world: Vec3, matrix: Mat4, bounds: Bounds<Pixels>) -> Option<Point<Pixels>> {
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

impl Studio {
    pub(crate) fn viewport(&self, cx: &mut Context<Self>) -> AnyElement {
        let geometry = self.selected_object().filter(|o| o.visible).map(|o| {
            let selected_face = self
                .selected_face
                .and_then(|face| o.mesh.faces.get(face))
                .map(|face| {
                    face.iter()
                        .map(|index| {
                            o.world_transform
                                .transform_point3(o.mesh.positions[*index as usize])
                        })
                        .collect::<Vec<_>>()
                });
            let selected_face = selected_face.filter(|face| {
                if face.is_empty() {
                    return false;
                }
                let center = face.iter().copied().sum::<Vec3>() / face.len() as f32;
                let origin = self.scene.camera.position();
                self.scene
                    .pick(forma_core::Ray {
                        origin,
                        direction: (center - origin).normalize(),
                    })
                    .is_some_and(|hit| {
                        hit.object_id == o.id && Some(hit.face) == self.selected_face
                    })
            });
            (
                o.world_transform.transform_point3(Vec3::ZERO),
                selected_face,
            )
        });
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
        let axis = self.transform_drag.as_ref().and_then(|d| d.axis);
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
                if let Some((center, face)) = &geometry {
                    if edit_mode && let Some(face) = face {
                        let mut points: Vec<_> = face
                            .iter()
                            .filter_map(|v| project(*v, matrix, bounds))
                            .collect();
                        if points.len() == face.len() && !points.is_empty() {
                            points.push(points[0]);
                            line(window, &points, 0xf4c27a, 2.);
                            for p in &points {
                                window.paint_quad(fill(
                                    Bounds::new(*p - point(px(2.), px(2.)), size(px(4.), px(4.))),
                                    rgb(0xffdcaa),
                                ));
                            }
                        }
                    }
                    if tool != Tool::Select {
                        let length = camera.distance * 0.105;
                        if let Some(origin) = project(*center, matrix, bounds) {
                            for (i, color) in crate::ui::AXIS.into_iter().enumerate() {
                                let mut end = *center;
                                end[i] += length;
                                if let Some(end) = project(end, matrix, bounds) {
                                    line(
                                        window,
                                        &[origin, end],
                                        if axis == Some(i) { 0xffffff } else { color },
                                        2.3,
                                    );
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            end - point(px(3.), px(3.)),
                                            size(px(6.), px(6.)),
                                        ),
                                        rgb(color),
                                    ));
                                }
                            }
                            window.paint_quad(fill(
                                Bounds::new(origin - point(px(3.), px(3.)), size(px(6.), px(6.))),
                                rgb(0xe6eeee),
                            ));
                        }
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
            .bg(rgb(0x15181a))
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
                    .when(
                        self.settings.mode == forma_render::RenderMode::MaterialPreview
                            && self.settings.preview.world_opacity > 0.,
                        |d| d.px(px(9.)).py(px(7.)).rounded(px(6.)).bg(rgba(0x101315d9)),
                    )
                    .text_size(px(11.))
                    .text_color(rgb(crate::ui::MUTED))
                    .child(view_label)
                    .child(
                        div()
                            .mt(px(4.))
                            .text_color(rgb(crate::ui::TEXT))
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
                        crate::ui::ALERT
                    } else {
                        crate::ui::MUTED
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
                drag.axis
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
                    .bg(rgb(crate::ui::ACTIVE))
                    .border_1()
                    .border_color(rgb(crate::ui::ACCENT_LINE))
                    .text_color(rgb(crate::ui::ACCENT))
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
    }

    pub(crate) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        if self.transform_drag.is_some() {
            self.finish_transform(event.button == MouseButton::Right, cx);
            return;
        }
        if event.button != MouseButton::Left || event.modifiers.alt {
            self.navigation = Some((event.button, event.modifiers.shift));
            return;
        }
        let bounds = self.bounds.get();
        let aspect = f32::from(bounds.size.width) / f32::from(bounds.size.height).max(1.);
        let matrix = self.scene.camera.projection_matrix(aspect) * self.scene.camera.view_matrix();
        if self.tool != Tool::Select
            && let Some(object) = self.selected_object()
        {
            let center = object.transform.translation;
            if let Some(origin) = project(center, matrix, bounds) {
                let axis = (0..3).find(|axis| {
                    let mut end = center;
                    end[*axis] += self.scene.camera.distance * 0.105;
                    project(end, matrix, bounds).is_some_and(|end| {
                        segment_distance(self.last_mouse, mouse_point(origin), mouse_point(end))
                            < 8.
                    })
                });
                if axis.is_some() {
                    self.start_transform(self.tool, axis, false, cx);
                    return;
                }
            }
        }
        let uv = (self.last_mouse - mouse_point(bounds.origin))
            / Vec2::new(bounds.size.width.into(), bounds.size.height.into());
        if let Some(hit) = self.scene.pick(self.scene.camera.ray(uv, aspect)) {
            let same = self.selected == Some(hit.object_id);
            self.selected = Some(hit.object_id);
            self.selected_face = self.edit_mode.then_some(hit.face);
            self.status = if self.edit_mode {
                format!("Face {} selected · E to extrude", hit.face + 1)
            } else {
                format!("Selected {}", self.selected_object().unwrap().name)
            };
            if same && self.tool != Tool::Select {
                self.start_transform(self.tool, None, false, cx);
            }
        } else {
            self.selected = None;
            self.selected_face = None;
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
        if self.transform_drag.is_some() {
            self.update_transform(position, event.modifiers.shift, cx);
        } else if let Some((button, pan)) = self.navigation {
            if event.pressed_button != Some(button) {
                self.navigation = None;
            } else {
                let delta = position - self.last_mouse;
                if pan {
                    self.scene
                        .camera
                        .pan_in_viewport(delta, f32::from(self.bounds.get().size.height));
                } else {
                    self.scene.camera.orbit(delta);
                }
                self.invalidate(false, cx);
            }
        }
        self.last_mouse = position;
    }

    pub(crate) fn start_transform(
        &mut self,
        tool: Tool,
        axis: Option<usize>,
        modal: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(original) = self
            .selected_object()
            .map(|instance| instance.object.clone())
        else {
            self.status = "Select an object first".into();
            cx.notify();
            return;
        };
        if self.edit_mode && self.selected_face.is_none() {
            self.status = "Select a face first".into();
            cx.notify();
            return;
        }
        self.tool = tool;
        self.transform_drag = Some(TransformDrag {
            tool,
            axis,
            start: self.last_mouse,
            original,
            before: self.scene.clone(),
            numeric: String::new(),
            modal,
            changed: false,
        });
        self.status = format!("{tool:?} · X / Y / Z to constrain · Type a value · Return to apply");
        cx.notify();
    }

    pub(crate) fn update_transform(
        &mut self,
        position: Vec2,
        precise: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = &mut self.transform_drag else {
            return;
        };
        let delta = (position - drag.start) * if precise { 0.1 } else { 1. };
        let explicit = drag
            .numeric
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite());
        let original = &drag.original;
        let mut edited_transform = original.transform;
        let camera = self.scene.camera;
        let inverse_view = camera.view_matrix().inverse();
        let right = inverse_view.x_axis.truncate();
        let up = inverse_view.y_axis.truncate();
        let scale = camera.distance * (camera.fov_y * 0.5).tan() * 2.
            / f32::from(self.bounds.get().size.height).max(1.);
        let free_translation = (right * delta.x - up * delta.y) * scale;
        let mut translation = free_translation;
        if let Some(axis) = drag.axis {
            let mut direction = Vec3::ZERO;
            direction[axis] = 1.;
            let projected = Vec2::new(direction.dot(right), -direction.dot(up));
            let amount = if projected.length_squared() > 0.02 {
                delta.dot(projected) / projected.length_squared() * scale
            } else {
                delta.x * scale
            };
            translation = direction * explicit.unwrap_or(amount).clamp(-100_000., 100_000.);
        } else if let Some(value) = explicit {
            translation = Vec3::X * value.clamp(-100_000., 100_000.);
        }
        let angle = explicit
            .map(f32::to_radians)
            .unwrap_or((delta.x - delta.y) * 0.01);
        let factor = explicit
            .unwrap_or(((delta.x - delta.y) * 0.007).exp())
            .clamp(0.001, 1000.);
        let rotation_axis = drag
            .axis
            .map(|a| {
                let mut v = Vec3::ZERO;
                v[a] = 1.;
                v
            })
            .unwrap_or(inverse_view.z_axis.truncate());
        if self.edit_mode {
            let original_mesh = drag.before.object_mesh(original.id);
            if let Some(face) = self
                .selected_face
                .and_then(|face| original_mesh?.faces.get(face))
            {
                let mesh = original_mesh.unwrap();
                let matrix = drag.before.world_transform(original.id).unwrap();
                let inverse = matrix.inverse();
                let center = face
                    .iter()
                    .map(|i| matrix.transform_point3(mesh.positions[*i as usize]))
                    .sum::<Vec3>()
                    / face.len() as f32;
                for index in face {
                    let p = matrix.transform_point3(mesh.positions[*index as usize]);
                    let transformed = match drag.tool {
                        Tool::Move => p + translation,
                        Tool::Rotate => {
                            center + Quat::from_axis_angle(rotation_axis, angle) * (p - center)
                        }
                        Tool::Scale => {
                            let mut scaling = Vec3::splat(factor);
                            if let Some(axis) = drag.axis {
                                scaling = Vec3::ONE;
                                scaling[axis] = factor;
                            }
                            center + (p - center) * scaling
                        }
                        Tool::Select => p,
                    };
                    if let Some(mesh) = self.scene.object_mesh_mut(original.id) {
                        mesh.positions[*index as usize] = inverse.transform_point3(transformed);
                    }
                }
            }
        } else {
            match drag.tool {
                Tool::Move => edited_transform.translation += translation,
                Tool::Rotate => {
                    let old = Quat::from_euler(
                        glam::EulerRot::XYZ,
                        original.transform.rotation.x,
                        original.transform.rotation.y,
                        original.transform.rotation.z,
                    );
                    let q = Quat::from_axis_angle(rotation_axis, angle) * old;
                    let (x, y, z) = q.to_euler(glam::EulerRot::XYZ);
                    edited_transform.rotation = Vec3::new(x, y, z);
                }
                Tool::Scale => {
                    if let Some(axis) = drag.axis {
                        edited_transform.scale[axis] =
                            (edited_transform.scale[axis] * factor).clamp(0.001, 1000.);
                    } else {
                        edited_transform.scale = (edited_transform.scale * factor)
                            .clamp(Vec3::splat(0.001), Vec3::splat(1000.));
                    }
                }
                Tool::Select => {}
            }
        }
        drag.changed = true;
        if !self.edit_mode
            && let Some(object) = self.scene.object_mut(original.id)
        {
            object.transform = edited_transform;
        }
        self.invalidate(true, cx);
    }

    pub(crate) fn finish_transform(&mut self, cancel: bool, cx: &mut Context<Self>) {
        let Some(drag) = self.transform_drag.take() else {
            return;
        };
        if cancel {
            self.scene = drag.before;
            self.status = "Transform cancelled".into();
            self.invalidate(true, cx);
        } else if drag.changed {
            if let Err(error) = self.scene.validate() {
                self.scene = drag.before;
                self.status = format!("Transform cancelled: {error}");
                self.invalidate(true, cx);
            } else {
                self.history.checkpoint(&drag.before);
                self.dirty = true;
                self.status = format!("{:?} applied", drag.tool);
                cx.notify();
            }
        }
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

    fn scroll(delta: gpui::ScrollDelta, modifiers: gpui::Modifiers) -> ScrollWheelEvent {
        ScrollWheelEvent {
            delta,
            modifiers,
            ..Default::default()
        }
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

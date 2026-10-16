use crate::app::{Command, EditMode, Studio, Tool};
use forma_render::RenderMode;
use glam::{Mat4, Vec2, Vec3};
use gpui::{
    AnyElement, Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, PathBuilder, Pixels,
    Point, ScrollWheelEvent, Size, TextRun, Window, canvas, div, fill, point, prelude::*, px, rgb,
    size,
};
use std::{collections::BTreeSet, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq)]
struct WorldBounds {
    min: Vec3,
    max: Vec3,
}

impl WorldBounds {
    fn from_points(points: impl IntoIterator<Item = Vec3>) -> Option<Self> {
        let mut points = points.into_iter().filter(|point| point.is_finite());
        let first = points.next()?;
        Some(points.fold(
            Self {
                min: first,
                max: first,
            },
            |bounds, point| Self {
                min: bounds.min.min(point),
                max: bounds.max.max(point),
            },
        ))
    }

    fn dimensions(self) -> Vec3 {
        self.max - self.min
    }
}

#[derive(Default)]
struct SelectionMeasurements {
    bounds: Option<WorldBounds>,
    edges: Vec<[Vec3; 2]>,
    edge_count: usize,
    edge_length: f64,
}

/// The selection key changes independently from geometry. Geometry invalidation
/// clears this cache, so camera motion and progressive frames do not walk meshes.
pub(crate) struct MeasurementCache {
    objects: BTreeSet<u64>,
    components: BTreeSet<forma_core::MeshElement>,
    mode: EditMode,
    values: Rc<SelectionMeasurements>,
}

fn measure_selection(
    scene: &forma_core::Scene,
    objects: &BTreeSet<u64>,
    components: &BTreeSet<forma_core::MeshElement>,
    mode: EditMode,
) -> SelectionMeasurements {
    let mut measurements = SelectionMeasurements::default();
    let mut object_bounds = Vec::new();
    for id in objects {
        if !scene.is_effectively_visible(*id) {
            continue;
        }
        let Some(object) = scene.mesh_instance(*id) else {
            continue;
        };
        let point = |index: u32| {
            object
                .mesh
                .positions
                .get(index as usize)
                .map(|point| object.world_transform.transform_point3(*point))
        };
        let bounds = if mode == EditMode::Object {
            WorldBounds::from_points(
                object
                    .mesh
                    .positions
                    .iter()
                    .map(|point| object.world_transform.transform_point3(*point)),
            )
        } else {
            WorldBounds::from_points(components.iter().flat_map(|element| {
                element
                    .vertex_indices(object.mesh)
                    .into_iter()
                    .filter_map(point)
            }))
        };
        if let Some(bounds) = bounds {
            object_bounds.extend([bounds.min, bounds.max]);
        }
        if mode == EditMode::Edge {
            // Canonicalize defensively so the same edge cannot count twice.
            let edges: BTreeSet<_> = components
                .iter()
                .filter_map(|element| match *element {
                    forma_core::MeshElement::Edge([a, b]) if a != b => Some([a.min(b), a.max(b)]),
                    _ => None,
                })
                .collect();
            for [a, b] in edges {
                let (Some(a), Some(b)) = (point(a), point(b)) else {
                    continue;
                };
                let length = a.as_dvec3().distance(b.as_dvec3());
                if !length.is_finite() {
                    continue;
                }
                measurements.edge_count += 1;
                measurements.edge_length += length;
                // Keep overlay work bounded even with thousands of selected edges.
                if measurements.edges.len() < 12 {
                    measurements.edges.push([a, b]);
                }
            }
        }
    }
    measurements.bounds = WorldBounds::from_points(object_bounds);
    measurements
}

fn measurement_value(value: f64) -> String {
    if value == 0. {
        "0".into()
    } else if !(0.001..100_000.).contains(&value.abs()) {
        format!("{value:.3e}")
    } else {
        format!("{value:.3}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .into()
    }
}

fn camera_view_label(camera: forma_core::Camera) -> &'static str {
    if !camera.orthographic {
        return "User perspective";
    }
    let facing = camera.view_matrix().inverse().z_axis.truncate();
    for (axis, positive, negative) in [
        (Vec3::X, "Right orthographic", "Left orthographic"),
        (Vec3::Y, "Top orthographic", "Bottom orthographic"),
        (Vec3::Z, "Front orthographic", "Back orthographic"),
    ] {
        if facing.dot(axis) > 0.99999 {
            return positive;
        }
        if facing.dot(-axis) > 0.99999 {
            return negative;
        }
    }
    "User orthographic"
}

struct ViewHint(&'static str, crate::theme::Colors);

impl Render for ViewHint {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.))
            .py(px(5.))
            .rounded(px(5.))
            .bg(rgb(self.1.panel))
            .border_1()
            .border_color(rgb(self.1.edge))
            .text_color(rgb(self.1.text))
            .text_size(px(11.))
            .child(self.0)
    }
}

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

fn measurement_label(
    text: String,
    center: Point<Pixels>,
    bounds: Bounds<Pixels>,
    color: u32,
    background: u32,
    window: &mut Window,
    cx: &mut gpui::App,
) -> Option<Bounds<Pixels>> {
    let run = TextRun {
        len: text.len(),
        font: window.text_style().font(),
        color: rgb(color).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let text = window
        .text_system()
        .shape_line(text.into(), px(11.), &[run], None);
    let label_bounds = Bounds::new(
        center - point((text.width + px(10.)) * 0.5, px(10.)),
        size(text.width + px(10.), px(20.)),
    );
    if !bounds.contains(&label_bounds.origin) || !bounds.contains(&label_bounds.bottom_right()) {
        return None;
    }
    window.paint_quad(fill(label_bounds, rgb(background)));
    let _ = text.paint(
        label_bounds.origin + point(px(5.), px(2.)),
        px(16.),
        window,
        cx,
    );
    Some(label_bounds)
}

fn paint_measurements(
    measurements: &SelectionMeasurements,
    matrix: Mat4,
    bounds: Bounds<Pixels>,
    colors: crate::theme::Colors,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let Some(world) = measurements.bounds else {
        return;
    };
    let world_center = (world.min + world.max) * 0.5;
    let projected_center = project(world_center, matrix, bounds).map(mouse_point);
    let mut labels: Vec<Bounds<Pixels>> = Vec::new();
    for axis in 0..3 {
        let mut end = world.min;
        end[axis] = world.max[axis];
        let (Some(a), Some(b)) = (
            project(world.min, matrix, bounds),
            project(end, matrix, bounds),
        ) else {
            continue;
        };
        let (a, b) = (mouse_point(a), mouse_point(b));
        if a.distance(b) < 42. {
            continue;
        }
        let direction = (b - a).normalize();
        let mut normal = Vec2::new(-direction.y, direction.x);
        if projected_center.is_some_and(|center| normal.dot((a + b) * 0.5 - center) < 0.) {
            normal = -normal;
        }
        let (start, end) = (a + normal * 18., b + normal * 18.);
        let points = [start, end].map(|p| point(px(p.x), px(p.y)));
        // A contrasting under-stroke keeps guides legible over every material.
        line(window, &points, colors.panel, 3.5);
        line(window, &points, colors.axis_ink[axis], 1.);
        for (from, to) in [
            (a + normal * 7., start + normal * 4.),
            (b + normal * 7., end + normal * 4.),
        ] {
            let points = [from, to].map(|p| point(px(p.x), px(p.y)));
            line(window, &points, colors.panel, 3.);
            line(window, &points, colors.axis_ink[axis], 1.);
        }
        let midpoint = (start + end) * 0.5 + normal * 12.;
        let position = point(px(midpoint.x), px(midpoint.y));
        if !labels.iter().any(|label| {
            label.intersects(&Bounds::new(
                position - point(px(40.), px(10.)),
                size(px(80.), px(20.)),
            ))
        }) && let Some(label) = measurement_label(
            format!(
                "{} {}",
                ["X", "Y", "Z"][axis],
                measurement_value(world.dimensions()[axis] as f64)
            ),
            position,
            bounds,
            colors.axis_ink[axis],
            colors.panel,
            window,
            cx,
        ) {
            labels.push(label);
        }
    }
    for [a, b] in &measurements.edges {
        let (Some(start), Some(end), Some(midpoint)) = (
            project(*a, matrix, bounds),
            project(*b, matrix, bounds),
            project((*a + *b) * 0.5, matrix, bounds),
        ) else {
            continue;
        };
        if mouse_point(start).distance(mouse_point(end)) < 34. {
            continue;
        }
        let position = midpoint + point(px(0.), px(-13.));
        if !labels.iter().any(|label| {
            label.intersects(&Bounds::new(
                position - point(px(32.), px(10.)),
                size(px(64.), px(20.)),
            ))
        }) && let Some(label) = measurement_label(
            measurement_value(a.as_dvec3().distance(b.as_dvec3())),
            position,
            bounds,
            colors.accent,
            colors.panel,
            window,
            cx,
        ) {
            labels.push(label);
        }
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
    query: &forma_core::SurfaceQuery,
    mode: EditMode,
    pointer: Vec2,
    bounds: Bounds<Pixels>,
) -> Option<(u64, [u32; 2])> {
    let aspect = f32::from(bounds.size.width) / f32::from(bounds.size.height).max(1.);
    let matrix = scene.camera.projection_matrix(aspect) * scene.camera.view_matrix();
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
            if distance < closest && visible_in_query(scene.camera, query, a.lerp(b, world_t)) {
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
    /// World-space extents of the visible selection, shared with verification
    /// and independent of camera projection or measurement display preference.
    pub(crate) fn selection_dimensions(&self) -> Option<Vec3> {
        self.selection_measurements()
            .bounds
            .map(WorldBounds::dimensions)
    }

    fn selection_measurements(&self) -> Rc<SelectionMeasurements> {
        let objects = self.selected_ids();
        let components = self.component_elements();
        let mut cache = self.measurement_cache.borrow_mut();
        if let Some(cache) = cache.as_ref()
            && cache.objects == objects
            && cache.components == components
            && cache.mode == self.edit_mode
        {
            return cache.values.clone();
        }
        let values = Rc::new(measure_selection(
            &self.scene,
            &objects,
            &components,
            self.edit_mode,
        ));
        *cache = Some(MeasurementCache {
            objects,
            components,
            mode: self.edit_mode,
            values: values.clone(),
        });
        values
    }

    fn orientation_gizmo(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.theme_colors();
        let view = self.scene.camera.view_matrix();
        let mut directions = Vec::with_capacity(6);
        for (axis, positive, negative, positive_hint, negative_hint) in [
            (
                0,
                Command::ViewRight,
                Command::ViewLeft,
                "Right view · click again for Left",
                "Left view · click again for Right",
            ),
            (
                1,
                Command::ViewTop,
                Command::ViewBottom,
                "Top view · click again for Bottom",
                "Bottom view · click again for Top",
            ),
            (
                2,
                Command::ViewFront,
                Command::ViewBack,
                "Front view · click again for Back",
                "Back view · click again for Front",
            ),
        ] {
            for positive_axis in [false, true] {
                let sign = if positive_axis { 1. } else { -1. };
                let vector = view.transform_vector3(glam::Mat3::IDENTITY.col(axis) * sign);
                let (target, opposite, hint) = if positive_axis {
                    (positive, negative, positive_hint)
                } else {
                    (negative, positive, negative_hint)
                };
                let active = self.scene.camera.orthographic && vector.z > 0.99999;
                directions.push((
                    vector,
                    axis,
                    positive_axis,
                    if active { opposite } else { target },
                    hint,
                    active,
                ));
            }
        }
        // Far-side handles paint first. Clicking the facing handle again reverses
        // it, so the opposite view remains reachable when two handles coincide.
        directions.sort_by(|a, b| a.0.z.total_cmp(&b.0.z));
        let mut gizmo = div()
            .absolute()
            .top(px(12.))
            .right(px(12.))
            .w(px(108.))
            .h(px(108.));
        for (vector, axis, positive, command, hint, active) in directions {
            let label = format!(
                "{}{}",
                if positive { "" } else { "−" },
                ["X", "Y", "Z"][axis]
            );
            gizmo = gizmo.child(
                div()
                    .id(("view-axis", axis * 2 + usize::from(positive)))
                    .absolute()
                    .left(px(43. + vector.x * 34.))
                    .top(px(43. - vector.y * 34.))
                    .w(px(22.))
                    .h(px(22.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(if active { colors.active } else { colors.panel }))
                    .border_1()
                    .border_color(rgb(if positive {
                        colors.axis_ink[axis]
                    } else {
                        colors.edge
                    }))
                    .text_size(px(10.))
                    .text_color(rgb(colors.axis_ink[axis]))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(colors.active_hover)))
                    .tooltip(move |_, cx| cx.new(|_| ViewHint(hint, colors)).into())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |studio, _, window, cx| {
                        cx.stop_propagation();
                        studio.execute(command, window, cx);
                    }))
                    .child(label),
            );
        }
        gizmo.into_any_element()
    }

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
        let query = self.visibility_query();
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
                    && visible_in_query(self.scene.camera, query, center)
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
        let measurements = self
            .show_measurements
            .then(|| self.selection_measurements());
        let painted_measurements = measurements.clone();
        let geometry = self.selection_center().filter(|_| {
            self.selected
                .is_some_and(|id| self.scene.is_effectively_visible(id))
        });
        let query = (!self.component_elements().is_empty() || self.selected_ids().len() > 1)
            .then(|| self.visibility_query());
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
                        if query.is_some_and(|q| {
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
                if let Some(measurements) = painted_measurements.as_ref() {
                    paint_measurements(measurements, matrix, bounds, t, window, cx);
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
                // The endpoint buttons use these same projected world axes.
                let origin = point(bounds.right() - px(66.), bounds.origin.y + px(66.));
                for (vector, color) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().zip(crate::ui::AXIS)
                {
                    let screen = view.transform_vector3(vector);
                    for sign in [-1., 1.] {
                        let end =
                            origin + point(px(screen.x * 34. * sign), px(-screen.y * 34. * sign));
                        line(
                            window,
                            &[origin, end],
                            color,
                            if sign > 0. { 2. } else { 1. },
                        );
                    }
                }
            },
        )
        .size_full();
        let view_label = camera_view_label(camera);
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
            .child(self.orientation_gizmo(cx))
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
        if let Some(measurements) = measurements {
            let mut panel = div()
                .absolute()
                .bottom(px(
                    if self
                        .transform_drag
                        .as_ref()
                        .is_some_and(|drag| drag.error.is_some())
                    {
                        100.
                    } else if self.transform_drag.is_some() {
                        58.
                    } else {
                        16.
                    },
                ))
                .right(px(16.))
                .px(px(11.))
                .py(px(9.))
                .rounded(px(6.))
                .bg(rgb(t.panel))
                .border_1()
                .border_color(rgb(t.edge))
                .text_size(px(11.))
                .text_color(rgb(t.muted))
                .child("World dimensions · m");
            if let Some(bounds) = measurements.bounds {
                let mut dimensions = div().flex().gap(px(14.)).mt(px(5.));
                for axis in 0..3 {
                    dimensions =
                        dimensions.child(div().text_color(rgb(t.axis_ink[axis])).child(format!(
                            "{} {}",
                            ["X", "Y", "Z"][axis],
                            measurement_value(bounds.dimensions()[axis] as f64)
                        )));
                }
                panel = panel.child(dimensions);
                if measurements.edge_count > 0 {
                    panel = panel.child(div().mt(px(5.)).text_color(rgb(t.text)).child(format!(
                        "{} edge{} · total {} m",
                        measurements.edge_count,
                        if measurements.edge_count == 1 {
                            ""
                        } else {
                            "s"
                        },
                        measurement_value(measurements.edge_length)
                    )));
                }
            } else {
                panel = panel.child(
                    div()
                        .mt(px(5.))
                        .text_color(rgb(t.text))
                        .child("Select geometry to measure"),
                );
            }
            viewport = viewport.child(panel);
        }
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
            let label = drag
                .mesh_tool
                .map(|tool| format!("{tool:?}"))
                .unwrap_or_else(|| format!("{:?}", drag.tool));
            let text = format!(
                "{}  {}  {}    ·    Enter to apply · Esc to cancel",
                label,
                if drag.mesh_tool == Some(crate::modelling::MeshTool::Bevel) {
                    "All edges".into()
                } else if drag.mesh_tool == Some(crate::modelling::MeshTool::Inset) {
                    "Face region".into()
                } else {
                    drag.constraint.label()
                },
                drag.numeric
            );
            viewport = viewport.child(
                div()
                    .absolute()
                    .bottom(px(16.))
                    .left(px(16.))
                    .max_w(gpui::relative(0.95))
                    .px(px(11.))
                    .py(px(7.))
                    .rounded(px(7.))
                    .bg(rgb(t.active))
                    .border_1()
                    .border_color(rgb(t.accent_line))
                    .text_color(rgb(t.accent))
                    .text_size(px(11.))
                    .child(text)
                    .when_some(drag.error.as_ref(), |d, error| {
                        d.child(
                            div()
                                .mt(px(4.))
                                .text_color(rgb(t.alert))
                                .child(error.clone()),
                        )
                    }),
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
        self.updates.open
            || self.updates.restart_pending
            || self.transform_drag.is_some()
            || self.shading_pie.is_some()
            || self.preview_open
            || self.help_open
            || self.palette_open
            || self.theme_picker.is_some()
            || self.stl_dialog.is_some()
    }

    pub(crate) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.updates.open || self.updates.restart_pending {
            cx.stop_propagation();
            return;
        }
        if self.stl_dialog.is_some() {
            cx.stop_propagation();
            return;
        }
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
            pick_component(
                &self.scene,
                self.visibility_query(),
                self.edit_mode,
                self.last_mouse,
                bounds,
            )
            .map(|(id, edge)| {
                (
                    id,
                    if self.edit_mode == EditMode::Vertex {
                        forma_core::MeshElement::Vertex(edge[0])
                    } else {
                        forma_core::MeshElement::Edge(edge)
                    },
                )
            })
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
        if self.updates.open || self.updates.restart_pending {
            cx.stop_propagation();
            return;
        }
        if self.stl_dialog.is_some() {
            cx.stop_propagation();
            return;
        }
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
    fn measurement_bounds_handle_empty_flat_and_invalid_points() {
        assert_eq!(WorldBounds::from_points([]), None);
        assert_eq!(WorldBounds::from_points([Vec3::splat(f32::NAN)]), None);
        let bounds = WorldBounds::from_points([
            Vec3::new(-3., 4., 0.),
            Vec3::new(2., -1., 0.),
            Vec3::splat(f32::INFINITY),
        ])
        .unwrap();
        assert_eq!(bounds.min, Vec3::new(-3., -1., 0.));
        assert_eq!(bounds.dimensions(), Vec3::new(5., 5., 0.));
    }

    #[test]
    fn measurement_bounds_include_parent_rotation_and_negative_scale() {
        let mut scene = forma_core::Scene::empty();
        let parent = scene.add_empty("Assembly").unwrap();
        let child = scene.add(forma_core::Primitive::Cube);
        scene.set_parent(child, Some(parent), false).unwrap();
        scene.object_mut(parent).unwrap().transform.translation = Vec3::new(10., 20., 30.);
        scene.object_mut(parent).unwrap().transform.rotation.z = std::f32::consts::FRAC_PI_2;
        scene.object_mut(child).unwrap().transform.scale = Vec3::new(-2., 3., 4.);
        let values = measure_selection(&scene, &[child].into(), &BTreeSet::new(), EditMode::Object);
        let bounds = values.bounds.unwrap();
        assert!((bounds.dimensions() - Vec3::new(6., 4., 8.)).length() < 0.00001);
        assert!(((bounds.min + bounds.max) * 0.5 - Vec3::new(10., 20., 30.)).length() < 0.00001);
        scene.object_mut(parent).unwrap().visible = false;
        assert!(
            measure_selection(&scene, &[child].into(), &BTreeSet::new(), EditMode::Object)
                .bounds
                .is_none()
        );
    }

    #[test]
    fn measurement_bounds_combine_objects_and_limit_component_selection() {
        use forma_core::MeshElement;
        let mut scene = forma_core::Scene::empty();
        let left = scene.add(forma_core::Primitive::Cube);
        let right = scene.add(forma_core::Primitive::Cube);
        scene.object_mut(right).unwrap().transform.translation.x = 6.;
        let values = measure_selection(
            &scene,
            &[left, right].into(),
            &BTreeSet::new(),
            EditMode::Object,
        );
        assert_eq!(values.bounds.unwrap().dimensions(), Vec3::new(8., 2., 2.));
        let values = measure_selection(
            &scene,
            &[left].into(),
            &[MeshElement::Edge([0, 1])].into(),
            EditMode::Edge,
        );
        assert_eq!(values.bounds.unwrap().dimensions(), Vec3::new(2., 0., 0.));
        assert!(
            measure_selection(&scene, &[left].into(), &BTreeSet::new(), EditMode::Vertex)
                .bounds
                .is_none()
        );
    }

    #[test]
    fn measurement_edge_totals_use_world_lengths_and_count_each_edge_once() {
        use forma_core::MeshElement;
        let mut scene = forma_core::Scene::empty();
        let id = scene.add(forma_core::Primitive::Cube);
        scene.object_mut(id).unwrap().transform.scale = Vec3::new(-2., 3., 4.);
        scene.object_mut(id).unwrap().transform.rotation.z = 0.37;
        let components = [
            MeshElement::Edge([0, 1]),
            MeshElement::Edge([1, 0]),
            MeshElement::Edge([1, 2]),
            MeshElement::Edge([0, 999]),
        ]
        .into();
        let values = measure_selection(&scene, &[id].into(), &components, EditMode::Edge);
        assert_eq!(values.edge_count, 2);
        assert!((values.edge_length - 10.).abs() < 0.00001);
        assert_eq!(values.edges.len(), 2);
    }

    #[test]
    fn measurement_labels_are_bounded_while_edge_totals_are_complete() {
        let mut scene = forma_core::Scene::empty();
        let id = scene.add(forma_core::Primitive::Sphere);
        let edges = scene.object_mesh(id).unwrap().edges();
        let expected: f64 = edges
            .iter()
            .map(|[a, b]| {
                let mesh = scene.object_mesh(id).unwrap();
                mesh.positions[*a as usize]
                    .as_dvec3()
                    .distance(mesh.positions[*b as usize].as_dvec3())
            })
            .sum();
        let components = edges
            .iter()
            .copied()
            .map(forma_core::MeshElement::Edge)
            .collect();
        let values = measure_selection(&scene, &[id].into(), &components, EditMode::Edge);
        assert!(values.edge_count > 12);
        assert_eq!(values.edge_count, edges.len());
        assert_eq!(values.edges.len(), 12);
        assert!((values.edge_length - expected).abs() < 0.00001);
    }

    #[test]
    fn measurement_values_preserve_small_lengths_and_trim_padding() {
        assert_eq!(measurement_value(0.), "0");
        assert_eq!(measurement_value(12.500), "12.5");
        assert_eq!(measurement_value(100.), "100");
        assert_eq!(measurement_value(0.00001), "1.000e-5");
        assert_eq!(measurement_value(1_000_000.), "1.000e6");
    }

    #[test]
    fn named_views_cover_both_sides_of_every_axis() {
        use std::f32::consts::{FRAC_PI_2, PI};
        for (yaw, pitch, label) in [
            (0., 0., "Front orthographic"),
            (PI, 0., "Back orthographic"),
            (FRAC_PI_2, 0., "Right orthographic"),
            (-FRAC_PI_2, 0., "Left orthographic"),
            (0., FRAC_PI_2, "Top orthographic"),
            (0., -FRAC_PI_2, "Bottom orthographic"),
        ] {
            let camera = forma_core::Camera {
                yaw,
                pitch,
                orthographic: true,
                ..Default::default()
            };
            assert_eq!(camera_view_label(camera), label);
        }
        assert_eq!(
            camera_view_label(forma_core::Camera::default()),
            "User perspective"
        );
    }

    #[test]
    fn component_picking_targets_vertices_and_polygon_edges() {
        let mut scene = forma_core::Scene::empty();
        let id = scene.add(forma_core::Primitive::Cube);
        scene.camera.target = Vec3::ZERO;
        scene.camera.yaw = 0.;
        scene.camera.pitch = 0.;
        scene.camera.distance = 6.;
        let bounds = Bounds::new(point(px(50.), px(30.)), size(px(800.), px(600.)));
        let query = forma_core::SurfaceQuery::new(&scene);
        for orthographic in [false, true] {
            scene.camera.orthographic = orthographic;
            let matrix = scene.camera.projection_matrix(800. / 600.) * scene.camera.view_matrix();
            let point_at = |p| mouse_point(project(p, matrix, bounds).unwrap());
            let vertex = pick_component(
                &scene,
                &query,
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
                &query,
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
                    &query,
                    EditMode::Edge,
                    point_at(Vec3::new(0., 0., 1.)),
                    bounds
                )
                .is_none(),
                "triangulation diagonal must not be selectable"
            );
            assert!(pick_component(&scene, &query, EditMode::Vertex, Vec2::ZERO, bounds).is_none());
        }
        scene.object_mut(id).unwrap().selectable = false;
        assert!(
            pick_component(
                &scene,
                &query,
                EditMode::Vertex,
                Vec2::new(450., 330.),
                bounds
            )
            .is_none()
        );
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

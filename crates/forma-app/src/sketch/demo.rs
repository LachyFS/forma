//! Prepared Draw mode states for `--ui-capture`, driven through the same
//! pointer handlers as real input.
use super::{DrawCommand, DrawTarget, DrawTool};
use crate::app::{Command, Studio};
use forma_core::{Camera, Primitive, Scene, SketchCurve, SketchEntity, Vec2, Vec3};
use gpui::{Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Window, point, px};
use std::cell::RefCell;

thread_local! {
    static SAVED: RefCell<Option<(Scene, Camera, Option<u64>)>> = const { RefCell::new(None) };
}

/// Saves the workspace, then clears the stage for a top-down drawing view:
/// the default scene's overhead area light would otherwise fill it.
fn save(s: &mut Studio, cx: &mut Context<Studio>) {
    SAVED.with(|saved| *saved.borrow_mut() = Some((s.scene.clone(), s.scene.camera, s.selected)));
    for object in &mut s.scene.objects {
        if object.name.starts_with("Key") {
            object.visible = false;
        }
    }
    s.select_objects(Default::default(), None);
    s.invalidate(true, cx);
}

/// Restores the workspace saved by a scenario's `enter`.
pub(crate) fn restore(s: &mut Studio, _: &mut Window, cx: &mut Context<Studio>) {
    s.abandon_draw();
    if let Some((scene, camera, selected)) = SAVED.with(|saved| saved.borrow_mut().take()) {
        s.scene = scene;
        s.scene.camera = camera;
        s.select_objects(selected.into_iter().collect(), selected);
    }
    s.dirty = false;
    s.sketch.revision += 1;
    s.invalidate(true, cx);
}

fn screen(s: &Studio, p: Vec2) -> gpui::Point<gpui::Pixels> {
    let frame = s.sketch_frame().expect("drawing");
    let v = s.plane_to_screen(&frame, p).unwrap_or_default();
    point(px(v.x), px(v.y))
}

pub(crate) fn hover(s: &mut Studio, p: Vec2, cx: &mut Context<Studio>) {
    let position = screen(s, p);
    s.draw_mouse_move(
        &MouseMoveEvent {
            position,
            ..Default::default()
        },
        cx,
    );
}

pub(crate) fn click(s: &mut Studio, p: Vec2, w: &mut Window, cx: &mut Context<Studio>) {
    let position = screen(s, p);
    hover(s, p, cx);
    s.draw_mouse_down(
        &MouseDownEvent {
            button: MouseButton::Left,
            position,
            click_count: 1,
            ..Default::default()
        },
        w,
        cx,
    );
    s.draw_mouse_up(
        &MouseUpEvent {
            button: MouseButton::Left,
            position,
            click_count: 1,
            ..Default::default()
        },
        cx,
    );
}

fn add(s: &mut Studio, curves: Vec<SketchCurve>, construction: bool, cx: &mut Context<Studio>) {
    s.edit_active_sketch(
        "Demo",
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
}

fn rectangle(min: Vec2, max: Vec2) -> Vec<SketchCurve> {
    let corners = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
    (0..4)
        .map(|i| SketchCurve::Line {
            start: corners[i],
            end: corners[(i + 1) % 4],
        })
        .collect()
}

fn circle(center: Vec2, radius: f32) -> SketchCurve {
    SketchCurve::Circle { center, radius }
}

/// Enters Draw on the ground with the camera already settled.
fn enter_ground(s: &mut Studio, cx: &mut Context<Studio>) {
    s.last_mouse = Vec2::splat(-10.0);
    s.enter_draw(DrawTarget::AtPointer, cx);
    s.complete_camera_animation(cx);
}

pub(crate) fn empty_plane(s: &mut Studio, _: &mut Window, cx: &mut Context<Studio>) {
    save(s, cx);
    s.scene.camera.distance = 7.0;
    enter_ground(s, cx);
    hover(s, Vec2::new(1.05, 0.62), cx);
}

pub(crate) fn line_inference(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    save(s, cx);
    s.scene.camera.distance = 7.0;
    enter_ground(s, cx);
    add(
        s,
        rectangle(Vec2::new(-2.0, -0.4), Vec2::new(-0.6, 1.0)),
        false,
        cx,
    );
    s.execute(Command::Draw(DrawCommand::Tool(DrawTool::Line)), w, cx);
    click(s, Vec2::new(0.2, -0.4), w, cx);
    click(s, Vec2::new(1.4, -0.4), w, cx);
    // Track the rectangle's top corner, then come back level with it.
    hover(s, Vec2::new(-0.6, 1.0), cx);
    hover(s, Vec2::new(1.43, 0.97), cx);
}

pub(crate) fn numeric_entry(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    save(s, cx);
    s.scene.camera.distance = 7.0;
    enter_ground(s, cx);
    s.execute(Command::Draw(DrawCommand::Tool(DrawTool::Rectangle)), w, cx);
    click(s, Vec2::new(-1.0, -0.5), w, cx);
    hover(s, Vec2::new(0.9, 0.6), cx);
    for key in ["1", "5", "0", "c", "m"] {
        s.sketch.session.as_mut().unwrap().numeric.push(key);
    }
    s.sketch.session.as_mut().unwrap().numeric.advance(2, false);
    for key in ["0", ".", "8"] {
        s.sketch.session.as_mut().unwrap().numeric.push(key);
    }
}

pub(crate) fn profiles(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    save(s, cx);
    s.scene.camera.distance = 7.0;
    enter_ground(s, cx);
    add(
        s,
        rectangle(Vec2::new(-2.0, -1.0), Vec2::new(0.6, 0.8)),
        false,
        cx,
    );
    add(s, vec![circle(Vec2::new(0.6, -0.1), 0.85)], false, cx);
    add(s, vec![circle(Vec2::new(-1.2, -0.1), 0.35)], false, cx);
    add(
        s,
        rectangle(Vec2::new(1.8, -1.0), Vec2::new(2.6, 0.9)),
        false,
        cx,
    );
    add(
        s,
        vec![SketchCurve::Line {
            start: Vec2::new(-2.3, -0.1),
            end: Vec2::new(2.9, -0.1),
        }],
        true,
        cx,
    );
    s.execute(Command::Draw(DrawCommand::Tool(DrawTool::Select)), w, cx);
    hover(s, Vec2::new(0.95, 0.35), cx);
}

pub(crate) fn tilted_face(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    save(s, cx);
    let block = s.scene.add(Primitive::Cube);
    if let Some(object) = s.scene.object_mut(block) {
        object.name = "Tilted block".into();
        object.transform.translation = Vec3::new(-0.2, 1.1, 0.4);
        object.transform.rotation = Vec3::new(0.5, 0.35, 0.25);
        object.transform.scale = Vec3::new(1.1, 0.35, 0.8);
    }
    s.invalidate(true, cx);
    let top = s
        .scene
        .world_transform(block)
        .unwrap()
        .transform_point3(Vec3::new(0.0, 1.0, 0.0));
    let bounds = s.bounds.get();
    if let Some(p) = crate::viewport::project(top, s.view_projection(), bounds) {
        s.last_mouse = crate::viewport::mouse_point(p);
    }
    s.enter_draw(DrawTarget::AtPointer, cx);
    s.complete_camera_animation(cx);
    let reference = s.sketch.session.as_ref().unwrap().reference.clone();
    let (min, max) = reference
        .iter()
        .map(SketchCurve::bounds)
        .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
        .unwrap_or((Vec2::ZERO, Vec2::ONE));
    let size = max - min;
    add(
        s,
        rectangle(
            min + size * Vec2::new(0.1, 0.18),
            min + size * Vec2::new(0.48, 0.82),
        ),
        false,
        cx,
    );
    add(
        s,
        vec![circle(min + size * Vec2::new(0.72, 0.5), size.y * 0.26)],
        false,
        cx,
    );
    s.execute(Command::Draw(DrawCommand::Tool(DrawTool::Circle)), w, cx);
    click(s, min + size * Vec2::new(0.29, 0.5), w, cx);
    hover(
        s,
        min + size * Vec2::new(0.29, 0.5) + Vec2::new(size.y * 0.13, 0.0),
        cx,
    );
}

pub(crate) fn finished_objects(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    save(s, cx);
    enter_ground(s, cx);
    add(
        s,
        rectangle(Vec2::new(-2.6, 0.9), Vec2::new(-1.2, 2.1)),
        false,
        cx,
    );
    add(s, vec![circle(Vec2::new(-1.9, 1.5), 0.35)], false, cx);
    let polygon = super::tools::polygon_points(Vec2::new(2.3, -1.4), Vec2::new(2.9, -1.4), 6);
    add(
        s,
        (0..6)
            .map(|i| SketchCurve::Line {
                start: polygon[i],
                end: polygon[(i + 1) % 6],
            })
            .collect(),
        false,
        cx,
    );
    s.finish_draw(cx);
    s.complete_camera_animation(cx);
    let first = s.selected;
    s.enter_draw(
        DrawTarget::Plane(forma_core::SketchPlane::front(), "Front plane".into()),
        cx,
    );
    s.complete_camera_animation(cx);
    add(
        s,
        rectangle(Vec2::new(1.6, 0.0), Vec2::new(2.6, 1.6)),
        false,
        cx,
    );
    add(
        s,
        vec![
            SketchCurve::arc_through(
                Vec2::new(1.6, 1.6),
                Vec2::new(2.1, 2.1),
                Vec2::new(2.6, 1.6),
            )
            .unwrap(),
        ],
        false,
        cx,
    );
    s.finish_draw(cx);
    s.complete_camera_animation(cx);
    s.scene.camera = Camera {
        distance: 11.0,
        ..Camera::default()
    };
    if let Some(first) = first {
        s.select_objects([first].into_iter().collect(), Some(first));
    }
    let _ = w;
    s.invalidate(false, cx);
}

/// Draw in a 1000 px window with a wide sidebar: the toolbar's compact fit.
pub(crate) fn narrow(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    save(s, cx);
    w.resize(gpui::size(px(1000.), px(650.)));
    s.panels.set_sidebar_width(560.);
    s.scene.camera.distance = 7.0;
    enter_ground(s, cx);
    s.execute(Command::Draw(DrawCommand::Tool(DrawTool::Rectangle)), w, cx);
    click(s, Vec2::new(-0.6, -0.4), w, cx);
    hover(s, Vec2::new(0.5, 0.3), cx);
}

pub(crate) fn narrow_leave(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    restore(s, w, cx);
    s.panels.set_sidebar_width(crate::ui::SIDEBAR_DEFAULT_WIDTH);
    w.resize(gpui::size(px(1512.), px(960.)));
    s.invalidate(false, cx);
}

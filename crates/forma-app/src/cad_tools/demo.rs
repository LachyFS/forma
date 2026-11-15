//! Prepared modelling-tool states for `--ui-capture`, driven through the
//! viewport's own pointer handlers. Parts are regenerated in-process here so
//! scenarios can pick their faces immediately; the app itself always uses
//! the worker.
use super::{Tool, ToolCommand};
use crate::app::{Command, EditMode, Studio};
use crate::theme::Theme;
use forma_core::{
    Camera, FeatureKind, FeatureOperation, PartData, Placement, Primitive, PrimitiveFeature,
    PrimitiveShape, Scene, Sketch, SketchCurve, SketchEntity, SketchPlane, Vec2, Vec3,
};
use gpui::{Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Window, point, px};
use std::cell::RefCell;

thread_local! {
    static SAVED: RefCell<Option<(Scene, Option<u64>)>> = const { RefCell::new(None) };
}

fn save(s: &mut Studio) {
    SAVED.with(|saved| *saved.borrow_mut() = Some((s.scene.clone(), s.selected)));
}

/// Restores the workspace saved by a scenario's `enter`.
pub(crate) fn restore(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    s.cad_tool.tool = None;
    s.cad_tool.last = None;
    s.cad_tool.pending.clear();
    s.cad_tool.region_hover = None;
    s.cad_tool.edit = Default::default();
    if s.edit_mode != EditMode::Object {
        s.execute(Command::SetEditMode(EditMode::Object), w, cx);
    }
    s.edit_mode = EditMode::Object;
    if let Some((scene, selected)) = SAVED.with(|saved| saved.borrow_mut().take()) {
        s.scene = scene;
        s.select_objects(selected.into_iter().collect(), selected);
    }
    s.dirty = false;
    s.invalidate(true, cx);
}

pub(crate) fn restore_theme(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    restore(s, w, cx);
    s.apply_theme(Theme::Forma, cx);
}

/// Pieces a scenario stages.
#[derive(Default, Clone, Copy)]
struct Stage {
    base: u64,
    ground: u64,
    top: u64,
    cube: u64,
    profile: u64,
}

fn rectangle(min: Vec2, max: Vec2) -> Vec<SketchEntity> {
    let c = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
    (0..4)
        .map(|i| {
            SketchCurve::Line {
                start: c[i],
                end: c[(i + 1) % 4],
            }
            .into()
        })
        .collect()
}

/// The studio plinth with a parametric block, a ring sketch on the ground,
/// a sketch on the block's top face, a mesh cube and a revolve profile.
fn stage(s: &mut Studio, cx: &mut Context<Studio>) -> Stage {
    save(s);
    let mut scene = Scene::default();
    let showcase: Vec<u64> = scene
        .objects
        .iter()
        .filter(|o| ["Torus", "Sphere", "Rounded cube"].contains(&o.name.as_str()))
        .map(|o| o.id)
        .collect();
    for id in showcase {
        scene.remove(id);
    }
    scene.purge_orphans();
    let mut part = PartData::new(0, "Block");
    part.push_feature(
        "Block",
        FeatureKind::Primitive(PrimitiveFeature {
            shape: PrimitiveShape::Box {
                size: Vec3::new(2.0, 0.5, 1.4),
            },
            placement: Placement {
                origin: Vec3::new(0.0, 0.25, 0.0),
                rotation: Vec3::ZERO,
            },
            operation: FeatureOperation::Join,
        }),
    );
    let base = scene.add_part("Block", part).unwrap();
    scene.object_mut(base).unwrap().transform.translation = Vec3::new(0.6, 0.0, 0.2);
    *scene.object_material_mut(base).unwrap() = forma_core::Material {
        base_color: Vec3::new(0.78, 0.8, 0.83),
        metallic: 0.85,
        roughness: 0.34,
        ..Default::default()
    };
    let mut ring = Sketch::new(SketchPlane::ground());
    ring.entities = rectangle(Vec2::new(-2.95, -0.55), Vec2::new(-1.45, 0.55));
    ring.entities.push(
        SketchCurve::Circle {
            center: Vec2::new(-2.2, 0.0),
            radius: 0.3,
        }
        .into(),
    );
    let ground = scene.add_sketch("Ring sketch", ring).unwrap();
    let mut top = Sketch::new(SketchPlane {
        origin: Vec3::new(0.6, 0.5, 0.2),
        ..SketchPlane::ground()
    });
    top.entities = rectangle(Vec2::new(-0.75, -0.35), Vec2::new(0.05, 0.35));
    top.entities.push(
        SketchCurve::Circle {
            center: Vec2::new(0.55, 0.0),
            radius: 0.26,
        }
        .into(),
    );
    let top = scene.add_sketch("Top sketch", top).unwrap();
    let cube = scene.add(Primitive::Cube);
    {
        let object = scene.object_mut(cube).unwrap();
        object.name = "Mesh cube".into();
        object.transform.translation = Vec3::new(2.45, 0.42, 1.25);
        object.transform.scale = Vec3::splat(0.42);
    }
    let mut profile = Sketch::new(SketchPlane::front());
    let p = Vec2::new;
    profile.entities = vec![
        SketchCurve::Line {
            start: p(0.12, 0.0),
            end: p(0.42, 0.0),
        }
        .into(),
        SketchCurve::Line {
            start: p(0.42, 0.0),
            end: p(0.42, 0.18),
        }
        .into(),
        SketchCurve::arc_through(p(0.42, 0.18), p(0.3, 0.5), p(0.22, 0.85))
            .unwrap()
            .into(),
        SketchCurve::Line {
            start: p(0.22, 0.85),
            end: p(0.12, 0.85),
        }
        .into(),
        SketchCurve::Line {
            start: p(0.12, 0.85),
            end: p(0.12, 0.0),
        }
        .into(),
        SketchEntity {
            curve: SketchCurve::Line {
                start: p(0.0, -0.1),
                end: p(0.0, 1.0),
            },
            construction: true,
        },
    ];
    let profile = scene.add_sketch("Vase profile", profile).unwrap();
    scene.object_mut(profile).unwrap().transform.translation = Vec3::new(-2.3, 0.0, -1.5);
    scene.camera = Camera {
        target: Vec3::new(-0.2, 0.35, 0.0),
        yaw: 0.42,
        pitch: 0.5,
        distance: 7.4,
        ..scene.camera
    };
    regenerate_now(&mut scene);
    s.scene = scene;
    s.select_objects(Default::default(), None);
    s.invalidate(true, cx);
    Stage {
        base,
        ground,
        top,
        cube,
        profile,
    }
}

/// Regenerates every stale part in-process (captures only).
fn regenerate_now(scene: &mut Scene) {
    let mut cache = forma_cad::RegenCache::default();
    let stale: Vec<PartData> = scene
        .parts
        .iter()
        .filter(|p| !p.is_evaluated())
        .cloned()
        .collect();
    for part in stale {
        let result = forma_cad::regenerate(
            &forma_cad::RegenRequest {
                part_id: part.id,
                part: part.clone(),
                skip: Vec::new(),
            },
            &mut cache,
            &mut |_| {},
        );
        let _ = scene.set_part_tessellation(part.id, part.stamp, result.mesh, result.issues);
    }
}

fn screen(s: &Studio, world: Vec3) -> gpui::Point<gpui::Pixels> {
    let v = s.world_to_screen(world).unwrap_or_default();
    point(px(v.x), px(v.y))
}

fn hover(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>, at: gpui::Point<gpui::Pixels>) {
    s.mouse_move(
        &MouseMoveEvent {
            position: at,
            ..Default::default()
        },
        w,
        cx,
    );
}

fn press(
    s: &mut Studio,
    w: &mut Window,
    cx: &mut Context<Studio>,
    at: gpui::Point<gpui::Pixels>,
    shift: bool,
) {
    hover(s, w, cx, at);
    s.mouse_down(
        &MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: gpui::Modifiers {
                shift,
                ..Default::default()
            },
            click_count: 1,
            ..Default::default()
        },
        w,
        cx,
    );
}

fn drag(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>, to: gpui::Point<gpui::Pixels>) {
    let from = s.last_mouse;
    let target = Vec2::new(to.x.into(), to.y.into());
    for step in 1..=8 {
        let p = from.lerp(target, step as f32 / 8.0);
        s.mouse_move(
            &MouseMoveEvent {
                position: point(px(p.x), px(p.y)),
                pressed_button: Some(MouseButton::Left),
                ..Default::default()
            },
            w,
            cx,
        );
    }
}

/// World point of a sketch-plane coordinate.
fn sketch_point(s: &Studio, object: u64, p: Vec2) -> Vec3 {
    let sketch = s.scene.object_sketch(object).unwrap();
    s.scene
        .world_transform(object)
        .unwrap()
        .transform_point3(sketch.plane.to_world(p))
}

pub(crate) fn region_hover(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    let stage = stage(s, cx);
    let at = screen(s, sketch_point(s, stage.ground, Vec2::new(-1.75, 0.3)));
    hover(s, w, cx, at);
    cx.notify();
}

fn extrude_on_top(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>, region: Vec2, up: f32) {
    let stage = stage(s, cx);
    let start = sketch_point(s, stage.top, region);
    s.last_mouse = Vec2::new(screen(s, start).x.into(), screen(s, start).y.into());
    s.start_sketch_tool(super::SketchToolKind::Extrude, None, cx);
    press(s, w, cx, screen(s, start), false);
    drag(s, w, cx, screen(s, start + Vec3::Y * up));
}

pub(crate) fn extrude_new(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    let stage = stage(s, cx);
    s.select_objects([stage.ground].into_iter().collect(), Some(stage.ground));
    s.start_sketch_tool(super::SketchToolKind::Extrude, Some(stage.ground), cx);
    let start = sketch_point(s, stage.ground, Vec2::new(-1.75, 0.3));
    press(s, w, cx, screen(s, start), false);
    drag(s, w, cx, screen(s, start + Vec3::Y * 0.62));
}

pub(crate) fn extrude_join(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    extrude_on_top(s, w, cx, Vec2::new(0.55, 0.0), 0.42);
}

pub(crate) fn extrude_cut(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    extrude_on_top(s, w, cx, Vec2::new(-0.35, 0.0), -0.3);
}

pub(crate) fn extrude_light(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    s.apply_theme(Theme::Paper, cx);
    extrude_join(s, w, cx);
}

pub(crate) fn revolve(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    let stage = stage(s, cx);
    s.scene.camera.target = Vec3::new(-2.15, 0.42, -1.5);
    s.scene.camera.distance = 3.1;
    s.scene.camera.yaw = 0.55;
    s.scene.camera.pitch = 0.38;
    s.select_objects([stage.profile].into_iter().collect(), Some(stage.profile));
    s.start_sketch_tool(super::SketchToolKind::Revolve, Some(stage.profile), cx);
    if let Some(Tool::Sketch(tool)) = s.cad_tool.tool.as_mut() {
        tool.angle = 270f32.to_radians();
    }
    s.sketch_tool_changed(false, cx);
    // Rest the pointer on the angle handle.
    let handle = match s.cad_tool.tool.as_ref() {
        Some(Tool::Sketch(tool)) => tool.angle_handle_screen(s),
        _ => None,
    };
    if let Some(handle) = handle {
        hover(s, w, cx, point(px(handle.x), px(handle.y)));
    }
    let _ = w;
}

pub(crate) fn push_pull_part(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    let stage = stage(s, cx);
    let face = Vec3::new(0.6, 0.25, 0.9);
    s.select_objects([stage.base].into_iter().collect(), Some(stage.base));
    s.execute(Command::CadTool(ToolCommand::PushPull), w, cx);
    press(s, w, cx, screen(s, face), false);
    drag(s, w, cx, screen(s, face + Vec3::Z * 0.5));
}

pub(crate) fn push_pull_mesh(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    let stage = stage(s, cx);
    let top = Vec3::new(2.45, 0.84, 1.25);
    s.select_objects([stage.cube].into_iter().collect(), Some(stage.cube));
    s.execute(Command::CadTool(ToolCommand::PushPull), w, cx);
    press(s, w, cx, screen(s, top), false);
    drag(s, w, cx, screen(s, top + Vec3::Y * 0.55));
}

/// The CAD showcase bracket in part edit mode.
fn bracket(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>, mode: EditMode) -> u64 {
    save(s);
    let mut scene = crate::cad::demo_scene();
    regenerate_now(&mut scene);
    let id = scene
        .objects
        .iter()
        .find(|o| o.name == "Bracket")
        .map(|o| o.id)
        .unwrap();
    scene.camera = Camera {
        target: Vec3::new(-2.0, 0.3, 0.25),
        yaw: 0.55,
        pitch: 0.55,
        distance: 4.4,
        ..scene.camera
    };
    s.scene = scene;
    s.select_objects([id].into_iter().collect(), Some(id));
    s.invalidate(true, cx);
    s.execute(Command::ToggleEdit, w, cx);
    if mode == EditMode::Edge {
        s.execute(Command::SetEditMode(EditMode::Edge), w, cx);
    }
    id
}

pub(crate) fn part_edit_faces(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    bracket(s, w, cx, EditMode::Face);
    // The plate's top, then hover the boss's side.
    let plate = Vec3::new(-2.0 - 0.55, 0.3, 0.25 + 0.25);
    press(s, w, cx, screen(s, plate), false);
    s.cad_tool_mouse_up(
        &MouseUpEvent {
            button: MouseButton::Left,
            ..Default::default()
        },
        cx,
    );
    let front = Vec3::new(-2.0 + 0.3, 0.15, 0.25 + 0.75);
    press(s, w, cx, screen(s, front), true);
    hover(
        s,
        w,
        cx,
        screen(s, Vec3::new(-2.0 + 0.27, 0.55, 0.25 + 0.27)),
    );
}

pub(crate) fn part_edit_edges(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    bracket(s, w, cx, EditMode::Edge);
    let front_top = Vec3::new(-2.0 + 0.2, 0.3, 0.25 + 0.75);
    let side_top = Vec3::new(-2.0 + 1.1, 0.3, 0.25 + 0.1);
    press(s, w, cx, screen(s, front_top), false);
    press(s, w, cx, screen(s, side_top), true);
    hover(
        s,
        w,
        cx,
        screen(s, Vec3::new(-2.0 + 0.96, 0.15, 0.25 + 0.75)),
    );
}

pub(crate) fn chamfer(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    bracket(s, w, cx, EditMode::Edge);
    let front_top = Vec3::new(-2.0 + 0.2, 0.3, 0.25 + 0.75);
    let side_top = Vec3::new(-2.0 + 1.1, 0.3, 0.25 + 0.1);
    press(s, w, cx, screen(s, front_top), false);
    press(s, w, cx, screen(s, side_top), true);
    let start = screen(s, front_top + Vec3::new(0.0, 0.1, 0.1));
    hover(s, w, cx, start);
    s.start_chamfer(cx);
    let end = screen(s, front_top + Vec3::new(0.0, 0.065, 0.065));
    hover(s, w, cx, end);
}

pub(crate) fn adjust(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    let stage = stage(s, cx);
    s.select_objects([stage.ground].into_iter().collect(), Some(stage.ground));
    s.start_sketch_tool(super::SketchToolKind::Extrude, Some(stage.ground), cx);
    if let Some(Tool::Sketch(tool)) = s.cad_tool.tool.as_mut() {
        tool.numeric = "0.45".into();
    }
    s.commit_cad_tool(cx);
    let _ = (w, stage.top, stage.cube);
}

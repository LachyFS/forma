//! Native smoke workflow for the modelling tools, through real keys (the
//! window's keystroke dispatch) and the viewport's pointer handlers, with
//! the real CAD worker: draw a ring, extrude it into a part, push/pull its
//! top, sketch on that face and cut through, push/pull a mesh, chamfer an
//! edge, adjust it in the panel, then undo and redo.
use super::{AdjustParam, Operation, Tool, ToolCommand, geometry::mesh_volume};
use crate::app::{Command, EditMode, Field, Studio};
use crate::smoke::{keys, settle, wait_for};
use anyhow::{Context as _, Result, ensure};
use forma_core::{FeatureKind, FeatureOperation, Primitive, Scene, Vec2, Vec3};
use forma_render::RenderMode;
use gpui::{
    AsyncApp, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Timer,
    WindowHandle, point, px,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
enum Pointer {
    Move,
    Drag,
    Down,
    Up,
}

/// Pointer input through the viewport's handlers and the window's release
/// handler (Draw and the tools both listen for releases there).
async fn pointer(
    window: WindowHandle<Studio>,
    kind: Pointer,
    at: Vec2,
    cx: &mut AsyncApp,
) -> Result<()> {
    let position = point(px(at.x), px(at.y));
    window.update(cx, |s, w, cx| match kind {
        Pointer::Move | Pointer::Drag => s.mouse_move(
            &MouseMoveEvent {
                position,
                modifiers: Modifiers::default(),
                pressed_button: matches!(kind, Pointer::Drag).then_some(MouseButton::Left),
            },
            w,
            cx,
        ),
        Pointer::Down => s.mouse_down(
            &MouseDownEvent {
                button: MouseButton::Left,
                position,
                click_count: 1,
                ..Default::default()
            },
            w,
            cx,
        ),
        Pointer::Up => {
            let event = MouseUpEvent {
                button: MouseButton::Left,
                position,
                click_count: 1,
                ..Default::default()
            };
            s.draw_mouse_up(&event, cx);
            s.cad_tool_mouse_up(&event, cx);
            s.navigation = None;
        }
    })?;
    Timer::after(Duration::from_millis(16)).await;
    Ok(())
}

async fn click(window: WindowHandle<Studio>, at: Vec2, cx: &mut AsyncApp) -> Result<()> {
    pointer(window, Pointer::Move, at, cx).await?;
    pointer(window, Pointer::Down, at, cx).await?;
    pointer(window, Pointer::Up, at, cx).await
}

/// Window position of a world point, which must be inside the viewport.
fn screen(s: &Studio, world: Vec3) -> Result<Vec2> {
    let p = s
        .world_to_screen(world)
        .with_context(|| format!("{world} is behind the camera"))?;
    ensure!(
        s.pointer_ray(p).is_some(),
        "{world} projects outside the viewport at {p}"
    );
    Ok(p)
}

/// Window position of a world point on the active sketch plane.
fn sketch_screen(s: &Studio, world: Vec3) -> Result<Vec2> {
    let frame = s.sketch_frame().context("Draw mode is not active")?;
    let local = frame.plane.to_local(frame.inverse.transform_point3(world));
    s.plane_to_screen(&frame, local)
        .context("sketch point behind the camera")
}

async fn sketch_click(window: WindowHandle<Studio>, world: Vec3, cx: &mut AsyncApp) -> Result<()> {
    let at = window.update(cx, |s, _, _| sketch_screen(s, world))??;
    click(window, at, cx).await
}

fn volume(s: &Studio, object: u64) -> f64 {
    s.scene.object_mesh(object).map_or(0.0, mesh_volume)
}

fn feature_kinds(s: &Studio, object: u64) -> Vec<&'static str> {
    s.scene
        .object_part(object)
        .map(|part| part.features.iter().map(|f| f.kind.label()).collect())
        .unwrap_or_default()
}

async fn type_text(window: WindowHandle<Studio>, text: &str, cx: &mut AsyncApp) -> Result<()> {
    let characters: Vec<String> = text.chars().map(|c| c.to_string()).collect();
    let sequence: Vec<&str> = characters.iter().map(String::as_str).collect();
    keys(window, &sequence, cx).await
}

async fn settle_camera(window: WindowHandle<Studio>, cx: &mut AsyncApp) -> Result<()> {
    window.update(cx, |s, _, cx| s.complete_camera_animation(cx))?;
    Timer::after(Duration::from_millis(40)).await;
    Ok(())
}

/// Area of the outermost profile of a sketch (outer loop minus holes).
fn profile_area(s: &Studio, sketch: u64, largest: bool) -> f64 {
    let profiles = s
        .scene
        .object_sketch(sketch)
        .map(|k| k.profiles(forma_core::SKETCH_TOLERANCE))
        .unwrap_or_default();
    let areas = profiles.iter().map(|p| p.area() as f64);
    if largest {
        areas.fold(0.0, f64::max)
    } else {
        areas.fold(f64::MAX, f64::min)
    }
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

pub(crate) async fn check_cad_tools_workflow(
    window: WindowHandle<Studio>,
    cx: &mut AsyncApp,
) -> Result<()> {
    let started = Instant::now();
    let saved = window.update(cx, |s, w, cx| {
        let saved = (
            s.scene.clone(),
            std::mem::take(&mut s.history),
            s.selected,
            s.objects_selected.clone(),
            s.settings.clone(),
            s.dirty,
        );
        s.execute(Command::SetEditMode(EditMode::Object), w, cx);
        s.scene = Scene::empty();
        s.scene.camera.target = Vec3::new(-0.6, 0.3, 0.0);
        s.scene.camera.yaw = 0.55;
        s.scene.camera.pitch = 0.55;
        s.scene.camera.distance = 7.5;
        s.scene.camera.orthographic = false;
        s.select_objects(Default::default(), None);
        s.settings.mode = RenderMode::Solid;
        s.invalidate(true, cx);
        saved
    })?;
    settle(window, cx).await?;
    let idle = |s: &Studio| s.cad_idle();

    // 1. Draw a 2 × 1.2 m rectangle with a 0.3 m circle on the ground.
    let empty = window.update(cx, |s, _, _| {
        let bounds = s.bounds.get();
        Vec2::new(
            f32::from(bounds.origin.x) + 30.0,
            f32::from(bounds.origin.y) + 30.0,
        )
    })?;
    pointer(window, Pointer::Move, empty, cx).await?;
    keys(window, &["d"], cx).await?;
    settle_camera(window, cx).await?;
    keys(window, &["r"], cx).await?;
    sketch_click(window, Vec3::new(-1.0, 0.0, 0.6), cx).await?;
    sketch_click(window, Vec3::new(1.0, 0.0, -0.6), cx).await?;
    keys(window, &["c"], cx).await?;
    sketch_click(window, Vec3::new(-0.4, 0.0, 0.0), cx).await?;
    sketch_click(window, Vec3::new(-0.1, 0.0, 0.0), cx).await?;
    keys(window, &["enter"], cx).await?;
    settle_camera(window, cx).await?;
    let sketch = window.update(cx, |s, _, _| -> Result<u64> {
        ensure!(!s.is_drawing(), "Return did not finish the sketch");
        let id = s.selected.context("the finished sketch is not selected")?;
        let sketch = s.scene.object_sketch(id).context("no sketch object")?;
        ensure!(
            sketch.entities.len() == 5 && sketch.profiles(forma_core::SKETCH_TOLERANCE).len() == 2,
            "the sketch has {} curves",
            sketch.entities.len()
        );
        Ok(id)
    })??;

    // 2. Hover the ring, E, type a distance, Return: a new part.
    let ring = window.update(cx, |s, _, _| screen(s, Vec3::new(0.55, 0.0, 0.0)))??;
    pointer(window, Pointer::Move, ring, cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.cad_tool.region_hover.is_some(),
            "hovering the ring did not highlight a region"
        );
        Ok(())
    })??;
    keys(window, &["e"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let Some(Tool::Sketch(tool)) = s.cad_tool.tool.as_ref() else {
            anyhow::bail!("E over a region did not start Extrude");
        };
        ensure!(
            tool.selected.len() == 1 && tool.operation() == Operation::NewPart,
            "Extrude picked {:?} as {:?}",
            tool.selected,
            tool.operation()
        );
        Ok(())
    })??;
    type_text(window, "0.5", cx).await?;
    keys(window, &["enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    let ring_area = window.update(cx, |s, _, _| profile_area(s, sketch, true))?;
    let part = window.update(cx, |s, _, _| -> Result<u64> {
        ensure!(!s.cad_tool_active(), "Return did not commit Extrude");
        let id = s.selected.context("the new part is not selected")?;
        let part = s
            .scene
            .object_part(id)
            .context("Extrude did not make a part")?;
        let FeatureKind::Extrude(extrude) = &part.features[0].kind else {
            anyhow::bail!(
                "the part's first feature is {}",
                part.features[0].kind.label()
            );
        };
        ensure!(
            part.features.len() == 1
                && part.features[0].source == Some(sketch)
                && extrude.profiles.len() == 1
                && (extrude.distance - 0.5).abs() < 1.0e-5
                && extrude.operation == FeatureOperation::Join,
            "unexpected extrusion {extrude:?}"
        );
        ensure!(
            !s.scene.object(sketch).unwrap().visible,
            "the consumed sketch is still visible"
        );
        ensure!(part.issues().is_empty(), "{:?}", part.issues());
        let v = volume(s, id);
        ensure!(
            close(v, ring_area * 0.5, ring_area * 0.5 * 0.01),
            "ring volume {v} (expected {})",
            ring_area * 0.5
        );
        ensure!(
            s.adjustable_operation().is_some(),
            "the adjust panel did not appear"
        );
        Ok(id)
    })??;

    // 3. Push/Pull the top face up by a typed 0.1 m during the drag.
    keys(window, &["p"], cx).await?;
    let top = window.update(cx, |s, _, _| screen(s, Vec3::new(0.55, 0.5, 0.2)))??;
    pointer(window, Pointer::Move, top, cx).await?;
    pointer(window, Pointer::Down, top, cx).await?;
    pointer(window, Pointer::Drag, top + Vec2::new(0.0, -30.0), cx).await?;
    type_text(window, "0.1", cx).await?;
    pointer(window, Pointer::Up, top + Vec2::new(0.0, -30.0), cx).await?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            feature_kinds(s, part) == ["Extrude", "Push/pull"],
            "after push/pull: {:?}",
            feature_kinds(s, part)
        );
        let v = volume(s, part);
        ensure!(
            close(v, ring_area * 0.6, ring_area * 0.6 * 0.01),
            "pulled volume {v}"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;

    // 4. Sketch on the top face: the plane is the whole face group's, its
    // grid follows the 2 m edge. Draw a 0.4 × 0.6 m rectangle in the ring.
    let on_top = window.update(cx, |s, _, _| screen(s, Vec3::new(0.65, 0.6, 0.35)))??;
    pointer(window, Pointer::Move, on_top, cx).await?;
    keys(window, &["d"], cx).await?;
    settle_camera(window, cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let frame = s
            .sketch_frame()
            .context("D did not start Draw on the face")?;
        let session = s.sketch.session.as_ref().unwrap();
        ensure!(
            frame.plane.normal().distance(Vec3::Y) < 1.0e-4
                && (frame.plane.origin.y - 0.6).abs() < 1.0e-4,
            "face plane {:?}",
            frame.plane
        );
        ensure!(
            frame.plane.x_axis.x.abs() > 0.999,
            "the grid does not follow the longest edge: {:?}",
            frame.plane.x_axis
        );
        ensure!(
            session.reference.len() > 4,
            "the face outline has only {} edges",
            session.reference.len()
        );
        Ok(())
    })??;
    keys(window, &["r"], cx).await?;
    sketch_click(window, Vec3::new(0.35, 0.6, 0.3), cx).await?;
    sketch_click(window, Vec3::new(0.75, 0.6, -0.3), cx).await?;
    keys(window, &["enter"], cx).await?;
    settle_camera(window, cx).await?;
    let pocket_area = window.update(cx, |s, _, _| -> Result<f64> {
        let id = s.selected.context("the face sketch is not selected")?;
        Ok(profile_area(s, id, true))
    })??;

    // 5. E over the new rectangle, type −0.8: inferred Cut through the part.
    let pocket = window.update(cx, |s, _, _| screen(s, Vec3::new(0.55, 0.6, 0.0)))??;
    pointer(window, Pointer::Move, pocket, cx).await?;
    keys(window, &["e"], cx).await?;
    type_text(window, "-0.8", cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let Some(Tool::Sketch(tool)) = s.cad_tool.tool.as_ref() else {
            anyhow::bail!("E did not start Extrude on the face sketch");
        };
        ensure!(
            tool.target
                .as_ref()
                .is_some_and(|t| t.object == part && t.outward.is_some()),
            "the face sketch did not target the part"
        );
        ensure!(
            tool.inferred() && tool.operation() == Operation::Cut,
            "pushing in inferred {:?}",
            tool.operation()
        );
        Ok(())
    })??;
    // Orbit while the kernel preview lands: the view must stay put.
    let orbited = window.update(cx, |s, _, cx| {
        s.scene.camera.yaw += 0.05;
        s.invalidate(false, cx);
        s.scene.camera
    })?;
    wait_for(window, cx, |s| {
        matches!(&s.cad_tool.tool, Some(Tool::Sketch(tool)) if tool.preview.as_ref().is_some_and(|p| p.outcome.is_some()))
    })
    .await
    .context("the kernel preview did not apply")?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.scene.camera == orbited,
            "applying the kernel preview moved the camera"
        );
        Ok(())
    })??;
    keys(window, &["enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    let cut_volume = ring_area * 0.6 - pocket_area * 0.6;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(s.scene.camera == orbited, "committing moved the camera");
        ensure!(
            feature_kinds(s, part) == ["Extrude", "Push/pull", "Extrude"],
            "after the cut: {:?}",
            feature_kinds(s, part)
        );
        let part_data = s.scene.object_part(part).unwrap();
        ensure!(
            matches!(&part_data.features[2].kind, FeatureKind::Extrude(e) if e.operation == FeatureOperation::Cut),
            "the third feature is not a cut"
        );
        ensure!(part_data.issues().is_empty(), "{:?}", part_data.issues());
        let v = volume(s, part);
        ensure!(close(v, cut_volume, cut_volume * 0.01), "cut volume {v} (expected {cut_volume})");
        Ok(())
    })??;

    // 6. Push/Pull a mesh: a box's top slides up 0.5 m, no faces added.
    let cube = window.update(cx, |s, w, cx| {
        s.execute(Command::Add(Primitive::Cube), w, cx);
        let id = s.selected.unwrap();
        let object = s.scene.object_mut(id).unwrap();
        object.transform.translation = Vec3::new(-2.6, 0.5, 0.0);
        object.transform.scale = Vec3::splat(0.5);
        s.invalidate(true, cx);
        id
    })?;
    settle(window, cx).await?;
    keys(window, &["p"], cx).await?;
    let cube_top = window.update(cx, |s, _, _| screen(s, Vec3::new(-2.6, 1.0, 0.0)))??;
    pointer(window, Pointer::Move, cube_top, cx).await?;
    pointer(window, Pointer::Down, cube_top, cx).await?;
    pointer(window, Pointer::Drag, cube_top + Vec2::new(0.0, -25.0), cx).await?;
    type_text(window, "0.5", cx).await?;
    pointer(window, Pointer::Up, cube_top + Vec2::new(0.0, -25.0), cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let mesh = s.scene.object_mesh(cube).unwrap();
        let world = s.scene.world_transform(cube).unwrap();
        let height = mesh
            .positions
            .iter()
            .map(|p| world.transform_point3(*p).y)
            .fold(f32::MIN, f32::max);
        ensure!(
            mesh.faces.len() == 6 && (height - 1.5).abs() < 1.0e-4,
            "mesh push/pull gave {} faces, top {height}",
            mesh.faces.len()
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;

    // 7. Part edit: Tab, edge mode, click a vertical corner edge, Ctrl+B,
    // type 0.05, Return.
    window.update(cx, |s, w, cx| {
        s.select_objects([part].into_iter().collect(), Some(part));
        s.execute(Command::Select(part), w, cx);
    })?;
    keys(window, &["tab", "2"], cx).await?;
    let corner = window.update(cx, |s, _, _| screen(s, Vec3::new(1.0, 0.3, 0.6)))??;
    click(window, corner, cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.part_editing() == Some(part) && s.edit_mode == EditMode::Edge,
            "Tab and 2 did not enter part edge editing"
        );
        ensure!(
            s.selected_edit_edges(part).len() == 1,
            "clicking the corner selected {} edges",
            s.selected_edit_edges(part).len()
        );
        Ok(())
    })??;
    keys(window, &["delete"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            feature_kinds(s, part).len() == 3 && s.status.contains("can't be deleted"),
            "Delete changed a B-rep part: {}",
            s.status
        );
        Ok(())
    })??;
    let before_chamfer = window.update(cx, |s, _, _| volume(s, part))?;
    keys(window, &[crate::platform_shortcut("cmd-b", "ctrl-b")], cx).await?;
    type_text(window, "0.05", cx).await?;
    keys(window, &["enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            feature_kinds(s, part) == ["Extrude", "Push/pull", "Extrude", "Chamfer"],
            "after chamfer: {:?}",
            feature_kinds(s, part)
        );
        let removed = before_chamfer - volume(s, part);
        ensure!(
            close(removed, 0.5 * 0.05 * 0.05 * 0.6, 2.0e-4),
            "the chamfer removed {removed} m³"
        );
        Ok(())
    })??;

    // 8. Adjust the chamfer to 0.1 m in the panel: same undo step.
    window.update(cx, |s, w, cx| {
        s.begin_field(Field::CadAdjust(AdjustParam::Distance), w, cx)
    })?;
    type_text(window, "0.1", cx).await?;
    keys(window, &["enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let part_data = s.scene.object_part(part).unwrap();
        let FeatureKind::Chamfer(chamfer) = &part_data.features[3].kind else {
            anyhow::bail!("the chamfer moved");
        };
        ensure!(
            (chamfer.distance - 0.1).abs() < 1.0e-5,
            "the panel set {}",
            chamfer.distance
        );
        let removed = before_chamfer - volume(s, part);
        ensure!(
            close(removed, 0.5 * 0.1 * 0.1 * 0.6, 4.0e-4),
            "the adjusted chamfer removed {removed} m³"
        );
        ensure!(
            s.adjustable_operation().is_some(),
            "the panel went away after adjusting"
        );
        Ok(())
    })??;

    // 9. One undo removes the chamfer (not just the adjustment); redo
    // brings back the adjusted value.
    window.update(cx, |s, w, cx| -> Result<()> {
        s.execute(Command::Undo, w, cx);
        ensure!(
            feature_kinds(s, part).len() == 3,
            "undo left {:?}",
            feature_kinds(s, part)
        );
        ensure!(
            s.adjustable_operation().is_none(),
            "the adjust panel outlived undo"
        );
        s.execute(Command::Redo, w, cx);
        let part_data = s.scene.object_part(part).unwrap();
        ensure!(
            matches!(&part_data.features.get(3).map(|f| &f.kind), Some(FeatureKind::Chamfer(c)) if (c.distance - 0.1).abs() < 1.0e-5),
            "redo did not restore the adjusted chamfer"
        );
        Ok(())
    })??;
    wait_for(window, cx, idle).await?;
    // Undo all the way back to the drawn sketch: the part disappears and
    // the sketch shows again.
    window.update(cx, |s, w, cx| -> Result<()> {
        s.execute(Command::SetEditMode(EditMode::Object), w, cx);
        // Chamfer, mesh push, add cube, cut, face sketch, push/pull, extrude.
        for _ in 0..7 {
            s.execute(Command::Undo, w, cx);
        }
        ensure!(
            s.scene.object(part).is_none() && s.scene.object(sketch).is_some_and(|o| o.visible),
            "undoing the extrusion did not restore the sketch"
        );
        Ok(())
    })??;

    window.update(cx, |s, _, cx| {
        let (scene, history, selected, objects, settings, dirty) = saved;
        s.scene = scene;
        s.history = history;
        s.select_objects(objects, selected);
        s.settings = settings;
        s.dirty = dirty;
        s.cad_tool.last = None;
        s.invalidate(true, cx);
    })?;
    wait_for(window, cx, idle).await?;
    settle(window, cx).await?;
    println!(
        "cad_tools_workflow=draw_ring,extrude_new_part,push_pull_part,sketch_on_face,cut_inference,push_pull_mesh,part_edit_delete_guard,chamfer,adjust_panel,undo_redo PASS ({:.1} s)",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

/// Empties the document for one regression case, with a fixed camera.
async fn fresh(window: WindowHandle<Studio>, cx: &mut AsyncApp) -> Result<()> {
    window.update(cx, |s, w, cx| {
        s.execute(Command::SetEditMode(EditMode::Object), w, cx);
        s.reset_cad_tools();
        s.scene = Scene::empty();
        s.history = Default::default();
        s.scene.camera.target = Vec3::ZERO;
        s.scene.camera.yaw = 0.6;
        s.scene.camera.pitch = 0.5;
        s.scene.camera.distance = 7.0;
        s.scene.camera.orthographic = false;
        s.select_objects(Default::default(), None);
        s.invalidate(true, cx);
    })?;
    settle(window, cx).await
}

/// A 2 × 1.2 m rectangle on `plane`, added and selected.
fn add_rectangle_sketch(s: &mut Studio, plane: forma_core::SketchPlane) -> Result<u64> {
    let mut sketch = forma_core::Sketch::new(plane);
    let (min, max) = (Vec2::new(-1.0, -0.6), Vec2::new(1.0, 0.6));
    rectangle_into(&mut sketch, min, max);
    let id = s.scene.add_sketch("Sketch", sketch)?;
    s.select_objects([id].into_iter().collect(), Some(id));
    Ok(id)
}

fn rectangle_into(sketch: &mut forma_core::Sketch, min: Vec2, max: Vec2) {
    let c = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
    sketch.entities = (0..4)
        .map(|i| -> forma_core::SketchEntity {
            forma_core::SketchCurve::Line {
                start: c[i],
                end: c[(i + 1) % 4],
            }
            .into()
        })
        .collect();
}

/// Adds a box part through the CAD command and waits for its tessellation.
async fn add_box_part(window: WindowHandle<Studio>, cx: &mut AsyncApp) -> Result<u64> {
    window.update(cx, |s, w, cx| {
        s.execute(
            Command::Cad(crate::cad::CadCommand::AddPart(crate::cad::PartShape::Box)),
            w,
            cx,
        )
    })?;
    wait_for(window, cx, |s| s.cad_idle()).await?;
    window.update(cx, |s, _, _| -> Result<u64> {
        let id = s.selected.context("no part selected")?;
        ensure!(s.scene.object_mesh(id).is_some(), "the box has no mesh");
        Ok(id)
    })?
}

/// Part-space bounds of an object's mesh.
fn mesh_bounds(s: &Studio, object: u64) -> Result<(Vec3, Vec3)> {
    let mesh = s.scene.object_mesh(object).context("no mesh")?;
    Ok(mesh.positions.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    ))
}

fn part_names(s: &Studio) -> Vec<String> {
    s.scene
        .objects
        .iter()
        .filter(|o| o.is_part())
        .map(|o| o.name.clone())
        .collect()
}

fn last_feature(s: &Studio) -> Option<&forma_core::Feature> {
    let last = s.cad_tool.last.as_ref()?;
    s.scene.part(last.part?)?.feature(last.feature)
}

/// Regressions from the tools review, each through the real window, keys
/// and CAD worker.
pub(crate) async fn check_cad_tools_regressions(
    window: WindowHandle<Studio>,
    output: &std::path::Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    let started = Instant::now();
    let saved = window.update(cx, |s, _, _| {
        (
            s.scene.clone(),
            std::mem::take(&mut s.history),
            s.selected,
            s.objects_selected.clone(),
            s.settings.clone(),
            s.dirty,
            s.path.clone(),
            s.project_name.clone(),
        )
    })?;
    let idle = |s: &Studio| s.cad_idle();
    let ctrl = |key: &'static str| -> &'static str {
        match key {
            "z" => crate::platform_shortcut("cmd-z", "ctrl-z"),
            _ => crate::platform_shortcut("cmd-b", "ctrl-b"),
        }
    };

    // Kernel previews stay out of the document: no part appears while one
    // shows, an inspector edit during the tool survives Esc, and Undo
    // never brings back the cancelled extrusion. Sketch on face ends it.
    fresh(window, cx).await?;
    let sketch = window.update(cx, |s, w, cx| -> Result<u64> {
        let id = add_rectangle_sketch(s, forma_core::SketchPlane::ground())?;
        s.execute(Command::CadTool(ToolCommand::Extrude), w, cx);
        Ok(id)
    })??;
    type_text(window, "0.5", cx).await?;
    wait_for(window, cx, |s| {
        matches!(&s.cad_tool.tool, Some(Tool::Sketch(tool)) if tool.preview.as_ref().is_some_and(|p| p.outcome.is_some()))
    })
    .await
    .context("the kernel preview did not land")?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.scene.parts.is_empty() && part_names(s).is_empty(),
            "the preview put a part in the document: {:?}",
            part_names(s)
        );
        s.begin_field(Field::Name, w, cx);
        s.active_field = Some((Field::Name, "Renamed sketch".into()));
        s.commit_field(cx);
        Ok(())
    })??;
    keys(window, &["escape", "escape"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(!s.cad_tool_active(), "Esc did not cancel Extrude");
        ensure!(
            part_names(s).is_empty()
                && s.scene.object(sketch).map(|o| o.name.as_str()) == Some("Renamed sketch"),
            "after Esc: parts {:?}, sketch {:?}",
            part_names(s),
            s.scene.object(sketch).map(|o| o.name.clone())
        );
        Ok(())
    })??;
    keys(window, &[ctrl("z")], cx).await?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            part_names(s).is_empty()
                && s.scene.object(sketch).map(|o| o.name.as_str()) == Some("Sketch"),
            "Undo after Esc: parts {:?}, sketch {:?}",
            part_names(s),
            s.scene.object(sketch).map(|o| o.name.clone())
        );
        s.select_objects([sketch].into_iter().collect(), Some(sketch));
        s.execute(Command::CadTool(ToolCommand::Extrude), w, cx);
        ensure!(s.cad_tool_active(), "Extrude did not start again");
        s.execute(Command::CadTool(ToolCommand::SketchOnFace), w, cx);
        ensure!(
            !s.cad_tool_active() && s.is_drawing(),
            "Sketch on face did not end Extrude"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    window.update(cx, |s, _, cx| {
        if s.is_drawing() {
            s.finish_draw(cx);
        }
    })?;
    settle_camera(window, cx).await?;

    // Unit letters belong to the typed distance: "2in" in Push/Pull keeps
    // the tool and applies 50.8 mm.
    fresh(window, cx).await?;
    let block = add_box_part(window, cx).await?;
    let top = window.update(cx, |s, _, _| -> Result<Vec2> {
        let (lo, hi) = mesh_bounds(s, block)?;
        let world = s.scene.world_transform(block).context("no transform")?;
        screen(
            s,
            world.transform_point3(Vec3::new(
                lo.x * 0.6 + hi.x * 0.4,
                hi.y,
                lo.z * 0.6 + hi.z * 0.4,
            )),
        )
    })??;
    keys(window, &["p"], cx).await?;
    click(window, top, cx).await?;
    type_text(window, "2in", cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let Some(Tool::PushPull(tool)) = s.cad_tool.tool.as_ref() else {
            anyhow::bail!("typing 2in ended Push/Pull: {}", s.status);
        };
        let numeric = tool.active.as_ref().map(|a| a.numeric.as_str());
        ensure!(numeric == Some("2in"), "typed {numeric:?}");
        Ok(())
    })??;
    keys(window, &["enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let part = s.scene.object_part(block).context("no part")?;
        let distance = part.features.iter().find_map(|f| match &f.kind {
            FeatureKind::PushPull(p) => Some(p.distance),
            _ => None,
        });
        ensure!(
            distance.is_some_and(|d| (d - 0.0508).abs() < 1.0e-5),
            "2in pushed {distance:?}"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;

    // The adjust panel: Tab stays on its one value (typing replaces it in
    // the panel's unit); an edit outside the tools hides the panel.
    fresh(window, cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        add_rectangle_sketch(s, forma_core::SketchPlane::ground())?;
        s.execute(Command::CadTool(ToolCommand::Extrude), w, cx);
        Ok(())
    })??;
    type_text(window, "0.5", cx).await?;
    keys(window, &["enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(s.adjustable_operation().is_some(), "no adjust panel");
        s.begin_field(Field::CadAdjust(AdjustParam::Distance), w, cx);
        Ok(())
    })??;
    keys(window, &["tab"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let field = s.active_field.as_ref().map(|(f, _)| *f);
        ensure!(
            field == Some(Field::CadAdjust(AdjustParam::Distance)),
            "Tab moved to {field:?}"
        );
        Ok(())
    })??;
    type_text(window, "5", cx).await?;
    keys(window, &["enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    let extruded = window.update(cx, |s, w, cx| -> Result<u64> {
        let distance = match last_feature(s).map(|f| &f.kind) {
            Some(FeatureKind::Extrude(e)) => e.distance,
            other => anyhow::bail!("the last feature is {other:?}"),
        };
        ensure!((distance - 5.0).abs() < 1.0e-5, "the panel set {distance}");
        let id = s.selected.context("the part is not selected")?;
        // Rename the part in the inspector: the panel no longer describes
        // the latest change.
        s.begin_field(Field::Name, w, cx);
        s.active_field = Some((Field::Name, "Renamed part".into()));
        s.commit_field(cx);
        ensure!(
            s.adjustable_operation().is_none(),
            "the adjust panel outlived an inspector edit"
        );
        Ok(id)
    })??;
    let _ = extruded;

    // Adjust Flip re-infers an inferred Cut as a Join.
    fresh(window, cx).await?;
    let block = add_box_part(window, cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        let (lo, hi) = mesh_bounds(s, block)?;
        let world = s.scene.world_transform(block).context("no transform")?;
        let centre =
            world.transform_point3(Vec3::new((lo.x + hi.x) * 0.5, hi.y, (lo.z + hi.z) * 0.5));
        let mut sketch = forma_core::Sketch::new(forma_core::SketchPlane {
            origin: centre,
            ..forma_core::SketchPlane::ground()
        });
        let half = ((hi - lo) * 0.2).min(Vec3::splat(0.2));
        rectangle_into(
            &mut sketch,
            -Vec2::new(half.x, half.z),
            Vec2::new(half.x, half.z),
        );
        let id = s.scene.add_sketch("Face sketch", sketch)?;
        s.select_objects([id].into_iter().collect(), Some(id));
        s.invalidate(true, cx);
        s.execute(Command::CadTool(ToolCommand::Extrude), w, cx);
        Ok(())
    })??;
    type_text(window, "-0.2", cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let Some(Tool::Sketch(tool)) = s.cad_tool.tool.as_ref() else {
            anyhow::bail!("Extrude did not start on the face sketch");
        };
        ensure!(
            tool.inferred() && tool.operation() == Operation::Cut,
            "pushing in inferred {:?}",
            tool.operation()
        );
        Ok(())
    })??;
    keys(window, &["enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, w, cx| {
        s.execute(Command::CadTool(ToolCommand::AdjustFlip), w, cx)
    })?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let Some(FeatureKind::Extrude(extrude)) = last_feature(s).map(|f| &f.kind) else {
            anyhow::bail!("the flipped feature is gone");
        };
        ensure!(
            extrude.operation == FeatureOperation::Join && (extrude.distance - 0.2).abs() < 1.0e-5,
            "Flip gave {:?} {}",
            extrude.operation,
            extrude.distance
        );
        let part = s.scene.object_part(block).context("no part")?;
        ensure!(part.issues().is_empty(), "{:?}", part.issues());
        Ok(())
    })??;

    // In part edit, Esc first deselects the timeline's feature.
    window.update(cx, |s, w, cx| {
        s.select_objects([block].into_iter().collect(), Some(block));
        s.execute(Command::Select(block), w, cx);
    })?;
    keys(window, &["tab"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(s.part_editing() == Some(block), "Tab did not edit the part");
        let feature = s.scene.object_part(block).context("no part")?.features[0].id;
        s.panels.timeline.select(crate::timeline::Selection {
            object: block,
            feature,
        });
        ensure!(s.selected_feature().is_some(), "no timeline selection");
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.selected_feature().is_none() && s.part_editing() == Some(block),
            "Esc: feature selected {}, editing {:?}",
            s.selected_feature().is_some(),
            s.part_editing()
        );
        Ok(())
    })??;

    // Pushing two selected faces at once: both ghosts wait for the part's
    // final content.
    // Two side faces (the flipped boss sits on top).
    let (right, back) = window.update(cx, |s, _, _| -> Result<_> {
        let (lo, hi) = mesh_bounds(s, block)?;
        let centre = (lo + hi) * 0.5;
        let low = lo.y * 0.9 + hi.y * 0.1;
        Ok((
            forma_core::FaceRef {
                point: Vec3::new(hi.x, low, centre.z),
                normal: Vec3::X,
            },
            forma_core::FaceRef {
                point: Vec3::new(centre.x, low, lo.z),
                normal: Vec3::NEG_Z,
            },
        ))
    })??;
    window.update(cx, |s, _, _| {
        s.cad_tool.edit.faces = vec![right, back];
    })?;
    keys(window, &["p"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let Some(Tool::PushPull(tool)) = s.cad_tool.tool.as_ref() else {
            anyhow::bail!("P did not push the selected faces: {}", s.status);
        };
        let count = tool.active.as_ref().map_or(0, |a| a.faces.len());
        ensure!(count == 2, "pushing {count} faces");
        Ok(())
    })??;
    type_text(window, "0.1", cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        s.execute(Command::CadTool(ToolCommand::Done), w, cx);
        let part = s.scene.object_part(block).context("no part")?;
        let (id, stamp) = (part.id, part.stamp);
        let ghosts: Vec<_> = s.cad_tool.pending.iter().filter(|g| g.part == id).collect();
        ensure!(
            ghosts.len() == 2 && ghosts.iter().all(|g| g.stamp == stamp),
            "{} ghosts, stamps {:?} vs final {stamp}",
            ghosts.len(),
            ghosts.iter().map(|g| g.stamp).collect::<Vec<_>>()
        );
        Ok(())
    })??;
    wait_for(window, cx, idle).await?;

    // Ctrl+B after Undo: a hovered edge index from the undone tessellation
    // is dropped instead of indexing the restored one.
    fresh(window, cx).await?;
    let block = add_box_part(window, cx).await?;
    keys(window, &["tab", "2"], cx).await?;
    let visible_edge = |s: &Studio, skip: usize| -> Result<Vec2> {
        let topology = s.part_topology(block).context("no topology")?;
        let instance = s.scene.mesh_instance(block).context("no instance")?;
        for edge in topology.edges.iter().skip(skip) {
            let a = instance.mesh.positions[edge.points[0] as usize];
            let b = instance.mesh.positions[*edge.points.last().unwrap() as usize];
            let mid = instance.world_transform.transform_point3((a + b) * 0.5);
            if let Some(p) = s.world_to_screen(mid)
                && s.pointer_ray(p).is_some()
                && crate::viewport::visible_in_query(s.scene.camera, s.visibility_query(), mid)
            {
                return Ok(p);
            }
        }
        anyhow::bail!("no visible edge past {skip}")
    };
    let (base, first) = window.update(cx, |s, _, _| -> Result<(usize, Vec2)> {
        ensure!(s.part_editing() == Some(block), "not editing the box");
        let base = s.part_topology(block).context("no topology")?.edges.len();
        Ok((base, visible_edge(s, 0)?))
    })??;
    pointer(window, Pointer::Move, first, cx).await?;
    keys(window, &[ctrl("b")], cx).await?;
    type_text(window, "0.1", cx).await?;
    keys(window, &["enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    let high = window.update(cx, |s, _, _| visible_edge(s, base))??;
    pointer(window, Pointer::Move, high + Vec2::new(0.5, 0.0), cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let hover = s.cad_tool.edit.hover(s.scene_revision);
        ensure!(
            matches!(hover, Some(super::edit::EditHover::Edge(i)) if i >= base),
            "could not hover a chamfer edge: {hover:?}"
        );
        Ok(())
    })??;
    keys(window, &[ctrl("z")], cx).await?;
    wait_for(window, cx, idle).await?;
    keys(window, &[ctrl("b")], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.cad_tool.edit.hover(s.scene_revision).is_none()
                && !matches!(s.cad_tool.tool, Some(Tool::Chamfer(_))),
            "Ctrl+B after Undo used a stale edge: {}",
            s.status
        );
        Ok(())
    })??;
    window.update(cx, |s, w, cx| {
        s.execute(Command::SetEditMode(EditMode::Object), w, cx)
    })?;

    // An Open that completes during a tool session replaces the document
    // and ends the tool; nothing it previewed reaches the opened project.
    fresh(window, cx).await?;
    let path = output.join("cad-tools-open.forma");
    let mut other = Scene::empty();
    let cube = other.add(Primitive::Cube);
    other.object_mut(cube).unwrap().name = "Opened cube".into();
    other.save(&path)?;
    window.update(cx, |s, w, cx| -> Result<()> {
        add_rectangle_sketch(s, forma_core::SketchPlane::ground())?;
        s.dirty = false;
        s.open_from(path.clone(), false, cx);
        s.execute(Command::CadTool(ToolCommand::Extrude), w, cx);
        ensure!(s.cad_tool_active(), "Extrude did not start");
        Ok(())
    })??;
    wait_for(window, cx, |s| s.status == "Project opened").await?;
    let names =
        |s: &Studio| -> Vec<String> { s.scene.objects.iter().map(|o| o.name.clone()).collect() };
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            !s.cad_tool_active() && s.cad_tool.last.is_none() && names(s) == ["Opened cube"],
            "after Open: tool {}, objects {:?}",
            s.cad_tool_active(),
            names(s)
        );
        Ok(())
    })??;
    type_text(window, "0.5", cx).await?;
    keys(window, &["enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            names(s) == ["Opened cube"] && !s.dirty,
            "typing after Open changed the project: {:?} dirty {}",
            names(s),
            s.dirty
        );
        Ok(())
    })??;

    // The selected feature's arrow: it lights under the pointer, a drag
    // edits the value live as one undo step, and a right-click mid-drag
    // puts the value back.
    fresh(window, cx).await?;
    let part = add_box_part(window, cx).await?;
    let height = |s: &Studio| -> Option<f32> {
        let part = s.scene.object_part(part)?;
        crate::timeline::params::value(&part.features[0], crate::timeline::FeatureField::Size(1))
    };
    let (start_height, base, tip) =
        window.update(cx, |s, w, cx| -> Result<(f32, Vec2, Vec2)> {
            let feature = s.scene.object_part(part).context("no part")?.features[0].id;
            s.execute(
                Command::Timeline(crate::timeline::TimelineCommand::Select(feature)),
                w,
                cx,
            );
            let overlay = s.cad_tool_overlay().context("no feature overlay")?;
            ensure!(
                overlay.prisms.len() == 1 && overlay.arrows.len() == 1,
                "the selected box shows {} ghosts and {} arrows",
                overlay.prisms.len(),
                overlay.arrows.len()
            );
            let (base, tip) = s.feature_arrow_screen().context("no feature arrow")?;
            Ok((height(s).context("no height")?, base, tip))
        })??;
    let grip = base.lerp(tip, 0.5);
    let along = (tip - base).normalize_or_zero();
    pointer(window, Pointer::Move, grip, cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.cad_tool.feature.hot,
            "the feature arrow did not light under the pointer"
        );
        Ok(())
    })??;
    let undo_before = window.update(cx, |s, _, _| s.history_version())?;
    pointer(window, Pointer::Down, grip, cx).await?;
    for step in 1..=4 {
        pointer(
            window,
            Pointer::Drag,
            grip + along * (12.0 * step as f32),
            cx,
        )
        .await?;
    }
    pointer(window, Pointer::Up, grip + along * 48.0, cx).await?;
    wait_for(window, cx, idle).await?;
    let dragged = window.update(cx, |s, _, _| -> Result<f32> {
        let value = height(s).context("no height")?;
        ensure!(
            value > start_height + 1.0e-3 && s.history_version() != undo_before,
            "dragging the arrow: height {start_height} -> {value}, undo version {} -> {}",
            undo_before,
            s.history_version()
        );
        Ok(value)
    })??;
    keys(window, &[ctrl("z")], cx).await?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let value = height(s).context("no height")?;
        ensure!(
            (value - start_height).abs() < 1.0e-5,
            "one Undo after the drag left the height at {value} (from {dragged}, started {start_height})"
        );
        Ok(())
    })??;
    // Right-click mid-drag cancels.
    let (base, tip) =
        window.update(cx, |s, _, _| s.feature_arrow_screen().context("no arrow"))??;
    let grip = base.lerp(tip, 0.5);
    pointer(window, Pointer::Move, grip, cx).await?;
    pointer(window, Pointer::Down, grip, cx).await?;
    pointer(window, Pointer::Drag, grip + along * 30.0, cx).await?;
    window.update(cx, |s, w, cx| {
        s.mouse_down(
            &MouseDownEvent {
                button: MouseButton::Right,
                position: point(px(grip.x), px(grip.y)),
                click_count: 1,
                ..Default::default()
            },
            w,
            cx,
        );
    })?;
    pointer(window, Pointer::Up, grip, cx).await?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let value = height(s).context("no height")?;
        ensure!(
            (value - start_height).abs() < 1.0e-5 && s.cad_tool.feature.drag.is_none(),
            "right-click mid-drag left the height at {value} (started {start_height})"
        );
        Ok(())
    })??;

    window.update(cx, |s, w, cx| {
        let (scene, history, selected, objects, settings, dirty, path, name) = saved;
        s.execute(Command::SetEditMode(EditMode::Object), w, cx);
        s.reset_cad_tools();
        s.scene = scene;
        s.history = history;
        s.select_objects(objects, selected);
        s.settings = settings;
        s.dirty = dirty;
        s.path = path;
        s.project_name = name;
        s.invalidate(true, cx);
    })?;
    wait_for(window, cx, idle).await?;
    settle(window, cx).await?;
    println!(
        "cad_tools_regressions=preview_isolation,sketch_on_face_ends_tool,push_pull_inches,adjust_tab,adjust_hides_after_edit,adjust_flip_reinfers,timeline_esc_first,multi_face_ghosts,chamfer_stale_hover,open_ends_tool,feature_handle_drag PASS ({:.1} s)",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

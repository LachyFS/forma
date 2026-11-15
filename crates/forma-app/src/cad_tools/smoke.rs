//! Native smoke workflow for the modelling tools, through real keys (the
//! window's keystroke dispatch) and the viewport's pointer handlers, with
//! the real CAD worker: draw a ring, extrude it into a part, push/pull its
//! top, sketch on that face and cut through, push/pull a mesh, chamfer an
//! edge, adjust it in the panel, then undo and redo.
use super::{AdjustParam, Operation, Tool, geometry::mesh_volume};
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
        let part = s.scene.object_part(id).context("Extrude did not make a part")?;
        let FeatureKind::Extrude(extrude) = &part.features[0].kind else {
            anyhow::bail!("the part's first feature is {}", part.features[0].kind.label());
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
        let frame = s.sketch_frame().context("D did not start Draw on the face")?;
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
            tool.target.as_ref().is_some_and(|t| t.object == part && t.outward.is_some()),
            "the face sketch did not target the part"
        );
        ensure!(
            tool.inferred() && tool.operation() == Operation::Cut,
            "pushing in inferred {:?}",
            tool.operation()
        );
        Ok(())
    })??;
    keys(window, &["enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    let cut_volume = ring_area * 0.6 - pocket_area * 0.6;
    window.update(cx, |s, _, _| -> Result<()> {
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

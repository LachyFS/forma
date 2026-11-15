//! Native smoke workflow for the part timeline. Chip clicks, the context
//! menu and the marker drag call the same handlers the rendered dock binds
//! (GPUI 0.2.2 keeps window-level mouse dispatch private); the marker drag
//! resolves positions against the chips' real laid-out bounds, and keys go
//! through the window's keystroke dispatch.
use crate::app::{Command, Field, Studio};
use crate::smoke::{keys, settle, wait_for};
use crate::timeline::{FeatureField, TimelineCommand};
use anyhow::{Context as _, Result, ensure};
use forma_core::{FeatureKind, Scene, Sketch, SketchCurve, SketchPlane, Vec2};
use forma_render::RenderMode;
use gpui::{
    AsyncApp, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    WindowHandle, point, px,
};
use std::time::Instant;

/// What a click on chip `index` does: select it, or edit it on a double-click.
fn click_chip(
    window: WindowHandle<Studio>,
    index: usize,
    double: bool,
    cx: &mut AsyncApp,
) -> Result<()> {
    window.update(cx, |s, w, cx| -> Result<()> {
        chip(s, index).context("the chip is not laid out")?;
        let id = s.timeline_part().context("no timeline")?.1.features[index].id;
        s.execute(Command::Timeline(TimelineCommand::Select(id)), w, cx);
        if double {
            s.execute(Command::Timeline(TimelineCommand::Edit), w, cx);
        }
        Ok(())
    })?
}

/// On-screen centre of child `index` of the chip strip (slots are even,
/// chips odd), from the last layout.
fn strip_item(s: &Studio, index: usize) -> Option<Point<Pixels>> {
    let scroll = &s.panels.timeline.scroll;
    let bounds = scroll.bounds_for_item(index)?;
    Some(bounds.center() + point(scroll.offset().x, px(0.)))
}

fn chip(s: &Studio, feature: usize) -> Option<Point<Pixels>> {
    strip_item(s, 2 * feature + 1)
}

/// Wait until the timeline for `count` features has been laid out.
async fn laid_out(window: WindowHandle<Studio>, count: usize, cx: &mut AsyncApp) -> Result<()> {
    wait_for(window, cx, move |s| {
        s.panels.timeline.scroll.children_count() == 2 * count + 1
    })
    .await
    .context("the timeline was not laid out")
}

fn face_groups(s: &Studio, object: u64) -> usize {
    s.scene
        .object_mesh(object)
        .map(|mesh| {
            mesh.groups
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        })
        .unwrap_or(0)
}

fn top(s: &Studio, object: u64) -> f32 {
    s.scene
        .object_mesh(object)
        .map(|mesh| mesh.positions.iter().map(|p| p.y).fold(f32::MIN, f32::max))
        .unwrap_or(f32::MIN)
}

fn extrude_of(s: &Studio, object: u64) -> Option<forma_core::ExtrudeFeature> {
    let part = s.scene.object_part(object)?;
    part.features
        .iter()
        .find_map(|feature| match &feature.kind {
            FeatureKind::Extrude(extrude) => Some(extrude.clone()),
            _ => None,
        })
}

fn line(a: (f32, f32), b: (f32, f32)) -> forma_core::SketchEntity {
    SketchCurve::Line {
        start: Vec2::new(a.0, a.1),
        end: Vec2::new(b.0, b.1),
    }
    .into()
}

pub(crate) async fn check_timeline_workflow(
    window: WindowHandle<Studio>,
    cx: &mut AsyncApp,
) -> Result<()> {
    use crate::cad::{CadCommand, PartShape};
    use forma_core::FeatureOperation;
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
        s.execute(Command::SetEditMode(crate::app::EditMode::Object), w, cx);
        s.scene = Scene::empty();
        s.select_objects(Default::default(), None);
        s.settings.mode = RenderMode::Solid;
        s.invalidate(true, cx);
        saved
    })?;
    let idle = |s: &Studio| s.cad_idle();

    // A unit square sketch, extruded into a part that stays linked to it,
    // then drilled by a thin cylinder (Cut).
    let (sketch, part) = window.update(cx, |s, w, cx| -> Result<(u64, u64)> {
        let mut square = Sketch::new(SketchPlane::ground());
        square.entities = vec![
            line((0.0, 0.0), (1.0, 0.0)),
            line((1.0, 0.0), (1.0, 1.0)),
            line((1.0, 1.0), (0.0, 1.0)),
            line((0.0, 1.0), (0.0, 0.0)),
        ];
        let sketch = s.scene.add_sketch("Profile", square)?;
        s.select_objects([sketch].into_iter().collect(), Some(sketch));
        s.execute(Command::Cad(CadCommand::ExtrudeSketch), w, cx);
        Ok((sketch, s.selected.context("no extrusion selected")?))
    })??;
    wait_for(window, cx, idle).await?;
    let tool = window.update(cx, |s, w, cx| {
        s.execute(
            Command::Cad(CadCommand::AddPart(PartShape::Cylinder)),
            w,
            cx,
        );
        let tool = s.selected.unwrap();
        let transform = &mut s.scene.object_mut(tool).unwrap().transform;
        transform.scale = glam::Vec3::new(0.2, 1.0, 0.2);
        transform.translation = glam::Vec3::new(0.5, 0.5, -0.5);
        s.invalidate(true, cx);
        tool
    })?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, w, cx| {
        s.select_objects([part, tool].into_iter().collect(), Some(part));
        s.execute(
            Command::Cad(CadCommand::Combine(FeatureOperation::Cut)),
            w,
            cx,
        );
        s.select_objects([part].into_iter().collect(), Some(part));
    })?;
    wait_for(window, cx, idle).await?;
    let drilled = window.update(cx, |s, _, _| -> Result<usize> {
        let features = &s.scene.object_part(part).unwrap().features;
        ensure!(
            features.len() == 2 && features[0].source == Some(sketch),
            "the extrusion is not linked to its sketch: {features:?}"
        );
        ensure!(
            s.timeline_object() == Some(part),
            "selecting a part did not show its timeline"
        );
        Ok(face_groups(s, part))
    })??;
    ensure!(drilled == 7, "the drilled extrusion has {drilled} faces");
    laid_out(window, 2, cx).await?;

    // Click the Extrude chip.
    click_chip(window, 0, false, cx)?;
    let extrude_id = window.update(cx, |s, _, _| -> Result<u64> {
        let selected = s
            .selected_feature()
            .context("clicking a chip selected nothing")?;
        ensure!(
            matches!(selected.feature.kind, FeatureKind::Extrude(_)),
            "the click selected {}",
            selected.feature.name
        );
        Ok(selected.feature.id)
    })??;

    // Scrub the distance: live regeneration, one undo step.
    window.update(cx, |s, w, cx| {
        let field = Field::Feature(FeatureField::Distance);
        let origin = point(px(1300.), px(400.));
        s.begin_scrub(
            field,
            &MouseDownEvent {
                button: MouseButton::Left,
                position: origin,
                click_count: 1,
                ..Default::default()
            },
            w,
            cx,
        );
        for step in 1..=6 {
            s.move_sidebar_resize(
                &MouseMoveEvent {
                    position: origin + point(px(step as f32 * 10.), px(0.)),
                    pressed_button: Some(MouseButton::Left),
                    ..Default::default()
                },
                w,
                cx,
            );
        }
        s.end_sidebar_resize(
            &MouseUpEvent {
                button: MouseButton::Left,
                position: origin + point(px(60.), px(0.)),
                click_count: 1,
                ..Default::default()
            },
            w,
            cx,
        );
    })?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        let distance = extrude_of(s, part).unwrap().distance;
        ensure!(
            (distance - 1.3).abs() < 1.0e-4,
            "scrubbing set the distance to {distance}"
        );
        ensure!(
            (top(s, part) - 1.3).abs() < 1.0e-3,
            "the regenerated part is {} tall",
            top(s, part)
        );
        s.execute(Command::Undo, w, cx);
        ensure!(
            extrude_of(s, part).unwrap().distance == 1.0,
            "one undo did not revert the whole scrub"
        );
        s.execute(Command::Redo, w, cx);
        ensure!(
            (extrude_of(s, part).unwrap().distance - 1.3).abs() < 1.0e-4,
            "redo did not restore the scrub"
        );
        Ok(())
    })??;
    wait_for(window, cx, idle).await?;

    // Drag the rollback marker from the end to between the two chips.
    let (marker, gap) = window.update(cx, |s, w, cx| {
        s.execute(
            Command::Timeline(TimelineCommand::Select(extrude_id)),
            w,
            cx,
        );
        (strip_item(s, 4), strip_item(s, 2))
    })?;
    let (marker, gap) = (
        marker.context("no rollback marker")?,
        gap.context("no gap between chips")?,
    );
    window.update(cx, |s, w, cx| {
        s.begin_marker_drag(
            &MouseDownEvent {
                button: MouseButton::Left,
                position: marker,
                click_count: 1,
                ..Default::default()
            },
            w,
            cx,
        );
        for step in 1..=4 {
            s.move_marker_drag(
                &MouseMoveEvent {
                    position: marker + (gap - marker) * (step as f32 / 4.),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                },
                w,
                cx,
            );
        }
        s.end_marker_drag(
            &MouseUpEvent {
                button: MouseButton::Left,
                position: gap,
                click_count: 1,
                ..Default::default()
            },
            w,
            cx,
        );
    })?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        let rollback = s.scene.object_part(part).unwrap().rollback;
        ensure!(
            rollback == Some(1),
            "dragging the marker left rollback at {rollback:?}"
        );
        ensure!(
            face_groups(s, part) == 6,
            "rolling back before the cut still shows the hole"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(
            s.scene.object_part(part).unwrap().rollback.is_none(),
            "one undo did not restore the marker"
        );
        Ok(())
    })??;
    wait_for(window, cx, idle).await?;

    // Right-click the Cut chip, then Suppress from its menu by keyboard.
    laid_out(window, 2, cx).await?;
    let cut_id = window.update(cx, |s, _, cx| -> Result<u64> {
        let target = chip(s, 1).context("no Cut chip")?;
        let id = s.timeline_part().unwrap().1.features[1].id;
        s.open_feature_menu(id, target, cx);
        ensure!(
            s.panels.timeline.menu.is_some(),
            "right-clicking a chip did not open its menu"
        );
        Ok(s.selected_feature()
            .context("no feature selected")?
            .feature
            .id)
    })??;
    keys(window, &["down", "enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let part = s.scene.object_part(part).unwrap();
        ensure!(
            s.panels.timeline.menu.is_none() && part.feature(cut_id).unwrap().suppressed,
            "the menu did not suppress the cut"
        );
        Ok(())
    })??;
    ensure!(
        window.update(cx, |s, _, _| face_groups(s, part))? == 6,
        "a suppressed cut still drilled the part"
    );

    // Delete the selected feature from the keyboard, then undo it.
    keys(window, &["delete"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let features = &s.scene.object_part(part).unwrap().features;
        ensure!(
            features.len() == 1 && s.selected_feature().map(|f| f.feature.id) == Some(extrude_id),
            "Delete did not remove the feature and select its neighbour"
        );
        ensure!(s.scene.object(part).is_some(), "Delete removed the object");
        Ok(())
    })??;
    keys(window, &["cmd-z"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        let restored = s.scene.object_part(part).unwrap();
        ensure!(
            restored.features.len() == 2 && restored.feature(cut_id).unwrap().suppressed,
            "one undo did not restore the deleted feature"
        );
        s.execute(Command::Timeline(TimelineCommand::Select(cut_id)), w, cx);
        s.execute(Command::Timeline(TimelineCommand::ToggleSuppress), w, cx);
        Ok(())
    })??;
    wait_for(window, cx, idle).await?;
    ensure!(
        window.update(cx, |s, _, _| face_groups(s, part))? == 7,
        "unsuppressing did not drill the part again"
    );

    // Double-click the Extrude chip: Draw opens on its linked sketch.
    laid_out(window, 2, cx).await?;
    click_chip(window, 0, true, cx)?;
    let pick = window.update(cx, |s, _, cx| -> Result<Vec2> {
        ensure!(
            s.is_drawing() && s.session_object() == Some(sketch),
            "double-clicking the extrusion did not open its sketch"
        );
        let pick = extrude_of(s, part).unwrap().profiles[0];
        // Draw a small circle over the picked point, clear of the square's
        // edges: it must become a hole, not steal the pick.
        ensure!(
            (0.06..0.94).contains(&pick.x) && (0.04..0.96).contains(&pick.y),
            "unexpected default pick {pick}"
        );
        ensure!(
            s.add_sketch_curves(
                "Circle",
                vec![SketchCurve::Circle {
                    center: pick + Vec2::new(0.0, 0.005),
                    radius: 0.035,
                }],
                cx,
            ),
            "the circle was not added"
        );
        Ok(pick)
    })??;
    window.update(cx, |s, _, _| -> Result<()> {
        let extrude = extrude_of(s, part).unwrap();
        ensure!(
            extrude.sketch.entities.len() == 5,
            "the extrusion did not follow its sketch"
        );
        let profiles = extrude.sketch.profiles(forma_core::SKETCH_TOLERANCE);
        let ring = profiles.iter().find(|p| !p.holes.is_empty());
        let moved = extrude.profiles.first().copied().unwrap_or(pick);
        ensure!(
            extrude.profiles.len() == 1
                && moved.distance(pick + Vec2::new(0.0, 0.005)) > 0.035
                && ring.is_some_and(|ring| ring.contains(moved)),
            "the region pick did not stay on the plate: {:?}",
            extrude.profiles
        );
        Ok(())
    })??;
    keys(window, &["enter"], cx).await?;
    wait_for(window, cx, idle).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            !s.is_drawing()
                && s.selected == Some(part)
                && s.selected_feature().map(|f| f.feature.id) == Some(extrude_id),
            "finishing did not return to the part's feature"
        );
        ensure!(
            face_groups(s, part) == 8,
            "the new hole did not reach the part ({} faces)",
            face_groups(s, part)
        );
        // Undo reverts the sketch and the part together.
        s.execute(Command::Undo, w, cx);
        let curves = s.scene.object_sketch(sketch).unwrap().entities.len();
        let embedded = extrude_of(s, part).unwrap();
        ensure!(
            curves == 4 && embedded.sketch.entities.len() == 4 && embedded.profiles[0] == pick,
            "undo split the sketch from its feature ({curves} vs {})",
            embedded.sketch.entities.len()
        );
        s.execute(Command::Redo, w, cx);
        ensure!(
            extrude_of(s, part).unwrap().sketch.entities.len() == 5,
            "redo did not restore the linked edit"
        );
        Ok(())
    })??;
    wait_for(window, cx, idle).await?;

    // Delete the sketch; Edit sketch rebuilds it, hidden and linked.
    let rebuilt = window.update(cx, |s, w, cx| -> Result<u64> {
        s.select_objects([sketch].into_iter().collect(), Some(sketch));
        s.execute(Command::Delete, w, cx);
        ensure!(
            s.scene.object(sketch).is_none(),
            "the sketch was not deleted"
        );
        s.select_objects([part].into_iter().collect(), Some(part));
        s.execute(
            Command::Timeline(TimelineCommand::Select(extrude_id)),
            w,
            cx,
        );
        s.execute(Command::Timeline(TimelineCommand::EditSketch), w, cx);
        let rebuilt = s
            .session_object()
            .context("Edit sketch did not open Draw")?;
        let source = s
            .scene
            .object_part(part)
            .unwrap()
            .feature(extrude_id)
            .unwrap()
            .source;
        ensure!(
            rebuilt != sketch
                && source == Some(rebuilt)
                && !s.scene.object(rebuilt).unwrap().visible
                && s.scene.object_sketch(rebuilt).unwrap().entities
                    == extrude_of(s, part).unwrap().sketch.entities,
            "the rebuilt sketch is not a hidden, linked copy"
        );
        s.execute(Command::Draw(crate::sketch::DrawCommand::Finish), w, cx);
        ensure!(
            s.selected == Some(part) && !s.scene.object(rebuilt).unwrap().visible,
            "finishing the rebuilt sketch did not return to the part"
        );
        s.execute(Command::Undo, w, cx);
        let source = s
            .scene
            .object_part(part)
            .unwrap()
            .feature(extrude_id)
            .unwrap()
            .source;
        ensure!(
            s.scene.object(rebuilt).is_none() && source == Some(sketch),
            "undo left the rebuilt sketch or its link behind"
        );
        Ok(rebuilt)
    })??;
    let _ = rebuilt;
    wait_for(window, cx, idle).await?;

    window.update(cx, |s, _, cx| {
        let (scene, history, selected, objects, settings, dirty) = saved;
        s.abandon_draw();
        s.scene = scene;
        s.history = history;
        s.select_objects(objects, selected);
        s.settings = settings;
        s.dirty = dirty;
        s.invalidate(true, cx);
    })?;
    settle(window, cx).await?;
    println!(
        "timeline_workflow=select_scrub_rollback_menu_suppress_delete_undo_edit_sketch_relink_pass ({:.1} s)",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

//! Full-window UI captures for design review and documentation.
//!
//! `forma --ui-capture DIR` opens an unfocused window, waits for real GPU frames,
//! then walks a fixed list of workspace states and saves one PNG per state. It is
//! deliberately separate from the long-running smoke regression so screenshots
//! can be regenerated in a few seconds after any UI change.
//!
//! On Linux, captures read the window through X11 (`DISPLAY` without
//! `WAYLAND_DISPLAY`, for example under Xwayland or Xvfb). macOS uses the smoke
//! test's CoreGraphics capture. Other platforms save the viewport frame only.
use crate::app::{Command, EditMode, Field, Studio};
use crate::sketch::demo;
use crate::smoke::{capture_window, prepare_capture};
use crate::theme::Theme;
use crate::ui::{Anchor, InspectorTab, MenuKind, PanelSection};
use anyhow::{Result, ensure};
use forma_render::RenderMode;
use gpui::{App, AsyncApp, Timer, WindowHandle};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// One named state of the workspace. `enter` prepares it; `leave` restores the
/// default workspace so states remain independent of their order.
pub(crate) struct Scenario {
    pub name: &'static str,
    pub enter: fn(&mut Studio, &mut gpui::Window, &mut gpui::Context<Studio>),
    pub leave: fn(&mut Studio, &mut gpui::Window, &mut gpui::Context<Studio>),
}

fn nothing(_: &mut Studio, _: &mut gpui::Window, _: &mut gpui::Context<Studio>) {}

fn theme(s: &mut Studio, theme: Theme, cx: &mut gpui::Context<Studio>) {
    s.apply_theme(theme, cx);
}
fn default_theme(s: &mut Studio, _: &mut gpui::Window, cx: &mut gpui::Context<Studio>) {
    theme(s, Theme::Forma, cx);
}
fn end_field(s: &mut Studio, _: &mut gpui::Window, cx: &mut gpui::Context<Studio>) {
    s.active_field = None;
    s.panels.rename_in_outliner = false;
    cx.notify();
}
pub(crate) fn resize(window: &mut gpui::Window, width: f32, height: f32) {
    window.resize(gpui::size(gpui::px(width), gpui::px(height)));
}
pub(crate) fn restore_size(s: &mut Studio, w: &mut gpui::Window, cx: &mut gpui::Context<Studio>) {
    resize(w, 1512., 960.);
    s.invalidate(false, cx);
}

/// Select the named object: component-editing states work on the torus,
/// because the startup selection is a CAD part.
pub(crate) fn select_named(s: &mut Studio, name: &str) {
    let id = s
        .scene
        .objects
        .iter()
        .find(|object| object.name == name)
        .map(|object| object.id);
    s.select_objects(id.into_iter().collect(), id);
}
/// Restore the startup selection (the scene's hero part).
pub(crate) fn select_hero(s: &mut Studio, cx: &mut gpui::Context<Studio>) {
    let id = crate::startup::hero(&s.scene);
    s.select_objects(id.into_iter().collect(), id);
    s.invalidate(false, cx);
}
/// Edit `mode` on the torus.
fn edit_torus(
    s: &mut Studio,
    w: &mut gpui::Window,
    cx: &mut gpui::Context<Studio>,
    mode: EditMode,
) {
    select_named(s, "Torus");
    s.execute(Command::SetEditMode(mode), w, cx);
}
fn leave_edit(s: &mut Studio, w: &mut gpui::Window, cx: &mut gpui::Context<Studio>) {
    s.execute(Command::SetEditMode(EditMode::Object), w, cx);
    select_hero(s, cx);
}

thread_local! {
    /// Scenes replaced by a scenario, restored when it leaves.
    static SAVED_SCENE: std::cell::RefCell<Option<forma_core::Scene>> = const {
        std::cell::RefCell::new(None)
    };
}

fn stash_scene(s: &mut Studio) {
    SAVED_SCENE.with(|saved| *saved.borrow_mut() = Some(s.scene.clone()));
}
fn restore_scene(s: &mut Studio, cx: &mut gpui::Context<Studio>) {
    if let Some(scene) = SAVED_SCENE.with(|saved| saved.borrow_mut().take()) {
        s.scene = scene;
        s.dirty = false;
        s.invalidate(true, cx);
    }
}

/// Remove every object, as on first launch with an empty project.
fn empty_scene(s: &mut Studio, cx: &mut gpui::Context<Studio>) {
    stash_scene(s);
    let ids: Vec<u64> = s.scene.objects.iter().map(|object| object.id).collect();
    s.select_objects(Default::default(), None);
    for id in ids {
        s.scene.remove(id);
    }
    s.invalidate(true, cx);
}
fn leave_empty_scene(s: &mut Studio, cx: &mut gpui::Context<Studio>) {
    restore_scene(s, cx);
    select_hero(s, cx);
}

/// The CAD showcase scene, with `select` selected, in `mode`.
pub(crate) fn parts(
    s: &mut Studio,
    w: &mut gpui::Window,
    cx: &mut gpui::Context<Studio>,
    mode: RenderMode,
    select: Option<&str>,
) {
    stash_scene(s);
    s.scene = crate::cad::demo_scene();
    let selected = select.and_then(|name| {
        s.scene
            .objects
            .iter()
            .find(|object| object.name == name)
            .map(|object| object.id)
    });
    s.select_objects(selected.into_iter().collect(), selected);
    s.execute(Command::SetMode(mode), w, cx);
    s.invalidate(true, cx);
}
pub(crate) fn leave_parts(s: &mut Studio, w: &mut gpui::Window, cx: &mut gpui::Context<Studio>) {
    restore_scene(s, cx);
    select_hero(s, cx);
    s.execute(Command::SetMode(RenderMode::MaterialPreview), w, cx);
}

/// Modelling-tool states (see `cad_tools::demo`).
fn tool_scenarios() -> Vec<Scenario> {
    use crate::cad_tools::demo;
    let scenario = |name, enter| Scenario {
        name,
        enter,
        leave: demo::restore,
    };
    vec![
        scenario("tools-region-hover", demo::region_hover),
        scenario("tools-extrude-new", demo::extrude_new),
        scenario("tools-extrude-join", demo::extrude_join),
        scenario("tools-extrude-cut", demo::extrude_cut),
        scenario("tools-revolve", demo::revolve),
        scenario("tools-push-pull-part", demo::push_pull_part),
        scenario("tools-push-pull-mesh", demo::push_pull_mesh),
        scenario("tools-part-edit-faces", demo::part_edit_faces),
        scenario("tools-part-edit-edges", demo::part_edit_edges),
        scenario("tools-chamfer", demo::chamfer),
        scenario("tools-adjust", demo::adjust),
        Scenario {
            name: "tools-light-theme",
            enter: demo::extrude_light,
            leave: demo::restore_theme,
        },
        // The inspector's selection and live-operation panels in the light
        // theme and in a narrow window.
        Scenario {
            name: "tools-chamfer-light",
            enter: |s, w, cx| {
                theme(s, Theme::Paper, cx);
                demo::chamfer(s, w, cx);
            },
            leave: demo::restore_theme,
        },
        Scenario {
            name: "tools-part-edit-faces-light",
            enter: |s, w, cx| {
                theme(s, Theme::Paper, cx);
                demo::part_edit_faces(s, w, cx);
            },
            leave: demo::restore_theme,
        },
        Scenario {
            name: "tools-chamfer-narrow",
            enter: |s, w, cx| {
                resize(w, 1000., 650.);
                demo::chamfer(s, w, cx);
            },
            leave: |s, w, cx| {
                demo::restore(s, w, cx);
                restore_size(s, w, cx);
            },
        },
        Scenario {
            name: "tools-part-edit-faces-narrow",
            enter: |s, w, cx| {
                resize(w, 1000., 650.);
                demo::part_edit_faces(s, w, cx);
            },
            leave: |s, w, cx| {
                demo::restore(s, w, cx);
                restore_size(s, w, cx);
            },
        },
    ]
}

/// Workspace states captured by `--ui-capture`. Add a scenario here whenever a
/// new panel, overlay or mode lands so its appearance can be reviewed.
pub(crate) fn scenarios() -> Vec<Scenario> {
    let mut scenarios = vec![
        Scenario {
            name: "workspace",
            enter: nothing,
            leave: nothing,
        },
        Scenario {
            name: "unsaved",
            enter: |s, _, cx| {
                s.dirty = true;
                cx.notify();
            },
            leave: |s, _, cx| {
                s.dirty = false;
                cx.notify();
            },
        },
        Scenario {
            name: "commands",
            enter: |s, w, cx| s.execute(Command::TogglePalette, w, cx),
            leave: |s, w, cx| s.execute(Command::TogglePalette, w, cx),
        },
        Scenario {
            name: "commands-search",
            enter: |s, w, cx| {
                s.execute(Command::TogglePalette, w, cx);
                s.palette_query = "view".into();
            },
            leave: |s, w, cx| s.execute(Command::TogglePalette, w, cx),
        },
        Scenario {
            // A mesh selected, with a few commands run before: the palette
            // opens on commands for meshes, then recent commands.
            name: "commands-mesh",
            enter: |s, w, cx| {
                select_named(s, "Torus");
                for command in [Command::FrameSelected, Command::ToggleGrid] {
                    s.panels.remember_command(command);
                }
                s.invalidate(false, cx);
                s.execute(Command::TogglePalette, w, cx);
            },
            leave: |s, w, cx| {
                s.execute(Command::TogglePalette, w, cx);
                s.panels.forget_recent_commands();
                select_hero(s, cx);
            },
        },
        Scenario {
            // A partial query ranks word starts first and highlights matches.
            name: "commands-search-part",
            enter: |s, w, cx| {
                s.execute(Command::TogglePalette, w, cx);
                s.palette_query = "ex st".into();
                s.palette_index = 1;
            },
            leave: |s, w, cx| s.execute(Command::TogglePalette, w, cx),
        },
        Scenario {
            // The list scrolls to keep a keyboard selection below the fold
            // in view, heading included.
            name: "commands-scrolled",
            enter: |s, w, cx| {
                s.execute(Command::TogglePalette, w, cx);
                s.palette_index = 16;
            },
            leave: |s, w, cx| s.execute(Command::TogglePalette, w, cx),
        },
        Scenario {
            name: "commands-narrow",
            enter: |s, w, cx| {
                resize(w, 1000., 650.);
                s.execute(Command::TogglePalette, w, cx);
            },
            leave: |s, w, cx| {
                s.execute(Command::TogglePalette, w, cx);
                restore_size(s, w, cx);
            },
        },
        Scenario {
            name: "export-formats",
            enter: |s, w, cx| {
                s.execute(Command::TogglePalette, w, cx);
                s.palette_exports_only = true;
            },
            leave: |s, w, cx| s.execute(Command::TogglePalette, w, cx),
        },
        Scenario {
            name: "export-unavailable",
            enter: |s, w, cx| {
                stash_scene(s);
                let parts: Vec<u64> = s
                    .scene
                    .objects
                    .iter()
                    .filter(|object| object.is_part())
                    .map(|object| object.id)
                    .collect();
                for id in parts {
                    s.scene.remove(id);
                }
                s.scene.purge_orphans();
                select_named(s, "Torus");
                s.invalidate(true, cx);
                s.execute(Command::TogglePalette, w, cx);
                s.palette_exports_only = true;
                s.palette_index = 3;
            },
            leave: |s, w, cx| {
                s.execute(Command::TogglePalette, w, cx);
                restore_scene(s, cx);
                select_hero(s, cx);
            },
        },
        Scenario {
            name: "help",
            enter: |s, w, cx| s.execute(Command::ToggleHelp, w, cx),
            leave: |s, w, cx| s.execute(Command::ToggleHelp, w, cx),
        },
        Scenario {
            name: "theme-picker",
            enter: |s, w, cx| s.execute(Command::ToggleTheme, w, cx),
            leave: |s, w, cx| s.execute(Command::ToggleTheme, w, cx),
        },
        // Every overlay in the light theme: the picker previews Paper, the
        // reference card and the shading pie sit over the light viewport.
        Scenario {
            name: "theme-picker-paper",
            enter: |s, w, cx| {
                theme(s, Theme::Paper, cx);
                s.execute(Command::ToggleTheme, w, cx);
            },
            leave: |s, w, cx| {
                s.execute(Command::ToggleTheme, w, cx);
                theme(s, Theme::Forma, cx);
            },
        },
        Scenario {
            name: "help-paper",
            enter: |s, w, cx| {
                theme(s, Theme::Paper, cx);
                s.execute(Command::ToggleHelp, w, cx);
            },
            leave: |s, w, cx| {
                s.execute(Command::ToggleHelp, w, cx);
                theme(s, Theme::Forma, cx);
            },
        },
        Scenario {
            name: "shading-pie-paper",
            enter: |s, _, cx| {
                theme(s, Theme::Paper, cx);
                let center = s.bounds.get().center();
                s.open_shading_pie(glam::Vec2::new(center.x.into(), center.y.into()), cx);
                if let Some(pie) = s.shading_pie.as_mut() {
                    pie.hovered = Some(RenderMode::Rendered);
                }
            },
            leave: |s, _, cx| {
                s.close_shading_pie(cx);
                theme(s, Theme::Forma, cx);
            },
        },
        Scenario {
            name: "add-menu-filter-paper",
            enter: |s, _, cx| {
                theme(s, Theme::Paper, cx);
                s.open_menu(MenuKind::Add, Anchor::ToolRail, cx);
                if let Some(menu) = s.panels.menu.as_mut() {
                    menu.query = "cyl".into();
                    menu.index = 1;
                }
            },
            leave: |s, _, cx| {
                s.panels.close_menu();
                theme(s, Theme::Forma, cx);
            },
        },
        Scenario {
            name: "add-menu",
            enter: |s, _, cx| s.open_menu(MenuKind::Add, Anchor::ToolRail, cx),
            leave: |s, _, _| s.panels.close_menu(),
        },
        Scenario {
            name: "add-menu-outliner",
            enter: |s, _, cx| s.open_menu(MenuKind::Add, Anchor::Outliner, cx),
            leave: |s, _, _| s.panels.close_menu(),
        },
        Scenario {
            name: "add-menu-paper",
            enter: |s, _, cx| {
                theme(s, Theme::Paper, cx);
                s.open_menu(MenuKind::Add, Anchor::ToolRail, cx);
            },
            leave: |s, _, cx| {
                s.panels.close_menu();
                theme(s, Theme::Forma, cx);
            },
        },
        Scenario {
            name: "view-menu",
            enter: |s, _, cx| s.open_menu(MenuKind::View, Anchor::ViewHud, cx),
            leave: |s, _, _| s.panels.close_menu(),
        },
        Scenario {
            name: "preview-lighting",
            enter: |s, w, cx| s.execute(Command::TogglePreviewSettings, w, cx),
            leave: |s, w, cx| s.execute(Command::TogglePreviewSettings, w, cx),
        },
        Scenario {
            name: "edit-mode",
            enter: |s, w, cx| {
                edit_torus(s, w, cx, EditMode::Face);
                s.execute(Command::SelectAll, w, cx);
            },
            leave: leave_edit,
        },
        Scenario {
            name: "edit-mode-light",
            enter: |s, w, cx| {
                theme(s, Theme::Paper, cx);
                edit_torus(s, w, cx, EditMode::Face);
                s.execute(Command::SelectAll, w, cx);
            },
            leave: |s, w, cx| {
                leave_edit(s, w, cx);
                theme(s, Theme::Forma, cx);
            },
        },
        Scenario {
            name: "transform",
            enter: |s, _, cx| {
                s.start_transform(crate::app::Tool::Move, Some(0), true, cx);
                if let Some(drag) = s.transform_drag.as_mut() {
                    drag.numeric = "0.25".into();
                }
                cx.notify();
            },
            leave: |s, _, cx| {
                s.finish_transform(true, cx);
                s.tool = crate::app::Tool::Select;
            },
        },
        Scenario {
            name: "measurements",
            enter: |s, w, cx| s.execute(Command::ToggleMeasurements, w, cx),
            leave: |s, w, cx| s.execute(Command::ToggleMeasurements, w, cx),
        },
        Scenario {
            name: "field-editing",
            enter: |s, w, cx| s.begin_field(Field::Translation(1), w, cx),
            leave: end_field,
        },
        Scenario {
            name: "outliner-rename",
            enter: |s, w, cx| {
                s.begin_field(Field::Name, w, cx);
                s.panels.rename_in_outliner = true;
            },
            leave: end_field,
        },
        Scenario {
            name: "inspector-scrolled",
            enter: |s, _, cx| {
                s.panels.scroll_inspector_to(500.);
                cx.notify();
            },
            leave: |s, _, cx| {
                s.panels.scroll_inspector_to(0.);
                cx.notify();
            },
        },
        Scenario {
            name: "render-tab",
            enter: |s, _, cx| s.set_inspector_tab(InspectorTab::Render, cx),
            leave: |s, _, cx| s.set_inspector_tab(InspectorTab::Object, cx),
        },
        Scenario {
            name: "rendered-mode",
            enter: |s, w, cx| {
                s.execute(Command::SetMode(RenderMode::Rendered), w, cx);
                s.set_inspector_tab(InspectorTab::Render, cx);
            },
            leave: |s, w, cx| {
                s.execute(Command::SetMode(RenderMode::MaterialPreview), w, cx);
                s.set_inspector_tab(InspectorTab::Object, cx);
            },
        },
        Scenario {
            // A mesh's surface: the torus selected in Material Preview.
            name: "material-preview",
            enter: |s, _, cx| {
                select_named(s, "Torus");
                s.invalidate(false, cx);
            },
            leave: |s, _, cx| select_hero(s, cx),
        },
        Scenario {
            name: "solid-mode",
            enter: |s, w, cx| s.execute(Command::SetMode(RenderMode::Solid), w, cx),
            leave: |s, w, cx| s.execute(Command::SetMode(RenderMode::MaterialPreview), w, cx),
        },
        Scenario {
            name: "wireframe-mode",
            enter: |s, w, cx| s.execute(Command::SetMode(RenderMode::Wireframe), w, cx),
            leave: |s, w, cx| s.execute(Command::SetMode(RenderMode::MaterialPreview), w, cx),
        },
        Scenario {
            name: "no-selection",
            enter: |s, _, cx| {
                s.select_objects(Default::default(), None);
                s.invalidate(false, cx);
            },
            leave: |s, _, cx| select_hero(s, cx),
        },
        Scenario {
            name: "empty-scene",
            enter: |s, _, cx| empty_scene(s, cx),
            leave: |s, _, cx| leave_empty_scene(s, cx),
        },
        Scenario {
            name: "empty-scene-paper",
            enter: |s, _, cx| {
                theme(s, Theme::Paper, cx);
                empty_scene(s, cx);
            },
            leave: |s, _, cx| {
                leave_empty_scene(s, cx);
                theme(s, Theme::Forma, cx);
            },
        },
        // The start card's "Front plane" row: Draw opens facing the front.
        Scenario {
            name: "empty-scene-front-plane",
            enter: |s, _, cx| {
                theme(s, Theme::Paper, cx);
                empty_scene(s, cx);
                s.enter_draw(
                    crate::sketch::DrawTarget::Plane(
                        forma_core::SketchPlane::front(),
                        "Front plane".into(),
                    ),
                    cx,
                );
            },
            leave: |s, w, cx| {
                s.execute(Command::Draw(crate::sketch::DrawCommand::Finish), w, cx);
                leave_empty_scene(s, cx);
                theme(s, Theme::Forma, cx);
            },
        },
        Scenario {
            name: "empty-scene-narrow",
            enter: |s, w, cx| {
                resize(w, 1000., 650.);
                empty_scene(s, cx);
            },
            leave: |s, w, cx| {
                leave_empty_scene(s, cx);
                restore_size(s, w, cx);
            },
        },
        Scenario {
            // With nothing in the scene the palette suggests ways to start.
            name: "commands-empty",
            enter: |s, w, cx| {
                empty_scene(s, cx);
                s.execute(Command::TogglePalette, w, cx);
            },
            leave: |s, w, cx| {
                s.execute(Command::TogglePalette, w, cx);
                leave_empty_scene(s, cx);
            },
        },
        Scenario {
            name: "stl-export",
            enter: |s, w, cx| s.execute(Command::ExportStl, w, cx),
            leave: |s, _, cx| {
                s.stl_dialog = None;
                cx.notify();
            },
        },
        Scenario {
            name: "updates",
            enter: |s, w, cx| s.execute(Command::ToggleUpdates, w, cx),
            leave: |s, w, cx| s.execute(Command::ToggleUpdates, w, cx),
        },
        Scenario {
            name: "update-ready",
            enter: |s, _, cx| {
                s.updates.state = crate::updater::State::Ready("0.3.0".into());
                cx.notify();
            },
            leave: |s, _, cx| {
                s.updates.state = crate::updater::State::Idle;
                cx.notify();
            },
        },
        Scenario {
            name: "shader-editor",
            enter: |s, w, cx| s.execute(Command::EditShader, w, cx),
            leave: |s, w, cx| s.execute(Command::CloseShader, w, cx),
        },
        Scenario {
            name: "shading-pie",
            enter: |s, _, cx| {
                let center = s.bounds.get().center();
                s.open_shading_pie(glam::Vec2::new(center.x.into(), center.y.into()), cx);
                if let Some(pie) = s.shading_pie.as_mut() {
                    pie.hovered = Some(RenderMode::Rendered);
                }
            },
            leave: |s, _, cx| s.close_shading_pie(cx),
        },
        Scenario {
            name: "narrow",
            enter: |_, w, _| resize(w, 1000., 650.),
            leave: restore_size,
        },
        Scenario {
            name: "narrow-wide-sidebar",
            enter: |s, w, cx| {
                resize(w, 1000., 650.);
                s.panels.set_sidebar_width(560.);
                edit_torus(s, w, cx, EditMode::Edge);
            },
            leave: |s, w, cx| {
                leave_edit(s, w, cx);
                s.panels.set_sidebar_width(crate::ui::SIDEBAR_DEFAULT_WIDTH);
                restore_size(s, w, cx);
            },
        },
        Scenario {
            name: "narrow-edit",
            enter: |s, w, cx| {
                resize(w, 1000., 650.);
                edit_torus(s, w, cx, EditMode::Vertex);
            },
            leave: |s, w, cx| {
                leave_edit(s, w, cx);
                restore_size(s, w, cx);
            },
        },
        Scenario {
            name: "theme-paper",
            enter: |s, _, cx| theme(s, Theme::Paper, cx),
            leave: default_theme,
        },
        Scenario {
            name: "theme-paper-commands",
            enter: |s, w, cx| {
                theme(s, Theme::Paper, cx);
                s.execute(Command::TogglePalette, w, cx);
            },
            leave: |s, w, cx| {
                s.execute(Command::TogglePalette, w, cx);
                theme(s, Theme::Forma, cx);
            },
        },
        Scenario {
            name: "theme-paper-render",
            enter: |s, w, cx| {
                theme(s, Theme::Paper, cx);
                s.execute(Command::SetMode(RenderMode::Rendered), w, cx);
                s.set_inspector_tab(InspectorTab::Render, cx);
            },
            leave: |s, w, cx| {
                s.execute(Command::SetMode(RenderMode::MaterialPreview), w, cx);
                s.set_inspector_tab(InspectorTab::Object, cx);
                theme(s, Theme::Forma, cx);
            },
        },
        Scenario {
            name: "theme-matcha",
            enter: |s, _, cx| theme(s, Theme::Matcha, cx),
            leave: default_theme,
        },
        Scenario {
            name: "theme-matcha-edit",
            enter: |s, w, cx| {
                theme(s, Theme::Matcha, cx);
                edit_torus(s, w, cx, EditMode::Edge);
                s.execute(Command::ToggleMeasurements, w, cx);
            },
            leave: |s, w, cx| {
                s.execute(Command::ToggleMeasurements, w, cx);
                leave_edit(s, w, cx);
                theme(s, Theme::Forma, cx);
            },
        },
        Scenario {
            name: "theme-midnight",
            enter: |s, _, cx| theme(s, Theme::Midnight, cx),
            leave: default_theme,
        },
        Scenario {
            name: "theme-synthwave",
            enter: |s, _, cx| theme(s, Theme::Synthwave, cx),
            leave: default_theme,
        },
        Scenario {
            name: "draw-empty",
            enter: demo::empty_plane,
            leave: demo::restore,
        },
        Scenario {
            name: "draw-line-inference",
            enter: demo::line_inference,
            leave: demo::restore,
        },
        Scenario {
            name: "draw-paper",
            enter: demo::line_inference_paper,
            leave: demo::restore_theme,
        },
        Scenario {
            name: "draw-numeric",
            enter: demo::numeric_entry,
            leave: demo::restore,
        },
        Scenario {
            name: "draw-profiles",
            enter: demo::profiles,
            leave: demo::restore,
        },
        Scenario {
            name: "draw-tilted-face",
            enter: demo::tilted_face,
            leave: demo::restore,
        },
        Scenario {
            name: "draw-objects",
            enter: demo::finished_objects,
            leave: demo::restore,
        },
        Scenario {
            name: "draw-narrow",
            enter: demo::narrow,
            leave: demo::narrow_leave,
        },
        Scenario {
            name: "parts-material",
            enter: |s, w, cx| parts(s, w, cx, RenderMode::MaterialPreview, Some("Bracket")),
            leave: leave_parts,
        },
        Scenario {
            name: "parts-solid",
            enter: |s, w, cx| parts(s, w, cx, RenderMode::Solid, None),
            leave: leave_parts,
        },
        Scenario {
            name: "parts-wireframe",
            enter: |s, w, cx| parts(s, w, cx, RenderMode::Wireframe, None),
            leave: leave_parts,
        },
        Scenario {
            name: "parts-inspector",
            enter: |s, w, cx| {
                parts(s, w, cx, RenderMode::MaterialPreview, Some("Bracket"));
                s.toggle_section(PanelSection::Transform, cx);
                s.toggle_section(PanelSection::Surface, cx);
            },
            leave: |s, w, cx| {
                s.toggle_section(PanelSection::Transform, cx);
                s.toggle_section(PanelSection::Surface, cx);
                leave_parts(s, w, cx);
            },
        },
        Scenario {
            name: "parts-wireframe-selected",
            enter: |s, w, cx| parts(s, w, cx, RenderMode::Wireframe, Some("Block")),
            leave: leave_parts,
        },
    ];
    scenarios.extend(crate::timeline::capture::scenarios());
    scenarios.extend(tool_scenarios());
    scenarios
}

pub fn start(window: WindowHandle<Studio>, output: PathBuf, filter: Vec<String>, cx: &mut App) {
    cx.spawn(
        async move |cx| match run(window, &output, &filter, cx).await {
            Ok(count) => {
                println!("FORMA_UI_CAPTURE_PASS captures={count}");
                let _ = cx.update(|cx| cx.quit());
            }
            Err(error) => {
                eprintln!("FORMA_UI_CAPTURE_FAIL: {error:#}");
                crate::smoke::exit_after_failure(window, cx);
            }
        },
    )
    .detach();
}

async fn settle(window: WindowHandle<Studio>, cx: &mut AsyncApp) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        Timer::after(Duration::from_millis(60)).await;
        let ready = window.update(cx, |s, _, _| s.samples > 0 && s.frame.is_some())?;
        if ready {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "viewport did not produce a frame within 20 seconds"
        );
    }
}

async fn run(
    window: WindowHandle<Studio>,
    output: &Path,
    filter: &[String],
    cx: &mut AsyncApp,
) -> Result<usize> {
    std::fs::create_dir_all(output)?;
    settle(window, cx).await?;
    // Let progressive modes accumulate enough samples to look representative.
    Timer::after(Duration::from_millis(1200)).await;
    let mut count = 0;
    for scenario in scenarios() {
        if !filter.is_empty() && !filter.iter().any(|name| name == scenario.name) {
            continue;
        }
        window.update(cx, |s, w, cx| {
            (scenario.enter)(s, w, cx);
            cx.notify();
        })?;
        // CAD parts regenerate in the worker process; wait for their meshes.
        let deadline = Instant::now() + Duration::from_secs(60);
        while !window.update(cx, |s, _, _| s.cad_idle())? {
            ensure!(
                Instant::now() < deadline,
                "CAD parts did not regenerate within 60 seconds"
            );
            Timer::after(Duration::from_millis(50)).await;
        }
        Timer::after(Duration::from_millis(450)).await;
        // Progressive modes: capture the converged image, as a user would see it
        // after a moment, rather than the first noisy samples.
        let deadline = Instant::now() + Duration::from_secs(30);
        while window.update(cx, |s, _, _| {
            s.settings.mode.progressive() && s.samples < s.settings.max_samples.min(96)
        })? && Instant::now() < deadline
        {
            Timer::after(Duration::from_millis(100)).await;
        }
        prepare_capture(window, cx).await?;
        let path = output.join(format!("{}.png", scenario.name));
        window.update(cx, |s, w, _| capture_window(s, w, &path))??;
        println!("ui_capture={}", path.display());
        count += 1;
        window.update(cx, |s, w, cx| {
            (scenario.leave)(s, w, cx);
            cx.notify();
        })?;
        Timer::after(Duration::from_millis(150)).await;
    }
    ensure!(count > 0, "no capture scenario matched {filter:?}");
    Ok(count)
}

/// Reads this application's own X11 window. Works on Xorg, Xvfb and Xwayland
/// (rootless Xwayland refuses root-window reads, but not reads of our window).
#[cfg(target_os = "linux")]
pub(crate) fn capture_x11(window: &gpui::Window, path: &Path) -> Result<()> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, ImageFormat};
    // GPUI's X11 backend does not expose raw window handles, so find our own
    // top-level window by its _NET_WM_PID and the current logical size.
    let size = window.bounds().size;
    let scale = window.scale_factor();
    let expected = (
        (f32::from(size.width) * scale).round() as u16,
        (f32::from(size.height) * scale).round() as u16,
    );
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let pid_atom = conn.intern_atom(true, b"_NET_WM_PID")?.reply()?.atom;
    ensure!(pid_atom != 0, "X11 server has no _NET_WM_PID atom");
    let pid = std::process::id();
    // Window managers may reparent top-levels into frames, so search the tree.
    let mut best = None;
    let mut pending = conn.query_tree(root)?.reply()?.children;
    let mut visited = 0;
    while let Some(child) = pending.pop() {
        visited += 1;
        if visited > 4096 {
            break;
        }
        if let Ok(tree) = conn.query_tree(child)?.reply() {
            pending.extend(tree.children);
        }
        let Ok(reply) = conn
            .get_property(false, child, pid_atom, AtomEnum::CARDINAL, 0, 1)?
            .reply()
        else {
            continue;
        };
        if reply.value32().and_then(|mut v| v.next()) != Some(pid) {
            continue;
        }
        let Ok(geometry) = conn.get_geometry(child)?.reply() else {
            continue;
        };
        let exact = (geometry.width, geometry.height) == expected;
        if exact || best.is_none() {
            best = Some((child, geometry.width, geometry.height));
        }
        if exact {
            break;
        }
    }
    let (id, width, height) =
        best.ok_or_else(|| anyhow::anyhow!("no X11 window belongs to this process"))?;
    let reply = match conn
        .get_image(ImageFormat::Z_PIXMAP, id, 0, 0, width, height, !0)?
        .reply()
    {
        Ok(reply) => reply,
        Err(_) => {
            // X11 refuses to read a window that is partly off-screen (BadMatch).
            // Other windows opening at the same moment can make the window
            // manager cascade ours past the screen edge, so ask it to move the
            // window to the origin and read again.
            use x11rb::protocol::xproto::ConfigureWindowAux;
            conn.configure_window(id, &ConfigureWindowAux::new().x(0).y(0))?;
            conn.flush()?;
            std::thread::sleep(std::time::Duration::from_millis(400));
            conn.get_image(ImageFormat::Z_PIXMAP, id, 0, 0, width, height, !0)?
                .reply()?
        }
    };
    ensure!(
        reply.depth >= 24,
        "unsupported X11 visual depth {}",
        reply.depth
    );
    let (width, height) = (u32::from(width), u32::from(height));
    let pixels = (width * height) as usize;
    ensure!(
        reply.data.len() >= pixels * 4,
        "X11 returned {} bytes for a {width}x{height} window",
        reply.data.len()
    );
    // Z-pixmap at depth 24/32 on little-endian servers is BGRX.
    let mut rgba = Vec::with_capacity(pixels * 4);
    for bgrx in reply.data.chunks_exact(4).take(pixels) {
        rgba.extend_from_slice(&[bgrx[2], bgrx[1], bgrx[0], 255]);
    }
    image::save_buffer(path, &rgba, width, height, image::ColorType::Rgba8)?;
    Ok(())
}

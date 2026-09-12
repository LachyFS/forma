mod app;
mod code_editor;
mod render_worker;
mod shading_pie;
mod smoke;
#[cfg(target_os = "macos")]
mod trackpad;
mod ui;
mod viewport;

use app::{Command, Studio};
use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, WindowBounds, WindowOptions,
    actions, px, size,
};

actions!(
    forma,
    [
        NewProject,
        OpenProject,
        SaveProject,
        SaveProjectAs,
        ImportObj,
        ExportObj,
        ExportPng,
        Undo,
        Redo,
        Commands,
        Help,
        Quit
    ]
);

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let smoke_output = args
        .iter()
        .position(|arg| arg == "--smoke-test")
        .map(|index| {
            args.get(index + 1)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "artifacts/native-smoke".into())
        });
    Application::new().run(move |cx: &mut App| {
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1512.), px(960.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: smoke_output.is_none(),
                    window_min_size: Some(size(px(1000.), px(650.))),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("Forma".into()),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Studio::new(window, cx)),
            )
            .expect("could not open Forma window");
        macro_rules! register {
            ($action:ty, $command:expr) => {
                cx.on_action(move |_: &$action, cx| {
                    let _ = window.update(cx, |s, w, cx| s.execute($command, w, cx));
                });
            };
        }
        register!(NewProject, Command::New);
        register!(OpenProject, Command::Open);
        register!(SaveProject, Command::Save);
        register!(SaveProjectAs, Command::SaveAs);
        register!(ImportObj, Command::ImportObj);
        register!(ExportObj, Command::ExportObj);
        register!(ExportPng, Command::ExportImage);
        register!(Undo, Command::Undo);
        register!(Redo, Command::Redo);
        register!(Commands, Command::TogglePalette);
        register!(Help, Command::ToggleHelp);
        cx.on_action(move |_: &Quit, cx| {
            let _ = window.update(cx, |s, w, cx| {
                if s.should_close(w, cx) {
                    w.remove_window();
                }
            });
        });
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", Quit, None),
        ]);
        cx.set_menus(vec![
            Menu {
                name: "Forma".into(),
                items: vec![
                    MenuItem::action("Keyboard shortcuts", Help),
                    MenuItem::separator(),
                    MenuItem::action("Quit Forma", Quit),
                ],
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("New project", NewProject),
                    MenuItem::action("Open…", OpenProject),
                    MenuItem::action("Save", SaveProject),
                    MenuItem::action("Save as…", SaveProjectAs),
                    MenuItem::separator(),
                    MenuItem::action("Import OBJ…", ImportObj),
                    MenuItem::action("Export OBJ…", ExportObj),
                    MenuItem::action("Export image…", ExportPng),
                ],
            },
            Menu {
                name: "Edit".into(),
                items: vec![
                    MenuItem::action("Undo", Undo),
                    MenuItem::action("Redo", Redo),
                ],
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action("Commands", Commands),
                    MenuItem::action("Keyboard shortcuts", Help),
                ],
            },
        ]);
        if let Some(output) = smoke_output {
            smoke::start(window, output, cx);
        } else {
            cx.activate(true);
        }
    });
}

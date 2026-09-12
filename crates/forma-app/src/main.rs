mod app;
mod code_editor;
mod render_worker;
mod shading_pie;
mod smoke;
mod theme;
#[cfg(target_os = "macos")]
mod trackpad;
mod ui;
mod viewport;

use anyhow::{Context as _, Result};
use app::{Command, Studio};
use forma_render::Backend;
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
        ColorTheme,
        Help,
        Quit
    ]
);

const USAGE: &str = "Forma — 3D modelling and GPU rendering\n\nUsage: forma [--renderer BACKEND] [--smoke-test [DIRECTORY]]\n\nBackends: auto, wgpu, native-metal, metal, dx12, vulkan\n  auto          Native Metal on macOS; wgpu DX12 on Windows; wgpu Vulkan on Linux\n  wgpu          wgpu using the platform's default GPU backend\n  native-metal  Existing native Metal renderer (macOS)\n  metal         wgpu Metal (macOS)\n  dx12          wgpu DirectX 12 (Windows)\n  vulkan        wgpu Vulkan (Windows/Linux)\n\nFORMA_RENDERER sets the default backend; --renderer overrides it.\n";

#[derive(Debug)]
struct LaunchOptions {
    backend: Backend,
    smoke_output: Option<std::path::PathBuf>,
    help: bool,
}

impl LaunchOptions {
    fn parse(args: impl IntoIterator<Item = String>, environment: Option<&str>) -> Result<Self> {
        let mut args = args.into_iter().peekable();
        let mut backend = None;
        let mut smoke_output = None;
        let mut help = false;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--renderer" => {
                    let value = args.next().context("--renderer requires a backend name")?;
                    backend = Some(value.parse().context("invalid --renderer backend")?);
                }
                "--smoke-test" => {
                    smoke_output = Some(if args.peek().is_some_and(|arg| !arg.starts_with('-')) {
                        args.next().unwrap().into()
                    } else {
                        "artifacts/native-smoke".into()
                    });
                }
                "--help" | "-h" => help = true,
                _ if arg.starts_with("--renderer=") => {
                    backend = Some(
                        arg["--renderer=".len()..]
                            .parse()
                            .context("invalid --renderer backend")?,
                    );
                }
                _ => anyhow::bail!("unknown argument {arg:?}; use --help for available options"),
            }
        }
        let backend = match backend {
            Some(backend) => backend,
            None if help => Backend::Auto,
            None => match environment {
                Some(value) => value.parse().context("invalid FORMA_RENDERER backend")?,
                None => Backend::Auto,
            },
        };
        Ok(Self {
            backend,
            smoke_output,
            help,
        })
    }
}

/// Product shortcuts follow Command on macOS and Control on Windows/Linux.
pub(crate) fn platform_shortcut(mac: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        mac
    } else {
        other
    }
}

pub(crate) fn command_modifier(modifiers: gpui::Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.platform
    } else {
        modifiers.control
    }
}

fn main() -> Result<()> {
    let environment = std::env::var("FORMA_RENDERER").ok();
    let options = LaunchOptions::parse(std::env::args().skip(1), environment.as_deref())?;
    if options.help {
        print!("{USAGE}");
        return Ok(());
    }
    let smoke_output = options.smoke_output;
    let backend = options.backend;
    Application::new().run(move |cx: &mut App| {
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        // Smoke tests use an isolated preference file and never change the user's theme.
        let theme_path = smoke_output
            .as_ref()
            .map(|output| output.join("preferences/color-theme"))
            .or_else(theme::preference_path);
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
                |window, cx| cx.new(|cx| Studio::new(backend, theme_path, window, cx)),
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
        register!(ColorTheme, Command::ToggleTheme);
        register!(Help, Command::ToggleHelp);
        cx.on_action(move |_: &Quit, cx| {
            let _ = window.update(cx, |s, w, cx| {
                if s.should_close(w, cx) {
                    w.remove_window();
                }
            });
        });
        cx.bind_keys([
            KeyBinding::new(platform_shortcut("cmd-q", "ctrl-q"), Quit, None),
            KeyBinding::new(platform_shortcut("cmd-w", "ctrl-w"), Quit, None),
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
                    MenuItem::action("Color Theme…", ColorTheme),
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(args: &[&str], environment: Option<&str>) -> Result<LaunchOptions> {
        LaunchOptions::parse(args.iter().map(|arg| (*arg).to_owned()), environment)
    }

    #[test]
    fn renderer_cli_overrides_environment_and_smoke_flags_do_not_become_paths() {
        let parsed = options(&["--smoke-test", "--renderer", "wgpu"], Some("invalid")).unwrap();
        assert_eq!(parsed.backend, Backend::Wgpu);
        assert_eq!(
            parsed.smoke_output.unwrap(),
            std::path::PathBuf::from("artifacts/native-smoke")
        );
        let parsed = options(
            &["--renderer=vulkan", "--smoke-test", "artifacts/linux"],
            None,
        )
        .unwrap();
        assert_eq!(parsed.backend, Backend::Vulkan);
        assert_eq!(
            parsed.smoke_output.unwrap(),
            std::path::PathBuf::from("artifacts/linux")
        );
        assert_eq!(options(&[], Some("dx12")).unwrap().backend, Backend::Dx12);
        assert_eq!(options(&[], None).unwrap().backend, Backend::Auto);
    }

    #[test]
    fn invalid_backends_and_unknown_arguments_report_errors_before_opening_a_window() {
        assert!(options(&["--renderer"], None).is_err());
        assert!(options(&["--renderer", "bogus"], None).is_err());
        assert!(options(&[], Some("bogus")).is_err());
        assert!(options(&["--unknown"], None).is_err());
        assert!(options(&["--help"], Some("bogus")).unwrap().help);
    }

    #[test]
    fn command_shortcuts_use_the_platform_modifier() {
        let command = gpui::Modifiers {
            platform: true,
            ..Default::default()
        };
        let control = gpui::Modifiers {
            control: true,
            ..Default::default()
        };
        assert_eq!(command_modifier(command), cfg!(target_os = "macos"));
        assert_eq!(command_modifier(control), !cfg!(target_os = "macos"));
    }
}

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
use crate::app::{Command, Studio};
use crate::smoke::{capture_window, prepare_capture};
use anyhow::{Result, ensure};
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

/// Workspace states captured by `--ui-capture`. Add a scenario here whenever a
/// new panel, overlay or mode lands so its appearance can be reviewed.
pub(crate) fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "workspace",
            enter: nothing,
            leave: nothing,
        },
        Scenario {
            name: "commands",
            enter: |s, w, cx| s.execute(Command::TogglePalette, w, cx),
            leave: |s, w, cx| s.execute(Command::TogglePalette, w, cx),
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
    ]
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
                std::process::exit(1);
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
        Timer::after(Duration::from_millis(450)).await;
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
    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, id, 0, 0, width, height, !0)?
        .reply()?;
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

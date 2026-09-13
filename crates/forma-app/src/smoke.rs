//! Opt-in runtime regression: exercises the real GPUI window, input dispatch and GPU worker.
use crate::app::{Command, EditMode, Field, Studio};
use crate::platform_shortcut;
use anyhow::{Result, ensure};
use forma_core::{Primitive, Scene};
use forma_render::{PreviewSettings, RenderMode, StudioLight};
use glam::Vec2;
use gpui::{
    AnyWindowHandle, App, AsyncApp, EntityInputHandler, KeyDownEvent, KeyUpEvent, Keystroke,
    MouseButton, MouseDownEvent, MouseMoveEvent, Timer, WindowHandle, point, px, size,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub fn start(window: WindowHandle<Studio>, output: PathBuf, cx: &mut App) {
    cx.spawn(async move |cx| match run(window, output, cx).await {
        Ok(()) => {
            println!("FORMA_NATIVE_SMOKE_PASS");
            let _ = cx.update(|cx| cx.quit());
        }
        Err(error) => {
            eprintln!("FORMA_NATIVE_SMOKE_FAIL: {error:#}");
            std::process::exit(1);
        }
    })
    .detach();
}

async fn settle(window: WindowHandle<Studio>, cx: &mut AsyncApp) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        Timer::after(Duration::from_millis(60)).await;
        let ready = window.update(cx, |s, w, _| {
            let expected = crate::viewport::render_dimensions(
                s.bounds.get().size,
                w.scale_factor(),
                s.settings.mode,
            );
            s.samples > 0
                && s.render_error.is_none()
                && (s.settings.width, s.settings.height) == expected
                && s.frame
                    .as_ref()
                    .is_some_and(|frame| (frame.width(), frame.height()) == expected)
        })?;
        if ready {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "viewport did not produce a valid current frame within 20 seconds"
        );
    }
}

async fn keys(window: WindowHandle<Studio>, sequence: &[&str], cx: &mut AsyncApp) -> Result<()> {
    for key in sequence {
        let key = if cfg!(target_os = "macos") {
            (*key).to_owned()
        } else {
            key.replace("cmd-", "ctrl-")
        };
        let key = Keystroke::parse(&key)?;
        let handle: AnyWindowHandle = window.into();
        handle.update(cx, |_, window, cx| {
            window.dispatch_keystroke(key, cx);
        })?;
        Timer::after(Duration::from_millis(24)).await;
    }
    Ok(())
}

async fn wait_for(
    window: WindowHandle<Studio>,
    cx: &mut AsyncApp,
    ready: impl Fn(&Studio) -> bool,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        Timer::after(Duration::from_millis(30)).await;
        if window.update(cx, |s, _, _| ready(s))? {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "background document operation timed out"
        );
    }
}

async fn preview_field(
    window: WindowHandle<Studio>,
    field: Field,
    value: &str,
    cx: &mut AsyncApp,
) -> Result<()> {
    window.update(cx, |s, w, cx| s.begin_field(field, w, cx))?;
    let characters: Vec<String> = value.chars().map(|value| value.to_string()).collect();
    let mut sequence: Vec<&str> = characters.iter().map(String::as_str).collect();
    sequence.push("enter");
    keys(window, &sequence, cx).await
}

async fn check_preview_workflow(
    window: WindowHandle<Studio>,
    output: &std::path::Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    let (scene, selected, dirty, undo, redo, settings) = window.update(cx, |s, _, _| {
        (
            s.scene.clone(),
            s.selected,
            s.dirty,
            s.history.can_undo(),
            s.history.can_redo(),
            s.settings.clone(),
        )
    })?;
    keys(window, &["c"], cx).await?;
    settle(window, cx).await?;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.samples == 1 && !s.settings.mode.progressive(),
            "material preview did not complete in one deterministic frame"
        );
        capture_window(s, w, &output.join("material-preview.png"))?;
        println!(
            "preview_resolution={}x{} logical={:.0}x{:.0} backing_scale={}",
            s.settings.width,
            s.settings.height,
            f32::from(s.bounds.get().size.width),
            f32::from(s.bounds.get().size.height),
            w.scale_factor()
        );
        s.execute(Command::TogglePreviewSettings, w, cx);
        ensure!(s.preview_open, "preview lighting popover did not open");
        s.execute(Command::SetPreviewStudio(StudioLight::Courtyard), w, cx);
        Ok(())
    })??;
    preview_field(window, Field::PreviewRotation, "35", cx).await?;
    preview_field(window, Field::PreviewStrength, "1.2", cx).await?;
    preview_field(window, Field::PreviewOpacity, "40", cx).await?;
    preview_field(window, Field::PreviewBlur, "65", cx).await?;
    settle(window, cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let preview = &s.settings.preview;
        ensure!(
            preview.studio == StudioLight::Courtyard
                && (preview.rotation.to_degrees() - 35.).abs() < 0.001
                && (preview.strength - 1.2).abs() < 0.001
                && (preview.world_opacity - 0.4).abs() < 0.001
                && (preview.background_blur - 0.65).abs() < 0.001
                && s.active_field.is_none(),
            "preview lighting fields did not apply their displayed units"
        );
        ensure!(s.preview_open, "numeric preview input closed its popover");
        Ok(())
    })??;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, w, _| {
        capture_window(s, w, &output.join("material-preview-lighting.png"))
    })??;
    window.update(cx, |s, w, cx| {
        s.execute(Command::TogglePreviewWorld, w, cx);
        s.execute(Command::TogglePreviewAo, w, cx);
    })?;
    settle(window, cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.settings.preview.use_scene_world
                && !s.settings.preview.ambient_occlusion
                && s.samples == 1,
            "scene-world / contact-shadow toggles did not produce a preview frame"
        );
        s.execute(Command::SetPreviewStudio(StudioLight::Sunset), w, cx);
        ensure!(
            !s.settings.preview.use_scene_world,
            "choosing a studio did not restore studio lighting"
        );
        Ok(())
    })??;
    settle(window, cx).await?;

    // Exercise the asynchronous decoder without depending on installed assets.
    let hdri = output.join("preview-fixture.hdr");
    let mut radiance = vec![image::Rgb([0.12f32, 0.18, 0.3]); 16 * 8];
    for y in 1..4 {
        for x in 3..7 {
            radiance[y * 16 + x] = image::Rgb([6., 4., 2.]);
        }
    }
    image::codecs::hdr::HdrEncoder::new(std::fs::File::create(&hdri)?).encode(&radiance, 16, 8)?;
    window.update(cx, |s, _, cx| s.load_preview_hdri(hdri.clone(), cx))?;
    wait_for(window, cx, |s| !s.preview_loading).await?;
    settle(window, cx).await?;
    let (loaded_settings, loaded_surface) = window.update(cx, |s, _, _| -> Result<_> {
        ensure!(
            s.settings.preview.hdri_path.as_ref() == Some(&hdri)
                && s.status.starts_with("HDR environment ·")
                && s.samples == 1,
            "valid HDR environment did not reach the native preview"
        );
        Ok((
            s.settings.preview.clone(),
            s.frame.as_ref().unwrap().clone(),
        ))
    })??;
    let invalid = output.join("invalid-preview.hdr");
    std::fs::write(&invalid, b"not a Radiance image")?;
    window.update(cx, |s, _, cx| s.load_preview_hdri(invalid, cx))?;
    wait_for(window, cx, |s| !s.preview_loading).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.settings.preview == loaded_settings
                && s.frame
                    .as_ref()
                    .is_some_and(|frame| frame.same_surface(&loaded_surface))
                && s.status.starts_with("HDR environment:")
                && s.render_error.is_none(),
            "invalid HDR input replaced a working preview or became a renderer error"
        );
        Ok(())
    })??;
    // Both actions occur in the same UI turn, so a decode completion cannot
    // install its result before the newer built-in selection is recorded.
    window.update(cx, |s, w, cx| {
        s.load_preview_hdri(hdri.clone(), cx);
        s.execute(Command::SetPreviewStudio(StudioLight::Studio), w, cx);
    })?;
    settle(window, cx).await?;
    Timer::after(Duration::from_millis(250)).await;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            !s.preview_loading
                && s.settings.preview.hdri_path.is_none()
                && s.settings.preview.studio == StudioLight::Studio,
            "stale HDR completion replaced a newer studio selection"
        );
        s.load_preview_hdri(hdri.clone(), cx);
        s.execute(Command::TogglePreviewWorld, w, cx);
        Ok(())
    })??;
    Timer::after(Duration::from_millis(250)).await;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            !s.preview_loading
                && s.settings.preview.use_scene_world
                && s.settings.preview.hdri_path.is_none(),
            "stale HDR completion overrode a newer Scene World selection"
        );
        s.execute(Command::ResetPreview, w, cx);
        s.execute(Command::TogglePreviewSettings, w, cx);
        ensure!(!s.preview_open, "preview popover did not close");
        ensure!(
            s.settings.preview == PreviewSettings::default(),
            "reset did not restore default preview lighting"
        );
        Ok(())
    })??;
    settle(window, cx).await?;

    // A large path-tracing target must not turn a preview export into thousands
    // of redundant single-frame renders or prevent its worker from finishing.
    let png_path = output.join("material-preview-export.png");
    let started = Instant::now();
    let dimensions = window.update(cx, |s, _, cx| {
        s.settings.max_samples = 4096;
        s.settings.max_bounces = 32;
        s.export_image_to(png_path.clone(), cx);
        (s.settings.width, s.settings.height)
    })?;
    wait_for(window, cx, |s| s.status.starts_with("Image saved")).await?;
    ensure!(
        started.elapsed() < Duration::from_secs(10),
        "single-frame preview export took more than ten seconds"
    );
    ensure!(
        image::image_dimensions(&png_path)? == dimensions,
        "preview export dimensions differ from its viewport snapshot"
    );
    println!(
        "material_preview_export_ms={:.1}",
        started.elapsed().as_secs_f64() * 1000.
    );

    let (origin, mut surface) = window.update(cx, |s, _, _| {
        let bounds = s.bounds.get();
        (
            bounds.origin + point(bounds.size.width * 0.5, bounds.size.height * 0.5),
            s.frame.as_ref().unwrap().clone(),
        )
    })?;
    window.update(cx, |s, w, cx| {
        s.mouse_down(
            &MouseDownEvent {
                button: MouseButton::Middle,
                position: origin,
                ..Default::default()
            },
            w,
            cx,
        );
    })?;
    let mut presentations = 0;
    for index in 1..=20 {
        window.update(cx, |s, w, cx| {
            s.mouse_move(
                &MouseMoveEvent {
                    position: origin + point(px(index as f32 * 1.4), px(0.)),
                    pressed_button: Some(MouseButton::Middle),
                    ..Default::default()
                },
                w,
                cx,
            );
        })?;
        Timer::after(Duration::from_millis(16)).await;
        window.update(cx, |s, _, _| {
            if let Some(frame) = &s.frame
                && !frame.same_surface(&surface)
            {
                presentations += 1;
                surface = frame.clone();
            }
        })?;
    }
    ensure!(
        presentations >= 2,
        "material preview orbit starved native frame presentation ({presentations} frames)"
    );
    // Measure input-handler to adoption of its completed native surface. This
    // includes dispatch, GPU work and UI wakeup; it excludes compositor scanout.
    let mut latencies = Vec::new();
    let mut render_times = Vec::new();
    for index in 21..=32 {
        let started = Instant::now();
        window.update(cx, |s, w, cx| {
            s.mouse_move(
                &MouseMoveEvent {
                    position: origin + point(px(index as f32 * 1.4), px(0.)),
                    pressed_button: Some(MouseButton::Middle),
                    ..Default::default()
                },
                w,
                cx,
            );
        })?;
        loop {
            Timer::after(Duration::from_millis(1)).await;
            if let Some(render_ms) = window.update(cx, |s, _, _| {
                (s.samples > 0 && s.render_error.is_none()).then_some(s.render_ms)
            })? {
                latencies.push(started.elapsed().as_secs_f64() * 1000.);
                render_times.push(render_ms);
                break;
            }
            ensure!(
                started.elapsed() < Duration::from_secs(2),
                "preview input failed to produce a current frame"
            );
        }
    }
    latencies.sort_by(f64::total_cmp);
    render_times.sort_by(f64::total_cmp);
    println!(
        "preview_navigation input_to_surface_median_ms={:.2} input_to_surface_p95_ms={:.2} render_median_ms={:.2}",
        latencies[6], latencies[11], render_times[6]
    );
    window.update(cx, |s, _, cx| -> Result<()> {
        s.navigation = None;
        ensure!(
            (s.scene.camera.yaw - scene.camera.yaw).abs() > 0.05,
            "material preview orbit did not respond to pointer input"
        );
        s.scene.camera = scene.camera;
        s.settings.max_samples = settings.max_samples;
        s.settings.max_bounces = settings.max_bounces;
        ensure!(
            s.scene == scene
                && s.selected == selected
                && s.dirty == dirty
                && s.history.can_undo() == undo
                && s.history.can_redo() == redo,
            "preview lighting modified the document, selection, dirty flag, or undo history"
        );
        s.invalidate(false, cx);
        Ok(())
    })??;
    settle(window, cx).await?;
    let original_size = window.update(cx, |_, w, _| {
        let size = w.viewport_size();
        w.resize(gpui::size(px(1000.), px(650.)));
        size
    })?;
    prepare_capture(window, cx).await?;
    settle(window, cx).await?;
    window.update(cx, |s, w, cx| {
        s.execute(Command::TogglePreviewSettings, w, cx)
    })?;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, w, _| -> Result<()> {
        ensure!(
            s.preview_open,
            "lighting popover closed at minimum window size"
        );
        capture_window(s, w, &output.join("material-preview-lighting-small.png"))
    })??;
    window.update(cx, |s, w, cx| {
        s.execute(Command::TogglePreviewSettings, w, cx);
        w.resize(original_size);
    })?;
    prepare_capture(window, cx).await?;
    settle(window, cx).await?;
    println!(
        "material_preview=lighting_fields,studio_world_ao,valid_invalid_stale_hdr,one_frame_export PASS navigation_presentations={presentations}"
    );
    Ok(())
}

async fn check_shading_pie(
    window: WindowHandle<Studio>,
    output: &std::path::Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    let release = KeyUpEvent {
        keystroke: Keystroke::parse("z")?,
    };
    let (scene, selected, dirty, center) = window.update(cx, |s, _, _| {
        let bounds = s.bounds.get();
        (
            s.scene.clone(),
            s.selected,
            s.dirty,
            Vec2::new(
                f32::from(bounds.left() + bounds.size.width * 0.5),
                f32::from(bounds.top() + bounds.size.height * 0.5),
            ),
        )
    })?;
    for (shortcut, expected) in [
        ("4", RenderMode::Wireframe),
        ("6", RenderMode::Solid),
        ("2", RenderMode::MaterialPreview),
        ("8", RenderMode::Rendered),
    ] {
        let previous = window.update(cx, |s, _, _| s.settings.mode)?;
        keys(window, &["z"], cx).await?;
        window.update(cx, |s, w, cx| -> Result<()> {
            ensure!(
                s.shading_pie.is_some() && s.settings.mode == previous && s.scene == scene,
                "Z did not open the shading pie without changing the scene or mode"
            );
            s.on_key_up(&release, w, cx);
            ensure!(
                s.shading_pie.as_ref().is_some_and(|p| !p.trigger_held),
                "neutral Z release did not latch the shading pie"
            );
            Ok(())
        })??;
        keys(window, &[shortcut], cx).await?;
        settle(window, cx).await?;
        window.update(cx, |s, _, _| -> Result<()> {
            ensure!(
                s.settings.mode == expected && s.shading_pie.is_none(),
                "shading pie key {shortcut} did not commit {expected:?}"
            );
            ensure!(
                s.scene == scene,
                "shading choice changed scene geometry or camera"
            );
            println!(
                "pie_mode={expected:?} samples={} render_ms={:.2} device={}",
                s.samples, s.render_ms, s.device_name
            );
            Ok(())
        })??;
    }

    keys(window, &["x"], cx).await?;
    settle(window, cx).await?;
    window.update(cx, |s, w, cx| {
        s.open_shading_pie(center, cx);
        s.on_key_up(&release, w, cx);
        s.mouse_move(
            &MouseMoveEvent {
                position: point(px(center.x), px(center.y - 103.0)),
                ..Default::default()
            },
            w,
            cx,
        );
    })?;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, w, _| -> Result<()> {
        ensure!(
            s.shading_pie.as_ref().and_then(|p| p.hovered) == Some(RenderMode::Rendered)
                && s.settings.mode == RenderMode::Solid,
            "hover changed the renderer before selection"
        );
        capture_window(s, w, &output.join("shading-pie.png"))?;
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;

    for button in [MouseButton::Right, MouseButton::Left] {
        window.update(cx, |s, w, cx| -> Result<()> {
            s.open_shading_pie(center, cx);
            s.on_key_up(&release, w, cx);
            s.mouse_down(
                &MouseDownEvent {
                    button,
                    position: point(px(center.x), px(center.y)),
                    ..Default::default()
                },
                w,
                cx,
            );
            ensure!(
                s.shading_pie.is_none(),
                "pie cancellation left an overlay open"
            );
            Ok(())
        })??;
    }
    window.update(cx, |s, w, cx| -> Result<()> {
        let bounds = s.bounds.get();
        let corner = Vec2::new(
            f32::from(bounds.left()) + 1.0,
            f32::from(bounds.top()) + 1.0,
        );
        s.open_shading_pie(corner, cx);
        s.on_key_up(&release, w, cx);
        ensure!(
            s.shading_pie
                .as_ref()
                .is_some_and(|p| p.hovered.is_none() && !p.trigger_held),
            "opening near a viewport edge selected a mode without movement"
        );
        Ok(())
    })??;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, w, _| {
        capture_window(s, w, &output.join("shading-pie-edge.png"))
    })??;
    keys(window, &["escape"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.shading_pie.is_none()
                && s.settings.mode == RenderMode::Solid
                && s.scene == scene
                && s.selected == selected
                && s.dirty == dirty
                && s.navigation.is_none(),
            "pie hover or cancellation changed the document, selection, or camera"
        );
        Ok(())
    })??;

    // A latched hover must survive subsequent key releases until clicked.
    window.update(cx, |s, w, cx| -> Result<()> {
        s.open_shading_pie(center, cx);
        s.on_key_up(&release, w, cx);
        let position = point(px(center.x), px(center.y + 103.0));
        s.mouse_move(
            &MouseMoveEvent {
                position,
                ..Default::default()
            },
            w,
            cx,
        );
        s.on_key_up(&release, w, cx);
        ensure!(
            s.shading_pie.is_some(),
            "a later Z release committed a latched hover"
        );
        s.mouse_down(
            &MouseDownEvent {
                button: MouseButton::Left,
                position,
                ..Default::default()
            },
            w,
            cx,
        );
        ensure!(
            s.settings.mode == RenderMode::MaterialPreview && s.shading_pie.is_none(),
            "clicking a latched pie card did not select its render mode"
        );
        Ok(())
    })??;
    settle(window, cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        let samples = s.samples;
        ensure!(samples == 1, "preview has no completed deterministic frame");
        s.open_shading_pie(center, cx);
        s.mouse_move(
            &MouseMoveEvent {
                position: point(px(center.x + 142.0), px(center.y)),
                ..Default::default()
            },
            w,
            cx,
        );
        ensure!(
            s.samples == samples,
            "pie hover invalidated the preview frame"
        );
        s.close_shading_pie(cx);
        s.execute(Command::SetMode(RenderMode::MaterialPreview), w, cx);
        ensure!(
            s.samples == samples,
            "choosing the current mode invalidated the preview frame"
        );
        s.open_shading_pie(center, cx);
        s.mouse_move(
            &MouseMoveEvent {
                position: point(px(center.x), px(center.y - 100.0)),
                ..Default::default()
            },
            w,
            cx,
        );
        s.on_key_up(&release, w, cx);
        ensure!(
            s.settings.mode == RenderMode::Rendered && s.shading_pie.is_none(),
            "held Z directional release did not commit Rendered"
        );
        // A number or click may commit while Z is still physically held.
        s.open_shading_pie(center, cx);
        s.on_key(
            &KeyDownEvent {
                keystroke: Keystroke::parse("8")?,
                is_held: false,
            },
            w,
            cx,
        );
        s.on_key(
            &KeyDownEvent {
                keystroke: release.keystroke.clone(),
                is_held: true,
            },
            w,
            cx,
        );
        ensure!(
            s.shading_pie.is_none(),
            "held Z auto-repeat reopened a committed pie"
        );
        s.on_key_up(&release, w, cx);
        ensure!(
            s.scene == scene && s.selected == selected,
            "pie selection leaked into modelling"
        );
        Ok(())
    })??;
    settle(window, cx).await?;
    println!("shading_pie=all_modes_latch_flick_click_cancel_edges_pass");
    Ok(())
}

async fn check_denoise_workflow(
    window: WindowHandle<Studio>,
    output: &std::path::Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    window.update(cx, |s, _, cx| {
        s.scene = Scene::default();
        s.selected = s.scene.objects.first().map(|object| object.id);
        s.settings.mode = RenderMode::Rendered;
        s.settings.exposure = 0.0;
        s.settings.max_samples = 16;
        s.scene.render.max_samples = 16;
        s.settings.denoise = Default::default();
        s.invalidate(true, cx);
    })?;
    wait_for(window, cx, |s| {
        s.denoised_generation.is_some() && s.frame.as_ref().is_some_and(|f| f.samples == 16)
    })
    .await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.samples == 16 && s.frame.as_ref().unwrap().denoised,
            "Viewport did not denoise its final sample"
        );
        capture_window(s, w, &output.join("ai-denoised.png"))?;
        s.execute(Command::ToggleViewportDenoise, w, cx);
        Ok(())
    })??;
    settle(window, cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.samples == 16 && !s.frame.as_ref().unwrap().denoised,
            "Disabling denoising did not restore the capped raw film"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(
            s.settings.denoise.viewport && s.scene.render.denoise.viewport,
            "Denoising preferences did not follow undo"
        );
        s.execute(
            Command::SetDenoiseQuality(forma_core::DenoiseQuality::High),
            w,
            cx,
        );
        Ok(())
    })??;
    wait_for(window, cx, |s| s.denoised_generation.is_some()).await?;
    // Changes made while denoising may finish out of order; only the current
    // request may replace the view. Resize also checks snapshot dimensions.
    for _ in 0..8 {
        window.update(cx, |s, _, cx| {
            s.scene.camera.orbit(Vec2::new(0.04, 0.0));
            s.invalidate(false, cx);
            assert!(s.denoised_generation.is_none());
        })?;
        Timer::after(Duration::from_millis(20)).await;
    }
    wait_for(window, cx, |s| {
        s.denoised_generation.is_some() && s.frame.as_ref().is_some_and(|f| f.samples == 16)
    })
    .await?;
    let path = output.join("ai-denoised-export.png");
    window.update(cx, |s, _, cx| s.export_image_to(path.clone(), cx))?;
    wait_for(window, cx, |s| {
        s.status.starts_with("Image saved") || s.status.starts_with("Image export failed")
    })
    .await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.status.starts_with("Image saved"),
            "Denoised export failed: {}",
            s.status
        );
        ensure!(
            image::image_dimensions(&path)? == (s.settings.width, s.settings.height),
            "Denoised export dimensions differ"
        );
        Ok(())
    })??;
    println!(
        "ai_denoising=viewport,final_sample,toggle_raw_without_restart,undo,quality,navigation,export PASS"
    );
    Ok(())
}

async fn check_theme_workflow(
    window: WindowHandle<Studio>,
    output: &std::path::Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    use crate::theme::Theme;
    let (scene, selected, dirty, undo, redo, samples, original, frame, settings) =
        window.update(cx, |s, _, _| {
            (
                s.scene.clone(),
                s.selected,
                s.dirty,
                s.history.can_undo(),
                s.history.can_redo(),
                s.samples,
                s.theme,
                s.frame_image.clone(),
                s.settings.clone(),
            )
        })?;
    let shortcut = platform_shortcut("cmd-shift-t", "ctrl-shift-t");
    keys(window, &[shortcut, "down"], cx).await?;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, w, _| {
        capture_window(s, w, &output.join("theme-picker-all.png"))
    })??;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.theme_picker
                .as_ref()
                .is_some_and(|picker| picker.preview() != original),
            "arrow navigation did not preview a new theme"
        );
        ensure!(
            s.theme == original && s.navigation_blocked(),
            "browsing committed the theme or left viewport navigation enabled"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.theme_picker.is_none() && s.theme == original,
            "Escape did not restore the saved theme"
        );
        Ok(())
    })??;
    // The regular command palette is another entry point, and search keystrokes
    // (including transform/shading shortcuts) must stay inside the theme picker.
    keys(
        window,
        &[
            platform_shortcut("cmd-k", "ctrl-k"),
            "t",
            "h",
            "e",
            "m",
            "e",
            "enter",
            "s",
            "y",
            "n",
            "t",
            "h",
        ],
        cx,
    )
    .await?;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, w, _| -> Result<()> {
        ensure!(
            s.theme_picker.as_ref().and_then(|picker| picker.selected()) == Some(Theme::Synthwave),
            "theme command/search did not select Synthwave"
        );
        capture_window(s, w, &output.join("theme-picker.png"))?;
        Ok(())
    })??;
    keys(window, &["enter"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.theme == Theme::Synthwave && s.theme_picker.is_none(),
            "Return did not commit the theme"
        );
        ensure!(
            Theme::load(s.theme_path.as_deref().unwrap())? == Theme::Synthwave,
            "theme preference did not persist"
        );
        Ok(())
    })??;
    keys(window, &[shortcut, "z", "z", "z", "enter"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.theme_picker
                .as_ref()
                .is_some_and(|picker| picker.selected().is_none())
                && s.theme == Theme::Synthwave,
            "empty search committed a theme"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    for theme in Theme::ALL {
        window.update(cx, |s, _, cx| s.apply_theme(theme, cx))?;
        prepare_capture(window, cx).await?;
        window.update(cx, |s, w, _| {
            capture_window(s, w, &output.join(format!("theme-{}.png", theme.id())))
        })??;
        if matches!(theme, Theme::Paper | Theme::Synthwave) {
            window.update(cx, |s, w, cx| -> Result<()> {
                s.execute(Command::EditShader, w, cx);
                let editor = s.shader_editor.as_ref().unwrap().read(cx);
                let colors = s.theme_colors();
                ensure!(
                    editor.colors.text == colors.text
                        && editor.colors.well == colors.well
                        && editor.colors.active == colors.active,
                    "shader editor did not inherit the selected theme"
                );
                Ok(())
            })??;
            prepare_capture(window, cx).await?;
            window.update(cx, |s, w, cx| -> Result<()> {
                capture_window(
                    s,
                    w,
                    &output.join(format!("theme-{}-shader.png", theme.id())),
                )?;
                s.execute(Command::CloseShader, w, cx);
                Ok(())
            })??;
        }
    }
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(
            s.scene == scene
                && s.selected == selected
                && s.dirty == dirty
                && s.history.can_undo() == undo
                && s.history.can_redo() == redo,
            "theme changes modified the document or undo history"
        );
        ensure!(
            s.samples == samples && s.settings == settings,
            "theme changes reset the viewport samples"
        );
        if let (Some(before), Some(after)) = (&frame, &s.frame_image) {
            ensure!(
                std::sync::Arc::ptr_eq(before, after),
                "theme changes replaced the completed viewport frame"
            );
        }
        s.apply_theme(original, cx);
        Ok(())
    })??;
    let original_size = window.update(cx, |s, w, cx| {
        let size = w.bounds().size;
        w.resize(gpui::size(px(1000.), px(650.)));
        s.execute(Command::ToggleTheme, w, cx);
        size
    })?;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, w, _| {
        capture_window(s, w, &output.join("theme-picker-minimum.png"))
    })??;
    keys(window, &["l", "i", "g", "h", "t", "down"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.theme_picker.as_ref().and_then(|picker| picker.selected()) == Some(Theme::Matcha),
            "filtered arrow navigation did not select Matcha"
        );
        s.mouse_down(
            &MouseDownEvent {
                button: MouseButton::Left,
                position: s.bounds.get().center(),
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            },
            w,
            cx,
        );
        ensure!(
            s.theme_picker.is_none() && s.theme == original && s.selected == selected,
            "outside click did not cancel the theme preview without selecting an object"
        );
        Ok(())
    })??;
    window.update(cx, |_, w, _| w.resize(original_size))?;
    settle(window, cx).await?;
    println!("editor_themes=search_preview_cancel_commit_persistence_document_isolation_pass");
    Ok(())
}

async fn run(window: WindowHandle<Studio>, output: PathBuf, cx: &mut AsyncApp) -> Result<()> {
    std::fs::create_dir_all(&output)?;
    settle(window, cx).await?;
    let device = window.update(cx, |s, _, _| s.device_name.clone())?;
    println!("renderer={device}");
    Timer::after(Duration::from_secs(1)).await;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, window, _| {
        match capture_window(s, window, &output.join("workspace.png")) {
            Ok(()) => println!("workspace_capture=ok"),
            Err(error) => println!("workspace_capture=unavailable ({error:#})"),
        }
    })?;
    check_theme_workflow(window, &output, cx).await?;
    check_preview_workflow(window, &output, cx).await?;
    check_shader_workflow(window, cx).await?;
    let palette_key = platform_shortcut("cmd-k", "ctrl-k");
    keys(window, &[palette_key, palette_key], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            !s.palette_open,
            "platform shortcut did not close the command palette"
        );
        Ok(())
    })??;
    keys(
        window,
        &[platform_shortcut("cmd-k", "ctrl-k"), "w", "i", "r", "e"],
        cx,
    )
    .await?;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, w, _| -> Result<()> {
        ensure!(
            s.palette_open && s.palette_query == "wire",
            "command search leaked keys into editing"
        );
        let _ = capture_window(s, w, &output.join("commands.png"));
        Ok(())
    })??;
    keys(window, &["enter"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.settings.mode == RenderMode::Wireframe && !s.palette_open,
            "command palette did not execute selection"
        );
        Ok(())
    })??;
    check_shading_pie(window, &output, cx).await?;
    // A render must be presented throughout a 120 Hz input stream, even when
    // pointer events are newer than the most recently completed GPU sample.
    let (origin, yaw, mut surface) = window.update(cx, |s, _, _| {
        let b = s.bounds.get();
        (
            b.origin + point(b.size.width * 0.5, b.size.height * 0.5),
            s.scene.camera.yaw,
            s.frame.as_ref().unwrap().clone(),
        )
    })?;
    window.update(cx, |s, w, cx| {
        s.mouse_down(
            &MouseDownEvent {
                button: MouseButton::Middle,
                position: origin,
                ..Default::default()
            },
            w,
            cx,
        );
    })?;
    let mut presentations = 0;
    for index in 1..=80 {
        let position = origin + point(px(index as f32 * 0.7), px(0.));
        window.update(cx, |s, w, cx| {
            s.mouse_move(
                &MouseMoveEvent {
                    position,
                    pressed_button: Some(MouseButton::Middle),
                    ..Default::default()
                },
                w,
                cx,
            );
        })?;
        Timer::after(Duration::from_millis(8)).await;
        window.update(cx, |s, _, _| {
            if let Some(frame) = &s.frame
                && !frame.same_surface(&surface)
            {
                presentations += 1;
                surface = frame.clone();
            }
        })?;
    }
    window.update(cx, |s, _, _| s.navigation = None)?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            (s.scene.camera.yaw - yaw).abs() > 0.1,
            "native orbit events did not reach viewport"
        );
        Ok(())
    })??;
    ensure!(
        presentations >= 5,
        "continuous orbit starved GPU frame presentation ({presentations} frames)"
    );
    println!("continuous_navigation_presentations={presentations}");
    // Exercise high-resolution trackpad input through the viewport's handler.
    let before_trackpad = window.update(cx, |s, _, _| s.scene.camera)?;
    let mut trackpad_presentations = 0;
    for _ in 0..80 {
        window.update(cx, |s, _, cx| {
            s.scroll_wheel(
                &gpui::ScrollWheelEvent {
                    position: origin,
                    delta: gpui::ScrollDelta::Pixels(point(px(0.7), px(0.15))),
                    ..Default::default()
                },
                cx,
            );
        })?;
        Timer::after(Duration::from_millis(8)).await;
        window.update(cx, |s, _, _| {
            if let Some(frame) = &s.frame
                && !frame.same_surface(&surface)
            {
                trackpad_presentations += 1;
                surface = frame.clone();
            }
        })?;
    }
    ensure!(
        trackpad_presentations >= 5,
        "trackpad orbit starved frame presentation"
    );
    println!("trackpad_navigation_presentations={trackpad_presentations}");
    settle(window, cx).await?;
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(
            (s.scene.camera.yaw - before_trackpad.yaw).abs() > 0.1,
            "fractional trackpad input did not orbit"
        );
        ensure!(
            s.scene.camera.distance == before_trackpad.distance,
            "trackpad orbit unexpectedly zoomed"
        );
        let event = gpui::ScrollWheelEvent {
            position: origin,
            delta: gpui::ScrollDelta::Pixels(point(px(12.), px(8.))),
            modifiers: gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            ..Default::default()
        };
        s.scroll_wheel(&event, cx);
        ensure!(
            s.scene.camera.target != before_trackpad.target,
            "trackpad pan did not move target"
        );
        let before = s.scene.camera;
        s.help_open = true;
        s.scroll_wheel(&event, cx);
        s.help_open = false;
        ensure!(
            s.scene.camera == before,
            "trackpad moved camera behind help overlay"
        );
        s.scroll_wheel(
            &gpui::ScrollWheelEvent {
                modifiers: gpui::Modifiers {
                    control: true,
                    ..Default::default()
                },
                ..event
            },
            cx,
        );
        ensure!(
            s.scene.camera.distance < before.distance,
            "trackpad zoom did not dolly in"
        );
        Ok(())
    })??;
    window.update(cx, |s, _, cx| -> Result<()> {
        let before = s.scene.camera;
        s.magnify(point(px(-1.), px(-1.)), 0.1, cx);
        ensure!(
            s.scene.camera == before,
            "pinch outside viewport changed camera"
        );
        s.preview_open = true;
        s.magnify(origin, 0.1, cx);
        s.preview_open = false;
        ensure!(
            s.scene.camera == before,
            "pinch changed camera behind popup"
        );
        s.magnify(origin, 0.1, cx);
        ensure!(
            s.scene.camera.distance < before.distance,
            "spread gesture did not zoom in"
        );
        s.magnify(origin, -0.1, cx);
        ensure!(
            (s.scene.camera.distance - before.distance).abs() < 0.0001,
            "inverse pinch did not restore distance"
        );
        Ok(())
    })??;
    println!("trackpad_navigation_pass");
    keys(window, &["0"], cx).await?;
    let (id, before_x) = window.update(cx, |s, w, cx| {
        s.execute(Command::Add(Primitive::Cube), w, cx);
        let object = s.selected_object().unwrap();
        (object.id, object.transform.translation.x)
    })?;
    settle(window, cx).await?;
    keys(window, &["g", "z"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.shading_pie.is_none()
                && s.transform_drag
                    .as_ref()
                    .is_some_and(|drag| drag.axis == Some(2)),
            "Z opened shading instead of constraining the active transform"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    keys(window, &["g", "x", "2", "enter"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            (s.scene.object(id).unwrap().transform.translation.x - before_x - 2.).abs() < 0.001,
            "numeric constrained transform failed"
        );
        Ok(())
    })??;
    keys(window, &[platform_shortcut("cmd-z", "ctrl-z")], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            (s.scene.object(id).unwrap().transform.translation.x - before_x).abs() < 0.001,
            "undo transform failed"
        );
        Ok(())
    })??;
    keys(
        window,
        &[
            platform_shortcut("cmd-shift-z", "ctrl-shift-z"),
            "g",
            "x",
            "4",
            "escape",
        ],
        cx,
    )
    .await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            (s.scene.object(id).unwrap().transform.translation.x - before_x - 2.).abs() < 0.001,
            "redo / cancel transform failed"
        );
        Ok(())
    })??;
    // Exercise every component transform through real keyboard dispatch, including
    // repeated numeric updates, mode switching and history.
    for mode in [EditMode::Vertex, EditMode::Edge, EditMode::Face] {
        let (before, selected, transform) = window.update(cx, |s, w, cx| {
            s.execute(Command::SetEditMode(mode), w, cx);
            match mode {
                EditMode::Vertex => s.selected_vertex = Some(0),
                EditMode::Edge => s.selected_edge = Some([0, 1]),
                EditMode::Face => s.selected_face = Some(0),
                EditMode::Object => unreachable!(),
            }
            (
                s.scene.object_mesh(id).unwrap().clone(),
                s.component_vertices(),
                s.scene.object(id).unwrap().transform,
            )
        })?;
        keys(window, &["g", "x", ".", "2", "5", "enter"], cx).await?;
        window.update(cx, |s, _, _| -> Result<()> {
            ensure!(
                s.scene.object(id).unwrap().transform == transform,
                "component move changed object transform"
            );
            let mesh = s.scene.object_mesh(id).unwrap();
            for (i, p) in mesh.positions.iter().enumerate() {
                let delta = if selected.contains(&(i as u32)) {
                    glam::Vec3::X * 0.25
                } else {
                    glam::Vec3::ZERO
                };
                ensure!(
                    p.distance(before.positions[i] + delta) < 0.001,
                    "{mode:?} moved the wrong vertices"
                );
            }
            Ok(())
        })??;
        keys(window, &[platform_shortcut("cmd-z", "ctrl-z")], cx).await?;
        window.update(cx, |s, w, cx| -> Result<()> {
            ensure!(
                *s.scene.object_mesh(id).unwrap() == before,
                "component undo did not restore mesh"
            );
            ensure!(
                s.component_vertices().is_empty(),
                "undo left stale components"
            );
            s.execute(Command::ToggleEdit, w, cx);
            ensure!(
                s.edit_mode == EditMode::Object && !s.settings.edit_wireframe,
                "Object mode retained wireframe"
            );
            s.execute(Command::ToggleEdit, w, cx);
            ensure!(
                s.edit_mode == mode && s.settings.edit_wireframe,
                "Tab did not restore component mode"
            );
            Ok(())
        })??;
    }
    window.update(cx, |s, w, cx| {
        s.execute(Command::SetEditMode(EditMode::Object), w, cx)
    })?;
    keys(window, &["tab"], cx).await?;
    window.update(cx, |s, _, _| {
        s.selected_face = Some(0);
    })?;
    keys(window, &["e"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.scene.object_mesh(id).unwrap().faces.len() == 10,
            "face extrusion did not add four side faces"
        );
        s.scene.validate()?;
        s.execute(Command::Undo, w, cx);
        s.execute(Command::Subdivide, w, cx);
        ensure!(
            s.scene.object_mesh(id).unwrap().faces.len() == 24,
            "subdivision did not create 24 faces"
        );
        s.begin_field(Field::Exposure, w, cx);
        Ok(())
    })??;
    keys(window, &["1", ".", "2", "5", "enter"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.scene.render.exposure == 1.25,
            "render exposure not persisted in scene"
        );
        Ok(())
    })??;
    window.update(cx, |s, w, cx| s.begin_field(Field::Name, w, cx))?;
    keys(window, &["z"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.shading_pie.is_none()
                && s.active_field
                    .as_ref()
                    .is_some_and(|(field, text)| *field == Field::Name && text == "z"),
            "typing Z in the name field opened shading or lost the character"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    window.update(cx, |s, w, cx| s.begin_field(Field::Name, w, cx))?;
    keys(window, &["h", "e", "r", "o", "enter"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.selected_object().unwrap().name == "hero",
            "object rename input failed"
        );
        s.begin_field(Field::Color(0), w, cx);
        Ok(())
    })??;
    keys(window, &["0", ".", "5", "enter"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            (s.selected_object().unwrap().material.base_color.x - 0.214041).abs() < 0.0001,
            "sRGB field was not decoded into linear material color"
        );
        Ok(())
    })??;
    keys(window, &["7"], cx).await?;
    let scene_path = output.join("native-smoke.forma");
    let obj_path = output.join("native-smoke.obj");
    window.update(cx, |s, w, cx| -> Result<()> {
        s.scene.save(&scene_path)?;
        let loaded = Scene::load(&scene_path)?;
        ensure!(
            loaded.render.exposure == 1.25 && loaded.camera.orthographic,
            "project roundtrip lost settings"
        );
        s.scene.export_obj(&obj_path)?;
        s.execute(Command::Duplicate, w, cx);
        s.execute(Command::Delete, w, cx);
        s.execute(Command::Undo, w, cx);
        s.scene.validate()?;
        w.resize(size(px(1120.), px(760.)));
        Ok(())
    })??;
    settle(window, cx).await?;
    Timer::after(Duration::from_millis(180)).await;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, w, _| -> Result<()> {
        ensure!(
            s.settings.width >= 100 && s.settings.height >= 100,
            "viewport collapsed on resize"
        );
        ensure!(
            s.bounds.get().size.width > px(100.),
            "viewport layout has no width"
        );
        println!(
            "resized_viewport={}x{}",
            s.settings.width, s.settings.height
        );
        let _ = capture_window(s, w, &output.join("workspace-small.png"));
        s.dirty = false;
        Ok(())
    })??;
    let async_path = output.join("async-save.forma");
    let count = window.update(cx, |s, w, cx| {
        let count = s.scene.objects.len();
        s.save_to(async_path.clone(), cx);
        s.execute(Command::Add(Primitive::Plane), w, cx);
        count
    })?;
    wait_for(window, cx, |s| s.status.starts_with("Project saved")).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(s.dirty, "async save cleared edits made while writing");
        ensure!(
            Scene::load(&async_path)?.objects.len() == count,
            "save did not use an immutable snapshot"
        );
        Ok(())
    })??;
    window.update(cx, |s, _, cx| s.save_to(async_path.clone(), cx))?;
    wait_for(window, cx, |s| !s.dirty).await?;
    window.update(cx, |s, w, cx| {
        s.open_from(scene_path.clone(), false, cx);
        s.execute(Command::Add(Primitive::Cube), w, cx);
    })?;
    wait_for(window, cx, |s| s.status.starts_with("Open skipped")).await?;
    window.update(cx, |s, _, cx| s.open_from(scene_path.clone(), false, cx))?;
    wait_for(window, cx, |s| s.status == "Project opened").await?;
    let import_count = window.update(cx, |s, _, cx| {
        let count = s.scene.objects.len();
        s.open_from(obj_path.clone(), true, cx);
        count
    })?;
    wait_for(window, cx, |s| s.status.starts_with("Imported ")).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.scene.objects.len() == import_count + 1,
            "background import did not add exactly one object"
        );
        s.scene.validate()
    })??;
    let async_obj = output.join("async-export.obj");
    window.update(cx, |s, _, cx| s.export_obj_to(async_obj.clone(), cx))?;
    wait_for(window, cx, |s| s.status.starts_with("OBJ exported")).await?;
    ensure!(
        std::fs::metadata(&async_obj)?.len() > 100,
        "background OBJ export is empty"
    );
    println!("background_documents=save_snapshot,dirty_guard,stale_open,open,import,export PASS");
    let png_path = output.join("async-render.png");
    let dimensions = window.update(cx, |s, w, cx| {
        s.settings.max_samples = 8;
        s.export_image_to(png_path.clone(), cx);
        s.execute(Command::Add(Primitive::Plane), w, cx);
        (s.settings.width, s.settings.height)
    })?;
    wait_for(window, cx, |s| s.status.starts_with("Image saved")).await?;
    ensure!(
        image::image_dimensions(&png_path)? == dimensions,
        "image export dimensions did not match the snapshot"
    );
    println!("background_image_export=PASS");
    if std::env::var_os("FORMA_SMOKE_DENOISE").is_some() {
        check_denoise_workflow(window, &output, cx).await?;
    }
    window.update(cx, |s, _, _| s.dirty = false)?;
    std::fs::write(
        output.join("native-smoke.txt"),
        format!(
            "PASS: real GPUI window; deterministic material preview; studio/world/contact lighting; preview numeric input without document or history mutation; valid/invalid/stale HDR loading; preview export with a 4096-sample path target; material-preview orbit; searchable command palette; four mode keyboard shortcuts and GPU frames; continuous orbit through pointer handlers at 8ms intervals ({presentations} frames presented); constrained numeric move; undo/redo; cancel; face extrusion; subdivision; numeric exposure; object rename; sRGB material input; exact top view; project/OBJ persistence; duplicate/delete/undo; resize; background save snapshot and dirty guard; stale-open rejection; background open/import/export; concurrent PNG export. Renderer: {device}. Native file-picker and OS pointer routing are separate manual checks.\n"
        ),
    )?;
    Ok(())
}

/// AppKit may suspend display-link ticks for a background smoke window. Ask its
/// own layer to display before capture, without activating or focusing the app.
#[cfg(target_os = "macos")]
async fn prepare_capture(window: WindowHandle<Studio>, cx: &mut AsyncApp) -> Result<()> {
    use objc2::{msg_send, runtime::AnyObject};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle: AnyWindowHandle = window.into();
    let view = handle.update(cx, |_, w, _| -> Result<*mut AnyObject> {
        let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(w)
            .map_err(|e| anyhow::anyhow!("window handle: {e:?}"))?
            .as_raw()
        else {
            anyhow::bail!("macOS window required");
        };
        w.refresh();
        Ok(handle.ns_view.as_ptr().cast())
    })??;
    // SAFETY: This foreground task is on AppKit's main thread. There is no await
    // between borrowing the live NSView and using it. Crucially the GPUI window
    // borrow has ended before displayIfNeeded calls back into GPUI to draw.
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        let _: () = msg_send![layer, setNeedsDisplay];
        let _: () = msg_send![layer, displayIfNeeded];
    }
    Timer::after(Duration::from_millis(100)).await;
    Ok(())
}

/// Captures only this application's own window. Never requests screen recording access.
#[cfg(target_os = "macos")]
fn capture_window(_studio: &Studio, window: &gpui::Window, path: &std::path::Path) -> Result<()> {
    use core_graphics::{
        color_space::CGColorSpace,
        context::CGContext,
        geometry::{CGPoint, CGRect, CGSize},
        image::CGImageAlphaInfo,
        window::*,
    };
    use objc2::{msg_send, runtime::AnyObject};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window)
        .map_err(|e| anyhow::anyhow!("window handle: {e:?}"))?
        .as_raw()
    else {
        anyhow::bail!("macOS window required");
    };
    // SAFETY: GPUI lends its live NSView on the main thread for this call. The
    // borrowed NSWindow remains alive throughout capture and is never released here.
    let number: i64 = unsafe {
        let view = handle.ns_view.as_ptr() as *mut AnyObject;
        let native: *mut AnyObject = msg_send![view, window];
        msg_send![native, windowNumber]
    };
    let null_rect = CGRect::new(
        &CGPoint::new(f64::INFINITY, f64::INFINITY),
        &CGSize::new(0., 0.),
    );
    let image = create_image(
        null_rect,
        kCGWindowListOptionIncludingWindow,
        number as u32,
        kCGWindowImageBoundsIgnoreFraming | kCGWindowImageBestResolution,
    )
    .ok_or_else(|| anyhow::anyhow!("window compositor did not provide an image"))?;
    let width = image.width();
    let height = image.height();
    let mut context = CGContext::create_bitmap_context(
        None,
        width,
        height,
        8,
        width * 4,
        &CGColorSpace::create_device_rgb(),
        CGImageAlphaInfo::CGImageAlphaPremultipliedLast as u32 | (4 << 12),
    );
    context.draw_image(
        CGRect::new(
            &CGPoint::new(0., 0.),
            &CGSize::new(width as f64, height as f64),
        ),
        &image,
    );
    image::save_buffer(
        path,
        context.data(),
        width as u32,
        height as u32,
        image::ColorType::Rgba8,
    )?;
    Ok(())
}

async fn check_shader_workflow(window: WindowHandle<Studio>, cx: &mut AsyncApp) -> Result<()> {
    let (before, history, dirty) = window.update(cx, |s, w, cx| {
        let before = s.scene.clone();
        let history = std::mem::take(&mut s.history);
        let dirty = s.dirty;
        s.execute(Command::SetShader(forma_core::ShaderKind::Glass), w, cx);
        (before, history, dirty)
    })?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.selected_object().unwrap().material.shader == forma_core::ShaderKind::Glass,
            "Glass shader was not selected"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(s.scene == before, "Glass shader change was not undoable");
        s.execute(Command::EditShader, w, cx);
        Ok(())
    })??;
    let source = window.update(cx, |s, _, cx| {
        s.shader_editor.as_ref().unwrap().read(cx).text.clone()
    })?;
    Timer::after(Duration::from_millis(100)).await;
    keys(
        window,
        &[
            platform_shortcut("cmd-a", "ctrl-a"),
            "a",
            "enter",
            "b",
            "left",
            "delete",
            "tab",
            "x",
        ],
        cx,
    )
    .await?;
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(
            s.shader_editor.as_ref().unwrap().read(cx).text == "a\n    x",
            "Shader multiline input/selection failed"
        );
        ensure!(s.scene == before, "Draft shader text modified the document");
        Ok(())
    })??;
    keys(
        window,
        &[
            platform_shortcut("cmd-z", "ctrl-z"),
            platform_shortcut("cmd-z", "ctrl-z"),
        ],
        cx,
    )
    .await?;
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(
            s.shader_editor.as_ref().unwrap().read(cx).text == "a\n",
            "Shader undo affected scene history instead of text"
        );
        Ok(())
    })??;
    keys(
        window,
        &[
            platform_shortcut("cmd-shift-z", "ctrl-shift-z"),
            platform_shortcut("cmd-shift-z", "ctrl-shift-z"),
            platform_shortcut("cmd-enter", "ctrl-enter"),
        ],
        cx,
    )
    .await?;
    wait_for(window, cx, |s| !s.shader_compiling).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(s.scene == before, "Failed compilation changed the scene");
        ensure!(
            s.shader_message
                .as_ref()
                .is_some_and(|m| m.contains("Compilation failed")),
            "Missing shader compiler diagnostics"
        );
        Ok(())
    })??;
    keys(window, &[platform_shortcut("cmd-a", "ctrl-a")], cx).await?;
    window.update(cx, |s, w, cx| {
        let editor = s.shader_editor.as_ref().unwrap().clone();
        editor.update(cx, |editor, cx| {
            editor.replace_text_in_range(None, &source, w, cx)
        });
    })?;
    keys(window, &[platform_shortcut("cmd-enter", "ctrl-enter")], cx).await?;
    wait_for(window, cx, |s| !s.shader_compiling).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.selected_object().unwrap().material.shader == forma_core::ShaderKind::Custom,
            "Valid shader was not applied"
        );
        ensure!(
            s.shader_message
                .as_ref()
                .is_some_and(|m| m.starts_with("Compiled and applied")),
            "Valid shader did not compile"
        );
        Ok(())
    })??;
    keys(
        window,
        &["escape", platform_shortcut("cmd-z", "ctrl-z")],
        cx,
    )
    .await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(s.shader_editor.is_none(), "Shader editor did not close");
        ensure!(
            s.scene == before,
            "Applying shader code was not one undo transaction"
        );
        Ok(())
    })??;
    window.update(cx, |s, _, cx| {
        s.history = history;
        s.dirty = dirty;
        cx.notify();
    })?;
    settle(window, cx).await?;
    println!("shader_editor=multiline,local_undo,compiler_diagnostics,atomic_apply,scene_undo");
    Ok(())
}

/// A portable run still exercises the actual GPUI window and event handlers.
/// Frame captures are explicitly labelled because GPUI does not expose a portable
/// window screenshot API; compositor routing remains a platform manual check.
#[cfg(not(target_os = "macos"))]
async fn prepare_capture(window: WindowHandle<Studio>, cx: &mut AsyncApp) -> Result<()> {
    window.update(cx, |_, window, _| window.refresh())?;
    Timer::after(Duration::from_millis(100)).await;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn capture_window(studio: &Studio, _window: &gpui::Window, path: &std::path::Path) -> Result<()> {
    let frame = studio
        .frame
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no completed viewport frame"))?;
    let rgba = frame
        .rgba()
        .ok_or_else(|| anyhow::anyhow!("viewport frame has no portable pixels"))?;
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let path = path.with_file_name(format!("{stem}-viewport.png"));
    image::save_buffer(
        &path,
        rgba,
        frame.width(),
        frame.height(),
        image::ColorType::Rgba8,
    )?;
    println!(
        "viewport_capture={} (full-window capture is available on macOS)",
        path.display()
    );
    Ok(())
}

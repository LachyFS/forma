//! Opt-in runtime regression: exercises the real GPUI window, input dispatch and GPU worker.
use crate::app::{Command, EditMode, Field, Studio};
use crate::platform_shortcut;
use anyhow::{Result, ensure};
use forma_core::{Primitive, Scene};
use forma_render::{PreviewSettings, RenderMode, StudioLight};
use glam::Vec2;
use gpui::{
    AnyWindowHandle, App, AsyncApp, EntityInputHandler, KeyDownEvent, KeyUpEvent, Keystroke,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Timer, WindowHandle, point, px,
    size,
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
    window.update(cx, |s, w, cx| {
        // Component overlays intentionally pause viewport denoising. Earlier
        // smoke checks may leave Face mode active, so start this workflow clean.
        s.execute(Command::SetEditMode(EditMode::Object), w, cx);
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

async fn check_panel_resize(window: WindowHandle<Studio>, cx: &mut AsyncApp) -> Result<()> {
    let (scene, selected, dirty, undo, redo, initial_width) = window.update(cx, |s, _, _| {
        (
            s.scene.clone(),
            s.selected,
            s.dirty,
            s.history.can_undo(),
            s.history.can_redo(),
            s.bounds.get().size.width,
        )
    })?;
    // Exercise the same pointer handlers as the divider, including large jumps
    // beyond its bounds and a release outside the window.
    for (delta, expected) in [(-120., 412.), (140., 272.), (-2000., 560.), (2000., 240.)] {
        let start = window.update(cx, |s, _, _| {
            point(s.bounds.get().right() + px(4.), px(120.))
        })?;
        let end = point(start.x + px(delta), start.y);
        window.update(cx, |s, w, cx| {
            s.begin_sidebar_resize(
                &MouseDownEvent {
                    button: MouseButton::Left,
                    position: start,
                    click_count: 1,
                    ..Default::default()
                },
                w,
                cx,
            );
            s.move_sidebar_resize(
                &MouseMoveEvent {
                    position: end,
                    pressed_button: Some(MouseButton::Left),
                    ..Default::default()
                },
                w,
                cx,
            );
        })?;
        wait_for(window, cx, |s| {
            (f32::from(s.bounds.get().size.width - initial_width) - (292. - expected)).abs() < 1.
        })
        .await?;
        settle(window, cx).await?;
        window.update(cx, |s, w, _| -> Result<()> {
            ensure!(
                (s.panels.sidebar_width(w.viewport_size().width.into()) - expected).abs() < 1.,
                "panel drag did not produce the expected sidebar width {expected}"
            );
            ensure!(
                (f32::from(s.bounds.get().size.width - initial_width) - (292. - expected)).abs()
                    < 1.,
                "viewport layout did not follow the sidebar width: initial {}, actual {}, sidebar {expected}",
                f32::from(initial_width),
                f32::from(s.bounds.get().size.width),
            );
            Ok(())
        })??;
        window.update(cx, |s, w, cx| {
            s.end_sidebar_resize(
                &MouseUpEvent {
                    button: MouseButton::Left,
                    position: point(px(-20.), px(-20.)),
                    ..Default::default()
                },
                w,
                cx,
            );
            s.move_sidebar_resize(
                &MouseMoveEvent {
                    position: start,
                    ..Default::default()
                },
                w,
                cx,
            );
        })?;
        window.update(cx, |s, w, _| -> Result<()> {
            ensure!(
                !s.panels.finish_sidebar_resize()
                    && (s.panels.sidebar_width(w.viewport_size().width.into()) - expected).abs()
                        < 1.,
                "releasing outside the window left panel resizing active"
            );
            Ok(())
        })??;
        Timer::after(Duration::from_millis(60)).await;
    }
    let divider = window.update(cx, |s, _, _| {
        point(s.bounds.get().right() + px(4.), px(120.))
    })?;
    window.update(cx, |s, w, cx| {
        s.begin_sidebar_resize(
            &MouseDownEvent {
                button: MouseButton::Left,
                position: divider,
                click_count: 2,
                ..Default::default()
            },
            w,
            cx,
        );
        s.end_sidebar_resize(
            &MouseUpEvent {
                button: MouseButton::Left,
                position: divider,
                click_count: 2,
                ..Default::default()
            },
            w,
            cx,
        );
    })?;
    wait_for(window, cx, |s| {
        f32::from(s.bounds.get().size.width - initial_width).abs() < 1.
    })
    .await?;
    settle(window, cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            (f32::from(s.bounds.get().size.width - initial_width)).abs() < 1.,
            "double-click did not restore the original panel widths"
        );
        ensure!(
            s.scene == scene
                && s.selected == selected
                && s.dirty == dirty
                && s.history.can_undo() == undo
                && s.history.can_redo() == redo,
            "panel resizing changed the scene, selection or undo history"
        );
        Ok(())
    })??;
    println!("panel_resize=drag_limits_viewport_release_reset_document_isolation_pass");
    Ok(())
}

async fn check_export_formats(window: WindowHandle<Studio>, cx: &mut AsyncApp) -> Result<()> {
    window.update(cx, |s, w, cx| -> Result<()> {
        s.execute(Command::TogglePalette, w, cx);
        s.palette_exports_only = true;
        let items = s.palette_items();
        ensure!(
            s.palette_open
                && s.palette_query.is_empty()
                && items.len() == 3
                && [Command::ExportObj, Command::ExportStl, Command::ExportImage]
                    .iter()
                    .all(|expected| items.iter().any(|(_, _, command)| command == expected)),
            "Formats did not show exactly OBJ, STL and image export"
        );
        cx.notify();
        Ok(())
    })??;
    keys(window, &["p", "n", "g"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let items = s.palette_items();
        ensure!(
            items.len() == 1 && items[0].2 == Command::ExportImage,
            "Formats did not find PNG image export by extension"
        );
        Ok(())
    })??;
    keys(
        window,
        &["backspace", "backspace", "backspace", "s", "t", "l"],
        cx,
    )
    .await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let items = s.palette_items();
        ensure!(
            items.len() == 1 && items[0].2 == Command::ExportStl,
            "Formats search did not stay scoped to STL export"
        );
        Ok(())
    })??;
    keys(window, &["enter"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            !s.palette_open
                && s.stl_dialog.is_some_and(|dialog| {
                    dialog.direction == crate::exchange::StlDirection::Export
                }),
            "Formats keyboard selection did not open STL export options"
        );
        Ok(())
    })??;
    keys(
        window,
        &["escape", platform_shortcut("cmd-k", "ctrl-k")],
        cx,
    )
    .await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.palette_open
                && !s.palette_exports_only
                && s.palette_query.is_empty()
                && s.palette_items().len() > 3,
            "Commands did not restore the unrestricted palette after Formats"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    println!("export_formats=three_formats_scoped_search_keyboard_dispatch_normal_reset_pass");
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
    check_panel_resize(window, cx).await?;
    check_theme_workflow(window, &output, cx).await?;
    check_preview_workflow(window, &output, cx).await?;
    check_shader_workflow(window, cx).await?;
    check_export_formats(window, cx).await?;
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
    check_modelling_workflow(window, cx).await?;
    check_precision_finishing_workflow(window, cx).await?;
    check_exchange_object_workflow(window, &output, cx).await?;
    check_numeric_entry_workflow(window, cx).await?;
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
                    .is_some_and(|drag| drag.constraint.axis == Some(2)),
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
    keys(window, &["e", ".", "3", "enter"], cx).await?;
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
            "PASS: real GPUI window; deterministic material preview; studio/world/contact lighting; preview numeric input without document or history mutation; valid/invalid/stale HDR loading; preview export with a 4096-sample path target; material-preview orbit; searchable command palette; four mode keyboard shortcuts and GPU frames; continuous orbit through pointer handlers at 8ms intervals ({presentations} frames presented); constrained numeric move; undo/redo; cancel; face extrusion; subdivision; numeric exposure; object rename; sRGB material input; exact top view; project/OBJ persistence; duplicate/delete/undo; resize; background save snapshot and dirty guard; stale-open rejection; background open/import/export; concurrent PNG export.\nPrecision: contextual units and arithmetic; dimensions; bevel including mirrored winding; triangulation and normals; framing and measurements; object scale/rotation/origin transactions; STL units, selected scope, immutable export, atomic import and stale guard; exact clipboard input; Tab/Shift+Tab validation and minimal scrolling at minimum window size.\nRenderer: {device}. Native file-picker and OS pointer routing are separate manual checks.\n"
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

async fn check_precision_finishing_workflow(
    window: WindowHandle<Studio>,
    cx: &mut AsyncApp,
) -> Result<()> {
    use glam::Vec3;
    let saved = window.update(cx, |s, _, cx| {
        let saved = (
            s.scene.clone(),
            std::mem::take(&mut s.history),
            s.selected,
            s.edit_mode,
            s.component_elements(),
            s.settings.clone(),
            s.tool,
            s.dirty,
            s.objects_selected.clone(),
            s.show_measurements,
        );
        s.scene = Scene::empty();
        s.selected = Some(s.scene.add(Primitive::Cube));
        s.objects_selected.clear();
        s.clear_components();
        s.edit_mode = EditMode::Object;
        s.settings.mode = RenderMode::Solid;
        s.invalidate(true, cx);
        saved
    })?;
    let before_invalid = window.update(cx, |s, _, _| s.scene.clone())?;
    preview_field(window, Field::Dimensions(0), "0mm", cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.scene == before_invalid && !s.history.can_undo() && s.active_field.is_some(),
            "Invalid dimension changed geometry/history or discarded editable input"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    preview_field(window, Field::Dimensions(0), "250mm", cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        let object = s.selected_object().unwrap();
        ensure!(
            (object.transform.scale.x - 0.125).abs() < 1e-6,
            "Dimension in millimetres was not applied exactly"
        );
        ensure!(
            (s.selection_dimensions().unwrap().x - 0.25).abs() < 1e-6,
            "World measurement did not update after an exact dimension edit"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(
            (s.selected_object().unwrap().transform.scale.x - 1.).abs() < 1e-6,
            "Dimensions did not undo in one step"
        );
        ensure!(
            (s.selection_dimensions().unwrap().x - 2.).abs() < 1e-6,
            "World measurement retained stale dimensions after undo"
        );
        Ok(())
    })??;
    preview_field(window, Field::Translation(0), "1/8in", cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            (s.selected_object().unwrap().transform.translation.x - 0.003175).abs() < 1e-7,
            "Fractional inch expression failed in inspector"
        );
        s.execute(Command::Undo, w, cx);
        Ok(())
    })??;
    keys(
        window,
        &["g", "x", "cmd-c", "cmd-z", "2", "5", "m", "m", "enter"],
        cx,
    )
    .await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            (s.selected_object().unwrap().transform.translation.x - 0.025).abs() < 1e-6,
            "Modal transform did not accept distance units"
        );
        s.execute(Command::Undo, w, cx);
        Ok(())
    })??;
    keys(window, &["r", "z", "1", "r", "a", "d", "enter"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            (s.selected_object().unwrap().transform.rotation.z - 1.).abs() < 1e-5,
            "Modal rotation did not accept radians"
        );
        s.execute(Command::Undo, w, cx);
        Ok(())
    })??;
    let original = window.update(cx, |s, _, _| s.scene.clone())?;
    keys(window, &["cmd-b", "5", "0", "m", "m", "enter"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(s.transform_drag.is_none(), "Valid bevel remained modal");
        ensure!(
            s.selected_object().unwrap().mesh.faces.len() == 26,
            "Cube bevel topology is incorrect"
        );
        s.scene.validate()?;
        s.execute(Command::Undo, w, cx);
        ensure!(s.scene == original, "Bevel did not undo as one transaction");
        s.execute(Command::Redo, w, cx);
        ensure!(
            s.selected_object().unwrap().mesh.faces.len() == 26,
            "Bevel redo failed"
        );
        s.execute(Command::Undo, w, cx);
        Ok(())
    })??;
    window.update(cx, |s, _, cx| {
        s.scene
            .object_mut(s.selected.unwrap())
            .unwrap()
            .transform
            .scale
            .x = -1.;
        s.invalidate(true, cx);
    })?;
    keys(window, &["cmd-b", "5", "0", "m", "m", "enter"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        let mesh = s.scene.object_mesh(s.selected.unwrap()).unwrap();
        ensure!(
            mesh.faces.len() == 26
                && mesh.faces.iter().enumerate().all(|(index, face)| {
                    let center = face
                        .iter()
                        .map(|&vertex| mesh.positions[vertex as usize])
                        .sum::<Vec3>()
                        / face.len() as f32;
                    mesh.face_normal(index).dot(center) > 0.
                }),
            "Mirrored bevel inverted local winding and rendered surface normals"
        );
        s.execute(Command::Undo, w, cx);
        s.scene
            .object_mut(s.selected.unwrap())
            .unwrap()
            .transform
            .scale
            .x = 1.;
        s.invalidate(true, cx);
        ensure!(
            s.scene == original,
            "Mirrored bevel undo lost the source geometry"
        );
        Ok(())
    })??;
    keys(window, &["cmd-b", "9", "9", "m", "enter"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(s.transform_drag.is_some(), "Oversized bevel was committed");
        ensure!(s.scene == original, "Rejected bevel changed the scene");
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.scene == original,
            "Bevel cancellation failed to restore original"
        );
        s.execute(Command::SetEditMode(EditMode::Face), w, cx);
        s.set_components([forma_core::MeshElement::Face(0)].into_iter().collect());
        s.execute(Command::Triangulate, w, cx);
        ensure!(
            s.selected_object().unwrap().mesh.faces.len() == 7 && s.component_elements().len() == 2,
            "Triangulation failed to preserve selected replacement faces"
        );
        s.execute(Command::Undo, w, cx);
        s.set_components([forma_core::MeshElement::Face(0)].into_iter().collect());
        let before = s.selected_object().unwrap().mesh.face_normal(0);
        s.execute(Command::FlipNormals, w, cx);
        ensure!(
            s.selected_object().unwrap().mesh.face_normal(0).dot(before) < -0.99,
            "Flip normals did not reverse selected face"
        );
        s.execute(Command::RecalculateNormals, w, cx);
        ensure!(
            s.selected_object().unwrap().mesh.face_normal(0).dot(before) > 0.99,
            "Normals repair did not orient cube outward"
        );
        s.execute(Command::SetEditMode(EditMode::Object), w, cx);
        s.execute(Command::Triangulate, w, cx);
        ensure!(
            s.component_elements().is_empty(),
            "Object triangulation retained stale component selections"
        );
        s.execute(Command::Undo, w, cx);
        let hidden = s.scene.add(Primitive::Cube);
        s.scene.object_mut(hidden).unwrap().transform.translation = Vec3::splat(10000.);
        s.scene.object_mut(hidden).unwrap().visible = false;
        s.execute(Command::FrameAll, w, cx);
        ensure!(
            s.scene.camera.target.length() < 1e-4 && s.scene.camera.distance < 20.,
            "Frame all included hidden geometry"
        );
        Ok(())
    })??;
    keys(window, &["alt-1"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            (s.scene.camera.yaw - std::f32::consts::PI).abs() < 1e-5,
            "Back view shortcut failed"
        );
        ensure!(
            (s.selection_dimensions().unwrap() - Vec3::splat(2.)).length() < 1e-5,
            "Camera changes altered world dimensions"
        );
        Ok(())
    })??;
    keys(window, &["alt-7"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            (s.scene.camera.pitch + std::f32::consts::FRAC_PI_2).abs() < 1e-5,
            "Bottom view shortcut failed"
        );
        let scene = s.scene.clone();
        let samples = s.samples;
        let dirty = s.dirty;
        let frame = s.frame_image.clone();
        let enabled = s.show_measurements;
        s.execute(Command::ToggleMeasurements, w, cx);
        ensure!(
            s.show_measurements != enabled
                && s.scene == scene
                && s.samples == samples
                && s.dirty == dirty,
            "Measurement overlay mutated the document or reset rendering"
        );
        ensure!(
            match (&frame, &s.frame_image) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            },
            "Measurements replaced the rendered frame"
        );
        Ok(())
    })??;
    // Let the real GPUI overlay paint once before restoring the prior document.
    Timer::after(Duration::from_millis(120)).await;
    window.update(cx, |s, _, cx| {
        s.scene = saved.0;
        s.history = saved.1;
        s.selected = saved.2;
        s.edit_mode = saved.3;
        s.set_components(saved.4);
        s.settings = saved.5;
        s.tool = saved.6;
        s.dirty = saved.7;
        s.objects_selected = saved.8;
        s.show_measurements = saved.9;
        s.active_field = None;
        s.invalidate(true, cx);
    })?;
    println!(
        "precision_finishing=units,dimensions,bevel,undo,redo,cancel,triangulate,normals,views,measurements PASS"
    );
    Ok(())
}

async fn check_numeric_entry_workflow(
    window: WindowHandle<Studio>,
    cx: &mut AsyncApp,
) -> Result<()> {
    let saved = window.update(cx, |s, w, cx| {
        let saved = (
            s.scene.clone(),
            std::mem::take(&mut s.history),
            s.selected,
            s.edit_mode,
            s.component_elements(),
            s.settings.clone(),
            s.tool,
            s.dirty,
            s.objects_selected.clone(),
            cx.read_from_clipboard(),
        );
        s.scene = Scene::empty();
        s.selected = Some(s.scene.add(Primitive::Cube));
        s.objects_selected.clear();
        s.clear_components();
        s.edit_mode = EditMode::Object;
        s.settings.mode = RenderMode::Solid;
        s.active_field = None;
        s.invalidate(true, cx);
        s.begin_field(Field::Translation(0), w, cx);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("1/8in".into()));
        saved
    })?;
    keys(window, &["cmd-v", "cmd-c", "cmd-a", "tab"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref()
                == Some("1/8in"),
            "Copy did not preserve the exact inspector expression"
        );
        ensure!(
            s.field_is_active(Field::Translation(1))
                && (s.selected_object().unwrap().transform.translation.x - 0.003175).abs() < 1e-7,
            "Tab failed to commit a changed expression after Select All"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(
            s.selected_object().unwrap().transform.translation.x == 0. && !s.history.can_undo(),
            "Inspector Tab did not create exactly one undo step"
        );
        s.begin_field(Field::Rotation(0), w, cx);
        Ok(())
    })??;
    keys(window, &["cmd-a", "tab", "shift-tab"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.field_is_active(Field::Rotation(0)) && !s.history.can_undo(),
            "Untouched Tab/Shift+Tab created history or navigated incorrectly"
        );
        s.begin_field(Field::Scale(0), w, cx);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("0".into()));
        Ok(())
    })??;
    keys(window, &["cmd-v", "tab", "shift-tab"], cx).await?;
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(
            s.field_is_active(Field::Scale(0))
                && s.field_value(Field::Scale(0)) == "0"
                && s.selected_object().unwrap().transform.scale.x == 1.
                && !s.history.can_undo(),
            "Invalid Tab lost editable input, advanced focus or changed history"
        );
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("2".into()));
        Ok(())
    })??;
    keys(window, &["cmd-a", "cmd-v", "shift-tab"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.field_is_active(Field::Rotation(2))
                && s.selected_object().unwrap().transform.scale.x == 2.,
            "Shift+Tab did not commit corrected scale and focus the preceding field"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(
            !s.history.can_undo(),
            "Corrected inspector input committed more than once"
        );
        s.begin_field(Field::Name, w, cx);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("x".repeat(129)));
        Ok(())
    })??;
    keys(window, &["cmd-v"], cx).await?;
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(
            s.field_value(Field::Name) == "Cube" && s.field_selection_active(Field::Name),
            "Oversized replacement paste cleared the original text or selection"
        );
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("é".repeat(64)));
        Ok(())
    })??;
    keys(window, &["cmd-v"], cx).await?;
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(
            s.field_value(Field::Name) == "é".repeat(64),
            "128-byte Unicode paste was not retained exactly"
        );
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("a".into()));
        Ok(())
    })??;
    keys(window, &["cmd-v", "cmd-c"], cx).await?;
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(
            s.field_value(Field::Name) == "é".repeat(64)
                && cx.read_from_clipboard().and_then(|item| item.text()) == Some("é".repeat(64)),
            "Oversized append truncated UTF-8 text or copy changed it"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    window.update(cx, |s, w, cx| {
        s.begin_field(Field::Translation(0), w, cx);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("1m;\n2m".into()));
    })?;
    keys(window, &["cmd-v"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.field_value(Field::Translation(0)) == "0"
                && s.field_selection_active(Field::Translation(0)),
            "Partially invalid paste mutated the selected expression"
        );
        Ok(())
    })??;
    keys(window, &["2", "cmd-a", "delete"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.field_value(Field::Translation(0)).is_empty()
                && !s.field_selection_active(Field::Translation(0)),
            "Delete did not remove and clear the whole-text selection"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    window.update(cx, |s, w, cx| {
        let id = s.selected.unwrap();
        s.scene.object_material_mut(id).unwrap().roughness = 1.;
        s.invalidate(true, cx);
        s.begin_field(Field::Roughness, w, cx);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("2".into()));
    })?;
    keys(window, &["cmd-v", "tab"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.field_is_active(Field::Metallic)
                && !s.history.can_undo()
                && s.selected_object().unwrap().material.roughness == 1.,
            "A clamped no-op edit created an undo step or prevented Tab navigation"
        );
        Ok(())
    })??;
    keys(window, &["escape", "g", "x"], cx).await?;
    window.update(cx, |_, _, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("25mm".into()))
    })?;
    keys(window, &["cmd-v", "cmd-c"], cx).await?;
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(
            (s.selected_object().unwrap().transform.translation.x - 0.025).abs() < 1e-7
                && cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref()
                    == Some("25mm"),
            "Modal paste/copy did not preserve and preview the exact expression"
        );
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("+5mm".into()));
        Ok(())
    })??;
    keys(window, &["cmd-v"], cx).await?;
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(
            s.transform_drag.as_ref().unwrap().numeric == "25mm+5mm"
                && (s.selected_object().unwrap().transform.translation.x - 0.03).abs() < 1e-7,
            "Modal paste without selection failed to append"
        );
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("1/8in".into()));
        Ok(())
    })??;
    keys(window, &["cmd-a", "cmd-v", "enter"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.transform_drag.is_none()
                && (s.selected_object().unwrap().transform.translation.x - 0.003175).abs() < 1e-7,
            "Modal Select All/paste did not replace and commit the expression"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(
            !s.history.can_undo(),
            "Modal text editing created more than one undo checkpoint"
        );
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("1".repeat(129)));
        Ok(())
    })??;
    keys(window, &["g", "x", "2", "cmd-a", "cmd-v"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let drag = s.transform_drag.as_ref().unwrap();
        ensure!(
            drag.numeric == "2" && drag.numeric_replace,
            "Oversized modal paste lost the previous expression or selection"
        );
        Ok(())
    })??;
    keys(window, &["delete"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        let drag = s.transform_drag.as_ref().unwrap();
        ensure!(
            drag.numeric.is_empty() && !drag.numeric_replace,
            "Modal Delete did not clear text and selection together"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    let original_size = window.update(cx, |s, w, cx| {
        let original = w.viewport_size();
        w.resize(size(px(1000.), px(650.)));
        s.begin_field(Field::Name, w, cx);
        original
    })?;
    keys(window, &["tab"], cx).await?;
    wait_for(window, cx, |s| {
        s.field_is_active(Field::Translation(0))
            && s.panels.active_field_is_visible(Field::Translation(0))
    })
    .await?;
    let visible_offset = window.update(cx, |s, _, _| s.panels.inspector_scroll_offset())?;
    keys(window, &["tab"], cx).await?;
    wait_for(window, cx, |s| {
        s.panels.active_field_is_visible(Field::Translation(1))
    })
    .await?;
    Timer::after(Duration::from_millis(80)).await;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.panels.inspector_scroll_offset() == visible_offset,
            "Tab unnecessarily scrolled an already-visible inspector field"
        );
        s.begin_field(Field::Color(2), w, cx);
        Ok(())
    })??;
    keys(window, &["tab"], cx).await?;
    wait_for(window, cx, |s| {
        s.field_is_active(Field::Roughness) && s.panels.active_field_is_visible(Field::Roughness)
    })
    .await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.panels.inspector_scroll_offset().y < visible_offset.y,
            "Tab did not scroll down to a previously hidden surface field"
        );
        s.begin_field(Field::Name, w, cx);
        Ok(())
    })??;
    keys(window, &["shift-tab"], cx).await?;
    wait_for(window, cx, |s| {
        s.field_is_active(Field::Dimensions(2))
            && s.panels.active_field_is_visible(Field::Dimensions(2))
    })
    .await?;
    let settled_offset = window.update(cx, |s, w, cx| {
        s.begin_field(Field::Name, w, cx);
        s.panels.inspector_scroll_offset()
    })?;
    let handle: AnyWindowHandle = window.into();
    handle.update(cx, |_, w, cx| -> Result<()> {
        // Both focus changes occur before layout. The older pending reveal must
        // not move the inspector after focus has returned to the fixed name row.
        for key in ["shift-tab", "tab"] {
            w.dispatch_keystroke(Keystroke::parse(key)?, cx);
        }
        Ok(())
    })??;
    Timer::after(Duration::from_millis(120)).await;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.field_is_active(Field::Name) && s.panels.inspector_scroll_offset() == settled_offset,
            "A stale Tab callback scrolled after focus moved elsewhere"
        );
        ensure!(
            !s.history.can_undo(),
            "Field navigation created an undo checkpoint"
        );
        Ok(())
    })??;
    keys(window, &["escape"], cx).await?;
    window.update(cx, |s, w, cx| {
        w.resize(original_size);
        s.scene = saved.0;
        s.history = saved.1;
        s.selected = saved.2;
        s.edit_mode = saved.3;
        s.set_components(saved.4);
        s.settings = saved.5;
        s.tool = saved.6;
        s.dirty = saved.7;
        s.objects_selected = saved.8;
        cx.write_to_clipboard(
            saved
                .9
                .unwrap_or_else(|| gpui::ClipboardItem::new_string(String::new())),
        );
        s.invalidate(true, cx);
    })?;
    println!(
        "numeric_entry=clipboard,select_all,tab,shift_tab,invalid,noop,limits,unicode,modal_paste,minimal_scroll,stale_focus PASS"
    );
    Ok(())
}

async fn check_exchange_object_workflow(
    window: WindowHandle<Studio>,
    output: &std::path::Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    use crate::exchange::{StlDirection, StlUnit};
    use glam::Vec3;
    let saved = window.update(cx, |s, _, cx| {
        let saved = (
            s.scene.clone(),
            std::mem::take(&mut s.history),
            s.selected,
            s.edit_mode,
            s.component_elements(),
            s.settings.clone(),
            s.tool,
            s.dirty,
            s.objects_selected.clone(),
            s.stl_unit,
        );
        s.scene = Scene::empty();
        let id = s.scene.add(Primitive::Cube);
        s.selected = Some(id);
        s.objects_selected.clear();
        s.clear_components();
        s.active_field = None;
        s.edit_mode = EditMode::Object;
        s.settings.mode = RenderMode::Solid;
        let object = s.scene.object_mut(id).unwrap();
        object.transform.translation = Vec3::new(1., 2., 3.);
        object.transform.rotation = Vec3::new(0.2, 0.4, -0.1);
        object.transform.scale = Vec3::new(-2., 3., 0.5);
        for point in &mut s.scene.object_mesh_mut(id).unwrap().positions {
            *point += Vec3::new(3., -2., 1.);
        }
        s.invalidate(true, cx);
        saved
    })?;
    window.update(cx, |s, w, cx| -> Result<()> {
        let original = s.scene.clone();
        let id = s.selected.unwrap();
        let original_dimensions = s.selection_dimensions().unwrap();
        let world_positions = |scene: &Scene| {
            let transform = scene.world_transform(id).unwrap();
            scene
                .object_mesh(id)
                .unwrap()
                .positions
                .iter()
                .map(|&point| transform.transform_point3(point))
                .collect::<Vec<_>>()
        };
        let expected = world_positions(&original);
        for command in [
            Command::ApplyScale,
            Command::ApplyRotationScale,
            Command::CenterOrigin,
        ] {
            s.execute(command, w, cx);
            ensure!(
                s.scene != original && s.history.can_undo(),
                "Object utility was not a document transaction"
            );
            ensure!(
                world_positions(&s.scene)
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| a.distance(*b) < 1e-4),
                "Object utility moved world geometry"
            );
            ensure!(
                s.selection_dimensions()
                    .unwrap()
                    .distance(original_dimensions)
                    < 1e-4,
                "Object utility left stale measurements"
            );
            if command != Command::CenterOrigin {
                ensure!(
                    s.selected_object().unwrap().transform.scale == Vec3::ONE,
                    "Apply scale kept local scale"
                );
            }
            if command == Command::ApplyRotationScale {
                ensure!(
                    s.selected_object().unwrap().transform.rotation == Vec3::ZERO,
                    "Apply rotation kept local rotation"
                );
                let applied = s.scene.clone();
                s.execute(Command::ApplyScale, w, cx);
                ensure!(s.scene == applied, "No-op apply scale changed the document");
            }
            let edited = s.scene.clone();
            s.execute(Command::Undo, w, cx);
            ensure!(
                s.scene == original,
                "Object utility did not undo in one step"
            );
            s.execute(Command::Redo, w, cx);
            ensure!(s.scene == edited, "Object utility redo changed its result");
            s.execute(Command::Undo, w, cx);
        }
        s.execute(Command::SetEditMode(EditMode::Face), w, cx);
        s.execute(Command::ApplyScale, w, cx);
        ensure!(
            s.scene == original,
            "Object utility bypassed component-mode guard"
        );
        s.execute(Command::SetEditMode(EditMode::Object), w, cx);
        s.scene = Scene::empty();
        let id = s.scene.add(Primitive::Cube);
        s.scene.object_mut(id).unwrap().transform.scale = Vec3::new(0.06, 0.04, 0.01);
        let other = s.scene.add(Primitive::Cube);
        s.scene.object_mut(other).unwrap().transform.translation.x = 10.;
        s.selected = Some(id);
        s.history = forma_core::History::default();
        s.dirty = false;
        s.invalidate(true, cx);
        s.execute(Command::ExportStl, w, cx);
        ensure!(
            s.stl_dialog.is_some_and(
                |dialog| dialog.direction == StlDirection::Export && dialog.selected_only
            ),
            "STL export did not default to selection"
        );
        Ok(())
    })??;
    let modal_scene = window.update(cx, |s, _, _| s.scene.clone())?;
    keys(window, &["g", "cmd-z", "right", "tab"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        let options = s.stl_dialog.unwrap();
        let selected = s.selected_ids();
        let last_mouse = s.last_mouse;
        let pointer = s.bounds.get().origin + point(px(40.), px(40.));
        for button in [MouseButton::Left, MouseButton::Middle] {
            s.mouse_down(
                &MouseDownEvent {
                    button,
                    position: pointer,
                    ..Default::default()
                },
                w,
                cx,
            );
        }
        s.mouse_move(
            &MouseMoveEvent {
                position: pointer + point(px(40.), px(20.)),
                pressed_button: Some(MouseButton::Middle),
                ..Default::default()
            },
            w,
            cx,
        );
        ensure!(
            s.selected_ids() == selected && s.navigation.is_none() && s.last_mouse == last_mouse,
            "STL options leaked pointer input to selection or viewport navigation"
        );
        ensure!(
            s.scene == modal_scene && s.transform_drag.is_none(),
            "STL options leaked shortcuts to editing"
        );
        ensure!(
            options.unit == StlUnit::Centimetres && !options.selected_only,
            "STL options keyboard navigation failed"
        );
        Ok(())
    })??;
    settle(window, cx).await?;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, w, _| {
        capture_window(s, w, &output.join("stl-exchange.png"))
    })??;
    keys(window, &["escape"], cx).await?;
    let exported = output.join("precision-part-mm.stl");
    window.update(cx, |s, _, cx| {
        s.export_stl_to(
            exported.clone(),
            StlUnit::Millimetres,
            Some(s.selected_ids()),
            cx,
        );
        // A real document edit must not strand the snapshot export's completion.
        s.scene
            .object_mut(s.selected.unwrap())
            .unwrap()
            .transform
            .translation
            .x = 2.;
        s.dirty = true;
        s.invalidate(true, cx);
    })?;
    wait_for(window, cx, |s| s.status.starts_with("STL exported")).await?;
    let mut round_trip = Scene::empty();
    let imported = round_trip.import_stl(&exported, StlUnit::Millimetres.metres())?;
    let positions = &round_trip.object_mesh(imported).unwrap().positions;
    let low = positions
        .iter()
        .copied()
        .fold(Vec3::splat(f32::INFINITY), Vec3::min);
    let high = positions
        .iter()
        .copied()
        .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
    ensure!(
        (high - low).distance(Vec3::new(0.12, 0.08, 0.02)) < 1e-6 && (high + low).length() < 1e-6,
        "STL export lost units, selected scope, or the original snapshot"
    );
    let before_import = window.update(cx, |s, _, _| s.scene.clone())?;
    window.update(cx, |s, _, cx| {
        s.open_stl_from(exported.clone(), StlUnit::Millimetres, cx)
    })?;
    wait_for(window, cx, |s| s.status.starts_with("Imported")).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.scene.objects.len() == before_import.objects.len() + 1 && s.history.can_undo(),
            "STL import did not add one undoable mesh"
        );
        ensure!(
            s.selection_dimensions()
                .unwrap()
                .distance(Vec3::new(0.12, 0.08, 0.02))
                < 1e-6,
            "STL import lost millimetre dimensions"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(
            s.scene == before_import,
            "STL import did not undo atomically"
        );
        Ok(())
    })??;
    let bad_path = output.join("invalid.stl");
    std::fs::write(
        &bad_path,
        b"solid broken\nfacet normal 0 0 1\nendsolid broken",
    )?;
    let invalid_before = window.update(cx, |s, _, _| (s.scene.clone(), s.history.can_redo()))?;
    window.update(cx, |s, _, cx| {
        s.open_stl_from(bad_path, StlUnit::Millimetres, cx)
    })?;
    wait_for(window, cx, |s| s.status.starts_with("Import failed:")).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.scene == invalid_before.0 && s.history.can_redo() == invalid_before.1,
            "Failed STL import changed geometry or history"
        );
        Ok(())
    })??;
    window.update(cx, |s, _, cx| {
        s.open_stl_from(exported, StlUnit::Millimetres, cx);
        s.scene
            .object_mut(s.selected.unwrap())
            .unwrap()
            .transform
            .translation
            .y += 1.;
        s.invalidate(true, cx);
    })?;
    let changed = window.update(cx, |s, _, _| s.scene.clone())?;
    wait_for(window, cx, |s| s.status.starts_with("Import skipped")).await?;
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(s.scene == changed, "Stale STL import replaced a newer edit");
        s.scene = saved.0;
        s.history = saved.1;
        s.selected = saved.2;
        s.edit_mode = saved.3;
        s.set_components(saved.4);
        s.settings = saved.5;
        s.tool = saved.6;
        s.dirty = saved.7;
        s.objects_selected = saved.8;
        s.stl_unit = saved.9;
        s.stl_dialog = None;
        s.active_field = None;
        s.invalidate(true, cx);
        Ok(())
    })??;
    println!(
        "exchange_object=apply_scale,apply_rotation,origin,undo,redo,stl_units,scope,snapshot,import,rollback,stale_guard PASS"
    );
    Ok(())
}

async fn check_modelling_workflow(window: WindowHandle<Studio>, cx: &mut AsyncApp) -> Result<()> {
    use crate::app::Tool;
    use glam::Vec3;
    let saved = window.update(cx, |s, w, cx| -> Result<_> {
        let saved = (
            s.scene.clone(),
            std::mem::take(&mut s.history),
            s.selected,
            s.edit_mode,
            s.component_elements(),
            s.settings.clone(),
            s.tool,
            s.dirty,
            s.objects_selected.clone(),
        );
        s.scene = Scene::empty();
        s.objects_selected.clear();
        s.selected = Some(s.scene.add(Primitive::Cube));
        s.clear_components();
        s.scene.camera.target = Vec3::ZERO;
        s.scene.camera.yaw = 0.;
        s.scene.camera.pitch = 0.;
        s.scene.camera.distance = 6.;
        s.scene.camera.orthographic = true;
        s.settings.mode = RenderMode::Solid;
        s.tool = Tool::Select;
        s.invalidate(true, cx);
        s.execute(Command::SetEditMode(EditMode::Vertex), w, cx);
        let ray = forma_core::Ray {
            origin: Vec3::Z * 3.,
            direction: Vec3::NEG_Z,
        };
        ensure!(
            s.visibility_query().occludes(ray, 2.5),
            "Initial visibility query missed the cube"
        );
        let camera = s.scene.camera;
        s.scene.camera.orbit(Vec2::new(1., 0.));
        s.invalidate(false, cx);
        ensure!(
            s.surface_query.get().is_some(),
            "Camera navigation discarded unchanged visibility geometry"
        );
        s.scene.camera = camera;
        let cube = s.selected.unwrap();
        s.execute(Command::ToggleVisible(cube), w, cx);
        ensure!(
            s.surface_query.get().is_none() && !s.visibility_query().occludes(ray, 2.5),
            "Hiding an object kept stale visibility geometry"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(
            s.surface_query.get().is_none() && s.visibility_query().occludes(ray, 2.5),
            "Undo failed to rebuild visibility geometry"
        );
        s.execute(Command::Redo, w, cx);
        ensure!(
            s.surface_query.get().is_none() && !s.visibility_query().occludes(ray, 2.5),
            "Redo failed to rebuild visibility geometry"
        );
        s.execute(Command::Undo, w, cx);
        s.history = forma_core::History::default();
        Ok(saved)
    })??;
    println!("visibility_query_cache=camera_reuse,hide,undo,redo PASS");
    let original = window.update(cx, |s, _, _| s.scene.clone())?;
    let id = original.objects[0].id;
    window.update(cx, |s, w, cx| -> Result<()> {
        let bounds = s.bounds.get();
        let matrix = s
            .scene
            .camera
            .projection_matrix(f32::from(bounds.size.width) / f32::from(bounds.size.height))
            * s.scene.camera.view_matrix();
        let projected = |p: Vec3| {
            let p = matrix * p.extend(1.);
            let p = p.truncate() / p.w;
            bounds.origin
                + point(
                    bounds.size.width * (p.x * 0.5 + 0.5),
                    bounds.size.height * (0.5 - p.y * 0.5),
                )
        };
        for (p, shift) in [(Vec3::new(-1., 1., 1.), false), (Vec3::ONE, true)] {
            s.mouse_down(
                &MouseDownEvent {
                    button: MouseButton::Left,
                    position: projected(p),
                    modifiers: gpui::Modifiers {
                        shift,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                w,
                cx,
            );
        }
        ensure!(
            s.component_vertices().len() == 2,
            "Shift-click did not extend selection"
        );
        s.mouse_down(
            &MouseDownEvent {
                button: MouseButton::Left,
                position: projected(Vec3::ONE),
                modifiers: gpui::Modifiers {
                    shift: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            w,
            cx,
        );
        ensure!(
            s.component_vertices().len() == 1,
            "Shift-click did not toggle selected vertex"
        );
        Ok(())
    })??;
    keys(window, &["g", "x", "1", "/", "4", "enter"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        let mesh = s.scene.object_mesh(id).unwrap();
        ensure!(
            mesh.positions
                .iter()
                .any(|p| p.distance(Vec3::new(-0.75, 1., 1.)) < 1e-5),
            "arithmetic transform failed"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(s.scene == original, "component move undo failed");
        Ok(())
    })??;
    keys(window, &["a"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(s.component_vertices().len() == 8, "Select all failed");
        Ok(())
    })??;
    keys(window, &["alt-a", "b"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(s.component_vertices().is_empty(), "Deselect all failed");
        let bounds = s.bounds.get();
        s.mouse_down(
            &MouseDownEvent {
                button: MouseButton::Left,
                position: bounds.origin + point(px(1.), px(1.)),
                ..Default::default()
            },
            w,
            cx,
        );
        s.mouse_move(
            &MouseMoveEvent {
                position: bounds.origin
                    + point(bounds.size.width - px(1.), bounds.size.height - px(1.)),
                pressed_button: Some(MouseButton::Left),
                ..Default::default()
            },
            w,
            cx,
        );
        s.finish_box_select(cx);
        ensure!(
            s.component_vertices().len() == 4,
            "Box selection included hidden vertices or missed front vertices"
        );
        Ok(())
    })??;
    keys(window, &["2"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.component_elements().len() == 4,
            "Vertex to edge conversion lost selection"
        );
        Ok(())
    })??;
    keys(window, &["3", "e", ".", "2", "escape"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.scene == original,
            "Cancelled extrusion left topology behind"
        );
        ensure!(
            s.component_elements().len() == 1,
            "Cancelled extrusion lost selection"
        );
        Ok(())
    })??;
    keys(window, &["e", ".", "2", "enter"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.scene.object_mesh(id).unwrap().faces.len() == 10,
            "Interactive extrusion failed"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(s.scene == original, "Extrusion was not one undo step");
        s.set_components([forma_core::MeshElement::Face(0)].into_iter().collect());
        Ok(())
    })??;
    keys(window, &["i", ".", "2", "enter"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.scene.object_mesh(id).unwrap().faces.len() == 10,
            "Interactive inset failed"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(s.scene == original, "Inset was not one undo step");
        s.set_components([forma_core::MeshElement::Face(0)].into_iter().collect());
        Ok(())
    })??;
    keys(window, &["i", "9", "enter"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(s.transform_drag.is_some(), "Invalid inset was committed");
        Ok(())
    })??;
    keys(window, &["escape", "shift-d", "x", ".", "5", "enter"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.scene.object_mesh(id).unwrap().faces.len() == 7,
            "Component duplicate failed"
        );
        s.execute(Command::Undo, w, cx);
        ensure!(
            s.scene == original,
            "Duplicate and move were not one undo step"
        );
        s.execute(Command::SetEditMode(EditMode::Object), w, cx);
        s.scene.object_mut(id).unwrap().transform.rotation.z = std::f32::consts::FRAC_PI_2;
        s.invalidate(true, cx);
        Ok(())
    })??;
    keys(window, &["g", "x", "x", "1", "enter"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.scene
                .object(id)
                .unwrap()
                .transform
                .translation
                .distance(Vec3::Y)
                < 1e-5,
            "Local axis translation failed"
        );
        s.execute(Command::Undo, w, cx);
        Ok(())
    })??;
    keys(window, &["g", "shift-z", "1", "enter"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.scene
                .object(id)
                .unwrap()
                .transform
                .translation
                .distance(Vec3::new(1., 1., 0.))
                < 1e-5,
            "Plane constraint failed"
        );
        s.execute(Command::Undo, w, cx);
        Ok(())
    })??;
    keys(window, &["g", "x", "1", "/", "enter"], cx).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            s.transform_drag.is_some(),
            "Incomplete numeric input was committed"
        );
        Ok(())
    })??;
    keys(window, &["escape", "shift-d", "escape"], cx).await?;
    let child = window.update(cx, |s, _, cx| -> Result<_> {
        ensure!(
            s.scene.objects.len() == 1 && s.selected == Some(id),
            "Cancelled duplicate lost original selection"
        );
        s.scene.object_mut(id).unwrap().transform = Default::default();
        let child = s.scene.add(Primitive::Cube);
        s.scene.object_mut(child).unwrap().parent = Some(id);
        s.scene.object_mut(child).unwrap().transform.translation.x = 3.;
        s.select_objects([id, child].into_iter().collect(), Some(id));
        s.invalidate(true, cx);
        Ok(child)
    })??;
    keys(window, &["g", "z", "1", "enter"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        ensure!(
            s.scene
                .world_transform(id)
                .unwrap()
                .transform_point3(Vec3::ZERO)
                .distance(Vec3::Z)
                < 1e-5,
            "Group move missed parent"
        );
        ensure!(
            s.scene
                .world_transform(child)
                .unwrap()
                .transform_point3(Vec3::ZERO)
                .distance(Vec3::new(3., 0., 1.))
                < 1e-5,
            "Group move transformed child twice"
        );
        s.execute(Command::Undo, w, cx);
        s.select_objects([id, child].into_iter().collect(), Some(id));
        Ok(())
    })??;
    keys(window, &["shift-d", "x", "2", "enter"], cx).await?;
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(
            s.scene.objects.len() == 4 && s.selected_ids().len() == 2,
            "Group duplicate lost selection"
        );
        let positions: Vec<_> = s
            .selected_ids()
            .iter()
            .map(|id| {
                s.scene
                    .world_transform(*id)
                    .unwrap()
                    .transform_point3(Vec3::ZERO)
            })
            .collect();
        ensure!(
            positions.iter().any(|p| p.distance(Vec3::X * 2.) < 1e-5)
                && positions.iter().any(|p| p.distance(Vec3::X * 5.) < 1e-5),
            "Duplicating parent and child changed their relative positions"
        );
        (
            s.scene,
            s.history,
            s.selected,
            s.edit_mode,
            _,
            s.settings,
            s.tool,
            s.dirty,
        ) = (
            saved.0,
            saved.1,
            saved.2,
            saved.3,
            (),
            saved.5,
            saved.6,
            saved.7,
        );
        s.set_components(saved.4);
        s.objects_selected = saved.8;
        s.invalidate(true, cx);
        Ok(())
    })??;
    println!(
        "modelling_workflow=multi_select,box,mode_conversion,arithmetic,local_axes,plane_constraints,extrude,inset,duplicate,cancel,undo PASS"
    );
    Ok(())
}

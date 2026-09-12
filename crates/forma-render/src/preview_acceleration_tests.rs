use super::*;
use forma_core::Primitive;

fn renderer_pair() -> Option<(Renderer, Renderer)> {
    let hardware = Renderer::with_preview_acceleration(true).unwrap();
    if !hardware.accelerated_preview {
        eprintln!("Skipping acceleration parity: this Metal device has no ray tracing support");
        return None;
    }
    let software = Renderer::with_preview_acceleration(false).unwrap();
    assert!(!software.accelerated_preview);
    Some((hardware, software))
}

fn settings() -> RenderSettings {
    RenderSettings {
        width: 128,
        height: 96,
        mode: RenderMode::MaterialPreview,
        show_grid: false,
        ..Default::default()
    }
}

/// Read both display bytes and the linear film: a matching tonemapped image
/// alone could conceal non-finite values or an incorrect accumulation weight.
fn pixels(renderer: &Renderer) -> Vec<[u8; 4]> {
    objc::rc::autoreleasepool(|| {
        let film = renderer.film.as_ref().unwrap();
        let count = (film.width * film.height) as usize;
        let stride = (film.width as usize * 4).div_ceil(256) * 256;
        let linear_bytes = count * 16;
        let linear = renderer
            .device
            .new_buffer(linear_bytes as u64, MTLResourceOptions::StorageModeShared);
        let display = renderer.device.new_buffer(
            (stride * film.height as usize) as u64,
            MTLResourceOptions::StorageModeShared,
        );
        let command = renderer.queue.new_command_buffer();
        let encoder = command.new_blit_command_encoder();
        encoder.copy_from_buffer(&film.accumulation, 0, &linear, 0, linear_bytes as u64);
        encoder.copy_from_texture_to_buffer(
            &film.rgba,
            0,
            0,
            MTLOrigin { x: 0, y: 0, z: 0 },
            MTLSize {
                width: film.width as u64,
                height: film.height as u64,
                depth: 1,
            },
            &display,
            0,
            stride as u64,
            (stride * film.height as usize) as u64,
            MTLBlitOption::None,
        );
        encoder.end_encoding();
        command.commit();
        command.wait_until_completed();
        assert_eq!(command.status(), MTLCommandBufferStatus::Completed);
        // SAFETY: both shared allocations outlive these slices, and the GPU
        // copies completed before the CPU reads their exact allocated ranges.
        let linear =
            unsafe { std::slice::from_raw_parts(linear.contents().cast::<[f32; 4]>(), count) };
        assert!(linear.iter().all(|pixel| {
            pixel.iter().all(|value| value.is_finite() && *value >= 0.0)
                && pixel[3] == renderer.samples as f32
        }));
        let display = unsafe {
            std::slice::from_raw_parts(
                display.contents().cast::<u8>(),
                stride * film.height as usize,
            )
        };
        display
            .chunks_exact(stride)
            .flat_map(|row| {
                row[..film.width as usize * 4]
                    .chunks_exact(4)
                    .map(|pixel| <[u8; 4]>::try_from(pixel).unwrap())
            })
            .collect()
    })
}

fn compare(
    name: &str,
    hardware: &mut Renderer,
    software: &mut Renderer,
    scene: &Scene,
    settings: &RenderSettings,
    revision: u64,
) -> Vec<[u8; 4]> {
    for renderer in [&mut *hardware, &mut *software] {
        let frame = renderer.render(scene, settings, revision).unwrap();
        assert_eq!(frame.samples, 1);
        assert_eq!(frame.surface.get_width(), settings.width as usize);
        assert_eq!(frame.surface.get_height(), settings.height as usize);
    }
    let accelerated = pixels(hardware);
    let reference = pixels(software);
    let mut total_difference = 0u64;
    let mut differing_pixels = 0usize;
    for (a, b) in accelerated.iter().zip(&reference) {
        let difference = std::array::from_fn::<_, 3, _>(|i| a[i].abs_diff(b[i]));
        total_difference += difference
            .iter()
            .map(|value| u64::from(*value))
            .sum::<u64>();
        differing_pixels += usize::from(difference.iter().any(|value| *value > 8));
        assert_eq!(a[3], 255);
        assert_eq!(b[3], 255);
    }
    let mean = total_difference as f64 / (reference.len() * 3) as f64;
    let large_fraction = differing_pixels as f64 / reference.len() as f64;
    // Native triangle edge/tie rules differ from Möller–Trumbore. Permit sparse
    // boundary differences while rejecting changes to interior material/AO shading.
    assert!(
        mean < 0.8 && large_fraction < 0.015,
        "{name}: accelerated visibility diverged: mean={mean:.4}/255, large pixels={:.3}%",
        large_fraction * 100.0
    );
    eprintln!(
        "{name}: mean={mean:.4}/255, large pixels={:.3}%",
        large_fraction * 100.0
    );
    accelerated
}

#[test]
fn accelerated_preview_preserves_materials_contact_ao_and_occluded_selection() {
    let Some((mut hardware, mut software)) = renderer_pair() else {
        return;
    };
    let scene = Scene::default();
    let mut settings = settings();
    settings.selected = Some(scene.objects[0].id);
    settings.show_grid = true;
    compare(
        "default selected",
        &mut hardware,
        &mut software,
        &scene,
        &settings,
        1,
    );

    let mut scene = Scene::default();
    scene.objects.clear();
    // The hidden object deliberately precedes the selected object, checking
    // selection IDs against scene order rather than compacted visible order.
    let hidden = scene.add(Primitive::Cube);
    scene.object_mut(hidden).unwrap().visible = false;
    let sphere = scene.add(Primitive::Sphere);
    let object = scene.object_mut(sphere).unwrap();
    object.transform.translation.y = 0.9;
    object.transform.scale = Vec3::new(-0.85, 0.9, 0.7);
    object.material.base_color = Vec3::new(0.75, 0.25, 0.06);
    object.material.metallic = 0.7;
    object.material.roughness = 0.3;
    let floor = scene.add(Primitive::Plane);
    scene.object_mut(floor).unwrap().transform.scale = Vec3::splat(5.0);
    let occluder = scene.add(Primitive::Cube);
    let object = scene.object_mut(occluder).unwrap();
    object.transform.translation = Vec3::new(0.55, 0.5, 1.0);
    object.transform.scale = Vec3::new(0.5, 0.5, 0.3);
    object.material.base_color = Vec3::new(0.03, 0.12, 0.6);
    scene.camera.target = Vec3::new(0.0, 0.7, 0.0);
    scene.camera.yaw = 0.0;
    scene.camera.pitch = 0.28;
    scene.camera.distance = 5.0;
    settings.selected = Some(sphere);
    settings.show_grid = false;
    let contact = compare(
        "contact and occlusion",
        &mut hardware,
        &mut software,
        &scene,
        &settings,
        2,
    );
    settings.preview.ambient_occlusion = false;
    let no_contact = compare(
        "AO disabled",
        &mut hardware,
        &mut software,
        &scene,
        &settings,
        2,
    );
    assert_ne!(
        contact, no_contact,
        "The fixture must exercise contact occlusion"
    );
}

#[test]
fn accelerated_preview_keeps_two_sided_thin_geometry_in_both_projections() {
    let Some((mut hardware, mut software)) = renderer_pair() else {
        return;
    };
    let mut scene = Scene::default();
    scene.objects.clear();
    let plane = scene.add(Primitive::Plane);
    let object = scene.object_mut(plane).unwrap();
    object.transform.rotation.x = std::f32::consts::FRAC_PI_2;
    object.transform.rotation.y = 0.17;
    object.transform.scale = Vec3::new(-0.95, 1.0, 0.6);
    object.material.base_color = Vec3::new(0.8, 0.35, 0.04);
    object.material.metallic = 0.9;
    object.material.roughness = 0.12;
    scene.camera.target = Vec3::ZERO;
    scene.camera.distance = 3.5;
    scene.camera.pitch = 0.0;
    let mut settings = settings();
    settings.selected = Some(plane);
    settings.preview.ambient_occlusion = false;
    for orthographic in [false, true] {
        scene.camera.orthographic = orthographic;
        for yaw in [0.0, std::f32::consts::PI] {
            scene.camera.yaw = yaw;
            let image = compare(
                "two-sided thin plane",
                &mut hardware,
                &mut software,
                &scene,
                &settings,
                1,
            );
            let center =
                image[(settings.width * (settings.height / 2) + settings.width / 2) as usize];
            assert!(
                center[0] > center[2] + 15,
                "The colored plane must remain visible from both sides: {center:?}"
            );
        }
    }
}

#[test]
fn preview_acceleration_reuses_camera_geometry_and_rebuilds_after_mode_and_scene_changes() {
    let Some((mut hardware, mut software)) = renderer_pair() else {
        return;
    };
    let mut scene = Scene::default();
    scene.objects.clear();
    let mut settings = settings();
    compare(
        "empty scene",
        &mut hardware,
        &mut software,
        &scene,
        &settings,
        1,
    );
    assert_eq!(hardware.geometry.as_ref().unwrap().counts[0], 0);
    assert!(
        hardware
            .geometry
            .as_ref()
            .unwrap()
            .preview_acceleration
            .is_some()
    );
    assert!(
        software
            .geometry
            .as_ref()
            .unwrap()
            .preview_acceleration
            .is_none()
    );

    let id = scene.add(Primitive::Cube);
    settings.selected = Some(id);
    compare(
        "populated scene",
        &mut hardware,
        &mut software,
        &scene,
        &settings,
        2,
    );
    // Retain the old AS so the allocator cannot reuse its address and make a
    // subsequent rebuild accidentally appear identical.
    let original = hardware
        .geometry
        .as_ref()
        .unwrap()
        .preview_acceleration
        .clone()
        .unwrap();
    scene.camera.yaw += 0.2;
    compare(
        "camera move",
        &mut hardware,
        &mut software,
        &scene,
        &settings,
        2,
    );
    let current = hardware
        .geometry
        .as_ref()
        .unwrap()
        .preview_acceleration
        .as_ref()
        .unwrap();
    assert_eq!(
        original.as_ptr(),
        current.as_ptr(),
        "Camera navigation must reuse geometry acceleration"
    );

    settings.mode = RenderMode::Rendered;
    settings.max_samples = 1;
    settings.max_bounces = 2;
    scene.object_mut(id).unwrap().transform.translation.x = 0.75;
    let hardware_rendered = compare(
        "Rendered remains software",
        &mut hardware,
        &mut software,
        &scene,
        &settings,
        3,
    );
    assert_eq!(
        hardware_rendered,
        pixels(&software),
        "Preview acceleration must not alter Rendered samples"
    );
    assert!(
        hardware
            .geometry
            .as_ref()
            .unwrap()
            .preview_acceleration
            .is_none(),
        "Geometry edits in Rendered must invalidate the old preview acceleration"
    );
    settings.mode = RenderMode::MaterialPreview;
    compare(
        "preview after Rendered edit",
        &mut hardware,
        &mut software,
        &scene,
        &settings,
        3,
    );
    let rebuilt = hardware
        .geometry
        .as_ref()
        .unwrap()
        .preview_acceleration
        .as_ref()
        .unwrap();
    assert_ne!(original.as_ptr(), rebuilt.as_ptr());

    scene.object_mut(id).unwrap().visible = false;
    compare(
        "all hidden",
        &mut hardware,
        &mut software,
        &scene,
        &settings,
        4,
    );
    assert_eq!(hardware.geometry.as_ref().unwrap().counts[0], 0);
    scene.objects.clear();
    settings.selected = None;
    compare(
        "empty again",
        &mut hardware,
        &mut software,
        &scene,
        &settings,
        5,
    );
}

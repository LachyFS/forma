//! Crash isolation: the host survives worker hangs, aborts and panics, and
//! still regenerates the rest of a part when one feature takes it down.
use forma_cad::host::{CadHost, HostConfig, HostEvent, Job, WorkerMode};
use forma_cad::protocol::DebugAction;
use forma_core::{
    FeatureKind, FeatureOperation, PartData, Placement, PrimitiveFeature, PrimitiveShape,
};
use glam::Vec3;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

fn host(timeout: Duration) -> (CadHost, Receiver<HostEvent>) {
    let mut config = HostConfig::new(WorkerMode::Process {
        program: env!("CARGO_BIN_EXE_forma-cad-worker").into(),
        args: Vec::new(),
        env: vec![(forma_cad::worker::TEST_HOOKS_ENV.into(), "1".into())],
    });
    config.feature_timeout = timeout;
    let (sender, receiver) = channel();
    let host = CadHost::new(config, move |event| {
        let _ = sender.send(event);
    });
    (host, receiver)
}

fn next(events: &Receiver<HostEvent>, wanted: impl Fn(&HostEvent) -> bool) -> HostEvent {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = events
            .recv_timeout(remaining)
            .expect("host event timed out");
        if wanted(&event) {
            return event;
        }
    }
}

fn box_feature(size: f32, x: f32, operation: FeatureOperation) -> FeatureKind {
    FeatureKind::Primitive(PrimitiveFeature {
        shape: PrimitiveShape::Box {
            size: Vec3::splat(size),
        },
        placement: Placement {
            origin: Vec3::new(x, 0.0, 0.0),
            rotation: Vec3::ZERO,
        },
        operation,
    })
}

fn regenerated(events: &Receiver<HostEvent>) -> forma_cad::RegenResult {
    match next(events, |e| {
        matches!(e, HostEvent::Regenerated(_) | HostEvent::RegenFailed { .. })
    }) {
        HostEvent::Regenerated(result) => result,
        other => panic!("regeneration failed: {other:?}"),
    }
}

#[test]
fn worker_process_regenerates_and_recovers_from_hangs_aborts_and_panics() {
    let (host, events) = host(Duration::from_millis(1500));
    let mut part = PartData::new(5, "Block");
    part.push_feature("Base", box_feature(2.0, 0.0, FeatureOperation::Join));
    host.regenerate(5, part.clone());
    let result = regenerated(&events);
    assert_eq!((result.part_id, result.stamp), (5, part.stamp));
    let mesh = result.mesh.expect("box tessellation");
    assert!(mesh.has_groups());
    assert_eq!(result.faces.len(), 6);

    // A hang is cut off by the timeout and the worker restarts.
    let started = Instant::now();
    host.submit(Job::Debug {
        ticket: 1,
        action: DebugAction::Hang { feature: 9 },
    });
    next(&events, |e| matches!(e, HostEvent::WorkerRestarted { .. }));
    assert!(started.elapsed() < Duration::from_secs(10));
    host.submit(Job::Debug {
        ticket: 2,
        action: DebugAction::Echo,
    });
    next(&events, |e| matches!(e, HostEvent::Echo { ticket: 2 }));

    // An abort (as truck's stack overflow does) is a crash, not a hang.
    host.submit(Job::Debug {
        ticket: 3,
        action: DebugAction::Abort { feature: 9 },
    });
    match next(&events, |e| matches!(e, HostEvent::WorkerRestarted { .. })) {
        HostEvent::WorkerRestarted { reason } => assert!(
            reason.contains("crashed") && reason.contains("134"),
            "{reason}"
        ),
        _ => unreachable!(),
    }
    // A panic is caught inside the worker, which keeps running.
    host.submit(Job::Debug {
        ticket: 4,
        action: DebugAction::Panic,
    });
    match next(&events, |e| matches!(e, HostEvent::WorkerRestarted { .. })) {
        HostEvent::WorkerRestarted { reason } => assert!(reason.contains("injected"), "{reason}"),
        _ => unreachable!(),
    }
    host.regenerate(5, part.clone());
    assert!(regenerated(&events).mesh.is_some());
}

#[test]
fn features_that_take_the_worker_down_are_skipped_and_reported() {
    let (host, events) = host(Duration::from_millis(1500));
    for trap in ["__abort__", "__hang__"] {
        let mut part = PartData::new(8, "Bracket");
        part.push_feature("Base", box_feature(2.0, 0.0, FeatureOperation::Join));
        let bad = part.push_feature(trap, box_feature(1.0, 1.0, FeatureOperation::Cut));
        // Overlapping the base (not flush), so the join is exact.
        part.push_feature("Boss", box_feature(1.0, -1.45, FeatureOperation::Join));
        host.regenerate(8, part.clone());
        let result = regenerated(&events);
        assert_eq!(result.stamp, part.stamp);
        assert_eq!(result.issues.len(), 1, "{trap}: {:?}", result.issues);
        assert_eq!(result.issues[0].feature, bad);
        assert!(result.issues[0].message.contains("skipped"));
        // The base and the boss after the failed feature still regenerate.
        let mesh = result.mesh.expect("rest of the part");
        let min_x = mesh
            .positions
            .iter()
            .map(|p| p.x)
            .fold(f32::INFINITY, f32::min);
        assert!(
            min_x < -1.9,
            "{trap}: the boss after the skipped feature is missing"
        );
    }
}

#[test]
fn queued_regenerations_coalesce_and_shutdown_ends_a_stuck_worker() {
    let (host, events) = host(Duration::from_secs(30));
    // Keep the worker busy so the next requests queue up.
    host.submit(Job::Debug {
        ticket: 1,
        action: DebugAction::Echo,
    });
    let mut part = PartData::new(3, "Part");
    part.push_feature("Base", box_feature(1.0, 0.0, FeatureOperation::Join));
    let mut stamps = Vec::new();
    for size in [1.0, 1.5, 2.0] {
        let mut edited = part.clone();
        edited.edit_feature(1, |feature| {
            feature.kind = box_feature(size, 0.0, FeatureOperation::Join)
        });
        stamps.push(edited.stamp);
        host.regenerate(3, edited);
    }
    let mut seen = Vec::new();
    while seen.last() != Some(&stamps[2]) {
        seen.push(regenerated(&events).stamp);
    }
    assert!(
        seen.len() <= 2,
        "older queued requests were coalesced: {seen:?}"
    );

    host.submit(Job::Debug {
        ticket: 9,
        action: DebugAction::Hang { feature: 1 },
    });
    std::thread::sleep(Duration::from_millis(300));
    let started = Instant::now();
    host.shutdown();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "shutdown waited for a hung worker"
    );
}

#[test]
fn in_process_mode_runs_without_a_worker() {
    let (sender, events) = channel();
    let host = CadHost::new(HostConfig::new(WorkerMode::InProcess), move |event| {
        let _ = sender.send(event);
    });
    let mut part = PartData::new(1, "Part");
    part.push_feature("Base", box_feature(1.0, 0.0, FeatureOperation::Join));
    host.submit(Job::PartToMesh { ticket: 4, part });
    match next(&events, |e| matches!(e, HostEvent::Mesh { .. })) {
        HostEvent::Mesh { ticket, result } => {
            assert_eq!(ticket, 4);
            assert_eq!(result.unwrap().faces.len(), 6);
        }
        _ => unreachable!(),
    }
}

#[test]
fn an_unreadable_reply_restarts_the_worker_and_never_wedges_shutdown() {
    // Review case: the reader thread used to stop on a decode error while
    // the worker lived on, and the host then waited on it forever while
    // holding the lock shutdown needs.
    let (host, events) = host(Duration::from_secs(30));
    let started = Instant::now();
    host.submit(Job::Debug {
        ticket: 1,
        action: DebugAction::BadFrame,
    });
    match next(&events, |e| matches!(e, HostEvent::WorkerRestarted { .. })) {
        HostEvent::WorkerRestarted { reason } => {
            assert!(reason.contains("could not be read"), "{reason}")
        }
        _ => unreachable!(),
    }
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the failure waited for a timeout"
    );
    // The next request gets a fresh worker.
    host.submit(Job::Debug {
        ticket: 2,
        action: DebugAction::Echo,
    });
    next(&events, |e| matches!(e, HostEvent::Echo { ticket: 2 }));
    // Shutdown is prompt even with another unreadable worker alive.
    host.submit(Job::Debug {
        ticket: 3,
        action: DebugAction::BadFrame,
    });
    std::thread::sleep(Duration::from_millis(200));
    let stopping = Instant::now();
    host.shutdown();
    assert!(stopping.elapsed() < Duration::from_secs(3));
}

#[test]
fn invalid_part_data_is_refused_before_it_reaches_the_worker() {
    let (host, events) = host(Duration::from_secs(30));
    let mut part = PartData::new(4, "Bad");
    part.push_feature("Base", box_feature(1.0, 0.0, FeatureOperation::Join));
    if let FeatureKind::Primitive(primitive) = &mut part.features[0].kind {
        primitive.placement.origin.x = f32::NAN;
    }
    host.regenerate(4, part);
    match next(&events, |e| {
        matches!(e, HostEvent::RegenFailed { .. } | HostEvent::Regenerated(_))
    }) {
        HostEvent::RegenFailed { issue, .. } => {
            assert!(issue.message.contains("invalid"), "{issue:?}")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn baking_and_exporting_skip_what_regeneration_skipped() {
    // Review case: Convert to mesh and STEP export re-ran a feature that had
    // already hung the worker, and reported no per-feature progress.
    let (host, events) = host(Duration::from_millis(1500));
    let mut part = PartData::new(11, "Bracket");
    part.push_feature("Base", box_feature(2.0, 0.0, FeatureOperation::Join));
    let bad = part.push_feature("__hang__", box_feature(1.0, 1.0, FeatureOperation::Cut));
    host.regenerate(11, part.clone());
    let result = regenerated(&events);
    assert_eq!(result.issues[0].feature, bad);
    let started = Instant::now();
    host.submit(Job::PartToMesh {
        ticket: 5,
        part: part.clone(),
    });
    match next(&events, |e| matches!(e, HostEvent::Mesh { .. })) {
        HostEvent::Mesh { ticket, result } => {
            assert_eq!(ticket, 5);
            assert_eq!(
                result.expect("baked without the hung feature").faces.len(),
                6
            );
        }
        _ => unreachable!(),
    }
    host.submit(Job::ExportStep {
        ticket: 6,
        parts: vec![forma_cad::StepPart {
            name: "Bracket".into(),
            part: part.clone(),
            transform: glam::Mat4::IDENTITY,
        }],
    });
    match next(&events, |e| matches!(e, HostEvent::Step { .. })) {
        HostEvent::Step { result, .. } => assert!(result.unwrap().starts_with("ISO-10303-21;")),
        _ => unreachable!(),
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a skipped feature ran again"
    );
    // A different version of the part gets no stale skips: the hang is
    // caught per feature instead of by the 120 s request limit.
    part.edit_feature(1, |feature| {
        feature.kind = box_feature(2.5, 0.0, FeatureOperation::Join)
    });
    let started = Instant::now();
    host.submit(Job::PartToMesh { ticket: 7, part });
    match next(&events, |e| matches!(e, HostEvent::Mesh { ticket: 7, .. })) {
        HostEvent::Mesh { result, .. } => assert!(result.is_err()),
        _ => unreachable!(),
    }
    assert!(started.elapsed() < Duration::from_secs(10));
}

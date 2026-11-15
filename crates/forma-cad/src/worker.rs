//! The CAD worker: a loop that evaluates requests from stdin and answers on
//! stdout. Run by `forma --cad-worker` (and the `forma-cad-worker` binary).
//!
//! A reader thread owns stdin: end of input means the application is gone,
//! so the worker exits at once, even while a kernel call is stuck.
use crate::features::{RegenCache, regenerate};
use crate::protocol::{DebugAction, Request, Response, read_frame, write_frame};
use std::sync::mpsc;

/// Environment variable enabling [`Request::Debug`] failure injection.
pub const TEST_HOOKS_ENV: &str = "FORMA_CAD_WORKER_TEST_HOOKS";

/// Serve requests on stdin/stdout until end of input or `Shutdown`.
pub fn serve_stdio() -> ! {
    let hooks = std::env::var(TEST_HOOKS_ENV).is_ok_and(|value| value == "1");
    // Kernel panics are caught and reported per request; keep stderr quiet.
    std::panic::set_hook(Box::new(|_| {}));
    let (sender, receiver) = mpsc::channel::<Request>();
    std::thread::Builder::new()
        .name("forma-cad-stdin".into())
        .spawn(move || {
            let mut stdin = std::io::stdin().lock();
            loop {
                match read_frame::<Request>(&mut stdin) {
                    Ok(Some(Request::Shutdown)) | Ok(None) | Err(_) => std::process::exit(0),
                    Ok(Some(request)) => {
                        if sender.send(request).is_err() {
                            std::process::exit(0);
                        }
                    }
                }
            }
        })
        .expect("could not start the CAD worker input thread");
    let stdout = std::io::stdout();
    let mut cache = RegenCache::default();
    for request in receiver {
        let mut emit = |response: Response| {
            let mut out = stdout.lock();
            if write_frame(&mut out, &response).is_err() {
                std::process::exit(0);
            }
        };
        handle(request, &mut cache, hooks, &mut emit);
    }
    std::process::exit(0)
}

/// Test hook: features named "__hang__" or "__abort__" take the worker down
/// when they are evaluated (only with [`TEST_HOOKS_ENV`] set).
struct Traps {
    hang: Option<u64>,
    abort: Option<u64>,
}

impl Traps {
    fn new<'a>(hooks: bool, parts: impl IntoIterator<Item = &'a forma_core::PartData>) -> Self {
        let mut traps = Self {
            hang: None,
            abort: None,
        };
        if hooks {
            for part in parts {
                for feature in forma_core::all_features(&part.features) {
                    match feature.name.as_str() {
                        "__hang__" => traps.hang = Some(feature.id),
                        "__abort__" => traps.abort = Some(feature.id),
                        _ => {}
                    }
                }
            }
        }
        traps
    }

    /// Report progress on `feature`, springing a trap if it has one.
    fn report(&self, id: u64, feature: u64, emit: &mut dyn FnMut(Response)) {
        emit(Response::Progress { id, feature });
        if Some(feature) == self.hang {
            loop {
                std::thread::park();
            }
        }
        if Some(feature) == self.abort {
            std::process::exit(134);
        }
    }
}

/// Evaluate one request, emitting progress and exactly one final response.
pub fn handle(
    request: Request,
    cache: &mut RegenCache,
    hooks: bool,
    emit: &mut dyn FnMut(Response),
) {
    let id = request.id();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match request {
        Request::Regenerate { id, regen } => {
            let traps = Traps::new(hooks, [&regen.part]);
            let result = regenerate(&regen, cache, &mut |feature| {
                traps.report(id, feature, emit)
            });
            Response::Regenerated { id, result }
        }
        Request::PartToMesh { id, part, skip } => {
            let traps = Traps::new(hooks, [&part]);
            let result = crate::step::part_to_mesh_cached(&part, &skip, cache, &mut |feature| {
                traps.report(id, feature, emit)
            });
            Response::Mesh {
                id,
                result: result.map_err(|error| error.to_string()),
            }
        }
        Request::ExportStep { id, parts, skips } => {
            let traps = Traps::new(hooks, parts.iter().map(|placed| &placed.part));
            let result = crate::step::export_step_cached(&parts, &skips, cache, &mut |feature| {
                traps.report(id, feature, emit)
            });
            Response::Step {
                id,
                result: result.map_err(|error| error.to_string()),
            }
        }
        Request::ImportStep { id, name, text } => Response::Imported {
            id,
            result: crate::step::import_step(&name, &text).map_err(|error| error.to_string()),
        },
        Request::Debug { id, action } => {
            if !hooks {
                return Response::Error {
                    id,
                    message: "Debug requests are disabled".into(),
                };
            }
            match action {
                DebugAction::Hang { feature } => {
                    emit(Response::Progress { id, feature });
                    loop {
                        std::thread::park();
                    }
                }
                DebugAction::Abort { feature } => {
                    emit(Response::Progress { id, feature });
                    // Die without answering, like a kernel stack overflow,
                    // but without leaving a core dump behind in tests.
                    std::process::exit(134);
                }
                DebugAction::Panic => panic!("injected worker panic"),
                DebugAction::BadFrame => {
                    // A well-framed reply whose JSON does not decode.
                    let mut out = std::io::stdout().lock();
                    let json = br#"{"response":"echo","id":null}"#;
                    let _ = std::io::Write::write_all(&mut out, b"\x00FCAD\x01\xfe\xff");
                    let _ = std::io::Write::write_all(&mut out, &(json.len() as u32).to_le_bytes());
                    let _ = std::io::Write::write_all(&mut out, json);
                    let _ = std::io::Write::flush(&mut out);
                    loop {
                        std::thread::park();
                    }
                }
                DebugAction::Echo => Response::Echo { id },
            }
        }
        Request::Shutdown => Response::Echo { id },
    }));
    let mut response = outcome.unwrap_or_else(|payload| {
        let detail = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown failure".into());
        // A panic may leave cached kernel state half updated.
        cache.clear();
        Response::Error {
            id,
            message: format!("The geometry kernel failed ({detail})"),
        }
    });
    response.sanitize();
    emit(response);
}

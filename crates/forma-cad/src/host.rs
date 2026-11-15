//! Runs CAD work off the UI thread, isolated in a worker process.
//!
//! truck can panic (caught in the worker), abort the process (stack
//! overflow) or loop forever on hard inputs. The host sends one request at a
//! time to `forma --cad-worker`, watches for progress, and kills and restarts
//! the worker when it crashes or a feature exceeds its time budget. A
//! regeneration that took the worker down is retried with the offending
//! feature skipped, so the rest of the part still appears and the feature is
//! reported. Queued regenerations of the same part are coalesced; results
//! carry the part id and stamp so callers drop stale ones.
use crate::features::RegenCache;
use crate::protocol::{
    DebugAction, RegenRequest, RegenResult, Request, Response, StepPart, read_frame, write_frame,
};
use forma_core::{FeatureIssue, Mesh, PartData};
use std::collections::VecDeque;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Where requests run.
#[derive(Clone, Debug)]
pub enum WorkerMode {
    /// Spawn `program args…` speaking the worker protocol on stdin/stdout.
    Process {
        program: PathBuf,
        args: Vec<String>,
        env: Vec<(String, String)>,
    },
    /// Run requests on the host thread. No crash isolation; for tests.
    InProcess,
}

impl WorkerMode {
    /// The current executable with `--cad-worker`, as Forma ships it.
    pub fn current_exe() -> std::io::Result<Self> {
        Ok(Self::Process {
            program: std::env::current_exe()?,
            args: vec!["--cad-worker".into()],
            env: Vec::new(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct HostConfig {
    pub mode: WorkerMode,
    /// Longest a single feature (or a non-regeneration request) may run.
    pub feature_timeout: Duration,
    /// Longest one request may run in total.
    pub request_timeout: Duration,
    /// Features a regeneration may skip after crashes or hangs.
    pub max_skips: usize,
}

impl HostConfig {
    pub fn new(mode: WorkerMode) -> Self {
        Self {
            mode,
            feature_timeout: Duration::from_secs(20),
            request_timeout: Duration::from_secs(120),
            max_skips: 3,
        }
    }
}

/// Work for the host.
#[derive(Clone, Debug)]
pub enum Job {
    Regenerate(RegenRequest),
    PartToMesh {
        ticket: u64,
        part: PartData,
    },
    ExportStep {
        ticket: u64,
        parts: Vec<StepPart>,
    },
    ImportStep {
        ticket: u64,
        name: String,
        text: String,
    },
    /// Test-only failure injection (needs `FORMA_CAD_WORKER_TEST_HOOKS=1`).
    Debug {
        ticket: u64,
        action: DebugAction,
    },
}

/// Results and notices from the host, delivered on its own thread.
#[derive(Clone, Debug)]
pub enum HostEvent {
    /// Evaluation of a part started.
    Started {
        part_id: u64,
        stamp: u64,
    },
    Regenerated(RegenResult),
    /// Regeneration produced no result at all; keep the cached tessellation
    /// and show `issue`.
    RegenFailed {
        part_id: u64,
        stamp: u64,
        issue: FeatureIssue,
    },
    Mesh {
        ticket: u64,
        result: Result<Mesh, String>,
    },
    Step {
        ticket: u64,
        result: Result<String, String>,
    },
    Imported {
        ticket: u64,
        result: Result<PartData, String>,
    },
    Echo {
        ticket: u64,
    },
    /// The worker crashed or hung and will be restarted for the next job.
    WorkerRestarted {
        reason: String,
    },
}

type Sink = Box<dyn Fn(HostEvent) + Send>;

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    stop: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    child: Mutex<Option<Child>>,
}

impl Shared {
    /// The worker process, removed from the shared slot. The lock is held
    /// only for the swap, so killing and reaping never block other threads.
    fn take_child(&self) -> Option<Child> {
        self.child.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// A handle to the CAD host thread. Dropping it shuts the worker down.
pub struct CadHost {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl CadHost {
    pub fn new(config: HostConfig, sink: impl Fn(HostEvent) + Send + 'static) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            child: Mutex::new(None),
        });
        let thread_shared = shared.clone();
        let sink: Sink = Box::new(sink);
        let thread = std::thread::Builder::new()
            .name("forma-cad-host".into())
            .spawn(move || run(config, thread_shared, sink))
            .expect("could not start the CAD host thread");
        Self {
            shared,
            thread: Mutex::new(Some(thread)),
        }
    }

    /// Queue a job without blocking. A queued regeneration of the same part
    /// is replaced by the newer one.
    pub fn submit(&self, job: Job) {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        if let Job::Regenerate(request) = &job {
            queue.jobs.retain(
                |queued| !matches!(queued, Job::Regenerate(old) if old.part_id == request.part_id),
            );
        }
        queue.jobs.push_back(job);
        drop(queue);
        self.shared.wake.notify_one();
    }

    pub fn regenerate(&self, part_id: u64, part: PartData) {
        self.submit(Job::Regenerate(RegenRequest {
            part_id,
            part,
            skip: Vec::new(),
        }));
    }

    /// Jobs waiting behind the current one.
    pub fn pending(&self) -> usize {
        self.shared
            .queue
            .lock()
            .map(|queue| queue.jobs.len())
            .unwrap_or(0)
    }

    /// Stop the host thread and terminate the worker process. Waits at most
    /// about a second; safe to call more than once.
    pub fn shutdown(&self) {
        {
            let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.stop = true;
            queue.jobs.clear();
        }
        self.shared.wake.notify_all();
        // Killing the worker unblocks a host thread waiting on a stuck call.
        // Never kill or wait while holding the lock: the host thread may
        // need it to finish.
        if let Some(mut child) = self.shared.take_child() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let thread = self.thread.lock().ok().and_then(|mut t| t.take());
        if let Some(thread) = thread {
            let deadline = Instant::now() + Duration::from_secs(1);
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for CadHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Why a request produced no response.
enum Failure {
    Hung {
        /// The feature being evaluated, if the worker reported one.
        feature: Option<u64>,
        /// That feature ran past its own time limit (rather than the whole
        /// request running past the request limit).
        over_feature_limit: bool,
    },
    Crashed {
        feature: Option<u64>,
        detail: String,
    },
}

/// What the reader thread passes on from the worker's output.
enum Incoming {
    Response(Response),
    /// A frame that could not be decoded: the worker is out of step.
    Unreadable(String),
}

struct Worker {
    stdin: BufWriter<ChildStdin>,
    responses: Receiver<Incoming>,
}

struct Runner {
    config: HostConfig,
    shared: Arc<Shared>,
    worker: Option<Worker>,
    cache: RegenCache,
    next_id: u64,
}

impl Runner {
    fn start(&mut self) -> Result<(), String> {
        if self.worker.is_some() {
            return Ok(());
        }
        let WorkerMode::Process { program, args, env } = &self.config.mode else {
            return Ok(());
        };
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (key, value) in env {
            command.env(key, value);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start the CAD worker: {error}"))?;
        let stdin = child.stdin.take().ok_or("CAD worker has no input pipe")?;
        let stdout = child.stdout.take().ok_or("CAD worker has no output pipe")?;
        let (sender, responses) = mpsc::channel();
        std::thread::Builder::new()
            .name("forma-cad-reader".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let incoming = match read_frame::<Response>(&mut reader) {
                        Ok(Some(response)) => Incoming::Response(response),
                        Ok(None) => break,
                        Err(error) => Incoming::Unreadable(error.to_string()),
                    };
                    let unreadable = matches!(incoming, Incoming::Unreadable(_));
                    // After an unreadable frame the stream position is
                    // unknown: report it and stop; the host kills the worker.
                    if sender.send(incoming).is_err() || unreadable {
                        break;
                    }
                }
            })
            .map_err(|error| format!("Could not start the CAD reader: {error}"))?;
        *self.shared.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        self.worker = Some(Worker {
            stdin: BufWriter::new(stdin),
            responses,
        });
        Ok(())
    }

    /// Kill and reap the worker, returning how it ended.
    fn stop_worker(&mut self) -> Option<std::process::ExitStatus> {
        self.worker = None;
        let mut child = self.shared.take_child()?;
        // An already exited worker keeps its own status; kill is a no-op.
        let _ = child.kill();
        child.wait().ok()
    }

    fn stopping(&self) -> bool {
        self.shared.queue.lock().map(|q| q.stop).unwrap_or(true)
    }

    /// Send one request and wait for its final response.
    fn call(&mut self, request: Request) -> Result<Response, Failure> {
        if matches!(self.config.mode, WorkerMode::InProcess) {
            let mut final_response = None;
            crate::worker::handle(request, &mut self.cache, true, &mut |response| {
                if response.is_final() {
                    final_response = Some(response);
                }
            });
            return final_response.ok_or(Failure::Crashed {
                feature: None,
                detail: "no response".into(),
            });
        }
        if let Err(detail) = self.start() {
            return Err(Failure::Crashed {
                feature: None,
                detail,
            });
        }
        let id = request.id();
        let worker = self.worker.as_mut().unwrap();
        if write_frame(&mut worker.stdin, &request).is_err() {
            self.stop_worker();
            return Err(Failure::Crashed {
                feature: None,
                detail: "the worker closed its input".into(),
            });
        }
        let started = Instant::now();
        let mut feature_started = Instant::now();
        let mut feature = None;
        loop {
            let remaining_feature = self
                .config
                .feature_timeout
                .saturating_sub(feature_started.elapsed());
            let remaining_request = self
                .config
                .request_timeout
                .saturating_sub(started.elapsed());
            let wait = remaining_feature.min(remaining_request);
            let received = self.worker.as_ref().unwrap().responses.recv_timeout(wait);
            match received {
                Ok(Incoming::Response(Response::Progress {
                    id: rid,
                    feature: f,
                })) if rid == id => {
                    feature = Some(f);
                    feature_started = Instant::now();
                }
                Ok(Incoming::Response(response)) if response.id() == id => return Ok(response),
                Ok(Incoming::Response(_)) => {} // a late answer to an abandoned request
                Ok(Incoming::Unreadable(error)) => {
                    self.stop_worker();
                    return Err(Failure::Crashed {
                        feature,
                        detail: format!("its reply could not be read: {error}"),
                    });
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.stop_worker();
                    return Err(Failure::Hung {
                        feature,
                        over_feature_limit: remaining_feature <= remaining_request,
                    });
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let status = self.stop_worker();
                    return Err(Failure::Crashed {
                        feature,
                        detail: status
                            .map(|s| format!("worker exited ({s})"))
                            .unwrap_or_else(|| "worker exited".into()),
                    });
                }
            }
        }
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn regenerate(&mut self, mut request: RegenRequest, sink: &Sink) {
        sink(HostEvent::Started {
            part_id: request.part_id,
            stamp: request.part.stamp,
        });
        // Invalid data (for example a non-finite value, which JSON cannot
        // carry) would make the worker fail to read the request.
        if let Err(error) = request.part.validate() {
            sink(HostEvent::RegenFailed {
                part_id: request.part_id,
                stamp: request.part.stamp,
                issue: FeatureIssue {
                    feature: 0,
                    message: format!("The part data is invalid: {error:#}"),
                    suggestion: "Undo the last change".into(),
                    warning: false,
                },
            });
            return;
        }
        for _ in 0..=self.config.max_skips {
            let id = self.id();
            let outcome = self.call(Request::Regenerate {
                id,
                regen: request.clone(),
            });
            let failure = match outcome {
                Ok(Response::Regenerated { result, .. }) => {
                    sink(HostEvent::Regenerated(result));
                    return;
                }
                Ok(Response::Error { message, .. }) => {
                    sink(HostEvent::RegenFailed {
                        part_id: request.part_id,
                        stamp: request.part.stamp,
                        issue: FeatureIssue {
                            feature: 0,
                            message,
                            suggestion: "Undo the last change or adjust it slightly".into(),
                            warning: false,
                        },
                    });
                    return;
                }
                Ok(_) => return,
                Err(failure) => failure,
            };
            if self.stopping() {
                return;
            }
            let (feature, what) = match &failure {
                Failure::Hung {
                    feature,
                    over_feature_limit: true,
                } => (
                    *feature,
                    format!(
                        "did not finish a feature within {} s",
                        self.config.feature_timeout.as_secs_f32().round()
                    ),
                ),
                // The whole request ran out of time: no single feature is to
                // blame, so nothing is skipped.
                Failure::Hung {
                    over_feature_limit: false,
                    ..
                } => (
                    None,
                    format!(
                        "did not finish within {} s",
                        self.config.request_timeout.as_secs_f32().round()
                    ),
                ),
                Failure::Crashed { feature, detail } => (*feature, format!("crashed ({detail})")),
            };
            sink(HostEvent::WorkerRestarted {
                reason: format!("The geometry kernel {what}"),
            });
            let culprit = feature.filter(|f| !request.skip.iter().any(|issue| issue.feature == *f));
            let Some(culprit) = culprit else {
                sink(HostEvent::RegenFailed {
                    part_id: request.part_id,
                    stamp: request.part.stamp,
                    issue: FeatureIssue {
                        feature: feature.unwrap_or(0),
                        message: format!("Regeneration failed: the geometry kernel {what}"),
                        suggestion: "Undo the last change or simplify the part".into(),
                        warning: false,
                    },
                });
                return;
            };
            let name = forma_core::all_features(&request.part.features)
                .find(|f| f.id == culprit)
                .map(|f| f.name.clone())
                .unwrap_or_else(|| format!("Feature {culprit}"));
            request.skip.push(FeatureIssue {
                feature: culprit,
                message: format!("{name}: the geometry kernel {what}, so this feature was skipped"),
                suggestion: "Change its dimensions slightly, or suppress it".into(),
                warning: false,
            });
        }
        sink(HostEvent::RegenFailed {
            part_id: request.part_id,
            stamp: request.part.stamp,
            issue: FeatureIssue {
                feature: 0,
                message: "Regeneration failed repeatedly".into(),
                suggestion: "Undo the last change".into(),
                warning: false,
            },
        });
    }

    fn simple(&mut self, request: Request, ticket: u64, sink: &Sink) {
        // Which event a failure is reported as (debug probes only restart).
        let debug = matches!(request, Request::Debug { .. });
        let fail: fn(u64, String) -> HostEvent = match &request {
            Request::PartToMesh { .. } => |ticket, message| HostEvent::Mesh {
                ticket,
                result: Err(message),
            },
            Request::ExportStep { .. } => |ticket, message| HostEvent::Step {
                ticket,
                result: Err(message),
            },
            Request::ImportStep { .. } => |ticket, message| HostEvent::Imported {
                ticket,
                result: Err(message),
            },
            _ => |_, reason| HostEvent::WorkerRestarted { reason },
        };
        let event = match self.call(request) {
            Ok(Response::Mesh { result, .. }) => HostEvent::Mesh { ticket, result },
            Ok(Response::Step { result, .. }) => HostEvent::Step { ticket, result },
            Ok(Response::Imported { result, .. }) => HostEvent::Imported { ticket, result },
            Ok(Response::Echo { .. }) => HostEvent::Echo { ticket },
            Ok(Response::Error { message, .. }) => fail(ticket, message),
            Ok(_) => return,
            Err(failure) => {
                let message = match failure {
                    Failure::Hung { .. } => "The geometry kernel did not finish in time".to_owned(),
                    Failure::Crashed { detail, .. } => {
                        format!("The geometry kernel crashed ({detail})")
                    }
                };
                if self.stopping() {
                    return;
                }
                sink(HostEvent::WorkerRestarted {
                    reason: message.clone(),
                });
                if debug {
                    return;
                }
                fail(ticket, message)
            }
        };
        sink(event);
    }
}

fn run(config: HostConfig, shared: Arc<Shared>, sink: Sink) {
    let mut runner = Runner {
        config,
        shared: shared.clone(),
        worker: None,
        cache: RegenCache::default(),
        next_id: 0,
    };
    loop {
        let job = {
            let mut queue = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            while queue.jobs.is_empty() && !queue.stop {
                queue = shared.wake.wait(queue).unwrap_or_else(|e| e.into_inner());
            }
            if queue.stop {
                break;
            }
            queue.jobs.pop_front().unwrap()
        };
        match job {
            Job::Regenerate(request) => runner.regenerate(request, &sink),
            Job::PartToMesh { ticket, part } => {
                let id = runner.id();
                runner.simple(Request::PartToMesh { id, part }, ticket, &sink)
            }
            Job::ExportStep { ticket, parts } => {
                let id = runner.id();
                runner.simple(Request::ExportStep { id, parts }, ticket, &sink)
            }
            Job::ImportStep { ticket, name, text } => {
                let id = runner.id();
                runner.simple(Request::ImportStep { id, name, text }, ticket, &sink)
            }
            Job::Debug { ticket, action } => {
                let id = runner.id();
                runner.simple(Request::Debug { id, action }, ticket, &sink)
            }
        }
    }
    // Ask politely, then make sure the process is gone.
    if let Some(worker) = runner.worker.as_mut() {
        let _ = write_frame(&mut worker.stdin, &Request::Shutdown);
    }
    runner.stop_worker();
}

//! The updater worker owns all network, hashing, download and installer work.
use forma_update::{Candidate, Preferences};
use std::{path::PathBuf, sync::mpsc, thread};

#[derive(Clone, Debug)]
pub(crate) enum State {
    Disabled(String),
    Idle,
    Checking,
    Available(String),
    Downloading(i16),
    Ready(String),
    Installing,
    Failed(String),
}

impl State {
    pub fn label(&self) -> String {
        match self {
            Self::Disabled(reason) => reason.clone(),
            Self::Idle => "No update pending".into(),
            Self::Checking => "Checking for updates…".into(),
            Self::Available(version) => format!("Forma {version} is available"),
            Self::Downloading(progress) => format!("Downloading update · {progress}%"),
            Self::Ready(version) => format!("Forma {version} is ready to install"),
            Self::Installing => "Restarting Forma…".into(),
            Self::Failed(error) => format!("Update failed: {error}"),
        }
    }
}

enum Request {
    Check {
        manual: bool,
        preferences: Preferences,
    },
    Download,
    Install(Vec<String>),
}

pub(crate) enum Event {
    State(State),
    Exit,
}

pub(crate) struct Updater {
    pub state: State,
    pub preferences: Preferences,
    pub open: bool,
    pub restart_pending: bool,
    busy: bool,
    path: Option<PathBuf>,
    requests: mpsc::Sender<Request>,
}

impl Updater {
    pub fn new(path: Option<PathBuf>, enabled: bool) -> (Self, async_channel::Receiver<Event>) {
        let (requests, incoming) = mpsc::channel();
        let (events, receiver) = async_channel::unbounded();
        let loaded = path.as_deref().map(Preferences::load).transpose();
        let (preferences, error) = match loaded {
            Ok(p) => (p.unwrap_or_default(), None),
            Err(error) => (
                Preferences {
                    automatic_checks: false,
                    automatic_downloads: false,
                    skipped_version: None,
                },
                Some(format!("Could not load update preferences: {error:#}")),
            ),
        };
        let available = if enabled {
            forma_update::installed_version().map(|_| ())
        } else {
            Err(anyhow::anyhow!("Updates are disabled for this session."))
        };
        let state = match available {
            Ok(()) => {
                thread::spawn(move || run(incoming, events));
                error.map(State::Failed).unwrap_or(State::Idle)
            }
            Err(error) => State::Disabled(format!("{error:#}")),
        };
        (
            Self {
                state,
                preferences,
                open: false,
                restart_pending: false,
                busy: false,
                path,
                requests,
            },
            receiver,
        )
    }

    pub fn check(&mut self, manual: bool) {
        if self.busy
            || matches!(
                self.state,
                State::Disabled(_) | State::Ready(_) | State::Available(_)
            )
        {
            return;
        }
        if !manual && !self.preferences.automatic_checks {
            return;
        }
        self.busy = true;
        self.state = State::Checking;
        if self
            .requests
            .send(Request::Check {
                manual,
                preferences: self.preferences.clone(),
            })
            .is_err()
        {
            self.receive(State::Failed("Updater is unavailable".into()));
        }
    }

    pub fn download(&mut self) {
        if self.busy || !matches!(self.state, State::Available(_)) {
            return;
        }
        self.busy = true;
        self.state = State::Downloading(0);
        if self.requests.send(Request::Download).is_err() {
            self.receive(State::Failed("Updater is unavailable".into()));
        }
    }

    pub fn install(&mut self, args: Vec<String>) {
        self.busy = true;
        self.state = State::Installing;
        if self.requests.send(Request::Install(args)).is_err() {
            self.receive(State::Failed("Updater is unavailable".into()));
        }
    }

    pub fn receive(&mut self, state: State) {
        self.busy = matches!(
            state,
            State::Checking | State::Downloading(_) | State::Installing
        );
        if matches!(state, State::Failed(_)) {
            self.restart_pending = false;
        }
        self.state = state;
    }

    pub fn save_preferences(&mut self) {
        let result = self
            .path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("No preference directory available"))
            .and_then(|path| self.preferences.save(path));
        if let Err(error) = result {
            self.receive(State::Failed(format!(
                "Could not save preferences: {error:#}"
            )));
        }
    }

    pub fn skip(&mut self) {
        if let State::Ready(version) | State::Available(version) = &self.state {
            self.preferences.skipped_version = Some(version.clone());
            self.state = State::Idle;
            self.open = false;
            self.save_preferences();
        }
    }
}

fn run(requests: mpsc::Receiver<Request>, events: async_channel::Sender<Event>) {
    let mut candidate: Option<Candidate> = None;
    for request in requests {
        let result = (|| -> anyhow::Result<()> {
            match request {
                Request::Check {
                    manual,
                    preferences,
                } => {
                    candidate = Candidate::check()?;
                    if let Some(update) = &candidate {
                        if !manual
                            && preferences.skipped_version.as_deref() == Some(update.version())
                        {
                            events.send_blocking(Event::State(State::Idle))?;
                        } else if preferences.automatic_downloads {
                            download(update, &events)?;
                        } else {
                            events.send_blocking(Event::State(State::Available(
                                update.version().into(),
                            )))?;
                        }
                    } else {
                        events.send_blocking(Event::State(State::Idle))?;
                    }
                }
                Request::Download => {
                    if let Some(update) = &candidate {
                        download(update, &events)?;
                    }
                }
                Request::Install(args) => {
                    let update = candidate
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("No verified update is ready"))?;
                    update.install_after_exit(args)?;
                    events.send_blocking(Event::Exit)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result
            && events
                .send_blocking(Event::State(State::Failed(format!("{error:#}"))))
                .is_err()
        {
            break;
        }
    }
}

fn download(update: &Candidate, events: &async_channel::Sender<Event>) -> anyhow::Result<()> {
    events.send_blocking(Event::State(State::Downloading(0)))?;
    let (progress, incoming) = mpsc::channel();
    let output = events.clone();
    let forward = thread::spawn(move || {
        for percent in incoming {
            if output
                .send_blocking(Event::State(State::Downloading(percent)))
                .is_err()
            {
                break;
            }
        }
    });
    let result = update.download(progress);
    let _ = forward.join();
    result?;
    events.send_blocking(Event::State(State::Ready(update.version().into())))?;
    Ok(())
}

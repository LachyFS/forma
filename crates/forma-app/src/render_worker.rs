//! A latest-request mailbox keeps input responsive when rendering is slower than input.
use forma_core::{Camera, Scene, World};
use forma_render::{Frame, RenderSettings, Renderer};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
};

#[derive(Clone)]
pub struct Request {
    pub scene: Arc<Scene>,
    pub camera: Camera,
    pub world: World,
    pub settings: RenderSettings,
    pub revision: u64,
    pub generation: u64,
}

#[derive(Default)]
struct Inbox {
    request: Option<Request>,
    stop: bool,
}

#[derive(Default)]
pub struct Output {
    pub frame: Option<(u64, Frame)>,
    pub device: Option<String>,
    pub messages: VecDeque<Result<String, String>>,
    pub render_error: Option<(u64, String)>,
    pub export_finished: bool,
}

/// The output itself is a latest-frame mailbox. A bounded wakeup only signals that
/// it changed, so a busy UI never accumulates a queue of obsolete frames.
#[derive(Clone)]
struct OutputMailbox {
    value: Arc<Mutex<Output>>,
    wake: async_channel::Sender<()>,
}

impl OutputMailbox {
    fn new() -> (Self, async_channel::Receiver<()>) {
        let (wake, receiver) = async_channel::bounded(1);
        (
            Self {
                value: Arc::new(Mutex::new(Output::default())),
                wake,
            },
            receiver,
        )
    }

    fn publish(&self, update: impl FnOnce(&mut Output)) {
        update(&mut self.value.lock().unwrap());
        // Never block the render thread on presentation. A full channel already
        // guarantees the UI will drain the latest output; a closed one means quit.
        let _ = self.wake.try_send(());
    }

    fn take(&self) -> Output {
        std::mem::take(&mut *self.value.lock().unwrap())
    }
}

pub struct RenderWorker {
    input: Arc<(Mutex<Inbox>, Condvar)>,
    output: OutputMailbox,
}

impl RenderWorker {
    pub fn new() -> (Self, async_channel::Receiver<()>) {
        let input = Arc::new((Mutex::new(Inbox::default()), Condvar::new()));
        let (output, notifications) = OutputMailbox::new();
        let worker_input = input.clone();
        let worker_output = output.clone();
        thread::Builder::new()
            .name("forma-viewport".into())
            .spawn(move || {
                let mut renderer = match Renderer::new() {
                    Ok(renderer) => renderer,
                    Err(error) => {
                        worker_output.publish(|output| {
                            output.render_error = Some((0, format!("Metal renderer: {error:#}")));
                        });
                        return;
                    }
                };
                worker_output.publish(|output| output.device = Some(renderer.device_name()));
                let mut active: Option<Request> = None;
                let mut active_snapshot: Option<Arc<Scene>> = None;
                let mut scene: Option<Scene> = None;
                let mut complete = true;
                loop {
                    let (lock, wake) = &*worker_input;
                    let mut inbox = lock.lock().unwrap();
                    while complete && inbox.request.is_none() && !inbox.stop {
                        inbox = wake.wait(inbox).unwrap();
                    }
                    if inbox.stop {
                        break;
                    }
                    let request = inbox.request.take();
                    drop(inbox);
                    if let Some(request) = request {
                        if active_snapshot
                            .as_ref()
                            .is_none_or(|old| !Arc::ptr_eq(old, &request.scene))
                        {
                            scene = Some((*request.scene).clone());
                            active_snapshot = Some(request.scene.clone());
                        }
                        if let Some(scene) = &mut scene {
                            scene.camera = request.camera;
                            scene.world = request.world.clone();
                        }
                        active = Some(request);
                    }
                    let Some(request) = &active else {
                        continue;
                    };
                    match renderer.render(
                        scene.as_ref().unwrap(),
                        &request.settings,
                        request.revision,
                    ) {
                        Ok(frame) => {
                            complete = !request.settings.mode.progressive()
                                || frame.samples >= request.settings.max_samples;
                            worker_output.publish(|output| {
                                output.frame = Some((request.generation, frame));
                                output.render_error = None;
                            });
                        }
                        Err(error) => {
                            complete = true;
                            worker_output.publish(|output| {
                                output.render_error =
                                    Some((request.generation, format!("Render failed: {error:#}")));
                            });
                        }
                    }
                }
            })
            .expect("could not start render worker");
        (Self { input, output }, notifications)
    }

    pub fn take_output(&self) -> Output {
        self.output.take()
    }

    pub fn request(&self, request: Request) {
        let (lock, wake) = &*self.input;
        lock.lock().unwrap().request = Some(request);
        wake.notify_one();
    }

    /// Exports use a separate renderer and immutable scene snapshot. Editing can continue.
    pub fn export(&self, mut request: Request, path: PathBuf) {
        request.settings.selected = None;
        request.settings.show_grid = false;
        let output = self.output.clone();
        thread::Builder::new()
            .name("forma-image-export".into())
            .spawn(move || {
                let result = (|| -> anyhow::Result<()> {
                    let mut scene = (*request.scene).clone();
                    scene.camera = request.camera;
                    scene.world = request.world.clone();
                    let mut renderer = Renderer::new()?;
                    loop {
                        let frame = renderer.render(&scene, &request.settings, request.revision)?;
                        if !request.settings.mode.progressive()
                            || frame.samples >= request.settings.max_samples
                        {
                            break;
                        }
                    }
                    renderer.export_png(&path)
                })();
                output.publish(|output| {
                    output.messages.push_back(Ok(match result {
                        Ok(()) => format!("Image saved · {}", path.display()),
                        Err(error) => format!("Image export failed: {error:#}"),
                    }));
                    output.export_finished = true;
                });
            })
            .expect("could not start image export");
    }
}

impl Drop for RenderWorker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.input;
        if let Ok(mut inbox) = lock.lock() {
            inbox.stop = true;
        }
        wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_notifications_coalesce_without_losing_latest_state_or_messages() {
        let (mailbox, notifications) = OutputMailbox::new();
        assert!(notifications.try_recv().is_err());
        for generation in 1..=100 {
            mailbox.publish(|output| {
                output.render_error = Some((generation, "latest".into()));
                output.messages.push_back(Ok(generation.to_string()));
            });
        }
        assert_eq!(notifications.len(), 1);
        notifications.try_recv().unwrap();
        let output = mailbox.take();
        assert_eq!(output.render_error.unwrap().0, 100);
        assert_eq!(output.messages.len(), 100);
        assert!(mailbox.take().messages.is_empty());

        mailbox.publish(|output| output.export_finished = true);
        notifications.try_recv().unwrap();
        assert!(mailbox.take().export_finished);
        assert!(notifications.try_recv().is_err());
    }
}

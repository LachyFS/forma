//! A latest-request mailbox keeps input responsive when rendering is slower than input.
use forma_core::{Camera, Scene, World};
use forma_render::{
    Backend, DenoiseInput, DenoiseQuality, Denoiser, Frame, RenderSettings, Renderer,
};
use gpui::RenderImage;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
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
    pub frame: Option<(u64, Frame, Option<Arc<RenderImage>>)>,
    pub device: Option<String>,
    pub denoised: Option<(u64, Frame, Arc<RenderImage>)>,
    pub denoise_error: Option<(u64, String)>,
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
    backend: Backend,
    denoise: DenoiseQueue,
}

impl RenderWorker {
    pub fn new(backend: Backend) -> (Self, async_channel::Receiver<()>) {
        let input = Arc::new((Mutex::new(Inbox::default()), Condvar::new()));
        let (output, notifications) = OutputMailbox::new();
        let denoise = DenoiseQueue::new(output.clone());
        let worker_denoise = denoise.clone();
        let worker_input = input.clone();
        let worker_output = output.clone();
        thread::Builder::new()
            .name("forma-viewport".into())
            .spawn(move || {
                let mut renderer = match Renderer::with_backend(backend) {
                    Ok(renderer) => renderer,
                    Err(error) => {
                        worker_output.publish(|output| {
                            output.render_error = Some((0, format!("GPU renderer: {error:#}")));
                        });
                        return;
                    }
                };
                worker_output.publish(|output| {
                    output.device = Some(format!(
                        "{} · {}",
                        renderer.backend(),
                        renderer.device_name()
                    ));
                });
                let mut active: Option<Request> = None;
                let mut active_snapshot: Option<Arc<Scene>> = None;
                let mut scene: Option<Scene> = None;
                let mut complete = true;
                let mut last_denoise_sample = 0;
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
                        last_denoise_sample = 0;
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
                    let rendered = renderer
                        .render(scene.as_ref().unwrap(), &request.settings, request.revision)
                        .and_then(|frame| {
                            // GPUI's portable image path uses BGRA. Prepare it on the
                            // worker so megapixel channel conversion never blocks input.
                            let image = frame
                                .rgba()
                                .map(|rgba| presentation_image(frame.width(), frame.height(), rgba))
                                .transpose()?;
                            Ok((frame, image))
                        });
                    match rendered {
                        Ok((frame, image)) => {
                            complete = !request.settings.mode.progressive()
                                || frame.samples >= request.settings.max_samples;
                            let samples = frame.samples;
                            worker_output.publish(|output| {
                                output.frame = Some((request.generation, frame, image));
                                output.render_error = None;
                            });
                            if request.settings.mode.progressive()
                                && request.settings.denoise.viewport
                                && denoise_due(
                                    samples,
                                    request.settings.max_samples,
                                    request.settings.denoise.start_sample,
                                    last_denoise_sample,
                                )
                                && (complete || worker_denoise.idle())
                                && worker_denoise.current.load(Ordering::Acquire)
                                    == request.generation
                            {
                                match renderer.read_denoise_input() {
                                    Ok(input) => {
                                        last_denoise_sample = samples;
                                        worker_denoise.submit(DenoiseJob {
                                            generation: request.generation,
                                            input,
                                            quality: request.settings.denoise.quality,
                                            exposure: request.settings.exposure,
                                        });
                                    }
                                    Err(error) => {
                                        last_denoise_sample = samples;
                                        worker_output.publish(|output| {
                                            output.denoise_error =
                                                Some((request.generation, format!("{error:#}")))
                                        });
                                    }
                                }
                            }
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
        (
            Self {
                input,
                output,
                backend,
                denoise,
            },
            notifications,
        )
    }

    pub fn take_output(&self) -> Output {
        self.output.take()
    }

    pub fn request(&self, request: Request) {
        self.denoise.set_generation(request.generation);
        let (lock, wake) = &*self.input;
        lock.lock().unwrap().request = Some(request);
        wake.notify_one();
    }

    /// Exports use a separate renderer and immutable scene snapshot. Editing can continue.
    pub fn export(&self, mut request: Request, path: PathBuf) {
        request.settings.selected = None;
        request.settings.show_grid = false;
        let output = self.output.clone();
        let backend = self.backend;
        thread::Builder::new()
            .name("forma-image-export".into())
            .spawn(move || {
                let result = (|| -> anyhow::Result<()> {
                    let mut scene = (*request.scene).clone();
                    scene.camera = request.camera;
                    scene.world = request.world.clone();
                    let mut renderer = Renderer::with_backend(backend)?;
                    loop {
                        let frame = renderer.render(&scene, &request.settings, request.revision)?;
                        if !request.settings.mode.progressive()
                            || frame.samples >= request.settings.max_samples
                        {
                            break;
                        }
                    }
                    if request.settings.mode.progressive() && request.settings.denoise.render {
                        output.publish(|output| {
                            output
                                .messages
                                .push_back(Ok("Denoising image · High quality".into()))
                        });
                        renderer.export_denoised_png(&path, &mut Denoiser::new()?)
                    } else {
                        renderer.export_png(&path)
                    }
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

/// Geometric cadence amortizes inference and readback. The final sample always
/// gets a pass, even when the target is below Start Sample or is not a power of 2.
fn denoise_due(samples: u32, target: u32, start: u32, last: u32) -> bool {
    samples > last
        && (samples >= target.max(1)
            || (samples >= start.max(1) && (last == 0 || samples >= last.saturating_mul(2))))
}

struct DenoiseJob {
    generation: u64,
    input: DenoiseInput,
    quality: DenoiseQuality,
    exposure: f32,
}

#[derive(Default)]
struct DenoiseInbox {
    pending: Option<DenoiseJob>,
    running: bool,
    stop: bool,
}

#[derive(Clone)]
struct DenoiseQueue {
    inbox: Arc<(Mutex<DenoiseInbox>, Condvar)>,
    current: Arc<AtomicU64>,
}

impl DenoiseQueue {
    fn new(output: OutputMailbox) -> Self {
        let queue = Self {
            inbox: Arc::new((Mutex::new(DenoiseInbox::default()), Condvar::new())),
            current: Arc::new(AtomicU64::new(0)),
        };
        let worker = queue.clone();
        thread::Builder::new()
            .name("forma-viewport-denoise".into())
            .spawn(move || {
                // Initialize lazily, once, on this thread. Native handles never cross
                // threads. A missing runtime is reported without stopping the tracer.
                let mut denoiser: Option<Result<Denoiser, String>> = None;
                loop {
                    let (lock, wake) = &*worker.inbox;
                    let mut inbox = lock.lock().unwrap();
                    while inbox.pending.is_none() && !inbox.stop {
                        inbox = wake.wait(inbox).unwrap();
                    }
                    if inbox.stop {
                        break;
                    }
                    let job = inbox.pending.take().unwrap();
                    inbox.running = true;
                    drop(inbox);
                    if worker.current.load(Ordering::Acquire) == job.generation {
                        let result = match denoiser
                            .get_or_insert_with(|| Denoiser::new().map_err(|e| format!("{e:#}")))
                        {
                            Ok(denoiser) => denoiser
                                .denoise_frame(&job.input, job.quality, false, job.exposure)
                                .and_then(|frame| {
                                    let image = presentation_image(
                                        frame.width(),
                                        frame.height(),
                                        frame.rgba().unwrap(),
                                    )?;
                                    Ok((frame, image))
                                })
                                .map_err(|e| format!("{e:#}")),
                            Err(error) => Err(error.clone()),
                        };
                        if worker.current.load(Ordering::Acquire) == job.generation {
                            output.publish(|output| match result {
                                Ok((frame, image)) => {
                                    output.denoised = Some((job.generation, frame, image))
                                }
                                Err(error) => output.denoise_error = Some((job.generation, error)),
                            });
                        }
                    }
                    lock.lock().unwrap().running = false;
                }
            })
            .expect("could not start denoising worker");
        queue
    }

    fn set_generation(&self, generation: u64) {
        self.current.store(generation, Ordering::Release);
        let mut inbox = self.inbox.0.lock().unwrap();
        if inbox
            .pending
            .as_ref()
            .is_some_and(|job| job.generation != generation)
        {
            inbox.pending = None;
        }
    }

    fn idle(&self) -> bool {
        let inbox = self.inbox.0.lock().unwrap();
        !inbox.running && inbox.pending.is_none()
    }

    fn submit(&self, job: DenoiseJob) {
        let (lock, wake) = &*self.inbox;
        let mut inbox = lock.lock().unwrap();
        if !inbox.stop && self.current.load(Ordering::Acquire) == job.generation {
            inbox.pending = Some(job); // bounded: one inference + one latest snapshot
            wake.notify_one();
        }
    }

    fn stop(&self) {
        self.current.store(u64::MAX, Ordering::Release);
        let (lock, wake) = &*self.inbox;
        let mut inbox = lock.lock().unwrap();
        inbox.stop = true;
        inbox.pending = None;
        wake.notify_one();
    }
}

fn presentation_image(width: u32, height: u32, rgba: &[u8]) -> anyhow::Result<Arc<RenderImage>> {
    let expected = u64::from(width) * u64::from(height) * 4;
    anyhow::ensure!(
        width > 0 && height > 0 && expected == rgba.len() as u64,
        "GPU frame dimensions do not match its RGBA pixels"
    );
    let mut bgra = rgba.to_vec();
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let pixels = image::RgbaImage::from_raw(width, height, bgra)
        .ok_or_else(|| anyhow::anyhow!("could not prepare the GPU frame for display"))?;
    Ok(Arc::new(RenderImage::new(smallvec::smallvec![
        image::Frame::new(pixels)
    ])))
}

impl Drop for RenderWorker {
    fn drop(&mut self) {
        self.denoise.stop();
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
    fn denoising_cadence_always_finishes_and_handles_resumed_targets() {
        let mut last = 0;
        let scheduled: Vec<_> = (1..=37)
            .filter(|&sample| {
                if denoise_due(sample, 37, 8, last) {
                    last = sample;
                    true
                } else {
                    false
                }
            })
            .collect();
        assert_eq!(scheduled, [8, 16, 32, 37]);
        assert!(!denoise_due(37, 37, 8, last));
        assert!(denoise_due(4, 4, 8, 0));
        assert!(denoise_due(38, 38, 8, last));
        assert!(!denoise_due(1, 128, 8, 0));
    }

    #[test]
    fn denoise_mailbox_discards_old_generations_and_bounds_pending_work() {
        let queue = DenoiseQueue {
            inbox: Arc::new((Mutex::new(DenoiseInbox::default()), Condvar::new())),
            current: Arc::new(AtomicU64::new(1)),
        };
        let job = |generation, samples| DenoiseJob {
            generation,
            quality: DenoiseQuality::Balanced,
            exposure: 0.0,
            input: DenoiseInput {
                width: 1,
                height: 1,
                samples,
                shader_error: None,
                color: vec![],
                albedo: vec![],
                normal: vec![],
            },
        };
        queue.submit(job(1, 8));
        queue.submit(job(1, 16));
        assert_eq!(
            queue
                .inbox
                .0
                .lock()
                .unwrap()
                .pending
                .as_ref()
                .unwrap()
                .input
                .samples,
            16
        );
        queue.set_generation(2);
        assert!(queue.idle());
        queue.submit(job(1, 32));
        assert!(queue.idle());
        queue.submit(job(2, 8));
        assert!(!queue.idle());
        queue.stop();
        queue.submit(job(2, 16));
        assert!(queue.inbox.0.lock().unwrap().pending.is_none());
    }

    #[test]
    fn portable_presentation_keeps_dimensions_alpha_and_bgra_channel_order() {
        let rgba = [255, 80, 10, 255, 12, 34, 56, 128];
        let image = presentation_image(2, 1, &rgba).unwrap();
        assert_eq!(image.size(0), gpui::size(2.into(), 1.into()));
        assert_eq!(
            image.as_bytes(0).unwrap(),
            &[10, 80, 255, 255, 56, 34, 12, 128]
        );
        assert_eq!(rgba[0], 255);
        assert!(presentation_image(3, 1, &rgba).is_err());
        assert!(presentation_image(0, 0, &[]).is_err());
    }

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

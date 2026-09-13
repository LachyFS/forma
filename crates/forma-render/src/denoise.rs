//! Open Image Denoise 2.4+ through its stable C API. Loading is optional so a
//! missing runtime never prevents editing or raw rendering. All device memory
//! is owned by OIDN; explicit copies work on CPU, Metal, CUDA, HIP and SYCL.
use crate::{DenoiseQuality, Frame};
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::{CStr, c_char, c_int, c_void},
    path::PathBuf,
    ptr,
    sync::Arc,
    time::Instant,
};

type Handle = *mut c_void;
pub(crate) type GuidePixels = (Vec<[f32; 4]>, Vec<[f32; 4]>);

/// Divide an accumulated film value by its sample weight, leaving alpha at 1.
pub(crate) fn normalized(value: [f32; 4], weight: f32) -> [f32; 4] {
    let weight = weight.max(1.0);
    [value[0] / weight, value[1] / weight, value[2] / weight, 1.0]
}

/// Both guides are written once per sample, so albedo's alpha is the weight for
/// the pair. That frees the normal's alpha to carry the selection outline
/// coverage of the same pixel, which denoising must composite back afterwards.
pub(crate) fn normalized_guides(albedo: [f32; 4], normal: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    let mut guide_normal = normalized(normal, albedo[3]);
    guide_normal[3] = normal[3];
    (normalized(albedo, albedo[3]), guide_normal)
}

/// An immutable, normalized snapshot. Guides use the same jitter and sample
/// weights as beauty; normals stay in world space, including negative values.
/// `selection` holds the viewport outline coverage, which is a display overlay
/// and never part of the film, and is empty when nothing is selected.
pub struct DenoiseInput {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub shader_error: Option<String>,
    pub color: Vec<[f32; 4]>,
    pub albedo: Vec<[f32; 4]>,
    pub normal: Vec<[f32; 4]>,
    pub selection: Vec<f32>,
}

impl DenoiseInput {
    fn validate(&self) -> Result<()> {
        ensure!(
            (1..=8192).contains(&self.width)
                && (1..=8192).contains(&self.height)
                && self.samples > 0,
            "Invalid denoising dimensions or sample count"
        );
        let count = self.width as usize * self.height as usize;
        for (name, pixels, min, max) in [
            ("color", &self.color, 0.0, f32::MAX),
            ("albedo", &self.albedo, 0.0, 1.0),
            ("normal", &self.normal, -1.0, 1.0),
        ] {
            ensure!(
                pixels.len() == count,
                "Denoising {name} dimensions do not match"
            );
            ensure!(
                pixels.iter().all(|p| p[..3]
                    .iter()
                    .all(|v| v.is_finite() && *v >= min && *v <= max)),
                "Denoising {name} contains invalid values"
            );
        }
        ensure!(
            self.selection.is_empty() || self.selection.len() == count,
            "Denoising selection overlay dimensions do not match"
        );
        ensure!(
            self.selection.iter().all(|v| (0.0..=1.0).contains(v)),
            "Denoising selection overlay contains invalid coverage"
        );
        Ok(())
    }
}

// Signatures and enum values from OpenImageDenoise/oidn.h, v2.4.1. Keep this
// small boundary private. No borrowed host pointers survive a synchronous call.
macro_rules! api {
    ($($field:ident: $ty:ty = $symbol:literal),* $(,)?) => {
        struct Api { $($field: $ty,)* _library: libloading::Library }
        impl Api {
            unsafe fn load(library: libloading::Library) -> Result<Self> {
                // SAFETY: each symbol is loaded with the corresponding C ABI signature.
                unsafe { Ok(Self { $($field: *library.get(concat!($symbol, "\0").as_bytes())?,)* _library: library }) }
            }
        }
    }
}
api! {
    new_device: unsafe extern "C" fn(c_int) -> Handle = "oidnNewDevice",
    commit_device: unsafe extern "C" fn(Handle) = "oidnCommitDevice",
    release_device: unsafe extern "C" fn(Handle) = "oidnReleaseDevice",
    get_device_int: unsafe extern "C" fn(Handle, *const c_char) -> c_int = "oidnGetDeviceInt",
    set_device_int: unsafe extern "C" fn(Handle, *const c_char, c_int) = "oidnSetDeviceInt",
    get_error: unsafe extern "C" fn(Handle, *mut *const c_char) -> c_int = "oidnGetDeviceError",
    new_buffer: unsafe extern "C" fn(Handle, usize) -> Handle = "oidnNewBuffer",
    release_buffer: unsafe extern "C" fn(Handle) = "oidnReleaseBuffer",
    write_buffer: unsafe extern "C" fn(Handle, usize, usize, *const c_void) = "oidnWriteBuffer",
    read_buffer: unsafe extern "C" fn(Handle, usize, usize, *mut c_void) = "oidnReadBuffer",
    new_filter: unsafe extern "C" fn(Handle, *const c_char) -> Handle = "oidnNewFilter",
    release_filter: unsafe extern "C" fn(Handle) = "oidnReleaseFilter",
    set_image: unsafe extern "C" fn(Handle, *const c_char, Handle, c_int, usize, usize, usize, usize, usize) = "oidnSetFilterImage",
    set_bool: unsafe extern "C" fn(Handle, *const c_char, bool) = "oidnSetFilterBool",
    set_int: unsafe extern "C" fn(Handle, *const c_char, c_int) = "oidnSetFilterInt",
    commit_filter: unsafe extern "C" fn(Handle) = "oidnCommitFilter",
    execute_filter: unsafe extern "C" fn(Handle) = "oidnExecuteFilter",
}

impl Api {
    fn check(&self, device: Handle) -> Result<()> {
        let mut message = ptr::null();
        // SAFETY: device is live (or null for creation errors); OIDN owns the
        // returned string until the next error query, and we copy it immediately.
        unsafe {
            let code = (self.get_error)(device, &mut message);
            if code != 0 {
                let message = if message.is_null() {
                    "Unknown error".into()
                } else {
                    CStr::from_ptr(message).to_string_lossy()
                };
                bail!("Open Image Denoise: {message} ({code})");
            }
        }
        Ok(())
    }
}

fn runtime_candidates() -> Vec<PathBuf> {
    // An explicit override is authoritative, including a missing/invalid path.
    if let Some(path) = std::env::var_os("FORMA_OIDN_LIBRARY") {
        return vec![path.into()];
    }
    let name = if cfg!(target_os = "windows") {
        "OpenImageDenoise.dll"
    } else if cfg!(target_os = "macos") {
        "libOpenImageDenoise.2.dylib"
    } else {
        "libOpenImageDenoise.so.2"
    };
    let library_dir = if cfg!(target_os = "windows") {
        "bin"
    } else {
        "lib"
    };
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        paths.push(dir.join("oidn").join(library_dir).join(name));
        // Cargo integration test / example executables live one level down.
        if matches!(
            dir.file_name().and_then(|n| n.to_str()),
            Some("deps" | "examples")
        ) && let Some(profile) = dir.parent()
        {
            paths.push(profile.join("oidn").join(library_dir).join(name));
        }
    }
    paths.push(name.into());
    paths
}

fn load_api() -> Result<Arc<Api>> {
    let mut errors = Vec::new();
    for path in runtime_candidates() {
        // SAFETY: loading a native library executes its initialization code. Only
        // explicit, executable-relative, or OS loader paths are considered (no CWD scan).
        let loaded = unsafe {
            #[cfg(target_os = "windows")]
            let library =
                libloading::os::windows::Library::load_with_flags(&path, 0x00000100 | 0x00001000)
                    .map(libloading::Library::from);
            #[cfg(not(target_os = "windows"))]
            let library = libloading::Library::new(&path);
            library
                .map_err(anyhow::Error::from)
                .and_then(|library| Api::load(library))
        };
        match loaded {
            Ok(api) => return Ok(Arc::new(api)),
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    bail!(
        "Open Image Denoise runtime unavailable. Run scripts/setup-denoiser.py or set FORMA_OIDN_LIBRARY to the OIDN 2.4+ library. {}",
        errors.join("; ")
    )
}

/// Create and use on a background thread. Device, weights, filters and buffers
/// are reused across viewport updates; resolution/quality changes rebuild them.
pub struct Denoiser {
    api: Arc<Api>,
    device: Handle,
    pub device_name: String,
    session: Option<Session>,
}

struct Session {
    api: Arc<Api>,
    key: (u32, u32, DenoiseQuality, bool),
    buffers: [Handle; 4],
    filters: Vec<Handle>, // optional albedo/normal prefilters, then beauty
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: synchronous operations have completed; release filters before
        // their buffers. The Arc retains the library throughout destruction.
        unsafe {
            for filter in &self.filters {
                (self.api.release_filter)(*filter);
            }
            for buffer in &self.buffers {
                if !buffer.is_null() {
                    (self.api.release_buffer)(*buffer);
                }
            }
        }
    }
}
impl Drop for Denoiser {
    fn drop(&mut self) {
        self.session.take();
        // SAFETY: the device remains live until all its resources are released.
        unsafe {
            (self.api.release_device)(self.device);
        }
    }
}

impl Denoiser {
    pub fn new() -> Result<Self> {
        let api = load_api()?;
        let mut last_error = None;
        // DEFAULT chooses the likely fastest supported device. CPU fallback
        // handles systems with an installed GPU module but an unusable driver.
        for kind in [0, 1] {
            // SAFETY: all calls use this API's handles and valid static C strings.
            unsafe {
                let device = (api.new_device)(kind);
                if device.is_null() {
                    last_error = Some(
                        api.check(device)
                            .err()
                            .unwrap_or_else(|| anyhow::anyhow!("No denoising device")),
                    );
                    continue;
                }
                let result = (|| {
                    ensure!(
                        (api.get_device_int)(device, c"version".as_ptr()) >= 20400,
                        "Open Image Denoise 2.4 or newer is required"
                    );
                    let device_type = (api.get_device_int)(device, c"type".as_ptr());
                    if device_type == 1 {
                        // Leave CPU capacity for navigation, geometry and presentation.
                        let threads = std::thread::available_parallelism()
                            .map_or(1, |n| (n.get() / 2).max(1));
                        (api.set_device_int)(device, c"numThreads".as_ptr(), threads as c_int);
                    }
                    (api.commit_device)(device);
                    api.check(device)?;
                    Ok(match device_type {
                        1 => "CPU",
                        2 => "Intel GPU",
                        3 => "NVIDIA GPU",
                        4 => "AMD GPU",
                        5 => "Apple GPU",
                        _ => "Auto",
                    }
                    .to_string())
                })();
                match result {
                    Ok(device_name) => {
                        return Ok(Self {
                            api,
                            device,
                            device_name,
                            session: None,
                        });
                    }
                    Err(error) => {
                        (api.release_device)(device);
                        last_error = Some(error);
                    }
                }
            }
        }
        Err(last_error.unwrap()).context("Could not initialize AI denoising")
    }

    pub fn denoise(
        &mut self,
        input: &DenoiseInput,
        quality: DenoiseQuality,
        prefilter: bool,
    ) -> Result<Vec<[f32; 4]>> {
        input.validate()?;
        // With no incident/emitted light the correct image is exactly black.
        // Avoid neural reconstruction bias and undefined automatic HDR scaling
        // on a zero-energy film (e.g. a black world with no emitters).
        if input.color.iter().all(|pixel| pixel[..3] == [0.0; 3]) {
            return Ok(vec![[0.0, 0.0, 0.0, 1.0]; input.color.len()]);
        }
        let key = (input.width, input.height, quality, prefilter);
        if self.session.as_ref().is_none_or(|s| s.key != key) {
            self.session = None;
            self.session = Some(self.create_session(key)?);
        }
        let session = self.session.as_ref().unwrap();
        let size = input.color.len() * 16;
        let mut output = vec![[0.0, 0.0, 0.0, 1.0]; input.color.len()];
        // SAFETY: lengths are validated, buffers have exactly size bytes, host
        // allocations live through blocking copies, and filters own no host pointers.
        unsafe {
            for (buffer, data) in
                session
                    .buffers
                    .iter()
                    .zip([&input.color, &input.albedo, &input.normal])
            {
                (self.api.write_buffer)(*buffer, 0, size, data.as_ptr().cast());
            }
            self.api.check(self.device)?;
            for filter in &session.filters {
                (self.api.execute_filter)(*filter);
                self.api.check(self.device)?;
            }
            (self.api.read_buffer)(session.buffers[3], 0, size, output.as_mut_ptr().cast());
            self.api.check(self.device)?;
        }
        ensure!(
            output
                .iter()
                .all(|p| p[..3].iter().all(|v: &f32| v.is_finite())),
            "Denoiser returned non-finite pixels"
        );
        for pixel in &mut output {
            for value in &mut pixel[..3] {
                *value = value.max(0.0);
            }
            pixel[3] = 1.0;
        }
        Ok(output)
    }

    pub fn denoise_frame(
        &mut self,
        input: &DenoiseInput,
        quality: DenoiseQuality,
        prefilter: bool,
        exposure: f32,
    ) -> Result<Frame> {
        ensure!(exposure.is_finite(), "Exposure must be finite");
        let started = Instant::now();
        let pixels = self.denoise(input, quality, prefilter)?;
        let mut rgba = display_pixels(&pixels, exposure);
        composite_selection(&mut rgba, &input.selection);
        Ok(Frame::from_denoised(
            input,
            rgba,
            self.device_name.clone(),
            started.elapsed().as_secs_f64() * 1000.0,
        ))
    }

    fn create_session(&self, key: (u32, u32, DenoiseQuality, bool)) -> Result<Session> {
        let (width, height, quality, prefilter) = key;
        let mut session = Session {
            api: self.api.clone(),
            key,
            buffers: [ptr::null_mut(); 4],
            filters: Vec::new(),
        };
        // SAFETY: handles are checked before use and owned by Session, including
        // during partial initialization failures. FLOAT3 uses a 16-byte pixel stride.
        unsafe {
            for buffer in &mut session.buffers {
                *buffer = (self.api.new_buffer)(self.device, width as usize * height as usize * 16);
                self.api.check(self.device)?;
                ensure!(!buffer.is_null(), "Could not allocate denoising buffer");
            }
            let bind = |filter, name: &CStr, index: usize| {
                (self.api.set_image)(
                    filter,
                    name.as_ptr(),
                    session.buffers[index],
                    3,
                    width as usize,
                    height as usize,
                    0,
                    16,
                    width as usize * 16,
                );
            };
            if prefilter {
                for (name, index) in [(c"albedo", 1), (c"normal", 2)] {
                    let filter = (self.api.new_filter)(self.device, c"RT".as_ptr());
                    ensure!(!filter.is_null(), "Could not create guide prefilter");
                    session.filters.push(filter);
                    bind(filter, name, index);
                    bind(filter, c"output", index);
                    (self.api.set_int)(filter, c"quality".as_ptr(), 6);
                    (self.api.commit_filter)(filter);
                    self.api.check(self.device)?;
                }
            }
            let filter = (self.api.new_filter)(self.device, c"RT".as_ptr());
            ensure!(!filter.is_null(), "Could not create denoising filter");
            session.filters.push(filter);
            for (name, index) in [
                (c"color", 0),
                (c"albedo", 1),
                (c"normal", 2),
                (c"output", 3),
            ] {
                bind(filter, name, index);
            }
            (self.api.set_bool)(filter, c"hdr".as_ptr(), true);
            (self.api.set_bool)(filter, c"cleanAux".as_ptr(), prefilter);
            (self.api.set_int)(
                filter,
                c"quality".as_ptr(),
                match quality {
                    DenoiseQuality::Fast => 4,
                    DenoiseQuality::Balanced => 5,
                    DenoiseQuality::High => 6,
                },
            );
            (self.api.commit_filter)(filter);
            self.api.check(self.device)?;
        }
        Ok(session)
    }
}

// Same ACES fit and sRGB OETF as both shaders. Use f64 intermediates to avoid
// overflow for bright, valid HDR values. Denoising always precedes this transform.
// Both shaders composite the selection outline over the tone mapped image at a
// fixed exposure; a denoised frame replaces every displayed pixel and so has to
// redraw the same overlay, from the same scene-linear color, to match them.
fn composite_selection(rgba: &mut [u8], coverage: &[f32]) {
    let outline = display_pixels(&[[0.055, 0.40, 0.95, 1.0]], 0.0);
    for (pixel, coverage) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(coverage) {
        for (value, line) in pixel[..3].iter_mut().zip(&outline) {
            *value = (f32::from(*value) + (f32::from(*line) - f32::from(*value)) * coverage).round()
                as u8;
        }
    }
}

fn display_pixels(pixels: &[[f32; 4]], exposure: f32) -> Vec<u8> {
    let scale = 2.0_f64.powf(exposure.clamp(-20.0, 20.0) as f64);
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        for value in &pixel[..3] {
            let x = f64::from(value.max(0.0)) * scale;
            let mapped = ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0);
            let srgb = if mapped <= 0.0031308 {
                12.92 * mapped
            } else {
                1.055 * mapped.powf(1.0 / 2.4) - 0.055
            };
            rgba.push((srgb * 255.0).round() as u8);
        }
        rgba.push(255);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_guides_before_native_calls_and_handles_hdr_display() {
        let mut input = DenoiseInput {
            width: 1,
            height: 1,
            samples: 1,
            shader_error: None,
            color: vec![[1.0; 4]],
            albedo: vec![[0.5; 4]],
            normal: vec![[-1.0, 0.0, 0.0, 1.0]],
            selection: vec![0.5],
        };
        input.validate().unwrap();
        input.selection[0] = 1.5;
        assert!(input.validate().is_err());
        input.selection = vec![0.5; 2];
        assert!(input.validate().is_err());
        input.selection.clear();
        input.validate().unwrap();
        input.normal[0][0] = -1.1;
        assert!(input.validate().is_err());
        input.normal.clear();
        assert!(input.validate().is_err());
        assert_eq!(
            display_pixels(&[[0.0, 1.0, f32::MAX, 1.0]], 0.0),
            [0, 232, 255, 255]
        );
    }

    /// The outline is opaque enough to read over any image, and pixels outside
    /// it keep the denoised result exactly.
    #[test]
    fn overlay_composites_only_where_the_outline_covers() {
        let outline = display_pixels(&[[0.055, 0.40, 0.95, 1.0]], 0.0);
        let mut rgba = vec![10, 20, 30, 255, 10, 20, 30, 255];
        composite_selection(&mut rgba, &[0.0, 1.0]);
        assert_eq!(rgba[..4], [10, 20, 30, 255]);
        assert_eq!(rgba[4..7], outline[..3]);
        assert_eq!(rgba[7], 255);
    }
}

# AI denoising

Forma uses **Open Image Denoise 2.4.1**, the open-source neural ray-tracing
filter also used by Blender. It works across the native Metal and wgpu renderers.
The Render inspector exposes independent **Viewport** and **Render export**
toggles, a viewport **Start sample** (8 by default), and Fast / Balanced / High
viewport quality. Both toggles default to on. Preferences are saved in `.forma`
projects and participate in undo/redo; older documents receive the defaults.

## Why Open Image Denoise

[Blender's sampling controls](https://docs.blender.org/manual/en/4.5/render/cycles/render_settings/sampling.html)
provide viewport and render denoising with guide passes and quality settings.
[OIDN's RT filter](https://www.openimagedenoise.org/documentation.html#rt)
accepts scene-linear HDR color, albedo, and signed shading normals, and supports
CPU and Apple, NVIDIA, AMD, and Intel GPUs. This makes it a better common backend
for Forma than the NVIDIA-specific OptiX denoiser. A hand-written spatial blur
would not provide the trained reconstruction that the requested AI workflow needs.

## Runtime installation and distribution

Normal platform bundle scripts include the complete official runtime, device
modules, and license notices. Building those bundles requires Python 3.12+ and
network access on the first run. The installer pins the official release and
verifies its published SHA-256 digest before unpacking; archives are cached under
`target/oidn-cache`. Application startup and rendering make no network requests.

For source development:

```sh
python3 scripts/setup-denoiser.py
cargo run --locked -p forma
```

For a release build, use `--output-dir target/release/oidn`. The default respects
`CARGO_TARGET_DIR`. The installed layout is `<executable-dir>/oidn/lib` on Unix
and `<executable-dir>/oidn/bin` on Windows. Preserve these directory names when
moving the runtime: upstream libraries depend on their relative paths. Tests in
Cargo's `deps` directory also find the parent profile's runtime.

A custom OIDN 2.4+ installation can be selected with `FORMA_OIDN_LIBRARY`, set to
the **full path to the main library**, including its filename. The override is
strict; a broken override does not silently load another runtime. Supported
prebuilt host packages are Linux x86-64, Windows x64, and macOS Apple Silicon
and Intel. Other architectures require a source-built OIDN runtime.

OIDN selects its likely fastest supported device and falls back to CPU if GPU
initialization fails. CPU inference uses half the available CPU threads to leave
capacity for the editor. An unavailable runtime leaves the viewport rendering
original samples and reports its status in the inspector. A requested denoised
export fails explicitly instead of silently saving an undenoised image. Install
or repair the runtime and restart Forma, or disable Render export denoising to
save original samples.

## Render and scheduling contract

Both shaders accumulate albedo and world-space shading normals from the exact
same jittered primary hits as beauty. Misses use unit albedo and zero normals;
surface albedo uses the evaluated material color, including image textures,
custom shader output and metallic reflectivity. Glass uses unit albedo. Shading
normals include normal maps and custom shader changes. Compiler diagnostics are
preserved on denoised frames. The portable backend packs both guides into one
storage binding to remain within the eight-buffer limit alongside material data. Normals retain their signed components. Guide sums reset,
resize, and resume with the original Monte Carlo film. Preview modes use tiny
placeholder bindings and perform no guide work.

The renderer reads normalized float passes only at denoising checkpoints. The
viewport queues the first eligible sample, then approximately doubles the sample
count between passes, and always queues the final sample (even if the target is
below Start sample). If inference is busy, intermediate checkpoints are skipped.
One dedicated thread owns a persistent OIDN device, model/filter state and reusable
buffers; the mailbox holds at most one active and one pending image. Camera/scene
changes discard pending snapshots and reject old completions. In-flight inference
can finish without blocking navigation or the render worker.

Once a clean image arrives, it stays visible while the original film refines;
the UI reports both the accumulated sample count and the displayed denoising
sample count. Turning denoising off restores the original film, including at the
sample cap, without retracing. Changing denoising settings does not enter the
accumulation identity. Final export uses an independent scene/renderer snapshot,
High quality, and separately prefiltered albedo/normal passes with `cleanAux=true`.
Viewport guides are noisy antialiased estimates, so `cleanAux=false` is essential.

Denoising happens **before exposure, the ACES fit and sRGB encoding**. Only display
pixels change. `Renderer::render`, `read_linear_pixels`, and `export_png` retain
their raw contracts. `read_denoise_input`, `Denoiser::denoise_frame` and
`Renderer::export_denoised_png` expose explicit denoising for other consumers.

Denoised display frames own immutable RGBA pixels, including on macOS. Native raw
frames retain the existing zero-copy NV12 path; denoising checkpoints incur float
readback and clean-frame upload. OIDN uses its own device buffers, with explicit
copies for cross-device compatibility. Zero-copy Metal/Vulkan interop is a future
optimization. The current filter is spatial, not temporally stabilized, and can
soften reflections, details seen through glass, or small details at very low
sample counts. Guide capture uses the first surface hit. More samples still
improve quality; adaptive sampling and temporal denoising are not implemented.

## Reproducible checks

```sh
python3 scripts/setup-denoiser.py
cargo test --locked --workspace -- --test-threads=1
cargo test --locked -p forma-render --test denoise_runtime -- --ignored
cargo run --locked -p forma-render --bin denoise-smoke -- artifacts/denoise 8
```

The explicit runtime test checks HDR noise reduction, guide edge placement,
Fast/Balanced/High modes, accurate prefiltering, buffer reuse and resize, alpha,
black images and source preservation. The GPU guide test checks actual primary
surface values, film reset, sample-cap toggles, resumed sampling and mode changes.
The smoke binary writes raw/denoised PNGs, a 256-sample reference and numerical
linear-RGB MSE, and fails if denoising does not materially reduce reference error.

`FORMA_SMOKE_DENOISE=1 forma --smoke-test artifacts/app-denoise` additionally
exercises the real GPUI controls, final viewport result, raw toggle at the cap,
undo, quality changes, navigation during inference and a denoised PNG export.

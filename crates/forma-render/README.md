# Forma renderer

Native Metal and portable wgpu compute renderers behind one `Renderer` API.
macOS defaults to the existing native Metal implementation; Windows uses wgpu
DirectX 12 and Linux uses wgpu Vulkan. wgpu Metal is also available on macOS.
Every backend implements all four modes, HDR lighting, and PNG export.

## Modes

| Mode | Behavior |
| --- | --- |
| Wireframe | Blender-style X-ray original polygon edges: bright opaque front edges with dimmed occluded edges, antialiased line widths; triangulation diagonals are excluded. |
| Solid | Depth-correct neutral clay shading under camera-relative studio lights. |
| Material Preview | Deterministic, single-frame metallic/roughness IBL with filtered HDR reflections, diffuse irradiance and optional contact shading. |
| Rendered | Progressive scene world and mesh-emission path tracing; no editor grid or selection decorations in the film. |

Wireframe uses transparent faces with X-ray edges; Material Preview
has its own immediate shading pipeline; Rendered performs progressive path tracing.

## Material Preview

Primary visibility uses the same triangle geometry and smooth normals as the
other modes. On the native macOS backend with devices supporting Metal ray tracing, a cached primitive acceleration
structure handles preview primary, antialiasing, AO and selection rays. Other native devices and all wgpu backends use the same BVH in GPU compute.
 Both paths keep four fixed antialiasing samples and eight
nearest-hit contact rays; navigation never reduces resolution or sampling quality.
The acceleration structure is built only when needed after geometry changes and
is reused by camera updates. This follows [Metal's acceleration-structure model](https://developer.apple.com/documentation/metal/ray-tracing-with-acceleration-structures).
Actual base color, metallic weight, roughness and visible emission
drive a split-sum GGX environment response and diffuse irradiance. Cached GPU
textures contain the diffuse convolution, nine directional specular roughness slices,
and integrated BRDF lookup. Camera and material edits reuse these textures.

Three built-in procedural HDR rigs provide Studio, Courtyard and Sunset lighting.
Custom Radiance `.hdr` panoramas are decoded as scene-linear float RGB and reduced
to a fixed lighting budget. Inputs are limited to 128 MiB, 16,384 × 8,192 pixels
and 32 megapixels; nonfinite, negative or excessively bright radiance is rejected.
Four prepared environments can remain cached across lighting changes.

Rotation and strength affect the selected environment. Background opacity and
blur affect only camera misses. The optional Scene World uses the project's
constant world radiance and ignores studio settings. Preview displays a surface's
own emission, without scene-emitter lighting or indirect color bounces. Contact
shading is a bounded ambient-occlusion approximation. Environment reflections do
not reflect nearby scene objects. Rendered remains available for full transport.

Preview completes after one frame regardless of path-tracing samples or bounce
limits. The worker sleeps after completion, and preview PNG export also needs
one frame. Preview settings never invalidate the Rendered film; scene-world edits
are ignored by studio-preview frame identity when Scene World is disabled.

## Transport and geometry

The PBR surface model combines energy-partitioned Lambert diffuse and
single-scattering isotropic GGX, with Schlick Fresnel and configurable dielectric IOR (1.5 by default).
Glass adds a rough GGX dielectric BSDF with exact Fresnel, refraction, total
internal reflection, matched sampling/PDFs and radiance IOR scaling.
A shared evaluator applies embedded image maps and optional custom Metal/WGSL code
in both Material Preview and Rendered. Preview follows glass interfaces through
scene objects and uses environment reflections; Rendered samples full light paths.
See the [material guide](../../docs/MATERIALS.md) for the shader contract, texture
mapping, persistence and fallback behavior.
GGX uses visible-normal sampling. Every scattering vertex samples finite area
emitters and the environment, combining each with BSDF sampling via power-
heuristic multiple importance sampling. Russian roulette starts after four
scattering events. The finite bounce limit includes the final emission/escape
evaluation so direct-light estimates retain their complementary MIS sample.

Emission is two-sided radiance. The world is a constant linear RGB radiance.
In Rendered mode, a float32 accumulation buffer stores linear radiance, followed
by exposure, a fitted ACES-style tone curve and sRGB encoding. No firefly clamp
or fabricated fill light is applied to Rendered mode.

A CPU-built binned-SAH BVH indexes transformed triangles; GPU traversal visits
nearer nodes first. A 48-level build cap fits the 64-entry traversal stack, with
a conservative exhaustive fallback if an invalid tree ever overflows it.
Polygon boundaries and 45-degree normal creases are preserved. Geometry and GPU
buffers persist across samples and camera moves. A scene revision triggers a
geometry fingerprint; an unchanged fingerprint avoids rebuilding or uploading.

## Backend selection

`Renderer::new()` honors `FORMA_RENDERER`, defaulting to `auto`.
`Renderer::with_backend(Backend::Wgpu)` chooses wgpu's platform API;
`Backend::Metal`, `Backend::Dx12`, `Backend::Vulkan` and `Backend::NativeMetal`
select an explicit implementation. `backend()` and `device_name()` report the
actual selection. Unavailable APIs return an actionable error.

Shader source is embedded in the executable. WGSL implements the existing Metal
transport equations and editor modes. Uniform layout, geometry fingerprints,
BVH construction, procedural lights and bounded HDR decoding are shared Rust.
Float32 environment sampling uses explicit interpolation, avoiding optional
float32 texture-filtering requirements on adapters. Preview lighting preparation
uses GPU compute and a bounded four-environment cache.

## Display and lifetime

`Renderer` belongs to a render worker. `render` waits for one GPU sample and
returns a completed `Frame`; it must not run on the GPUI event loop. The app can
publish frames through a bounded channel. Camera, world, dimensions, exposure,
mode and bounce count reset Rendered accumulation; viewport overlays reset only
the modes that display them. Raising the Rendered sample target
resumes it. Increment the scene revision after geometry or material changes.

On native Metal, the RGBA film is GPU-converted to full-range BT.601 NV12 because GPUI 0.2.2's
macOS compositor explicitly requires that pixel format. An IOSurface-backed
CoreVideo pixel-buffer pool supplies the two Metal output planes. Frames retain
their pixel buffer; the renderer never overwrites a published frame. Metal
texture wrappers remain alive through GPU completion. The pool can recycle
storage after consumers release it. Normal display performs no CPU pixel copy.
Odd render dimensions round up to even dimensions for 4:2:0 chroma.

wgpu keeps geometry and accumulation resident on the GPU. After each completed
sample it copies the RGBA8 display film into a reusable aligned staging buffer,
unpads the rows, and publishes immutable `Arc<[u8]>` pixels. The app prepares a
BGRA `RenderImage` on the worker and uploads it through GPUI's image compositor.
Old atlas entries are removed as frames are replaced. This portable display
path adds a CPU transfer and texture upload per frame; it is not zero-copy.
Retained frames remain valid through resize and renderer destruction.

`Frame` exposes `width()`, `height()`, `rgba()`, and, on macOS, `native_surface()`.
Use `same_surface()` to test allocation identity without comparing pixels.
`read_linear_pixels()` explicitly returns normalized scene-linear RGBA for
numerical checks. GPU allocation, validation, mapping, and completion errors are
returned as `Result` errors. The portable renderer checks adapter limits before
allocating geometry or film storage.

`export_png` saves the full-resolution RGB film, preserving RGB chroma rather
than exporting the subsampled native display surface. Native Metal reads the film
on demand; wgpu reuses the completed RGBA readback. Export saves the
current mode and its current sample count.

## Validation

```sh
cargo test -p forma-render
cargo run -p forma-render --bin render-smoke -- artifacts/render 128
cargo run --release -p forma-render --bin navigation-bench -- artifacts/navigation
```

The tests execute the selected backend’s real GPU kernels and cover film reset/resume, mode behavior,
black-world lighting, preview/world independence, BVH enclosure/coverage,
original edges, nonuniform-scale smoothing, retained surface handoff, and
floating-point radiometry. Dedicated preview tests cover single-frame completion,
material changes, environment controls, emission isolation, HDR validation and
failed-import recovery. Hardware/software comparisons cover two-sided and transformed
geometry, contact AO, selection occlusion, cache lifetime, empty scenes and Rendered
compatibility. The navigation benchmark measures 40 warmed camera updates at
2020 × 1390 and reports median/p95 latency without imposing device-specific thresholds.
The smoke tool
writes all four modes to PNG and reports measured sample cost. The shared tests run on all three operating systems. Native acceleration and
Metal/wgpu parity tests run on macOS. Shader tests validate WGSL and generate
SPIR-V, MSL and HLSL without a GPU. See [platform validation](../../docs/PLATFORMS.md).

## Deliberate scope

This is a foundational path tracer, not Cycles feature parity. Volumes,
nested dielectric media, subsurface scattering, anisotropy, displacement, alpha
cutouts, denoising, adaptive sampling, motion blur and animation are not yet
implemented. Very smooth roughness values are bounded at 0.025 to avoid a delta
BSDF singularity. GGX uses single scattering, so rough metals lose the energy
that a multiple-scattering model would recover. Mesh lights are sampled uniformly
by triangle, which is unbiased but can be noisy for uneven emitter tessellation.
Rendered uses a software BVH in GPU compute; native preview acceleration does not change
its estimator or sample sequence.

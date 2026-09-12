# Forma renderer

Native macOS Metal compute renderer with a GPUI surface display path. Build and
run on a Mac with a Metal GPU; there is no browser or software pixel renderer.

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
other modes. On devices supporting Metal ray tracing, a cached primitive acceleration
structure handles preview primary, antialiasing, AO and selection rays. Other devices
use software BVH traversal. Both paths keep four fixed antialiasing samples and eight
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

The opaque surface model combines energy-partitioned Lambert diffuse and
single-scattering isotropic GGX, with Schlick Fresnel and dielectric IOR 1.5.
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

## Display and lifetime

`Renderer` belongs to a render worker. `render` waits for one GPU sample and
returns a completed `Frame`; it must not run on the GPUI event loop. The app can
publish frames through a bounded channel. Camera, world, dimensions, exposure,
mode and bounce count reset Rendered accumulation; viewport overlays reset only
the modes that display them. Raising the Rendered sample target
resumes it. Increment the scene revision after geometry or material changes.

The RGBA film is GPU-converted to full-range BT.601 NV12 because GPUI 0.2.2's
macOS compositor explicitly requires that pixel format. An IOSurface-backed
CoreVideo pixel-buffer pool supplies the two Metal output planes. Frames retain
their pixel buffer; the renderer never overwrites a published frame. Metal
texture wrappers remain alive through GPU completion. The pool can recycle
storage after consumers release it. Normal display performs no CPU pixel copy.
Odd render dimensions round up to even dimensions for 4:2:0 chroma.

`export_png` is an explicit readback of the full-resolution RGB film, preserving
RGB chroma rather than exporting the subsampled display surface. Export saves the
current mode and its current sample count.

## Validation

```sh
cargo test -p forma-render
cargo run -p forma-render --bin render-smoke -- artifacts/render 128
cargo run --release -p forma-render --bin navigation-bench -- artifacts/navigation
```

The tests execute real Metal kernels and cover film reset/resume, mode behavior,
black-world lighting, preview/world independence, BVH enclosure/coverage,
original edges, nonuniform-scale smoothing, retained surface handoff, and
floating-point radiometry. Dedicated preview tests cover single-frame completion,
material changes, environment controls, emission isolation, HDR validation and
failed-import recovery. Hardware/software comparisons cover two-sided and transformed
geometry, contact AO, selection occlusion, cache lifetime, empty scenes and Rendered
compatibility. The navigation benchmark measures 40 warmed camera updates at
2020 × 1390 and reports median/p95 latency without imposing device-specific thresholds.
The smoke tool
writes all four modes to PNG and reports measured sample cost. Tests require a
Metal GPU and should run on a macOS runner.

## Deliberate scope

This is a foundational path tracer, not Cycles feature parity. Transmission,
refraction, volumes, subsurface scattering, anisotropy, material texture maps,
denoising, adaptive sampling, motion blur and animation are not yet
implemented. Very smooth roughness values are bounded at 0.025 to avoid a delta
BSDF singularity. GGX uses single scattering, so rough metals lose the energy
that a multiple-scattering model would recover. Mesh lights are sampled uniformly
by triangle, which is unbiased but can be noisy for uneven emitter tessellation.
Rendered uses a software BVH in Metal compute; preview acceleration does not change
its estimator or sample sequence.

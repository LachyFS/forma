# Implementation validation

## CAD exchange and object editing (2026-10-15)

Validated on Linux with Rust 1.92.0 and Mesa llvmpipe Vulkan:

- **219 workspace tests passed**, including 14 STL exchange/review tests,
  12 object-transform regressions, five single-line input tests and three
  inspector scrolling tests. Strict workspace Clippy (`--all-targets -- -D warnings`),
  rustfmt and `git diff --check` passed; the native editor builds successfully.
- STL tests cover ASCII/binary parsing, `solid`-prefixed binary headers,
  LF/CRLF/CR line endings, exact coordinate welding, signed zero, explicit units,
  selected/visible scopes, inherited visibility, hierarchy, reflected winding,
  precision collapse, malformed input, geometry budgets and atomic destination
  replacement. Import failure preserves scene data and allocation IDs.
- Object utilities preserve world vertices and surface normals, descendant
  transforms including shear, material links and unrelated shared meshes.
  Tests cover parent/child selection order, mirrored scales, no-op identity,
  failure rollback, save/load and history. Centering scans each shared mesh's
  bounds once. Already-applied or centered selections skip staged scene copies.
- Review added an early bevel output-budget rejection before topology allocation;
  a closed 666,668-face fixture exercises that path. Both apply-transform and
  world-space bevel integration preserve face orientation through reflections.
- The real GPUI smoke workflow passed in an isolated Xvfb/Openbox display. It
  exercises object utility commit/undo/redo, STL unit conversion and selected
  export snapshots while editing continues, imported dimensions, invalid/stale
  import rejection, modal keyboard/pointer isolation, exact clipboard operations,
  selected-all input, Unicode/byte limits, invalid field correction, modal paste,
  and Tab/Shift+Tab at 1000 × 650. Visible fields do not jump; hidden fields scroll
  into view; superseded focus changes cannot apply stale scroll requests.
  Earlier renderer, preview, shading, modelling, document and PNG-export flows
  also passed. Evidence is in `artifacts/cad-workflow-smoke/`.

STL is geometry exchange with explicit units, unchanged Y-up axes and no material
data. It does not certify a printable solid. File-picker selection is outside the
automated workflow, and software-rendered Linux checks do not establish hardware
GPU performance or native execution on macOS/Windows. Large valid mesh-finishing
operations remain synchronous in the editor; scene history still uses snapshots.

## Precision modelling and mesh finishing (2026-10-15)

Validated on Linux with Rust 1.92.0 and Mesa llvmpipe Vulkan:

- **184 workspace tests passed**, including 13 mesh-finishing regressions,
  precision expression/dimension tests, and seven measurement/navigation tests.
  Geometry cases cover all 64 cube winding combinations, disconnected solids,
  open surfaces, nonorientable and pinched topology, concave polygons, extreme
  aspect ratios, mirrored/sheared geometry, exact bevel widths and atomic errors.
- Strict workspace Clippy (`--all-targets -- -D warnings`), rustfmt and
  `git diff --check` passed. The native application builds successfully.
- The real GPUI smoke workflow passed on an isolated Xvfb/Openbox display.
  New keyboard-driven cases exercise millimetre dimensions, fractional inches,
  distance transforms, radian rotation, invalid dimensions retained for correction,
  command modifiers during transforms, bevel commit/undo/redo/rejection/cancel,
  remapped triangulation selections, normals repair, opposite views, hidden-object
  exclusion during framing, and measurement refresh after edits/undo.
  Existing renderer, material/shader, navigation, theme, resizing and background
  document/export checks also passed. Output is in `artifacts/quality-smoke/`.
- A separate XTest walkthrough drove OS keyboard/mouse events to create a
  4 × 2 × 1 metre part, bevel it by 100 mm, display dimensions, and reject a
  99-metre bevel without changing the part. Full-window screenshots were visually
  inspected at normal size and at the 1000 × 650 minimum, including validation
  feedback and the scrolling inspector; they are in the same artifact directory.
- Measurement toggles preserve scene data, dirty state, the rendered frame and
  progressive sample count. Local and world dimensions cache geometry; camera
  movement does not rescan unchanged meshes. Individual edge labels are capped
  at 12 while the total includes every selected edge.

Bevel is a single-segment, whole-mesh operation for one selected closed convex
mesh. It remains destructive and rejects unsupported inputs. Convexity work is
bounded to eight million vertex/plane comparisons. Development timing samples
on this host measured about 1.7 ms for 482 vertices and 16.7 ms for 1,986 vertices;
these observations informed the work limit and are not frame-rate guarantees.

The smoke run uses software rendering; it does not establish hardware GPU
performance or macOS/Windows execution. Native file-picker interactions remain
outside the automated workflow. This work does not provide a parametric CAD
kernel, constraints, booleans or Blender feature parity.

## Picking, visibility and renderer preparation (2026-10-15)

Three parallel experiments investigated object picking, visibility queries and
renderer geometry preparation, followed by independent cross-review. The editor
also retains one lazily built visibility index across camera, selection and frame
updates; geometry/visibility changes and undo/redo clear it. This avoids repeated
index construction during unchanged-scene overlays at the cost of retaining one
world-space triangle/BVH copy until the next geometry invalidation.

Measurements use Rust 1.92.0 release builds on Linux, AMD Ryzen 7 9800X3D, against
`b3d6b90` (after the mesh-allocation improvements below). These are medians of five
runs, with baseline/current order alternating, and no concurrent builds or other
experiment workloads. Picking uses three warmups and 21 samples per run; query
and geometry benchmarks use five warmups and 31 samples. Geometry preparation
alternates the reference/current implementations within each sample pair and
includes BVH construction and output destruction. These CPU measurements do not
establish viewport frame rates or gains on other hardware.

Picking skips triangulation for meshes whose conservative local bounds miss the
ray. Times below are microseconds per ray; viewport batches contain 96 rays.
The default scene has 5 objects/2,702 triangles, and V8 has 276 objects/81,212
triangles. Hit object, polygon, distance and tie order remain unchanged.

| Picking workload | Before (µs) | After (µs) | Change |
| --- | ---: | ---: | ---: |
| Default viewport | 78.416 | 5.188 | 93% less |
| V8 viewport | 2168.279 | 90.091 | 96% less |
| Default, hit rays only | 78.309 | 9.925 | 87% less |
| V8, hit rays only | 2175.507 | 158.140 | 93% less |
| Dense sphere, missed ray | 449.434 | 6.306 | 99% less |
| Dense sphere, hit ray | 449.032 | 456.715 | 2% more |
| Single cube, hit ray | 0.440 | 0.444 | 1% more |

The bounds scan adds work for rays that hit the only dense mesh. It is bypassed
when unused mesh positions outnumber face corners: a cube with 100,000 unused
vertices takes 0.433 µs/ray in the final harness. An earlier unguarded prototype
spent about 78 µs/ray scanning those unused positions.

Visibility construction transforms shared vertices once and reserves a compact
node array. Query traversal tests three slab axes together using division,
retaining the original child order and parallel-ray cutoff. Each query batch
contains 8,192 rays; the dense fixture contains 256 spheres/245,760 triangles.
Hit counts match baseline for every fixture.

| Visibility workload | Before (ms) | After (ms) | Less time |
| --- | ---: | ---: | ---: |
| Default, build | 0.190 | 0.172 | 9% |
| V8, build | 8.948 | 8.280 | 7% |
| Dense, build | 28.879 | 26.739 | 7% |
| Default, queries | 1.232 | 0.762 | 38% |
| V8, queries | 4.014 | 3.193 | 20% |
| Dense, queries | 1.067 | 0.809 | 24% |

Renderer preparation stores incident corners contiguously, collects polygon
edges directly and reuses mesh-space preparation across linked instances within
one build. Shared preparation is released before BVH construction. Triangle
attributes, ordering, BVH nodes and emissive-light indices match the original
implementation byte for byte.

| Renderer geometry workload | Before (ms) | After (ms) | Less time |
| --- | ---: | ---: | ---: |
| Default | 1.079 | 0.927 | 14% |
| V8 | 36.609 | 33.314 | 9% |
| 128 linked spheres, 122,880 triangles | 44.136 | 26.716 | 39% |
| Dense grid, 32,768 triangles | 13.256 | 11.480 | 13% |

Rejected experiments: near-first visibility traversal slowed the measured query
workloads; precomputed angle-weighted normals increased adjacency storage from
16 to 24 bytes per corner without a consistent additional gain. Reciprocal
multiplication was avoided because it can change grazing-box rejection through
rounding.

All **158 workspace tests** pass with Mesa llvmpipe Vulkan, along with strict
workspace Clippy, formatting and diff checks. The real OIDN runtime test passes
separately on CPU. New differential tests cover transformed/mirrored hierarchies,
locked and hidden objects, linked mesh edits, concave and degenerate polygons,
crease boundaries, tiny/large scales, signed-zero and grazing rays, and randomized
box comparisons. Exact byte comparisons run before renderer timing as well.

The full standard GPUI smoke workflow passes on Linux/X11 with llvmpipe,
including cache reuse during camera movement, invalidation on hide/undo/redo,
component selection, modelling, navigation, document I/O and concurrent image
export. An earlier run with optional `FORMA_SMOKE_DENOISE=1` reached the first
16-sample denoising wait and exceeded its 20-second limit; its cause is not
established by the smoke log. The separate CPU denoiser test passes as noted
above. Native macOS/Windows UI execution remains unverified here.

Reproduce the workloads with:

```sh
cargo run --locked --release -p forma-core --bin picking-bench
cargo run --locked --release -p forma-core --bin surface-query-bench
cargo test --locked --release -p forma-render --lib geometry_preparation_benchmark -- --ignored --nocapture
```

Local timing logs and medians are in `artifacts/performance-round2/final/`.

## Mesh processing allocations (2026-10-15)

Mesh triangulation now reuses projection, ear-clipping and triangle scratch
buffers across faces and reserves the output once. Validation reuses its vertex
uniqueness buffer and shares one polygon projection between intersection checks
and triangulation. Triangle order, polygon attribution and validation errors are
preserved.

Measured with Rust 1.92.0 release builds on Linux, AMD Ryzen 7 9800X3D. Each
workload has 65,536 faces; `mesh-bench` performs five warmups and 31 timed samples,
including output allocation and destruction. The table shows the median of five
alternating baseline/current runs, against baseline `c255dbb` with the same
benchmark. Times are milliseconds; these are CPU microbenchmarks, not viewport
frame-rate measurements or cross-device guarantees.

| Mesh | Operation | Before | After | Less time |
| --- | --- | ---: | ---: | ---: |
| Triangles | Triangulation | 0.360 | 0.284 | 21% |
| Triangles | Triangulation with face indices | 0.479 | 0.206 | 57% |
| Triangles | Validation | 2.167 | 1.256 | 42% |
| Quads | Triangulation | 1.933 | 1.593 | 18% |
| Quads | Triangulation with face indices | 2.205 | 1.582 | 28% |
| Quads | Validation | 4.395 | 2.559 | 42% |
| Concave pentagons | Triangulation | 3.058 | 2.407 | 21% |
| Concave pentagons | Triangulation with face indices | 3.485 | 2.435 | 30% |
| Concave pentagons | Validation | 6.266 | 4.046 | 35% |

All **55 core tests**, strict core Clippy (`--all-targets -- -D warnings`),
formatting and diff checks passed. New regression tests cover mixed polygon
sizes, winding and all projection axes, original quad diagonals, failed-face
isolation and validation after buffer reuse. A separate local comparison against
the previous implementation also matched exact triangle output, face indices and
validation results for both bundled V8 scenes and 10,000 generated mixed-face
meshes, including invalid polygons.

Reproduce the timing workloads with:

```sh
cargo run --locked --release -p forma-core --bin mesh-bench
```

## Rust 1.92.0 compatibility (2026-09-13)

The repository now pins Rust 1.92.0. All **113 workspace tests**, strict workspace
Clippy and formatting checks passed using that toolchain and the existing
`Cargo.lock`; dependency versions were unchanged. The real OIDN runtime test
passed separately on the automatically selected GPU.
Renderer Clippy with `--all-targets -- -D warnings`
passed for `aarch64-apple-darwin` and `x86_64-pc-windows-msvc`. The older Clippy
required simplifying one nested `else if` in native Metal pipeline selection,
with the same selection behavior. These cross-compilation checks do not establish
native macOS or Windows execution.

## Denoising merge validation (2026-09-12)

Validated denoising together with the current material shaders, compact panels,
themes, CI configuration and website on the pinned Rust 1.98.1 toolchain:

- All **113 workspace tests** passed. New renderer coverage checks guides from
  textures, custom shader colors/normals and glass, and preserves shader diagnostics.
- The real OIDN runtime test passed separately on the selected NVIDIA GPU.
- Strict workspace Clippy and renderer Clippy for Apple ARM64 and Windows x64
  passed, as did formatting, actionlint, ShellCheck and all **8 website tests**.
  Cross-compilation does not establish native execution on macOS or Windows.
- The full GPUI smoke passed on X11 with llvmpipe Vulkan and
  `FORMA_SMOKE_DENOISE=1`, including themes, material shaders, navigation,
  final-sample denoising, raw/denoised switching, undo and concurrent PNG export.
  The report and images are in `artifacts/app-denoise-merged/`; the full-window
  capture was inspected with the denoising controls in the compact render card.

## AI denoising (2026-09-12)

- `cargo test --locked --workspace -- --test-threads=1`: **93 passed**; the
  runtime-dependent OIDN test is ignored by default and was executed separately.
- `cargo test --locked -p forma-render --test denoise_runtime -- --ignored`:
  **passed on the automatically selected NVIDIA GPU and with
  `OIDN_DEFAULT_DEVICE=cpu`**. Covers all three quality modes, accurate guide
  prefiltering, HDR edge/noise behavior, cached buffers, resize, alpha, unchanged
  source data and exactly black films. The black-film case caught neural
  reconstruction bias and now takes an exact zero-energy fast path.
- Workspace clippy with `--all-targets -- -D warnings`, formatting, and diff checks:
  passed. Renderer clippy with the same options passed for
  `aarch64-apple-darwin` and `x86_64-pc-windows-msvc`; native execution on those
  platforms remains unverified.
- `denoise-smoke artifacts/denoise 8` rendered the default scene at 512 × 384
  on RADV Vulkan, then used OIDN 2.4.1 on its selected NVIDIA GPU. Against a
  256-sample reference, linear RGB MSE fell from **0.00091826 to 0.00032393**
  (**64.7% reduction**). The Balanced denoise call, including first filter setup,
  took **6.4 ms** on this run. These observations are scene/device-specific.
  Raw, viewport-denoised, High-quality export and reference PNGs were inspected.
- The complete real GPUI smoke passed on X11 with llvmpipe Vulkan and
  `FORMA_SMOKE_DENOISE=1`. It additionally checked the final denoised viewport,
  restoring raw samples at the cap without retracing, undo, quality changes,
  navigation during inference and a separate denoised PNG export. A full X11
  capture of this app's window verified the inspector layout at 1120 × 760.
- `scripts/bundle-linux.sh debug`: passed with the verified OIDN runtime and
  notices included. Loading the bundled library by its relocated path passed.
  An explicitly missing `FORMA_OIDN_LIBRARY` returned an actionable error
  without loading a different runtime. macOS/Windows package scripts were
  updated but not executed on this Linux host.

This worktree's host lacked the XKB X11 runtime/development libraries needed to
link the existing GPUI app. Ubuntu packages were extracted under
`target/native-libs`, with local linker/runtime search paths used for the app
checks; no system packages were changed. Source setup and the design are in
[AI denoising](DENOISING.md).

## CI checks (2026-09-12)

Validated the CI configuration locally on Linux with the repository's pinned
Rust 1.98.1 toolchain:

- All **103 workspace tests** passed with `FORMA_RENDERER=vulkan` and the Mesa
  llvmpipe driver explicitly selected, including shader translation tests.
- `cargo build --locked --workspace --bins` built the Linux application and
  renderer tools successfully.
- Strict workspace Clippy, rustfmt, actionlint 1.7.12, ShellCheck, JavaScript
  syntax checks, all **7 website tests**, and `git diff --check` passed.
- The headless smoke tool exported all four nonblank 640 × 480 modes through
  llvmpipe, using four samples for Rendered, to `artifacts/ci-render/`.
- cargo-audit 0.22.2 reported **zero known vulnerabilities**. Unmaintained
  transitive dependencies remain reported as warnings; no advisories are ignored.
- The workflow's checksum-verified actionlint download and Mesa-driver selection
  steps were executed locally. GitHub-hosted execution of the revised workflows,
  Windows/macOS builds, and Pages deployment are not established by these checks.

## Cross-platform implementation (2026-09-12)

Executed locally with Rust 1.98.1 on Linux, Mesa 26.0.3 and an AMD Ryzen 7
9800X3D integrated RADV Vulkan adapter:

- `cargo test --locked --workspace -- --test-threads=1`: **88 passed**, including
  22 application tests, 36 core tests, 8 renderer unit tests, 19 renderer
  integration tests and 3 shader translation/layout tests.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `cargo run --locked -p forma-render --bin render-smoke -- artifacts/linux-render-smoke 32`
  with `FORMA_RENDERER=vulkan`: all four 640 × 480 modes completed and exported
  nonblank PNGs. Observed sample costs were 10.4 ms Wireframe, 6.4 ms Solid,
  149.0 ms initial Material Preview (including environment baking), and 29.2 ms
  Rendered at 32 samples. These observations are not performance guarantees.
- Renderer `cargo check` and `cargo clippy -- -D warnings`, both `--all-targets
  --locked`, passed for **aarch64-apple-darwin** and **x86_64-pc-windows-msvc**.
  This includes native Metal, wgpu Metal and wgpu DirectX 12 Rust paths. The
  Windows check caught and fixed a `gpu-allocator`/`wgpu-hal` Windows crate
  version mismatch in the lockfile.
- `scripts/bundle-linux.sh debug`: built a host Linux directory and tarball
  containing the executable, desktop file and icons. Windows/macOS package scripts
  were not executed on this host.
- WGSL validation and native SPIR-V/MSL/HLSL generation passed for all rendering
  and IBL baking kernels, without optional shader capabilities.

The full live GPUI smoke workflow passed on **X11 with Mesa llvmpipe Vulkan**:

```sh
env -u WAYLAND_DISPLAY VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json \
  target/debug/forma --renderer vulkan --smoke-test artifacts/linux-app-smoke
```

It exercised all viewport modes, HDR/preview controls, invalid and stale imports,
commands, constrained transforms, undo/redo, extrusion/subdivision, resizing,
project/OBJ persistence, asynchronous save/open/import/export and concurrent PNG
export. During continuous input it presented nine mouse-navigation frames and
ten trackpad-navigation frames; the pre-existing smoke thresholds were retained.
The report is `artifacts/linux-app-smoke/native-smoke.txt`. Portable captures are
explicitly labelled viewport images; they are not full compositor screenshots.
Material Preview and Rendered PNGs were visually inspected after the run.

This machine's hardware-backed **GPUI/Blade window presentation** failed before
application rendering: Wayland rejected DMA-BUF import, and X11 reported swapchain
initialization failure. Headless hardware Vulkan rendering passed separately.
The llvmpipe override above is a diagnostic workaround, not the default renderer
or a hardware-performance result. The app leaves platform/driver selection to
GPUI and does not globally change display environment variables.

Native execution on macOS and Windows, full application linking/packaging on
those hosts, hardware window presentation on this Linux desktop, native file
pickers and OS input routing remain unverified in this session. CI now supplies
per-platform compile/lint/unit checks and Linux renderer execution; it has been
added to the repository but was not run remotely. Shader translation and
cross-compilation do not substitute for Metal/DirectX driver tests. See
[platform support](PLATFORMS.md) for the intended release validation matrix.

## Earlier native Metal validation

The remaining record describes checks executed before this portability change
on macOS with an Apple M4 Pro. It is historical evidence for the native renderer,
not a claim that the new wgpu Metal path was run on that hardware.

## Surface shader integration (2026-09-12)

The shader branch was integrated with both the cross-platform renderer and the
compact workspace UI. Checks executed on Linux with Rust 1.98.1:

- **101 tests passed**: 22 application, 41 core, 12 renderer unit, 23 GPU
  integration and three shader translation/layout tests. Core coverage includes
  embedded images, source-language persistence, legacy material migration and
  undo/redo. GPU regressions used Mesa llvmpipe Vulkan and exercised both preview
  and rendered PBR/glass/textures/custom code, compile-error fallback/recovery,
  incompatible-language recovery, the default WGSL template and finite film output.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Renderer `cargo check --locked --all-targets` passed for
  `aarch64-apple-darwin` and `x86_64-pc-windows-msvc`.
- WGSL validation and SPIR-V/MSL/HLSL generation passed for render, preview and
  IBL kernels. Layout checks cover the expanded triangles and material buffers.
- The complete Linux GPUI application builds with the actual Vulkan renderer.
  The live X11/llvmpipe smoke workflow passed, including multiline shader input,
  local undo/redo, failed compilation with document retention, successful WGSL
  apply, scene undo, all viewport modes, preview/HDR controls, navigation,
  resizing, asynchronous project/OBJ operations and concurrent image export.
  It reported `FORMA_NATIVE_SMOKE_PASS`.

Native Metal runtime compilation and DirectX 12 GPU execution still require
validation on their respective operating systems. Cross-target Rust checks and
WGSL translation do not establish native Metal source compilation or hardware
behavior. The earlier Mac validation below describes pre-existing functionality.

## Geometry and documents

`cargo test -p forma-core` passed 30 tests covering primitive winding, closed topology,
concave-polygon triangulation, Catmull–Clark interior and boundary rules,
extrusion, perspective/orthographic ray agreement, picking, undo/redo,
versioned scene persistence, invalid input rejection, and OBJ transforms.
The suite also checks exact top/bottom camera bases, persisted render
preferences and their undo/redo behavior, and legacy-document defaults.

Independent cases in `crates/forma-core/tests/regression_review.rs` verify:

- An asymmetric crossing polygon with nonzero signed area is rejected.
- A validated cube at a small nonsingular scale remains selectable.
- Inward extrusion preserves oppositely wound shared edges and changes the
  signed volume by the expected amount.
- Importing beyond the object limit fails without changing the scene or its
  next object ID. This initially failed and passed after the import transaction
  was fixed to check aggregate limits before mutation.

## GPU rendering

`cargo test -p forma-render` passes eight unit tests and fifteen integration
tests. These execute Metal, including floating-point white-furnace checks
and linear world-intensity scaling. Other checks cover BVH leaf coverage,
host/shader layouts, original polygon edge masks, nonuniform-scale smooth
normals, progressive film capping and reset, mode differences, black-world
energy, preview/world separation, and the lifetime of published GPU surfaces
across further renders and a worker-thread handoff.

The renderer smoke tool produced all four 640 × 480 modes under
`artifacts/material-preview-engine/`. Material Preview was visually checked:
clean studio reflections, distinct roughness/metallic response, contact shading,
and no progressive grain. Rendered retains Monte Carlo noise without denoising.

Material Preview completes in one frame, including spatial antialiasing. Its
first 640 × 480 frame, including source generation, BRDF integration and all
nine directional roughness bakes, took 46.7 ms in a local M4 Pro smoke run.
The native viewport subsequently reported an 11.9 ms warm preview frame.
These are small-scene smoke observations, not large-scene benchmarks or an
application frame-rate guarantee.

Ten dedicated preview integration cases verify material/geometry edits, one-frame
completion independent of path targets, studio/world isolation, rotation and
strength, background-only changes, visible emission, custom HDRs, invalid-file
rejection and retention of a previously completed frame after failed loading.
Raw float tests verify diffuse/metal energy, unattenuated emission, HDR values
above one, linear intensity scaling and bright-texel resampling.

Independent review found that ordinary specular mip levels collapsed directional
lighting at maximum roughness. `tests/preview_review.rs` reproduced identical
rough-metal results under opposite HDR orientations; it passes after replacing
collapsed mips with nine directional roughness slices. The full workspace also
retains the original Rendered revision/reset regression.

## Scope of the evidence

The renderer supports opaque metallic/roughness surfaces, two-sided emission,
and a constant world, procedural preview environments or imported HDR lighting.
Preview uses split-sum IBL and contact AO; local scene reflections and indirect
light transport remain Rendered features. Numerical tests cover
the implemented estimators; they do not establish Cycles feature parity,
reference-image parity, or performance at the maximum import limits.

## Editor themes (Linux, September 2026)

The integrated theme change passes all **107 workspace tests** using software
Vulkan, plus formatting and strict Clippy across all targets. Four theme unit tests cover search/navigation, contrast,
preference-file replacement and recovery, and platform configuration paths.

The native smoke test passes on Linux/X11 with software Vulkan (llvmpipe). It
checks theme shortcuts and command search, live preview, Escape and outside-click
cancellation, empty search, commit/persistence, and all five presets. Scene data,
selection, dirty state, undo/redo availability, render settings, samples, and the
completed viewport image remain unchanged during appearance changes. Smoke
preferences are isolated inside the output directory.

Full-window X11 captures of the theme picker and light/dark presets were visually
inspected, including all five choices at the 1000 × 650 minimum window size.
The shader editor inherits the selected palette; Paper and Synthwave code-editor
captures were also inspected. The combined smoke run exercises shader editing,
compiler diagnostics, apply and undo using platform-appropriate shortcuts.
Artifacts are in `artifacts/theme-smoke/`. Native theme interactions on macOS and
Windows have not been run in this environment. The default GPU test run exited
with SIGSEGV in the material renderer tests; the complete suite passed with
`VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json`.

## Native editor

`cargo test --locked --workspace` passes **71 tests**. The full workspace passes
strict Clippy (`--all-targets -- -D warnings`) and formatting checks.

`cargo run -p forma -- --smoke-test artifacts/native-smoke` opens a real GPUI
window and passes the following runtime checks:

- Searchable command palette and four render modes through GPUI keyboard dispatch.
- Material Preview lighting popover and numeric controls preserve the document
  and undo history. Custom HDR success, invalid-file recovery and stale-load
  rejection are exercised, including switching to Scene World during a load.
  A preview export with a 4096-sample target still completes as one frame.
  During 20 preview camera events at 16 ms intervals, 20 completed GPU surfaces
  reached the editor in the debug smoke run.
- Blender-style Z shading pie: tap-to-latch, hold/flick/release, numeric mode
  selection, mouse hover/click, cancellation, edge placement, and repeat-key
  suppression. Opening, hovering, and selecting the current mode
  preserve its completed frame. Z remains a transform constraint and text input
  when editing an object name.
- Metal frame presentation during continuous pointer-handler input at 8 ms
  intervals: 44 distinct completed surfaces reached the editor during 80 events.
- Constrained numeric transforms, cancel, undo/redo, face extrusion, subdivision.
- Object renaming, sRGB-to-linear color entry, exposure persistence and exact top view.
- Project and OBJ round trips; duplicate, delete and undo.
- Resize from the initial 1512-point window to 1120 × 760, retaining a 608 × 652 viewport.
- Preview lighting layout at the 1000 × 650 minimum window size, with a scrolling popover.
- Background saves use immutable snapshots and preserve newer unsaved edits.
- Stale opens are rejected; background open/import/OBJ export complete successfully.
- A separate PNG export completes with the snapshot dimensions while editing continues.

Own-window captures were visually inspected at normal and minimum sizes. The titlebar overlap
found in the first capture was corrected, and inspector/outliner scrolling keeps
controls accessible at the smaller size. Captures live under
`artifacts/native-smoke/`; the default workspace and command palette are also
included under `docs/images/`.

Ten pure pie interaction tests verify directional mapping, the deadzone,
one-shot release, scaled placement, and clamped-edge gestures. Independent
subagent review found and verified fixes for outside-viewport pointer origins
and the visible center's deadzone. The pie uses shared descriptors for drawing,
pointer hit targets, and numeric shortcuts.

The shading-pie captures were inspected at the viewport center and near an edge.
The smoke harness explicitly refreshes its own AppKit layer before capture:
macOS may suspend display-link updates in a background window. This preserves
the user's keyboard focus while capturing the current native GPUI interface.

Retina validation now compares the actual completed CoreVideo surface against
the window's backing scale, including mode changes and resize. On the 2× display,
a 1010 × 695 point preview produced a 2020 × 1390 pixel surface. Previously it
rendered one pixel per logical point and enlarged that image for Retina display.
Three sizing tests cover 1×/2× displays, fractional bounds, NV12 alignment,
wide/portrait aspect limits, and Rendered's retained logical-pixel budget.
Independent review verified that backing-scale changes between displays reach
the bounds observer. Captures request the compositor's best resolution so that
Retina sharpness can be inspected rather than downsampled to logical dimensions.
Before the navigation optimization, the debug smoke run reported a 28.5 ms preview
frame at the higher resolution and 18 completed surfaces during 20 camera events.

Navigation now uses event-driven input submission and frame delivery, with no
16 ms polling loop. A cached Metal acceleration structure handles the same preview
rays on supported GPUs; software traversal remains available. Four antialiasing
samples, eight nearest-hit AO rays and backing-pixel resolution are retained.
Three independent regression groups compare both backends across 14 views and
state changes, with zero measured 8-bit differences in these fixtures. They cover
two-sided and transformed geometry, selection/occlusion, empty scenes, acceleration
cache lifetime and unchanged Rendered output. The notification test verifies
coalesced wakeups preserve the latest state and queued messages.
The 2020 × 1390 benchmark reference comparison had a mean channel difference of
0.0019/255; 76 pixels (0.0027%) differed by more than eight levels, consistent with
sparse intersection-edge differences between traversal implementations.

`navigation-bench` measures 40 camera updates after eight warmup frames at
2020 × 1390. On this Apple M4 Pro, the selected Material Preview median fell from
13.89 ms to 7.62 ms (p95: 14.11 ms to 8.17 ms). The native debug workflow measured
9.34 ms median and 10.43 ms p95 from input-handler invocation to adoption of the
completed surface, with an 8.64 ms median render cost. This excludes compositor
scanout and uses 1 ms observation intervals. These are small-scene measurements,
not guarantees for every GPU or document.

The smoke test drives keyboard events through GPUI and pointer events through
the editor's actual handlers. It does not automate OS mouse routing or native
file-picker selection. Those remain manual checks. The application bundle is
local and unsigned for distribution; notarization is outside this build.

## Release bundle

`./scripts/bundle-macos.sh release` built the optimized ARM64 bundle at
`target/release/Forma.app`. Its Info.plist validates and it contains the generated
multi-resolution icon. Running the bundled executable with `--smoke-test
artifacts/release-smoke` passes the complete native workflow suite, including
continuous input, preview/HDR workflows and concurrent PNG export. The final
source also passes all 71 workspace tests, strict Clippy, and formatting checks.
The release run published 20 preview surfaces during 20 camera events; its
single-frame preview PNG export completed in 62.3 ms with a 4096-sample target.
At 2020 × 1390, the release input-to-surface median was 9.31 ms (p95 10.40 ms),
including a 9.05 ms median render cost. The mode-switch preview measured 10.33 ms.
The separate 80-event continuous-navigation check adopted 80 completed surfaces.

## Solid and Material Preview performance — 0.2.3

Measured on 2026-10-19 with Rust 1.92.0 release builds on an Apple M4 Pro
(24 GB), comparing the 0.2.2 renderer at `4adf710` with 0.2.3. Each case uses
eight warmups and 40 identical camera updates. The following paired run was
repeated with candidate/baseline order reversed; the listed medians reproduced
within 2%. These are warmed renderer frame costs, excluding initial shader/HDR
preparation and compositor scanout, not guarantees for every GPU or document.

| Scene / resolution | Mode, selection enabled | Before median / p95 (ms) | After median / p95 (ms) |
| --- | --- | --- | --- |
| Default, 2020 × 1390 | Material Preview | 10.31 / 10.51 | 7.75 / 7.99 |
| Default, 2020 × 1390 | Solid | 4.65 / 4.84 | 2.25 / 2.48 |
| V8, 276 objects, 2560 × 1600 | Material Preview | 14.08 / 14.33 | 11.22 / 11.51 |
| V8, 276 objects, 2560 × 1600 | Solid | 25.88 / 27.05 | 3.33 / 3.56 |

Solid now has a dedicated Metal kernel using the same cached primary/selection
acceleration structures as Material Preview. Opaque preview pipelines omit glass
continuation state; an additional untextured variant omits texture sampling state.
Glass, textured and custom materials retain the appropriate full evaluation path.
Untextured built-in materials also skip UV/derivative calculation in both native
Metal and WGSL. Resolution, four fixed preview AA samples, eight nearest-hit AO
rays, grid shading and precise floating-point lighting math are retained.

The default and V8 preview exports match the baseline byte-for-byte, including
selection-disabled and AO-disabled cases. Accelerated Solid differs at sparse
triangle boundaries: 2 of 2,807,800 default-scene pixels and 15 of 4,096,000 V8
pixels differ by more than eight channel levels. Mean differences are 0.000081
and 0.000186 out of 255 respectively, consistent with hardware/software triangle
intersection tie rules. Native differential tests cover selection occlusion and
holes, reflected transforms, component overlays, projection changes, mode
switches, cache reuse/invalidation, hidden/deleted objects and empty scenes.
Opaque/full-pipeline comparisons cover small and padded dimensions, glass and
custom-material transitions, display bytes and the scene-linear film. Identity
texture comparisons cover every mapping in Preview and Rendered, and separate
tests ensure that each texture slot activates coordinate evaluation.

The full suite also exposed an existing portable preview defect: `atan2(0, 0)`
at an environment-map pole could produce invalid coordinates and make horizontal
surfaces black on wgpu Metal. The shared WGSL direction-to-UV helper now explicitly
chooses longitude zero at either pole. Constant-HDR tests compare top/bottom plane
lighting against the equivalent scene world, and native/wgpu four-mode parity
passes without weakening its tolerance.

Validation: 237 workspace tests pass (two manual benchmarks and one runtime test
are ignored by the default command); the ignored neural denoiser runtime test was
then run successfully. Ten additional grid/material/preview-review tests pass
with `FORMA_RENDERER=metal`. Strict workspace/all-target Clippy, formatting,
release-version validation, Python release tests and website tests pass. A native
optimized application bundle was built with the verified OIDN 2.4.1 runtime.

Reproduce the renderer measurements with:

```sh
cargo run --locked --release -p forma-render --bin navigation-bench -- artifacts/navigation
cargo run --locked --release -p forma-render --bin navigation-bench -- artifacts/navigation-v8 docs/scenes/v8-engine.forma 2560 1600
```

The benchmark accepts `OUTPUT [PROJECT|-] [WIDTH] [HEIGHT]`; `-` selects the
default scene when specifying dimensions. It also reports no-selection, no-AO
and no-grid cases. Local logs and images are under
`artifacts/preview-performance/`.

The packaged native app's complete smoke workflow passes when launched through
LaunchServices with `open -n -W ... --args --smoke-test ...`. It covers panel
resizing, themes and update state, HDR and shader editing, all four shading modes,
continuous pointer/trackpad navigation, precision modelling, Unicode/clipboard
entry, STL exchange, project persistence and concurrent image export. Continuous
navigation delivered 79 of 80 sampled pointer presentations and 80 of 80 trackpad
presentations. The run also exposed and fixed a GPUI ellipsis-layout crash on
narrow fields containing the multibyte caret: numeric/name/search inputs now clip
single-line text without mutating shaped text-run lengths. The complete rerun,
including narrow-field and Unicode entry cases, ends in `FORMA_NATIVE_SMOKE_PASS`.

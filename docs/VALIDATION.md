# Implementation validation

Validated on macOS with an Apple M4 Pro. This records executed checks, rather
than asserting equivalence to Blender or Cycles.

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

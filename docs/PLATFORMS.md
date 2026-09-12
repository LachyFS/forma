# Platform support

The application and `forma-render` expose the same modelling tools, project
format, four viewport modes, HDR import and PNG export on macOS, Windows and
Linux. The viewport renderer is independent of GPUI's own UI compositor.

| Selection | macOS | Windows | Linux |
| --- | --- | --- | --- |
| `auto` | Native Metal | wgpu DirectX 12 | wgpu Vulkan |
| `wgpu` | wgpu Metal | wgpu DirectX 12 | wgpu Vulkan |
| `native-metal` | Native Metal | Error | Error |
| `metal` | wgpu Metal | Error | Error |
| `dx12` | Error | wgpu DirectX 12 | Error |
| `vulkan` | Error | wgpu Vulkan | wgpu Vulkan |

Use `forma --renderer <selection>` or `FORMA_RENDERER=<selection>`. The command
line wins over the environment. `Renderer::with_backend(Backend)` is the
equivalent library API. Invalid values and unavailable graphics APIs fail with
an error instead of switching silently. Renderer tests and headless tools honor
`FORMA_RENDERER` through `Renderer::new()`.

## Product and display behavior

No document migration or backend-specific scene data is needed. Camera behavior,
selection, scene revisions, progressive sample resume, lighting controls, export
and worker scheduling use a shared contract. Windows/Linux use Ctrl for primary
shortcuts and Alt where macOS uses Option; shortcut labels follow the platform.

Native Metal keeps the existing IOSurface/CoreVideo NV12 display and hardware
preview ray tracing where available. wgpu runs the same transport and preview
algorithms as GPU compute, traversing the shared BVH without requiring hardware
ray tracing. It uses float32 accumulation, HDR diffuse/specular convolution and
BRDF baking, antialiasing, original polygon wire edges, AO and selection overlays.

wgpu display currently reads the RGBA8 film back once per frame and uploads a
BGRA image through GPUI. Conversion is on the worker, and replaced UI images are
evicted. This preserves appearance and lifetime behavior but adds transfer cost
compared with native Metal's zero-copy display. There is no claim of equal frame
times across implementations. PNG exports use full RGB chroma on every backend.
Both implementations round odd render dimensions up to even dimensions.

The rendering API permits dimensions up to 8192 × 8192, subject to the selected
adapter's texture and storage-buffer limits. Unsupported allocations fail before
submission. On wgpu, no optional float32 filtering or ray-tracing feature is
required. Up-to-date platform GPU drivers are required; software Vulkan can be
used for correctness checks, with substantially slower rendering.

## Builds and packaging

See the root README for platform build dependencies. WGSL and Metal source are
embedded in the executable. The wgpu DirectX 12 path uses its default Windows
shader compiler; it does not require shipping an extra DXC DLL. GPUI's Windows
build requires the Windows SDK compiler. GPUI continues to own the native window,
text rendering, dialogs, keyboard dispatch and UI graphics backend.

- macOS: `scripts/bundle-macos.sh release` creates `target/release/Forma.app`.
- Windows: `scripts/bundle-windows.ps1 -Profile release` creates a portable
  executable directory and zip in `target/release`.
- Linux: `scripts/bundle-linux.sh release` creates a portable directory and
  tarball with desktop/icon files in `target/release`. Run its `bin/forma`.

These scripts build the host architecture. Linux bundles depend on the host
distribution's graphics, windowing and font libraries; they are not static
cross-distribution packages. They do not install, sign, notarize or publish.

## Validation matrix

The CI workflow compiles and lints all targets on Linux, Windows and macOS, runs
core/application unit tests, and validates WGSL plus SPIR-V, MSL and HLSL source
generation. Linux also executes the shared renderer suite through Mesa Vulkan.
Native acceleration comparisons and native-versus-wgpu Metal parity run only
on macOS with an available GPU.

Use these commands on machines exposing the relevant GPU API:

```sh
cargo test --locked -p forma-render -- --test-threads=1
cargo run --locked -p forma-render --bin render-smoke -- artifacts/render 32
cargo run --locked -p forma -- --renderer wgpu --smoke-test artifacts/app
```

On macOS run the renderer suite once with `FORMA_RENDERER=native-metal` and once
with `FORMA_RENDERER=metal`. On Windows run with `FORMA_RENDERER=dx12`, and test
`vulkan` if distributing that alternative. On Linux use `vulkan`. The shared
suite includes accumulation/reset, retained frames, nonblank distinct modes,
HDR validation and recovery, preview materials/lighting, emission isolation and
linear radiometry. Shader translation alone does not establish driver runtime
correctness, and Linux results do not establish Metal or DirectX performance.

On Linux, the viewport renderer and GPUI's window compositor are separate GPU
clients. If the compositor rejects imported DMA-BUFs or swapchain creation,
changing `--renderer` alone cannot fix the window driver. See the local
`VALIDATION.md` record for the X11/software-Vulkan diagnostic used on this host.

The application smoke workflow exports viewport PNGs on Linux/Windows; macOS
additionally captures its own window. File-picker interaction and physical OS
input routing remain manual platform checks. See `VALIDATION.md` for recorded
local execution and limitations.

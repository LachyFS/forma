# Surface shaders and image textures

Select a mesh, then choose **PBR**, **Glass**, or **Custom** under **Surface** in
the inspector. Material Preview and Rendered use the same textures and custom
surface function. Solid and Wireframe retain their modelling display.

- **PBR** combines diffuse shading with metallic/roughness GGX reflection. IOR
  controls dielectric reflectance; metallic surfaces use their base color.
- **Glass** starts with a white tint, roughness 0.06 and the material's IOR
  (1.5 by default). Rendered samples rough dielectric reflection and refraction,
  including total internal reflection and the radiance IOR correction. Use a
  closed mesh for a solid glass object; tint applies at each transmitted interface.
- **Custom** opens a multiline code editor. Native Metal uses Metal code; wgpu
  (Vulkan, DirectX 12 or Metal) uses WGSL. **Compile & apply** (⌘Return on macOS,
  Ctrl+Return on Windows/Linux)
  validates both rendering entry points on a background executor. Compiler errors
  appear below the code with file/line diagnostics. A failed compile leaves the
  applied material and document history intact. Apply creates one undo step;
  ⌘Z / Ctrl+Z inside the editor edits the draft's separate undo history.

Switching shader types or color presets retains image slots and saved custom
code. Glass initializes its tint/roughness when selected. Color presets return
the material to PBR. Editing a linked material changes all objects sharing it.

## Image textures

Each slot has an image picker and a remove button. PNG and JPEG files are decoded
in the background, and a failed or cancelled import leaves the material intact.
The filename and dimensions appear on the slot. Images are embedded in `.forma`
projects; the source file can be moved or removed after import.

| Slot | Interpretation |
| --- | --- |
| Base color | sRGB converted to linear, multiplied by base color/tint |
| Roughness | Linear red channel, replaces the roughness value |
| Metallic | Linear red channel, replaces metallic weight (ignored by Glass) |
| Normal | Linear OpenGL tangent-space RGB, +Y; adjustable strength |
| Emission | sRGB converted to linear, multiplied by emission strength |

Color conversion happens before mip generation. Sampling uses repeat wrapping,
linear filtering between texels and mip levels, and an approximate ray footprint.
Embedded image alpha is retained but does not control surface opacity; use Glass
for transmission. There is no displacement, alpha-cutout or UV editing yet.

**Box**, **Sphere**, and **Plane XZ** generate coordinates from the mesh's local
bounding box, so moving/rotating an object does not slide the texture. Tile U/V
and offset U/V affect every slot and the custom shader's `input.uv`. Sphere mapping
is equirectangular. Generated box mapping has seams at projection boundaries.
Imported OBJ texture coordinates are not retained.

Images are limited to PNG/JPEG, 32 MiB compressed, 8192 pixels per dimension and
four megapixels per image. A scene may contain 64 MiB of decoded RGBA texture
references. Projects use base64 for image bytes; history shares immutable pixels
and counts them conservatively toward its memory budget.

## Custom shader contract

Enter a **function body** in the language shown by the editor. Forma supplies the
function signature and WGSL return statement. Texture slots are sampled
first. `surface` contains the resulting values and `input` provides coordinates.
The source runs at each shaded intersection, including secondary rays and sampled
emitters in Rendered mode. This is a surface-parameter shader, not a replacement
for the path tracer or its BSDF sampling/PDF implementation.

| Value | Type / meaning |
| --- | --- |
| `input.uv` | `float2`, projected coordinates after tiling and offset |
| `input.generated` | `float3`, local bounding-box coordinates, usually 0–1 |
| `input.position` | `float3`, world position |
| `input.normal` | `float3`, normal after image normal mapping |
| `input.view_direction` | `float3`, direction toward the incident ray origin |
| `surface.color` | `float3`, linear base color or glass tint, 0–1 |
| `surface.emission` | `float3`, linear emitted radiance, 0–1,000,000 |
| `surface.roughness` | `float`, bounded to 0.025–1 in the renderer |
| `surface.metallic` | `float`, 0–1 |
| `surface.ior` | `float`, 1.01–3 |
| `surface.normal` | `float3`, normalized and checked against the visible hemisphere |
| `surface.glass` | `bool`, choose the glass BSDF instead of PBR |

Types in the table use Metal notation; WGSL uses `vec2<f32>`, `vec3<f32>` and
`f32` in place of `float2`, `float3` and `float`. The fields have the same meaning
on every backend. A material stores its source language with its body. Opening a
project on an incompatible renderer reports an error and uses textured PBR until
the code is adapted. The editor preserves existing code and identifies the
required language; pristine example bodies adapt to the selected renderer.

For example, keep the selected image texture and add stripes in Metal:

```cpp
float stripe = step(0.5f, fract(input.uv.x * 12.0f));
surface.color *= mix(float3(0.08f), float3(1.0f), stripe);
surface.roughness = mix(0.15f, 0.65f, stripe);
```

The equivalent WGSL body:

```wgsl
let stripe = step(0.5, fract(input.uv.x * 12.0));
surface.color *= mix(vec3(0.08), vec3(1.0), stripe);
surface.roughness = mix(0.15, 0.65, stripe);
```

A tinted glass shader in Metal:

```cpp
surface.glass = true;
surface.color = float3(0.9f, 0.98f, 1.0f);
surface.ior = 1.45f;
surface.roughness = 0.08f;
```

Code is limited to 64 KiB per material and 32 unique active custom functions per
scene. Identical functions share a dispatch entry. Pipelines are compiled only
when active source or its language changes; camera, texture and scalar edits reuse them. Applying
code swaps both pipelines together. Invalid source loaded from a project displays
compiler diagnostics and uses textured PBR as a fallback, allowing the document
to remain editable. Fixed code restores custom rendering on the next revision.
Nonfinite shader outputs fall back to the material values instead of contaminating
the film. Source is GPU code; successful compilation
does not guarantee a user-written function will finish or run cheaply.

Only applied code is saved. Closing the editor discards unapplied draft edits.
Version-3 projects store these materials and embedded images; version-1 and
version-2 projects migrate to PBR with empty texture slots.

## Preview and transport limits

Glass preview follows up to eight interfaces through scene geometry. Reflections
use the studio environment, and environment filtering approximates roughness;
rough transmitted scene objects are not blurred. Rendered uses the rough GGX
reflection/transmission distribution. Nested media, volume absorption, thin-sheet
special cases, dispersion and subsurface scattering are not implemented. Glass
caustics can converge slowly with this path tracer's existing light sampling.

The transport follows the dielectric model described in
[PBRT's Dielectric BSDF chapter](https://pbr-book.org/4ed/Reflection_Models/Dielectric_BSDF).
Native runtime compilation uses Metal's
[source library API](https://developer.apple.com/documentation/metal/mtldevice/makelibrary(source:options:)).

Run `cargo test --locked -p forma-core` for persistence and validation tests, and
`cargo test --locked -p forma-render --lib` for CPU mip/source/geometry tests.
`cargo test --locked -p forma-render` runs actual GPU texture, glass, custom
compilation, fallback and recovery regressions on the selected backend. Use
`FORMA_RENDERER` to choose a backend. Shader translation tests generate SPIR-V,
MSL and HLSL on any host. Native Metal output still requires a Mac; cross-target
Rust checks do not compile native Metal source.

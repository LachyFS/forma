# V8 engine mesh study

`v8-engine.forma` is an original illustrative engine assembly, authored for the
Forma website. It contains 276 independently selectable mesh objects and 43,982
vertices: castings, ribbed cam covers, eight flared intake stacks, fuel rails,
curved exhaust headers, pulleys, an alternator, hoses, and fasteners.

This is a visual mesh modelling study, not a dimensioned or mechanically
validated engine design. It uses ordinary Forma meshes, transforms, materials,
and a saved camera; it requires no new application features.

## Recreate and open

The standard-library Python generator creates the scene deterministically:

```sh
python3 scripts/create-engine-study.py
```

In Forma, use **Open** to load `docs/scenes/v8-engine.forma`. The saved camera
frames the complete engine. Switch between Material Preview, Solid, and
Wireframe with the viewport controls. Each part remains editable, and the
materials are ordinary PBR materials.

## Website captures

The website images were captured from the actual GPUI app at commit `10d7ae8`,
on Linux/X11 with the Vulkan renderer, at 1918 × 1118 pixels. An isolated capture
launcher opened this scene instead of the default primitives and set the window
size and project label; the editor UI and renderer were unchanged. Capture
launcher changes are not included in the application.

The three captures are `site/assets/engine-workspace.webp`,
`engine-solid.webp`, and `engine-wireframe.webp`. They were encoded as WebP from
full-window X11 captures, with no compositing or content retouching. To recreate,
open the scene in Forma Dark, select the engine block, set the viewport mode,
and capture the app window. Renderer and operating system differences can
change the exact appearance.

The scene and generator use the same license terms as the repository.

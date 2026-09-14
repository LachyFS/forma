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

## Exploded editing study

```sh
python3 scripts/create-engine-study.py --exploded
```

This writes `v8-engine-exploded.forma`. It separates the covers, intake assembly,
headers, sump, and front drive using normal object transforms. Geometry and
materials are shared in design with the assembled study; no exploded-view tool
is added to the app. The saved camera frames the separated assemblies, and the
right cam cover is first in the object list for convenient selection.

## Website captures

The current comparison was captured from Forma at commit `87de1e0`, on Linux/X11
with the Vulkan renderer, at 1918 × 1118 pixels. An isolated capture launcher
opened the exploded scene, set its project label and window size, and called the
existing Face-mode and Move-tool commands with a cam-cover face selected. The
editor UI and renderer were unchanged. Capture launcher changes are not included
in the application.

`site/assets/engine-editing-shaded.webp` and `engine-editing-wireframe.webp`
show the exact same camera and editing selection; only viewport mode differs.
The website clips the two captures along a draggable divider. This is a website
comparison, not an app split-viewport feature. Original screenshots are encoded
as WebP without compositing or retouching, and each can be opened separately.

To recreate, open the exploded scene in Forma Dark, select the right cam cover,
choose Face mode and its top face, and activate Move. Capture the app window in
Material Preview and Wireframe without moving the camera. Renderer and operating
system differences can change the exact appearance.

The previous assembled-scene captures remain at `site/assets/engine-workspace.webp`,
`engine-solid.webp`, and `engine-wireframe.webp` for existing image links.

The scene and generator use the same license terms as the repository.

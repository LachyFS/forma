Forma 0.2.0 adds precision modelling tools and CAD mesh exchange:

- Enter dimensions, positions and rotations using arithmetic, parentheses, scientific notation and explicit units, including millimetres, fractional inches and radians. Copy/paste exact values and move between inspector fields with Tab or Shift+Tab.
- Bevel closed convex meshes, triangulate selected faces, flip normals and repair surface orientation. Bevel uses a world-space width and currently supports a single segment across the whole selected mesh.
- Import ASCII or binary STL with explicit units. Export selected or visible geometry as binary STL, with transforms and mirrored winding handled correctly. STL preserves Forma's Y-up axes and contains no materials.
- Apply scale or rotation and scale, and center object origins while preserving world geometry, child objects and unrelated linked instances. Each operation supports undo and redo.
- Inspect world dimensions and selected edge lengths, frame all visible geometry, use opposite orthographic views, and choose PNG, OBJ or STL from the titlebar Formats menu. Inspector fields remain accessible in smaller windows.

This release also includes the picking, visibility-cache and geometry-allocation improvements merged since 0.1.2.

Download the package for your operating system and processor below. `SHA256SUMS` contains SHA-256 checksums for every asset. The Open Image Denoise runtime is included; no separate denoiser installation is needed. Packaged builds now check for authenticated updates in the background. Choose **Updates → Restart to update** when ready; unsaved work offers Save, Discard, or Cancel. Older portable builds need this first installation manually. The `.nupkg` and `update-*.json` files are for the updater.

- **macOS:** Unzip and move `Forma.app` to Applications. Choose `macos-arm64` for Apple silicon or `macos-x86_64` for Intel. These builds are ad-hoc signed, not Developer ID signed or notarized; macOS may require approval in System Settings → Privacy & Security.
- **Windows x64:** Run the `.exe` installer. These builds are unsigned and may trigger SmartScreen. A DirectX 12 GPU/driver is required.
- **Linux x86_64:** Make the `.AppImage` executable (`chmod +x Forma-*.AppImage`) and run it from a writable location. Built on Ubuntu 24.04; requires glibc 2.39+, desktop runtime libraries, an X11/Wayland session, and a Vulkan GPU/driver. Install the FUSE 2 compatibility library, or launch with `APPIMAGE_EXTRACT_AND_RUN=1` if FUSE is unavailable.

Keep the macOS app bundle intact. See the repository README for controls, platform dependencies, and current limitations.

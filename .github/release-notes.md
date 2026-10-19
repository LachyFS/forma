Forma 0.2.3 improves Solid and Material Preview navigation:

- Solid uses a dedicated Metal kernel with cached hardware acceleration for primary visibility and selection on supported Macs. Solid and Material Preview reuse the same acceleration structures when switching modes.
- Opaque Material Preview scenes use a smaller shading pipeline. Glass, textures, custom shaders, contact shading, and selection outlines retain their existing behavior.
- Untextured built-in materials skip unnecessary texture-coordinate calculations on both native Metal and portable renderers.
- Fixes portable Material Preview lighting on horizontal surfaces, which could previously appear black when their normals pointed at an environment-map pole.
- Fixes a crash while editing narrow numeric, name, and search fields.
- Full backing-pixel resolution, four preview antialiasing samples, and eight contact-shading rays are retained. Regression tests compare optimized and reference rendering, including mode changes and geometry-cache invalidation.
- The navigation benchmark accepts project files and viewport sizes so complex scenes can be measured reproducibly. See `docs/VALIDATION.md` for measured results and their scope.
- Native packages cover macOS Apple silicon, macOS Intel, Windows x64, and Linux x86_64, with the Open Image Denoise runtime included.

**Upgrading from 0.2.0 or earlier:** download and install this release manually once. Future packaged releases can update through Forma.

**Upgrading from 0.2.1 or later:** use **Updates → Restart to update** once the new version has downloaded, or install the package manually.

Download the package for your operating system and processor below. `SHA256SUMS` contains SHA-256 checksums for every asset. The `.nupkg` and `update-*.json` files are for the updater.

- **macOS:** Unzip and move `Forma.app` to Applications. Choose `macos-arm64` for Apple silicon or `macos-x86_64` for Intel. These builds are ad-hoc signed, not Developer ID signed or notarized; macOS may require approval in System Settings → Privacy & Security.
- **Windows x64:** Run the `.exe` installer. These builds are unsigned and may trigger SmartScreen. A DirectX 12 GPU/driver is required.
- **Linux x86_64:** Make the `.AppImage` executable (`chmod +x Forma-*.AppImage`) and run it from a writable location. Built on Ubuntu 24.04; requires glibc 2.39+, desktop runtime libraries, an X11/Wayland session, and a Vulkan GPU/driver. Install the FUSE 2 compatibility library, or launch with `APPIMAGE_EXTRACT_AND_RUN=1` if FUSE is unavailable.

Keep the macOS app bundle intact. See the repository README for controls, platform dependencies, and current limitations.

Forma 0.2.2 optimizes Material Preview on supported Metal GPUs:

- Selection outlines now use a cached acceleration structure containing only the selected object, removing software BVH traversal from the accelerated preview kernel.
- Camera movement reuses the acceleration structure. Outlines continue through occluders and preserve holes, with full backing-pixel resolution, four antialiasing samples, and eight contact-shading rays retained.
- Adds native regression coverage and an A/B benchmark comparing the previous selection path with GPU-accelerated selection. Actual frame-time gains depend on the Mac and scene.
- Updates the updater dependency to use a patched ZIP library, addressing the newly published extraction security advisories.
- Native packages cover macOS Apple silicon, macOS Intel, Windows x64, and Linux x86_64, with the Open Image Denoise runtime included.

**Upgrading from 0.2.0 or earlier:** download and install this release manually once. Future packaged releases can update through Forma.

**Upgrading from 0.2.1:** use **Updates → Restart to update** once the new version has downloaded, or install the package manually.

Download the package for your operating system and processor below. `SHA256SUMS` contains SHA-256 checksums for every asset. The `.nupkg` and `update-*.json` files are for the updater.

- **macOS:** Unzip and move `Forma.app` to Applications. Choose `macos-arm64` for Apple silicon or `macos-x86_64` for Intel. These builds are ad-hoc signed, not Developer ID signed or notarized; macOS may require approval in System Settings → Privacy & Security.
- **Windows x64:** Run the `.exe` installer. These builds are unsigned and may trigger SmartScreen. A DirectX 12 GPU/driver is required.
- **Linux x86_64:** Make the `.AppImage` executable (`chmod +x Forma-*.AppImage`) and run it from a writable location. Built on Ubuntu 24.04; requires glibc 2.39+, desktop runtime libraries, an X11/Wayland session, and a Vulkan GPU/driver. Install the FUSE 2 compatibility library, or launch with `APPIMAGE_EXTRACT_AND_RUN=1` if FUSE is unavailable.

Keep the macOS app bundle intact. See the repository README for controls, platform dependencies, and current limitations.

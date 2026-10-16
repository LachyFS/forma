Forma 0.2.1 adds authenticated automatic updates:

- Packaged builds check for updates in the background and download new versions automatically. The Updates dialog also offers manual checks, skipped versions, and preferences for automatic checks and downloads.
- Every update uses signed metadata and a verified package checksum before installation. Stable releases and previews stay on separate channels.
- Choose **Updates → Restart to update** when ready. Unsaved work offers Save, Discard, or Cancel, and active imports, exports, or background jobs finish before a restart can proceed. Saved projects reopen after updating.
- Native packages cover macOS Apple silicon, macOS Intel, Windows x64, and Linux x86_64, with the Open Image Denoise runtime included.

**Upgrading from 0.2.0 or earlier:** download and install this release manually once. Future packaged releases can update through Forma.

Download the package for your operating system and processor below. `SHA256SUMS` contains SHA-256 checksums for every asset. The `.nupkg` and `update-*.json` files are for the updater.

- **macOS:** Unzip and move `Forma.app` to Applications. Choose `macos-arm64` for Apple silicon or `macos-x86_64` for Intel. These builds are ad-hoc signed, not Developer ID signed or notarized; macOS may require approval in System Settings → Privacy & Security.
- **Windows x64:** Run the `.exe` installer. These builds are unsigned and may trigger SmartScreen. A DirectX 12 GPU/driver is required.
- **Linux x86_64:** Make the `.AppImage` executable (`chmod +x Forma-*.AppImage`) and run it from a writable location. Built on Ubuntu 24.04; requires glibc 2.39+, desktop runtime libraries, an X11/Wayland session, and a Vulkan GPU/driver. Install the FUSE 2 compatibility library, or launch with `APPIMAGE_EXTRACT_AND_RUN=1` if FUSE is unavailable.

Keep the macOS app bundle intact. See the repository README for controls, platform dependencies, and current limitations.

Download the archive for your operating system and processor below. `SHA256SUMS` contains SHA-256 checksums for all four packages. The Open Image Denoise runtime is included; no separate denoiser installation is needed.

- **macOS:** Unzip and move `Forma.app` to Applications. Choose `macos-arm64` for Apple silicon or `macos-x86_64` for Intel. These builds are ad-hoc signed, not Developer ID signed or notarized; macOS may require approval in System Settings → Privacy & Security.
- **Windows x64:** Extract the entire ZIP and run `forma.exe` inside the extracted directory. These builds are unsigned and may trigger SmartScreen. A DirectX 12 GPU/driver is required.
- **Linux x86_64:** Extract the archive and run `bin/forma` inside the extracted directory. Built on Ubuntu 24.04; requires glibc 2.39+, desktop runtime libraries, an X11/Wayland session, and a Vulkan GPU/driver. This is a portable application directory, not a fully static build.

Keep the bundled `oidn` directory beside the executable. See the repository README for controls, platform dependencies, and current limitations.

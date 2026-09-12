#!/usr/bin/env bash
# Build a portable directory on the host Linux architecture.
set -euo pipefail
profile="${1:-release}"
if [[ $# -gt 1 || ! "$profile" =~ ^(debug|release)$ ]]; then
    printf 'Usage: %s [debug|release]\n' "$0" >&2
    exit 2
fi
if [[ "$(uname -s)" != Linux ]]; then
    printf 'Build the Linux bundle on Linux.\n' >&2
    exit 1
fi
repository="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd -- "$repository"
arguments=(build --locked -p forma --target-dir "$repository/target")
if [[ "$profile" == release ]]; then arguments+=(--release); fi
cargo "${arguments[@]}"
output="$repository/target/$profile"
bundle="$output/forma-linux-$(uname -m)"
if [[ -L "$bundle" ]]; then
    printf 'Refusing symlinked bundle output: %s\n' "$bundle" >&2
    exit 1
fi
mkdir -p -- "$bundle/bin" "$bundle/share/applications" "$bundle/share/icons/hicolor/64x64/apps" "$bundle/share/icons/hicolor/scalable/apps"
install -m 755 "$output/forma" "$bundle/bin/forma"
install -m 644 README.md "$bundle/README.md"
install -m 644 assets/Forma-64.png "$bundle/share/icons/hicolor/64x64/apps/forma.png"
install -m 644 assets/Forma.svg "$bundle/share/icons/hicolor/scalable/apps/forma.svg"
cat > "$bundle/share/applications/studio.forma.editor.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Forma
Comment=Geometry editor and GPU renderer
Exec=forma %f
Icon=forma
Terminal=false
Categories=Graphics;3DGraphics;
DESKTOP
tar -C "$output" -czf "$bundle.tar.gz" "$(basename -- "$bundle")"
printf 'Bundle: %s.tar.gz\nRun: %s/bin/forma\n' "$bundle" "$bundle"

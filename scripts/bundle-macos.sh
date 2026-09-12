#!/bin/bash
# Build a host-architecture local app bundle. All build/package output stays in
# this repository's target directory, regardless of CARGO_TARGET_DIR.
set -euo pipefail

if [[ $# -gt 1 ]]; then
    printf 'Usage: %s [debug|release]\n' "$0" >&2
    exit 2
fi

profile="${1:-release}"
case "$profile" in
    debug|release) ;;
    *) printf 'Expected debug or release, got: %s\n' "$profile" >&2; exit 2 ;;
esac

if [[ "$(uname -s)" != Darwin ]]; then
    printf 'Forma.app requires macOS and a Metal GPU.\n' >&2
    exit 1
fi

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository="$(cd -- "$script_directory/.." && pwd -P)"
target_directory="$repository/target"
profile_directory="$target_directory/$profile"
bundle="$profile_directory/Forma.app"

for directory in "$target_directory" "$profile_directory" "$bundle"; do
    if [[ -L "$directory" ]]; then
        printf 'Refusing a symlinked build output: %s\n' "$directory" >&2
        exit 1
    fi
done

cd -- "$repository"
build_arguments=(build --locked -p forma --target-dir "$target_directory")
if [[ "$profile" == release ]]; then
    build_arguments+=(--release)
fi
cargo "${build_arguments[@]}"

executable="$profile_directory/forma"
if [[ ! -f "$executable" || ! -x "$executable" || -L "$executable" ]]; then
    printf 'Expected a native executable at %s\n' "$executable" >&2
    exit 1
fi

version="$(awk '
    /^\[workspace.package\]/ { package = 1; next }
    package && /^\[/ { exit }
    package && /^version[[:space:]]*=/ { split($0, fields, "\""); print fields[2]; exit }
' "$repository/Cargo.toml")"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    printf 'Expected a numeric workspace package version, got: %s\n' "$version" >&2
    exit 1
fi

icon="$repository/assets/Forma.icns"
icon_generator="$script_directory/generate-icon.swift"
if [[ ! -s "$icon" || "$icon_generator" -nt "$icon" ]]; then
    swift "$icon_generator" "$repository/assets"
fi
if [[ ! -f "$icon" || -L "$icon" ]]; then
    printf 'Expected the generated native icon at %s\n' "$icon" >&2
    exit 1
fi

staging="$(mktemp -d "$profile_directory/.Forma.app.XXXXXX")"
cleanup() {
    if [[ -n "$staging" && -d "$staging" ]]; then
        rm -rf -- "$staging"
    fi
}
trap cleanup EXIT

mkdir -p -- "$staging/Contents/MacOS" "$staging/Contents/Resources"
install -m 755 "$executable" "$staging/Contents/MacOS/forma"
install -m 644 "$icon" "$staging/Contents/Resources/Forma.icns"
cat > "$staging/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Forma</string>
    <key>CFBundleDisplayName</key><string>Forma</string>
    <key>CFBundleIdentifier</key><string>studio.forma.editor</string>
    <key>CFBundleExecutable</key><string>forma</string>
    <key>CFBundleIconFile</key><string>Forma.icns</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST
plutil -lint "$staging/Contents/Info.plist"

# Replace only this script's known output after the complete new bundle exists.
if [[ -e "$bundle" ]]; then
    if [[ ! -d "$bundle" ]]; then
        printf 'Bundle destination is not a directory: %s\n' "$bundle" >&2
        exit 1
    fi
    previous="$staging/previous.app"
    mv -- "$bundle" "$previous"
    if ! mv -- "$staging" "$bundle"; then
        mv -- "$previous" "$bundle"
        printf 'Could not install new app bundle; restored the previous bundle.\n' >&2
        exit 1
    fi
    staging=""
    rm -rf -- "$bundle/previous.app"
else
    mv -- "$staging" "$bundle"
    staging=""
fi

printf 'Built %s\nOpen with: open "%s"\n' "$bundle" "$bundle"

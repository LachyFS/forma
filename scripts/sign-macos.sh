#!/usr/bin/env bash
# Sign nested native libraries before signing the containing application.
set -euo pipefail
if [[ $# != 1 || "$(uname -s)" != Darwin ]]; then
    printf 'Usage on macOS: %s <Forma.app>\n' "$0" >&2
    exit 2
fi
bundle="$1"
if [[ ! -f "$bundle/Contents/MacOS/forma" || ! -d "$bundle/Contents/Frameworks/oidn/lib" ]]; then
    printf 'Expected a complete Forma application bundle: %s\n' "$bundle" >&2
    exit 1
fi
while IFS= read -r -d '' library; do
    codesign --force --sign - "$library"
done < <(find "$bundle/Contents/Frameworks" -type f -name '*.dylib' -print0)
codesign --force --sign - "$bundle"
codesign --verify --deep --strict "$bundle"

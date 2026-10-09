#!/usr/bin/env bash
# Exercise the real packaging/signing scripts with a tiny native executable and
# the actual OIDN runtime, without waiting for a complete Rust application build.
set -euo pipefail
repository="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT
mkdir -p "$fixture/scripts" "$fixture/assets" "$fixture/target/debug" "$fixture/tools"
cp "$repository/Cargo.toml" "$fixture/"
cp "$repository/scripts/bundle-macos.sh" "$repository/scripts/setup-denoiser.py" "$repository/scripts/generate-icon.swift" "$fixture/scripts/"
cp "$repository/assets/Forma.icns" "$fixture/assets/"
printf 'int main(void) { return 0; }\n' | cc -x c -o "$fixture/target/debug/forma" -
printf '#!/bin/sh\nexit 0\n' > "$fixture/tools/cargo"
chmod +x "$fixture/tools/cargo"
PATH="$fixture/tools:$PATH" bash "$fixture/scripts/bundle-macos.sh" debug
bundle="$fixture/target/debug/Forma.app"
bash "$repository/scripts/sign-macos.sh" "$bundle"
FORMA_TEST_BUNDLE="$bundle" python3 - <<'PY'
import ctypes
import os
from pathlib import Path

bundle = Path(os.environ['FORMA_TEST_BUNDLE'])
assert not (bundle / 'Contents/MacOS/oidn').exists()
assert (bundle / 'Contents/Resources/oidn/FORMA-OIDN-VERSION').read_text().strip() == '2.4.1'
library = bundle / 'Contents/Frameworks/oidn/lib/libOpenImageDenoise.2.dylib'
assert not (library.parent / 'cmake').exists()
api = ctypes.CDLL(str(library))
api.oidnNewDevice.argtypes = [ctypes.c_int]
api.oidnNewDevice.restype = ctypes.c_void_p
api.oidnCommitDevice.argtypes = [ctypes.c_void_p]
api.oidnReleaseDevice.argtypes = [ctypes.c_void_p]
api.oidnGetDeviceError.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_char_p)]
api.oidnGetDeviceError.restype = ctypes.c_int
device = api.oidnNewDevice(1)  # CPU; also exercises loading the native device module.
assert device
try:
    api.oidnCommitDevice(device)
    message = ctypes.c_char_p()
    error = api.oidnGetDeviceError(device, ctypes.byref(message))
    assert error == 0, (error, message.value)
finally:
    api.oidnReleaseDevice(device)
print('Signed app bundle and relocated OIDN runtime verified')
PY

#!/usr/bin/env python3
"""Exercise a real signed full-package A -> B upgrade with the native updater."""
import argparse
import base64
import importlib.util
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import time
import zipfile


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def run(platform, vpk, preview=False):
    packager = module("package_update", "package-update.py")
    signer = module("sign_updates", "sign-updates.py")
    before, after = ("1.0.1-beta.1", "1.0.1-beta.2") if preview else ("1.0.0", "1.0.1")
    with tempfile.TemporaryDirectory(prefix="forma-upgrade-") as temporary:
        root = Path(temporary)
        key = subprocess.check_output(["openssl", "genpkey", "-algorithm", "ED25519"])
        key_path = root / "key.pem"
        key_path.write_bytes(key)
        public = subprocess.check_output(["openssl", "pkey", "-in", str(key_path), "-pubout", "-outform", "DER"])
        public = base64.b64encode(public[-32:]).decode()
        public_path = root / "public.txt"
        public_path.write_text(public)
        for version in (before, after):
            subprocess.run(["cargo", "build", "--locked", "-p", "forma-update", "--example", "update_fixture"], env=dict(os.environ, FIXTURE_VERSION=version), check=True)
            executable = "update_fixture.exe" if platform.startswith("windows") else "update_fixture"
            source = root / version / "stage"
            if platform.startswith("macos"):
                source = source / "Forma.app"
                binary_dir = source / "Contents/MacOS"
                binary_dir.mkdir(parents=True)
                resources = source / "Contents/Resources"
                resources.mkdir()
                shutil.copy2("assets/Forma.icns", resources / "Forma.icns")
                with (source / "Contents/Info.plist").open("wb") as stream:
                    plistlib.dump({"CFBundleName": "Forma", "CFBundleIdentifier": "studio.forma.update-fixture", "CFBundleExecutable": executable,
                                  "CFBundleVersion": version, "CFBundleShortVersionString": version, "CFBundlePackageType": "APPL"}, stream)
            else:
                binary_dir = source
                binary_dir.mkdir(parents=True)
            shutil.copy2(Path("target/debug/examples") / executable, binary_dir / executable)
            (binary_dir / "oidn").mkdir()
            (binary_dir / "oidn/fixture-version").write_text(version)
            packager.package("v" + version, platform, source, root / version / "dist", "update_fixture", vpk, root / version / "vpk")
        latest = root / after / "dist"
        signer.sign(latest, key.decode(), public)
        installed = root / "installed"
        installed.mkdir()
        initial = root / before / "dist"
        if platform.startswith("macos"):
            subprocess.run(["ditto", "-x", "-k", str(next(initial.glob("*.zip"))), str(installed)], check=True)
            binary = installed / "Forma.app/Contents/MacOS/update_fixture"
        elif platform.startswith("windows"):
            with zipfile.ZipFile(next((root / before / "vpk").glob("*Portable.zip"))) as archive:
                archive.extractall(installed)
            binary = installed / "current/update_fixture.exe"
        else:
            binary = installed / "Forma.AppImage"
            shutil.copy2(next(initial.glob("*.AppImage")), binary)
            binary.chmod(0o755)
        result = root / "result.txt"
        environment = dict(os.environ, APPIMAGE_EXTRACT_AND_RUN="1")
        subprocess.run([str(binary), "--update", str(latest), str(public_path), str(result)], env=environment, check=True, timeout=90)
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            if result.exists() and result.read_text() == f"{after}\n{after}":
                print(f"PASS: {platform} upgraded and restarted the executable and companion runtime from {before} to {after}")
                return
            time.sleep(0.5)
        raise RuntimeError(f"Update did not relaunch successfully: {result.read_text() if result.exists() else 'no result'}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform")
    parser.add_argument("--vpk", default="vpk")
    parser.add_argument("--preview", action="store_true")
    args = parser.parse_args()
    run(args.platform, args.vpk, args.preview)

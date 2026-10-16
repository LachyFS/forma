#!/usr/bin/env python3
"""Turn a staged native Forma bundle into a Velopack release and signed-feed input."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

RUNTIMES = {"macos-arm64": "osx-arm64", "macos-x86_64": "osx-x64", "windows-x64": "win-x64", "linux-x86_64": "linux-x64"}


def package(tag, platform, source, destination, executable="forma", vpk="vpk", keep_output=None):
    version = tag.removeprefix("v")
    channel = "preview" if "-" in version else "stable"
    destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="forma-vpk-") as temporary:
        output = Path(temporary)
        command = [vpk, "pack", "--packId", "Forma", "--packTitle", "Forma", "--packVersion", version,
                   "--packDir", str(source.resolve()), "--outputDir", str(output),
                   "--channel", f"{channel}-{platform}", "--runtime", RUNTIMES[platform]]
        if platform.startswith("macos"):
            command += ["--signAppIdentity", os.environ.get("FORMA_MAC_SIGN_IDENTITY") or "-"]
            if os.environ.get("FORMA_MAC_INSTALL_IDENTITY"):
                command += ["--signInstallIdentity", os.environ["FORMA_MAC_INSTALL_IDENTITY"]]
            if os.environ.get("FORMA_MAC_NOTARY_PROFILE"):
                command += ["--notaryProfile", os.environ["FORMA_MAC_NOTARY_PROFILE"]]
        else:
            command += ["--mainExe", executable + (".exe" if platform.startswith("windows") else ""),
                        "--icon", str(Path("assets/Forma.ico" if platform.startswith("windows") else "assets/Forma-64.png").resolve())]
        if platform.startswith("windows") and os.environ.get("FORMA_WINDOWS_SIGN_PARAMS"):
            command += ["--signParams", os.environ["FORMA_WINDOWS_SIGN_PARAMS"]]
        subprocess.run(command, check=True)
        if keep_output:
            shutil.copytree(output, keep_output)
        feed = json.loads((output / f"releases.{channel}-{platform}.json").read_text())
        full = [asset for asset in feed["Assets"] if asset["Type"] == "Full" and asset["Version"] == version]
        if len(full) != 1:
            raise ValueError("Expected exactly one full update package")
        asset = full[0]
        package_path = output / asset["FileName"]
        name = f"Forma-{tag}-{platform}-full.nupkg"
        shutil.copy2(package_path, destination / name)
        asset["FileName"] = name
        with package_path.open("rb") as stream:
            asset["SHA256"] = hashlib.file_digest(stream, "sha256").hexdigest()
        asset["Size"] = package_path.stat().st_size
        if platform.startswith("macos"):
            candidates, extension = list(output.glob("*Portable.zip")), "zip"
        elif platform.startswith("windows"):
            candidates, extension = list(output.glob("*Setup.exe")), "exe"
        else:
            candidates, extension = list(output.glob("*.AppImage")), "AppImage"
        if len(candidates) != 1:
            raise ValueError(f"Expected one installer/archive, got {candidates}")
        shutil.copy2(candidates[0], destination / f"Forma-{tag}-{platform}.{extension}")
        payload = {"schema": 1, "repository": "lachyfs/forma", "platform": platform,
                   "channel": channel, "version": version, "asset": asset}
        (destination / f"update-{platform}.payload.json").write_text(json.dumps(payload, sort_keys=True, separators=(",", ":")), encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag")
    parser.add_argument("platform", choices=RUNTIMES)
    parser.add_argument("source", type=Path)
    parser.add_argument("--output", type=Path, default=Path("dist"))
    parser.add_argument("--executable", default="forma")
    parser.add_argument("--vpk", default="vpk")
    args = parser.parse_args()
    package(args.tag, args.platform, args.source, args.output, args.executable, args.vpk)

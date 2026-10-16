#!/usr/bin/env python3
"""Validate release versions and assemble the complete release asset manifest."""
import argparse
import hashlib
import base64
import json
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib

PLATFORMS = {
    "macos-arm64": "zip",
    "macos-x86_64": "zip",
    "windows-x64": "exe",
    "linux-x86_64": "AppImage",
}
TAG = re.compile(r"v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(alpha|beta|rc)\.(0|[1-9]\d*))?")


def validate(tag, manifest):
    match = TAG.fullmatch(tag)
    if not match:
        raise ValueError("Expected vX.Y.Z or vX.Y.Z-{alpha,beta,rc}.N")
    version = ".".join(match.group(1, 2, 3))
    if manifest["workspace"]["package"]["version"] != version:
        raise ValueError("Release tag must match the workspace version in Cargo.toml")
    return match.group(4) is not None


def checksums(directory, tag, unsigned=False, public_key=None):
    expected = {f"Forma-{tag}-{platform}.{extension}" for platform, extension in PLATFORMS.items()}
    expected.update(f"Forma-{tag}-{platform}-full.nupkg" for platform in PLATFORMS)
    suffix = ".payload.json" if unsigned else ".json"
    expected.update(f"update-{platform}{suffix}" for platform in PLATFORMS)
    actual = {path.name for path in directory.iterdir() if path.name != "SHA256SUMS"}
    if actual != expected:
        raise ValueError(f"Incomplete release: missing {expected - actual}, unexpected {actual - expected}")
    if not unsigned and public_key is None:
        public_key = Path("crates/forma-update/update-public-key.txt").read_text()
    for platform in PLATFORMS:
        data = (directory / f"update-{platform}{suffix}").read_bytes()
        if not unsigned:
            envelope = json.loads(data)
            data = base64.b64decode(envelope["payload"], validate=True)
            signature = base64.b64decode(envelope["signature"], validate=True)
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / "public.der").write_bytes(bytes.fromhex("302a300506032b6570032100") + base64.b64decode(public_key.strip(), validate=True))
                (root / "payload").write_bytes(data)
                (root / "signature").write_bytes(signature)
                subprocess.run(["openssl", "pkeyutl", "-verify", "-pubin", "-keyform", "DER", "-inkey", str(root / "public.der"), "-rawin", "-in", str(root / "payload"), "-sigfile", str(root / "signature")], check=True, capture_output=True)
        payload = json.loads(data)
        version = tag.removeprefix("v")
        channel = "preview" if "-" in version else "stable"
        if any(payload.get(key) != value for key, value in {"schema": 1, "repository": "lachyfs/forma", "version": version, "platform": platform, "channel": channel}.items()):
            raise ValueError(f"Incorrect update identity for {platform}")
        asset = payload["asset"]
        package = directory / f"Forma-{tag}-{platform}-full.nupkg"
        if asset["FileName"] != package.name or asset["Version"] != version or asset["PackageId"] != "Forma" or asset["Type"] != "Full":
            raise ValueError(f"Incorrect package identity for {platform}")
        with package.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        if asset["SHA256"] != digest or asset["Size"] != package.stat().st_size:
            raise ValueError(f"Incorrect update hash or size for {platform}")
    lines = []
    for name in sorted(expected):
        path = directory / name
        if not path.is_file() or path.is_symlink() or path.stat().st_size == 0:
            raise ValueError(f"Empty or invalid release asset: {name}")
        with path.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        lines.append(f"{digest}  {name}\n")
    (directory / "SHA256SUMS").write_text("".join(lines), encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag")
    parser.add_argument("--assets", type=Path)
    parser.add_argument("--unsigned", action="store_true")
    parser.add_argument("--public-key", type=Path)
    args = parser.parse_args()
    with Path("Cargo.toml").open("rb") as source:
        prerelease = validate(args.tag, tomllib.load(source))
    if args.assets:
        checksums(args.assets, args.tag, args.unsigned, args.public_key.read_text() if args.public_key else None)
    print(f"prerelease={str(prerelease).lower()}")

#!/usr/bin/env python3
"""Validate release versions and assemble the complete release asset manifest."""
import argparse
import hashlib
from pathlib import Path
import re
import tomllib

PLATFORMS = {
    "macos-arm64": "zip",
    "macos-x86_64": "zip",
    "windows-x64": "zip",
    "linux-x86_64": "tar.gz",
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


def checksums(directory, tag):
    expected = {f"Forma-{tag}-{platform}.{extension}" for platform, extension in PLATFORMS.items()}
    actual = {path.name for path in directory.iterdir() if path.name != "SHA256SUMS"}
    if actual != expected:
        raise ValueError(f"Incomplete release: missing {expected - actual}, unexpected {actual - expected}")
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
    args = parser.parse_args()
    with Path("Cargo.toml").open("rb") as source:
        prerelease = validate(args.tag, tomllib.load(source))
    if args.assets:
        checksums(args.assets, args.tag)
    print(f"prerelease={str(prerelease).lower()}")

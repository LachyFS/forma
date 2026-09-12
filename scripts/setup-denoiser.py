#!/usr/bin/env python3
"""Install the pinned official OIDN runtime next to Forma (Python 3.12+).

Development: python3 scripts/setup-denoiser.py
Packaging:   python3 scripts/setup-denoiser.py --output-dir <executable-dir>/oidn
No administrator privileges, system installs or runtime network access required.
"""
import argparse
import hashlib
import os
from pathlib import Path
import platform
import shutil
import tarfile
import tempfile
import urllib.request
import zipfile

VERSION = "2.4.1"
# SHA-256 digests published on the official RenderKit/oidn v2.4.1 release.
RELEASES = {
    ("Linux", "x86_64"): ("x86_64.linux.tar.gz", "b69ca2443a226ef692ca46bdc4f89995b1e99091f2665b906cbe07e9673e48cc"),
    ("Darwin", "arm64"): ("arm64.macos.tar.gz", "f1d7370bc09242bbd72d405b424ba240fd4d64103f3e607cdfeeaa2f2718cfb8"),
    ("Darwin", "x86_64"): ("x86_64.macos.tar.gz", "b9addf2855ee36d7768fd02d4d540e64612096487bad302e608d04e639ae1584"),
    ("Windows", "x86_64"): ("x64.windows.zip", "682d94ba57525ed177d73412e0ed903f576867bd048f830a5c6f63c56b25e8b8"),
}


def install(output):
    machine = platform.machine().lower()
    machine = {"amd64": "x86_64", "aarch64": "arm64"}.get(machine, machine)
    release = RELEASES.get((platform.system(), machine))
    if release is None:
        raise SystemExit("No official OIDN binary for this host. Build OIDN 2.4+ from source and set FORMA_OIDN_LIBRARY.")
    suffix, digest = release
    archive_name = f"oidn-{VERSION}.{suffix}"
    cache = Path(__file__).resolve().parent.parent / "target" / "oidn-cache"
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / archive_name
    if not archive.exists() or hashlib.sha256(archive.read_bytes()).hexdigest() != digest:
        url = f"https://github.com/RenderKit/oidn/releases/download/v{VERSION}/{archive_name}"
        print(f"Downloading {url}", flush=True)
        with tempfile.NamedTemporaryFile(dir=cache, delete=False) as download:
            temporary = Path(download.name)
            try:
                with urllib.request.urlopen(url, timeout=120) as response:
                    shutil.copyfileobj(response, download)
            except BaseException:
                temporary.unlink(missing_ok=True)
                raise
        if hashlib.sha256(temporary.read_bytes()).hexdigest() != digest:
            temporary.unlink()
            raise SystemExit("OIDN archive checksum mismatch")
        temporary.replace(archive)
    if output.is_symlink():
        raise SystemExit(f"Refusing symlinked runtime directory: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=cache) as unpacked:
        if archive.suffix == ".zip":
            with zipfile.ZipFile(archive) as bundle:
                bundle.extractall(unpacked)
        else:
            with tarfile.open(archive) as bundle:
                bundle.extractall(unpacked, filter="data")
        root = next(Path(unpacked).iterdir())
        # OIDN finds its CPU/GPU modules and weights alongside its main library.
        source = root / ("bin" if os.name == "nt" else "lib")
        with tempfile.TemporaryDirectory(dir=output.parent) as staged:
            runtime = Path(staged) / "oidn"
            runtime.mkdir()
            # Preserve lib/ because upstream Linux/macOS binaries use ../lib rpaths.
            shutil.copytree(source, runtime / source.name, symlinks=os.name != "nt")
            for notice in ("LICENSE.txt", "LICENSE", "THIRD-PARTY-PROGRAMS", "third-party-programs.txt"):
                if (root / notice).is_file():
                    shutil.copy2(root / notice, runtime / notice)
            if (root / "doc").is_dir():
                shutil.copytree(root / "doc", runtime / "doc")
            (runtime / "FORMA-OIDN-VERSION").write_text(VERSION + "\n")
            # Only replace a directory created by this installer.
            if output.exists():
                if not (output / "FORMA-OIDN-VERSION").is_file():
                    raise SystemExit(f"Refusing to replace an unmanaged directory: {output}")
                previous = Path(staged) / "previous"
                output.rename(previous)
                try:
                    runtime.rename(output)
                except BaseException:
                    previous.rename(output)
                    raise
            else:
                runtime.rename(output)
    print(f"Open Image Denoise {VERSION} installed: {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    default_target = Path(os.environ.get("CARGO_TARGET_DIR", Path(__file__).resolve().parent.parent / "target"))
    parser.add_argument("--output-dir", type=Path, default=default_target / "debug" / "oidn")
    arguments = parser.parse_args()
    install(arguments.output_dir.absolute())

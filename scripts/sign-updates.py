#!/usr/bin/env python3
"""Sign completed update manifests; the private key is never passed to build jobs."""
import argparse
import base64
import json
import os
from pathlib import Path
import subprocess
import tempfile


def sign(directory, key, public_key):
    with tempfile.TemporaryDirectory(prefix="forma-sign-") as temporary:
        private = Path(temporary) / "key.pem"
        private.write_text(key, encoding="utf-8")
        private.chmod(0o600)
        public = subprocess.check_output(["openssl", "pkey", "-in", str(private), "-pubout", "-outform", "DER"])
        # RFC 8410 SubjectPublicKeyInfo prefix for an Ed25519 public key.
        expected = bytes.fromhex("302a300506032b6570032100") + base64.b64decode(public_key.strip(), validate=True)
        if public != expected:
            raise ValueError("Signing key does not match the application's public key")
        payloads = sorted(directory.glob("update-*.payload.json"))
        if not payloads:
            raise ValueError("No update manifests to sign")
        for payload in payloads:
            data = payload.read_bytes()
            signature = subprocess.check_output(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(private), "-in", str(payload)])
            envelope = {"payload": base64.b64encode(data).decode(), "signature": base64.b64encode(signature).decode()}
            output = directory / payload.name.replace(".payload.json", ".json")
            output.write_text(json.dumps(envelope, separators=(",", ":")), encoding="utf-8")
            payload.unlink()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--public-key", type=Path, default=Path("crates/forma-update/update-public-key.txt"))
    args = parser.parse_args()
    sign(args.directory, os.environ["FORMA_UPDATE_SIGNING_KEY"], args.public_key.read_text())

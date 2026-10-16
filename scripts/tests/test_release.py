"""Release validation must fail before incomplete or mis-versioned publication."""
import importlib.util
import base64
import hashlib
import json
from pathlib import Path
import tempfile
import subprocess
import unittest

spec = importlib.util.spec_from_file_location("release", Path(__file__).parents[1] / "release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
sign_spec = importlib.util.spec_from_file_location("sign_updates", Path(__file__).parents[1] / "sign-updates.py")
sign_updates = importlib.util.module_from_spec(sign_spec)
sign_spec.loader.exec_module(sign_updates)


def assets(directory):
    for platform, extension in release.PLATFORMS.items():
        (directory / f"Forma-v1.2.3-{platform}.{extension}").write_bytes(b"abc")
        name = f"Forma-v1.2.3-{platform}-full.nupkg"
        (directory / name).write_bytes(b"abc")
        payload = {"schema": 1, "repository": "lachyfs/forma", "platform": platform,
                   "channel": "stable", "version": "1.2.3", "asset": {
                       "FileName": name, "Version": "1.2.3", "PackageId": "Forma", "Type": "Full",
                       "Size": 3, "SHA256": hashlib.sha256(b"abc").hexdigest()}}
        (directory / f"update-{platform}.payload.json").write_text(json.dumps(payload))


class ReleaseTests(unittest.TestCase):
    def test_version_and_prerelease(self):
        manifest = {"workspace": {"package": {"version": "1.2.3"}}}
        self.assertFalse(release.validate("v1.2.3", manifest))
        for stage in ("alpha", "beta", "rc"):
            self.assertTrue(release.validate(f"v1.2.3-{stage}.1", manifest))
        for tag in ("1.2.3", "v1.2", "v01.2.3", "v1.2.3-rc.01", "v1.2.4", "v1.2.3\n", "v1.2.3-other"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.validate(tag, manifest)

    def test_complete_asset_manifest_and_hashes(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with self.assertRaises(ValueError):
                release.checksums(directory, "v1.2.3", unsigned=True)
            assets(directory)
            release.checksums(directory, "v1.2.3", unsigned=True)
            lines = (directory / "SHA256SUMS").read_text().splitlines()
            self.assertEqual(len(lines), 12)
            self.assertEqual(sum(line.startswith("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  ") for line in lines), 8)
            release.checksums(directory, "v1.2.3", unsigned=True)  # Safe to regenerate.
            extra = directory / "unexpected.zip"
            extra.write_bytes(b"abc")
            with self.assertRaises(ValueError):
                release.checksums(directory, "v1.2.3", unsigned=True)
            extra.unlink()
            next(directory.glob("*.zip")).write_bytes(b"")
            with self.assertRaises(ValueError):
                release.checksums(directory, "v1.2.3", unsigned=True)

    def test_sign_verify_and_reject_modified_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "dist"
            directory.mkdir()
            assets(directory)
            key = subprocess.check_output(["openssl", "genpkey", "-algorithm", "ED25519"])
            key_path = Path(temporary) / "key.pem"
            key_path.write_bytes(key)
            public = subprocess.check_output(["openssl", "pkey", "-in", str(key_path), "-pubout", "-outform", "DER"])
            public = base64.b64encode(public[-32:]).decode()
            sign_updates.sign(directory, key.decode(), public)
            release.checksums(directory, "v1.2.3", public_key=public)
            package = next(directory.glob("*-full.nupkg"))
            package.write_bytes(b"abd")
            with self.assertRaises(ValueError):
                release.checksums(directory, "v1.2.3", public_key=public)
            package.write_bytes(b"abc")
            manifest = next(directory.glob("update-*.json"))
            envelope = json.loads(manifest.read_text())
            envelope["signature"] = base64.b64encode(bytes(64)).decode()
            manifest.write_text(json.dumps(envelope))
            with self.assertRaises(subprocess.CalledProcessError):
                release.checksums(directory, "v1.2.3", public_key=public)

    def test_signer_refuses_wrong_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            key = subprocess.check_output(["openssl", "genpkey", "-algorithm", "ED25519"])
            with self.assertRaises(ValueError):
                sign_updates.sign(Path(temporary), key.decode(), base64.b64encode(bytes(32)).decode())


if __name__ == "__main__":
    unittest.main()

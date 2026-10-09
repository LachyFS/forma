"""Release validation must fail before incomplete or mis-versioned publication."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("release", Path(__file__).parents[1] / "release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


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
                release.checksums(directory, "v1.2.3")
            for platform, extension in release.PLATFORMS.items():
                (directory / f"Forma-v1.2.3-{platform}.{extension}").write_bytes(b"abc")
            release.checksums(directory, "v1.2.3")
            lines = (directory / "SHA256SUMS").read_text().splitlines()
            self.assertEqual(len(lines), 4)
            self.assertTrue(all(line.startswith("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  ") for line in lines))
            release.checksums(directory, "v1.2.3")  # Safe to regenerate.
            extra = directory / "unexpected.zip"
            extra.write_bytes(b"abc")
            with self.assertRaises(ValueError):
                release.checksums(directory, "v1.2.3")
            extra.unlink()
            next(directory.glob("*.zip")).write_bytes(b"")
            with self.assertRaises(ValueError):
                release.checksums(directory, "v1.2.3")


if __name__ == "__main__":
    unittest.main()

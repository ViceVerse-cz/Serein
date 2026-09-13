"""Small trust-boundary regression check; native signature checks run during build."""

import argparse
import hashlib
from pathlib import Path
import tempfile
import unittest

from build import validate
from verify_downloads import verify


class RepositoryInputs(unittest.TestCase):
    def test_downloads_require_complete_matching_checksums(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / "serein.deb"
            package.write_bytes(b"synthetic package")
            manifest = root / "SHA256SUMS.txt"
            entry = hashlib.sha256(package.read_bytes()).hexdigest() + "  ./serein.deb\n"
            manifest.write_text(entry + "0" * 64 + "  ./macOS.zip\n")
            verify(root)
            package.write_bytes(b"corrupted")
            with self.assertRaisesRegex(ValueError, "mismatch"):
                verify(root)
            package.write_bytes(b"synthetic package")
            (root / "unlisted.rpm").write_bytes(b"unlisted")
            with self.assertRaisesRegex(ValueError, "missing"):
                verify(root)
            (root / "unlisted.rpm").unlink()
            manifest.write_text(entry + entry)
            with self.assertRaisesRegex(ValueError, "duplicate"):
                verify(root)
            manifest.write_text("0" * 64 + "  ../escape.deb\n")
            with self.assertRaisesRegex(ValueError, "unsafe"):
                verify(root)
            manifest.write_text(entry)
            package.unlink()
            package.symlink_to(manifest)
            with self.assertRaisesRegex(ValueError, "unsafe"):
                verify(root)

    def test_rejects_unsafe_paths_keys_and_urls(self):
        good = dict(distribution="ubuntu-26.04", architecture="amd64",
                    key="A" * 40, base_url="https://packages.example.org/serein")
        validate(argparse.Namespace(**good))
        for field, value in [("distribution", "../escape"), ("architecture", "/amd64"),
                             ("key", "ABC123"), ("key", "A" * 40 + "\n"),
                             ("base_url", "http://example.org"),
                             ("base_url", "https://user:secret@example.org"),
                             ("base_url", "https://example.org/\nenabled=0"),
                             ("base_url", "https://example.org/?token=secret")]:
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                validate(argparse.Namespace(**(good | {field: value})))


if __name__ == "__main__":
    unittest.main()

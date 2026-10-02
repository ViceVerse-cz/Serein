"""Small trust-boundary regression check; native signature checks run during build."""

import argparse
import hashlib
from pathlib import Path
import subprocess
import tempfile
import unittest

from build import input_packages, validate
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
            package.unlink()
            flatpak_pkg = root / "serein.flatpak"
            flatpak_pkg.write_bytes(b"synthetic flatpak")
            flatpak_entry = hashlib.sha256(flatpak_pkg.read_bytes()).hexdigest() + "  ./serein.flatpak\n"
            manifest.write_text(flatpak_entry)
            verify(root)

    def test_mixed_release_packages_are_selected_per_distribution_and_architecture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            names = [
                "serein-v1.0.0-Linux-ubuntu-26.04-serein_1.0.0_arm64.deb",
                "serein-v1.0.0-Linux-ubuntu-26.04-serein_1.0.0_amd64.deb",
                "serein-v1.0.0-Linux-ubuntu-24.04-serein_1.0.0_arm64.deb",
                "serein-v1.0.0-Linux-fedora-44-serein-1.0.0.x86_64.rpm",
                "serein-v1.0.0-Linux-x86_64.flatpak",
                "SHA256SUMS.txt",
            ]
            for name in names:
                (root / name).write_bytes(b"synthetic release package")
            for arch, index in [("arm64", 0), ("amd64", 1)]:
                selected = input_packages(root, "deb", "ubuntu-26.04", arch)
                self.assertEqual(selected, [root / names[index]])
            self.assertEqual(input_packages(root, "rpm", "fedora-44", "x86_64"), [root / names[3]])
            with self.assertRaisesRegex(ValueError, "1–100 packages"):
                input_packages(root, "deb", "ubuntu-26.04", "armhf")
            self.assertEqual(len(input_packages(root, "deb")), 3)

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

    def test_setup_script_syntax_and_default_fingerprint(self):
        setup_sh = Path(__file__).resolve().parent / "setup.sh"
        self.assertTrue(setup_sh.is_file())
        res = subprocess.run(["sh", "-n", str(setup_sh)], capture_output=True, text=True)
        self.assertEqual(res.returncode, 0, res.stderr)
        content = setup_sh.read_text()
        self.assertIn("EXPECTED_FINGERPRINT=", content)
        self.assertIn("CA19DA939E9BCAB500751CE480FE95CAD86141A5", content)

    def test_generate_index(self):
        from build import generate_index
        with tempfile.TemporaryDirectory() as temporary:
            dest = Path(temporary) / "site"
            generate_index(dest)
            index_file = dest / "index.html"
            self.assertTrue(index_file.is_file())
            content = index_file.read_text()
            self.assertIn("Serein Linux Repositories", content)
            self.assertIn("setup.sh", content)


if __name__ == "__main__":
    unittest.main()

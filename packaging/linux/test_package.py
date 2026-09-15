"""Synthetic native package regression check; never launches Serein or installs it."""

from pathlib import Path
import argparse
import shutil
import tempfile
import unittest

import package as packaging

FORMAT = "deb"
ARTIFACTS = None


class NativePackageTest(unittest.TestCase):
    def test_package_allowlist_and_corrupt_archive_detection(self):
        with tempfile.TemporaryDirectory(prefix="serein-debian-test-") as directory:
            root = Path(directory)
            staged = root / "staged"
            staged.mkdir()
            shutil.copyfile("/bin/true", staged / "serein")
            for name in ["README.md", "LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY_NOTICES.md"]:
                (staged / name).write_text("synthetic package fixture\n")
            (staged / "docs").mkdir()
            for source in Path("docs").glob("*.md"):
                (staged / "docs" / source.name).write_text("synthetic documentation\n")
            (staged / "licenses").mkdir()
            for name in ["NotoSansCJK-LICENSE.txt", "NotoSansArabic-OFL.txt", "Inter-OFL.txt",
                         "Twemoji-CC-BY-4.0.txt", "Unicode-LICENSE.txt", "Phosphor-Icons-MIT.txt", "Simple-Icons-CC0.txt"]:
                (staged / "licenses" / name).write_text("synthetic license\n")
            for name in ["licenses/files", "licenses/notifications", "licenses/login",
                         "licenses/voice", "licenses/audio", "licenses/dependencies", "source/hpke-rs"]:
                source = Path("vendor/hpke-rs") if name.startswith("source/") else Path("assets") / name
                shutil.copytree(source, staged / name)
                (staged / name / "stale-nested.log").write_text("synthetic private marker\n")
            # Simulate a dirty dist directory: none of these belong to the archive.
            (staged / "voice").mkdir()
            (staged / "voice/stale.deb").write_text("old package")
            (staged / "debug.log").write_text("synthetic private marker")
            (staged / "docs/stale.log").write_text("synthetic private marker")
            (staged / "previous.deb").write_text("old package")
            if FORMAT != "deb":
                packaging.native_package(staged, "0.1.0-test", FORMAT)
                if FORMAT == "arch":
                    artifact, = staged.glob("*.pkg.tar.*")
                    metadata = packaging.output("bsdtar", "-xOf", str(artifact), ".PKGINFO")
                    self.assertIn("depend = gst-plugins-good", metadata.splitlines())
                    version = next(line.removeprefix("pkgver = ") for line in metadata.splitlines()
                                   if line.startswith("pkgver = "))
                    self.assertEqual(packaging.output("vercmp", version, "0.1.0-1"), "-1")
                if FORMAT == "rpm":
                    artifact, = staged.glob("*.rpm")
                    distro = packaging.platform.freedesktop_os_release()["ID"]
                    plugin = "gstreamer1-plugins-good" if distro == "fedora" else "gstreamer-plugins-good"
                    self.assertIn(plugin, packaging.output("rpm", "-qp", "--requires", str(artifact)).splitlines())
                if ARTIFACTS and FORMAT != "dir":
                    ARTIFACTS.mkdir(parents=True, exist_ok=True)
                    for artifact in staged.glob("*.rpm" if FORMAT == "rpm" else "*.pkg.tar.*"):
                        shutil.copyfile(artifact, ARTIFACTS / artifact.name)
                if FORMAT == "dir":
                    listing = "\n".join(packaging.payload_files(staged / "linux-root"))
                    for excluded in ["debug.log", "stale.log", "stale.deb", "previous.deb", "stale-nested.log"]:
                        self.assertNotIn(excluded, listing)
                    self.assertIn("licenses/voice/", listing)
                    with self.assertRaisesRegex(ValueError, "already exists"):
                        packaging.native_package(staged, "0.1.0-test", FORMAT)
                with self.assertRaisesRegex(ValueError, "semantic application version"):
                    packaging.native_package(staged, "0.1.0\nmalformed", FORMAT)
                (staged / "serein").write_bytes(b"MZ synthetic wrong architecture")
                with self.assertRaisesRegex(ValueError, "ELF executable"):
                    packaging.native_package(staged, "0.1.0", FORMAT)
                return
            packaging.package(staged, "0.1.0-test")
            artifact = next(staged.glob("serein_*.deb"))
            self.assertEqual(packaging.output("dpkg-deb", "--field", str(artifact), "Package"), "serein")
            self.assertIn("gstreamer1.0-plugins-good", packaging.output(
                "dpkg-deb", "--field", str(artifact), "Depends").split(", "))
            listing = packaging.output("dpkg-deb", "--contents", str(artifact))
            for excluded in ["debug.log", "stale.log", "stale.deb", "previous.deb", "stale-nested.log"]:
                self.assertNotIn(excluded, listing)
            self.assertNotIn("usr/share/doc/serein/docs/", listing)
            self.assertNotIn("source/hpke-rs/", listing)
            self.assertIn("licenses/dependencies/PROVENANCE.md", listing)
            self.assertIn("licenses/voice/", listing)
            # Preserve valid metadata while making the expected payload disagree.
            wrong_stage = root / "wrong-stage"
            wrong_stage.mkdir()
            check = root / "check"
            check.mkdir()
            with self.assertRaisesRegex(ValueError, "payload differs"):
                packaging.smoke(
                    artifact, wrong_stage, check, "0.1.0~test-1",
                    packaging.output("dpkg", "--print-architecture"),
                    packaging.output("dpkg-deb", "--field", str(artifact), "Depends"))
            (staged / "serein").write_bytes(b"MZ synthetic wrong architecture")
            with self.assertRaisesRegex(ValueError, "ELF executable"):
                packaging.package(staged, "0.1.0")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--format", choices=["deb", "rpm", "arch", "dir"], default="deb")
    parser.add_argument("--artifacts", type=Path, help="Retain synthetic packages for repository checks")
    args, remaining = parser.parse_known_args()
    FORMAT = args.format
    ARTIFACTS = args.artifacts
    unittest.main(argv=[__file__, *remaining])

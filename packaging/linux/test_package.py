"""Synthetic native package regression check; never launches Serein or installs it."""

from pathlib import Path
import argparse
import shutil
import subprocess
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
            (staged / "lib").mkdir()
            # Exercise the actual private SONAME closure without any media APIs.
            for name, symbol in (("libavutil-serein.so.59", "synthetic_util"), ("libopenh264-serein.so.8", "synthetic_h264")):
                source = root / (symbol + ".c")
                source.write_text(f"int {symbol}(void) {{ return 0; }}\n")
                subprocess.run(["cc", "-shared", "-fPIC", "-Wl,-soname," + name,
                                "-o", str(staged / "lib" / name), str(source)], check=True)
            source = root / "codec.c"
            source.write_text("int synthetic_util(void); int synthetic_h264(void);\n"
                              "int synthetic_codec(void) { return synthetic_util() + synthetic_h264(); }\n")
            subprocess.run(["cc", "-shared", "-fPIC", "-Wl,-soname,libavcodec-serein.so.61", "-Wl,-rpath,$ORIGIN",
                            "-o", str(staged / "lib/libavcodec-serein.so.61"), str(source), "-L" + str(staged / "lib"),
                            "-l:libavutil-serein.so.59", "-l:libopenh264-serein.so.8"], check=True)
            source = root / "main.c"
            source.write_text("int synthetic_codec(void); int main(void) { return synthetic_codec(); }\n")
            subprocess.run(["cc", "-o", str(staged / "serein"), str(source), "-L" + str(staged / "lib"),
                            "-Wl,-rpath,$ORIGIN/lib:$ORIGIN/../lib/serein", "-Wl,-rpath-link," + str(staged / "lib"),
                            "-l:libavcodec-serein.so.61"], check=True)
            (staged / "ffmpeg-source/source").mkdir(parents=True)
            for name in ("build.json", "configure.json", "build-ffmpeg.py", "serein-ffmpeg.patch", "serein-openh264.patch", "COPYING.LGPLv2.1", "OpenH264-LICENSE",
                         "nv-codec-headers-README", "source/ffmpeg-7.1.5.tar.xz", "source/openh264-2.6.0-source.tar.bz2",
                         "source/nv-codec-headers-12.2.72.0.tar.gz", "AMF-LICENSE", "source/AMF-1.4.36-headers.tar",
                         "oneVPL-LICENSE", "oneVPL-third-party-programs.txt", "source/libvpl-2.14.0.tar.gz"):
                (staged / "ffmpeg-source" / name).write_text("synthetic FFmpeg provenance\n")
            vulkan_files = ("Vulkan-Headers-LICENSE.md", "Vulkan-Headers-LICENSES/Apache-2.0.txt",
                            "Vulkan-Headers-LICENSES/MIT.txt", "source/Vulkan-Headers-1.3.290.tar.gz")
            for name in vulkan_files:
                path = staged / "ffmpeg-source" / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("synthetic Vulkan source/notice\n")
            (staged / "ffmpeg-source/source/AMF-1.4.36.tar.gz").write_text("full SDK must not ship\n")
            (staged / "lib/libvpl.so.2").write_text("dynamic dispatcher must not ship\n")
            for name in ["README.md", "LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY_NOTICES.md"]:
                (staged / name).write_text("synthetic package fixture\n")
            (staged / "docs").mkdir()
            for source in Path("docs").glob("*.md"):
                (staged / "docs" / source.name).write_text("synthetic documentation\n")
            (staged / "licenses").mkdir()
            for name in ["NotoSansCJK-LICENSE.txt", "NotoSansArabic-OFL.txt", "NotoSansMath-OFL.txt", "NotoSansSymbols2-OFL.txt", "Inter-OFL.txt",
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
                    self.assertTrue(any(line.startswith("optdepend = gst-plugin-va:") for line in metadata.splitlines()))
                    self.assertTrue(any(line.startswith("optdepend = gst-plugins-bad-libs:") for line in metadata.splitlines()))
                    version = next(line.removeprefix("pkgver = ") for line in metadata.splitlines()
                                   if line.startswith("pkgver = "))
                    self.assertEqual(packaging.output("vercmp", version, "0.1.0-1"), "-1")
                if FORMAT == "rpm":
                    artifact, = staged.glob("*.rpm")
                    distro = packaging.platform.freedesktop_os_release()["ID"]
                    plugin = "gstreamer1-plugins-good" if distro == "fedora" else "gstreamer-plugins-good"
                    self.assertIn(plugin, packaging.output("rpm", "-qp", "--requires", str(artifact)).splitlines())
                    hardware_plugin = "gstreamer1-plugins-bad-free" if distro == "fedora" else "gstreamer-plugins-bad"
                    self.assertIn(hardware_plugin, packaging.output("rpm", "-qp", "--recommends", str(artifact)).splitlines())
                if ARTIFACTS and FORMAT != "dir":
                    ARTIFACTS.mkdir(parents=True, exist_ok=True)
                    for artifact in staged.glob("*.rpm" if FORMAT == "rpm" else "*.pkg.tar.*"):
                        shutil.copyfile(artifact, ARTIFACTS / artifact.name)
                if FORMAT == "dir":
                    listing = "\n".join(packaging.payload_files(staged / "linux-root"))
                    for name in vulkan_files:
                        self.assertIn("ffmpeg-source/" + name, listing)
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
            self.assertIn("gstreamer1.0-plugins-base", packaging.output(
                "dpkg-deb", "--field", str(artifact), "Depends").split(", "))
            self.assertIn("gstreamer1.0-plugins-bad", packaging.output(
                "dpkg-deb", "--field", str(artifact), "Recommends").split(", "))
            listing = packaging.output("dpkg-deb", "--contents", str(artifact))
            for excluded in ["debug.log", "stale.log", "stale.deb", "previous.deb", "stale-nested.log"]:
                self.assertNotIn(excluded, listing)
            self.assertNotIn("usr/share/doc/serein/docs/", listing)
            self.assertNotIn("source/hpke-rs/", listing)
            self.assertIn("licenses/dependencies/PROVENANCE.md", listing)
            self.assertIn("licenses/voice/", listing)
            self.assertIn("usr/lib/serein/libavcodec-serein.so.61", listing)
            self.assertIn("ffmpeg-source/source/ffmpeg-7.1.5.tar.xz", listing)
            for name in vulkan_files:
                self.assertIn("ffmpeg-source/" + name, listing)
            for name in ("openh264-2.6.0-source.tar.bz2", "AMF-1.4.36-headers.tar", "libvpl-2.14.0.tar.gz",
                         "AMF-LICENSE", "oneVPL-LICENSE", "oneVPL-third-party-programs.txt"):
                self.assertIn(name, listing)
            self.assertNotIn("AMF-1.4.36.tar.gz", listing)
            self.assertNotIn("libvpl.so", listing)
            depends = packaging.output("dpkg-deb", "--field", str(artifact), "Depends")
            self.assertNotIn("libavcodec", depends)
            self.assertNotIn("libopenh264", depends)
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

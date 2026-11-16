"""Offline release archive checks; fixtures never execute a Windows binary."""

from pathlib import Path
import tempfile
import unittest
import zipfile

from check_update_archive import check_archive


class UpdateArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="serein-windows-media-archive-")
        self.root = Path(self.temporary.name)
        self.addCleanup(self.temporary.cleanup)
        self.payload = {
            name: b"synthetic fixture; never loaded"
            for name in (
                "serein.exe", "avcodec-serein-61.dll", "avutil-serein-59.dll", "openh264.dll",
                "avformat-serein-61.dll", "avfilter-serein-10.dll", "swscale-serein-8.dll", "swresample-serein-5.dll",
                "msvcp140.dll", "msvcp140_1.dll", "vcruntime140.dll", "vcruntime140_1.dll",
                "voice-enhancement/onnxruntime.dll",
    'voice-enhancement/eigen-1d8b82b0740839c0de7f1242a3585e3390ff5f33.zip', "voice-enhancement/backbone.onnx",
                "voice-enhancement/spec_head.onnx", "voice-enhancement/denoiser_core.onnx",
                "voice-enhancement/ONNXRuntime-LICENSE", "voice-enhancement/ONNXRuntime-ThirdPartyNotices.txt",
                "voice-enhancement/MSVC-LICENSE.txt", "voice-enhancement/build.json",
                "THIRD_PARTY_NOTICES.md", "licenses/voice/fixture.txt", "README.md",
                "ffmpeg-source/build.json", "ffmpeg-source/configure.json",
                "ffmpeg-source/build-ffmpeg.py", "ffmpeg-source/serein-ffmpeg.patch",
                "ffmpeg-source/serein-dav1d.patch",
                "ffmpeg-source/COPYING.LGPLv2.1", "ffmpeg-source/COPYING.GPLv2", "ffmpeg-source/OpenH264-LICENSE",
                "ffmpeg-source/camera-filter/denoise.rs", "ffmpeg-source/camera-filter/gpu.rs", "ffmpeg-source/camera-filter/camera_filter.wgsl",
                "ffmpeg-source/dav1d-COPYING", "ffmpeg-source/source/dav1d-1.5.1.tar.xz",
                "ffmpeg-source/source/ffmpeg-7.1.5.tar.xz",
                "ffmpeg-source/source/openh264-2.6.0-source.tar.bz2",
            )
        }

    def archive(self, name, payload=None):
        path = self.root / name
        with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for filename, data in (self.payload if payload is None else payload).items():
                archive.writestr(filename, data)
        return path

    def test_complete_packages_for_both_architectures_and_channels(self):
        for version in ("v1.2.3", "v1.2.3-nightly.20261005.1"):
            for arch in ("X64", "ARM64"):
                check_archive(self.archive(f"serein-{version}-Windows-{arch}-media-v5.zip"))

    def test_media_payload_cannot_be_published_under_legacy_update_name(self):
        for arch in ("X64", "ARM64"):
            for suffix in ("", "-media-v2", "-media-v3", "-media-v4"):
                with self.assertRaisesRegex(ValueError, "legacy update name"):
                    check_archive(self.archive(f"serein-v1.2.3-Windows-{arch}{suffix}.zip"))

    def test_missing_empty_libraries_or_corresponding_source_are_rejected(self):
        for filename in (
            "avcodec-serein-61.dll", "avutil-serein-59.dll", "openh264.dll",
            "avformat-serein-61.dll", "avfilter-serein-10.dll", "swscale-serein-8.dll", "swresample-serein-5.dll",
            "ffmpeg-source/COPYING.GPLv2", "ffmpeg-source/camera-filter/camera_filter.wgsl", "ffmpeg-source/dav1d-COPYING", "ffmpeg-source/source/dav1d-1.5.1.tar.xz",
            "ffmpeg-source/serein-dav1d.patch",
            "ffmpeg-source/source/ffmpeg-7.1.5.tar.xz", "licenses/voice/fixture.txt",
            "voice-enhancement/denoiser_core.onnx", "voice-enhancement/onnxruntime.dll",
            "voice-enhancement/backbone.onnx", "voice-enhancement/spec_head.onnx",
            "msvcp140_1.dll", "voice-enhancement/ONNXRuntime-LICENSE",
        ):
            for empty in (False, True):
                payload = dict(self.payload)
                if empty:
                    payload[filename] = b""
                else:
                    del payload[filename]
                with self.subTest(filename=filename, empty=empty):
                    with self.assertRaisesRegex(ValueError, "Incomplete"):
                        check_archive(self.archive("serein-v1.2.3-Windows-X64-media-v5.zip", payload))

    def test_accidental_package_nesting_or_installer_in_zip_is_rejected(self):
        for payload in (
            {f"dist/{name}": value for name, value in self.payload.items()},
            self.payload | {"serein-1.2.3-setup.exe": b"synthetic installer"},
        ):
            with self.assertRaisesRegex(ValueError, "Unexpected update payload"):
                check_archive(self.archive("serein-v1.2.3-Windows-X64-media-v5.zip", payload))

    def test_corrupted_payload_is_rejected_before_publication(self):
        path = self.root / "serein-v1.2.3-Windows-X64-media-v5.zip"
        with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_STORED) as archive:
            for filename, data in self.payload.items():
                archive.writestr(filename, data)
        with zipfile.ZipFile(path) as archive:
            entry = archive.getinfo("serein.exe")
            offset = entry.header_offset + 30 + len(entry.filename.encode()) + len(entry.extra)
        with path.open("r+b") as stream:
            stream.seek(offset)
            stream.write(b"X")
        with self.assertRaisesRegex(ValueError, "Corrupt Windows update payload"):
            check_archive(path)


if __name__ == "__main__":
    unittest.main()

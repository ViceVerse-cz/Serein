"""Check a Windows media-v5 release ZIP before uploading it as an update asset."""

import argparse
from pathlib import Path
import re
import stat
import zipfile


REQUIRED_FILES = (
    "serein.exe",
    "avcodec-serein-61.dll",
    "avutil-serein-59.dll",
    "avformat-serein-61.dll", "avfilter-serein-10.dll",
    "swscale-serein-8.dll",
    "swresample-serein-5.dll",
    "openh264.dll",
    "msvcp140.dll", "msvcp140_1.dll", "vcruntime140.dll", "vcruntime140_1.dll",
    "voice-enhancement/onnxruntime.dll",
    'voice-enhancement/eigen-1d8b82b0740839c0de7f1242a3585e3390ff5f33.zip',
    "voice-enhancement/backbone.onnx", "voice-enhancement/spec_head.onnx", "voice-enhancement/denoiser_core.onnx",
    "voice-enhancement/ONNXRuntime-LICENSE", "voice-enhancement/ONNXRuntime-ThirdPartyNotices.txt",
    "voice-enhancement/MSVC-LICENSE.txt", "voice-enhancement/build.json",
    "THIRD_PARTY_NOTICES.md",
    "ffmpeg-source/build.json",
    "ffmpeg-source/configure.json",
    "ffmpeg-source/build-ffmpeg.py",
    "ffmpeg-source/serein-ffmpeg.patch",
    "ffmpeg-source/serein-dav1d.patch",
    "ffmpeg-source/COPYING.LGPLv2.1",
    "ffmpeg-source/COPYING.GPLv2",
    "ffmpeg-source/camera-filter/denoise.rs",
    "ffmpeg-source/camera-filter/gpu.rs",
    "ffmpeg-source/camera-filter/camera_filter.wgsl",
    "ffmpeg-source/OpenH264-LICENSE",
    "ffmpeg-source/dav1d-COPYING",
    "ffmpeg-source/source/ffmpeg-7.1.5.tar.xz",
    "ffmpeg-source/source/openh264-2.6.0-source.tar.bz2",
    "ffmpeg-source/source/dav1d-1.5.1.tar.xz",
)
ALLOWED_ROOTS = {
    "serein.exe", "avcodec-serein-61.dll", "avutil-serein-59.dll", "openh264.dll",
    "avformat-serein-61.dll", "avfilter-serein-10.dll", "swscale-serein-8.dll", "swresample-serein-5.dll",
    "ffmpeg-source", "README.md", "LICENSE-MIT", "LICENSE-APACHE",
    "THIRD_PARTY_NOTICES.md", "docs", "licenses", "source",
    "install-notifications.ps1", "setup.ps1",
    "voice-enhancement", "msvcp140.dll", "msvcp140_1.dll", "vcruntime140.dll", "vcruntime140_1.dll",
}


def check_archive(path):
    path = Path(path)
    if not re.fullmatch(r"serein-v[A-Za-z0-9.+-]{1,95}-Windows-(X64|ARM64)-media-v5\.zip", path.name):
        raise ValueError("Windows FFmpeg ZIPs must use the media-v5 asset name, never the legacy update name")
    if path.stat().st_size > 512 * 1024 * 1024:
        raise ValueError("Windows update ZIP exceeds the download limit")
    with zipfile.ZipFile(path) as archive:
        entries = archive.infolist()
        if len(entries) > 8192 or sum(entry.file_size for entry in entries) > 1024 * 1024 * 1024:
            raise ValueError("Windows update ZIP exceeds the extracted package limit")
        seen, files = set(), set()
        for entry in entries:
            name = entry.filename.rstrip("/")
            parts = name.split("/")
            if "\\" in name or any(part in ("", ".", "..") for part in parts):
                raise ValueError(f"Invalid update path: {entry.filename}")
            if parts[0] not in ALLOWED_ROOTS:
                raise ValueError(f"Unexpected update payload: {entry.filename}")
            if name.lower() in seen:
                raise ValueError(f"Duplicate update path: {entry.filename}")
            seen.add(name.lower())
            mode = stat.S_IFMT(entry.external_attr >> 16)
            if entry.flag_bits & 1 or mode not in (0, stat.S_IFREG, stat.S_IFDIR):
                raise ValueError(f"Encrypted or special update entry: {entry.filename}")
            if not entry.is_dir() and entry.file_size:
                files.add(name)
        missing = set(REQUIRED_FILES) - files
        if missing or not any(name.startswith("licenses/") for name in files):
            raise ValueError(f"Incomplete Windows media package: {sorted(missing)}; bundled licenses are required")
        corrupt = archive.testzip()
        if corrupt is not None:
            raise ValueError(f"Corrupt Windows update payload: {corrupt}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    args = parser.parse_args()
    check_archive(args.archive)
    print(f"Windows media-v5 update asset checked: {args.archive.name}")


if __name__ == "__main__":
    main()

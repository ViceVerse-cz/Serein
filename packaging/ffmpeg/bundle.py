"""Stage the pinned shared FFmpeg build and its corresponding source."""

import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import tempfile


LIBRARIES = {
    "Linux": ("libavcodec-serein.so.61", "libavutil-serein.so.59", "libopenh264-serein.so.8"),
    "Darwin": ("libavcodec-serein.61.dylib", "libavutil-serein.59.dylib", "libopenh264.8.dylib"),
    "Windows": ("avcodec-serein-61.dll", "avutil-serein-59.dll", "openh264.dll"),
}


def replace_darwin_library(origin, destination):
    # A launched app may still map this inode, and macOS caches its code signature.
    # Replace the file after staging completes, as xtask does for the executable.
    with tempfile.NamedTemporaryFile(prefix=f".{destination.name}.", suffix=".staging",
                                     dir=destination.parent, delete=False) as temporary:
        staging = Path(temporary.name)
    try:
        shutil.copyfile(origin, staging)
        shutil.copymode(destination if destination.exists() else origin, staging)
        os.replace(staging, destination)
    finally:
        staging.unlink(missing_ok=True)


def bundle(root, prefix=None):
    system = platform.system()
    if system not in LIBRARIES:
        raise ValueError(f"Unsupported FFmpeg bundle platform: {system}")
    value = prefix or os.environ.get("FFMPEG_DIR")
    if not value:
        raise ValueError("Set FFMPEG_DIR to the prefix produced by scripts/build-ffmpeg.py")
    prefix = Path(value).resolve()
    provenance = prefix / "share/serein-ffmpeg"
    recipe = json.loads((provenance / "build.json").read_text())
    if recipe["system"] != system or recipe["sources"]["ffmpeg"]["sha256"] != \
            "de668509caf9e35e3cd162473441fdb29538c6d96ed080292b3cf9e6fc5d558f":
        raise ValueError("Expected Serein's pinned LGPL-only FFmpeg build")
    if system == "Darwin":
        libraries = root / "Serein.app/Contents/Frameworks"
        source = root / "Serein.app/Contents/Resources/ffmpeg-source"
    else:
        libraries = root if system == "Windows" else root / "lib"
        source = root / "ffmpeg-source"
    libraries.mkdir(parents=True, exist_ok=True)
    source.mkdir(parents=True, exist_ok=True)
    for name in LIBRARIES[system]:
        origin = prefix / ("bin" if system == "Windows" else "lib") / name
        # Installed linker aliases are intentionally dereferenced: package archives
        # contain regular files, preserving the existing no-symlink payload policy.
        if system == "Darwin":
            replace_darwin_library(origin, libraries / name)
        else:
            shutil.copyfile(origin, libraries / name)
    for name in ("build.json", "configure.json", "build-ffmpeg.py", "serein-ffmpeg.patch", "COPYING.LGPLv2.1", "OpenH264-LICENSE"):
        shutil.copyfile(provenance / name, source / name)
    if system == "Linux":
        shutil.copyfile(provenance / "serein-openh264.patch", source / "serein-openh264.patch")
    (source / "source").mkdir(exist_ok=True)
    for name in ("ffmpeg-7.1.5.tar.xz", "openh264-2.6.0-source.tar.bz2"):
        shutil.copyfile(provenance / "source" / name, source / "source" / name)
    if recipe["nvenc"]:
        shutil.copyfile(provenance / "nv-codec-headers-README", source / "nv-codec-headers-README")
        name = "nv-codec-headers-12.2.72.0.tar.gz"
        shutil.copyfile(provenance / "source" / name, source / "source" / name)
    if recipe["amf"]:
        shutil.copyfile(provenance / "AMF-LICENSE", source / "AMF-LICENSE")
        name = "AMF-1.4.36-headers.tar"
        shutil.copyfile(provenance / "source" / name, source / "source" / name)
        if system == "Linux":
            for name in ("Vulkan-Headers-LICENSE.md", "Vulkan-Headers-LICENSES/Apache-2.0.txt",
                         "Vulkan-Headers-LICENSES/MIT.txt", "source/Vulkan-Headers-1.3.290.tar.gz"):
                destination = source / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(provenance / name, destination)
    if recipe["qsv"]:
        for name in ("oneVPL-LICENSE", "oneVPL-third-party-programs.txt"):
            shutil.copyfile(provenance / name, source / name)
        name = "libvpl-2.14.0.tar.gz"
        shutil.copyfile(provenance / "source" / name, source / "source" / name)
    print(f"Staged FFmpeg shared libraries and corresponding source: {root}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("staging_root", type=Path)
    args = parser.parse_args()
    bundle(args.staging_root)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Reproduce bundled GPU patches from the pinned pristine FFmpeg source archive.

No network, drivers, encoder initialization or frames. A successful round trip
proves the shipped LGPL patch reconstructs exactly the sources the recipe uses.
"""
import argparse
import hashlib
import importlib.util
from pathlib import Path
import subprocess
import tarfile
import tempfile


def run(archive):
    repository = Path(__file__).resolve().parents[4]
    spec = importlib.util.spec_from_file_location("ffmpeg_recipe", repository / "scripts/build-ffmpeg.py")
    recipe = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(recipe)
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == recipe.SOURCES["ffmpeg"]["sha256"]
    with tempfile.TemporaryDirectory(prefix="serein-ffmpeg-patch-") as directory:
        root = Path(directory)
        modified = root / "modified"
        reproduced = root / "reproduced"
        # Only these upstream files are involved; no large build/capture fixture.
        paths = ["configure", "libavcodec/libavcodec.v", "libavutil/libavutil.v", "libavcodec/qsvenc.c",
                 "libavcodec/amfenc.c", "libavcodec/amfenc.h", "libavcodec/amfenc_hevc.c",
                 "libavcodec/amfenc_av1.c", "libavcodec/videotoolboxenc.c", "libavutil/hwcontext_vulkan.c"]
        with tarfile.open(archive) as source:
            for path in paths:
                content = source.extractfile("ffmpeg-7.1.5/" + path).read()
                for tree in [modified, reproduced]:
                    (tree / path).parent.mkdir(parents=True, exist_ok=True)
                    (tree / path).write_bytes(content)
        patch = recipe.patch_ffmpeg(modified)
        subprocess.run(["patch", "--batch", "--fuzz=0", "-p1"], input=patch, text=True,
                       cwd=reproduced, stdout=subprocess.DEVNULL, check=True)
        original_paths = paths.copy()
        generated_paths = ["libavcodec/serein_qsv_feature_validation.h", "libavcodec/serein_amf_split_encoding.h"]
        paths.extend(generated_paths)
        for path in paths:
            assert (modified / path).read_bytes() == (reproduced / path).read_bytes(), path
        subprocess.run(["patch", "--batch", "--fuzz=0", "--reverse", "-p1"], input=patch, text=True,
                       cwd=reproduced, stdout=subprocess.DEVNULL, check=True)
        with tarfile.open(archive) as source:
            for path in original_paths:
                assert (reproduced / path).read_bytes() == source.extractfile("ffmpeg-7.1.5/" + path).read()
        for path in generated_paths:
            assert not (reproduced / path).exists()
        try:
            recipe.patch_ffmpeg(modified)
        except ValueError:
            pass
        else:
            raise AssertionError("Already-patched source must reject a second application")
    print("Pinned FFmpeg GPU patches reproduce byte-for-byte, reverse cleanly and reject duplicate application")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True, type=Path)
    run(parser.parse_args().archive.resolve())

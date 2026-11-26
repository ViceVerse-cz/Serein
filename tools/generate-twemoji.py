#!/usr/bin/env python3
"""Rebuild bundled scalable Twemoji artwork and its index; requires the zstd command."""

import argparse
import hashlib
import io
import struct
from pathlib import Path
import subprocess
import tarfile
import urllib.request

COMMIT = "b6b55fef1e8636b540a6d016a4729ca8cdf2e60b"  # jdecked/twemoji v17.0.3
ARCHIVE_SHA256 = "705d79de1460e5e775f362f0d0f01fbe3ef8d65bf4648c490e4649704584f747"
COUNT = 4009


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, help="use a previously downloaded pinned tar.gz")
    args = parser.parse_args()
    if args.archive:
        data = args.archive.read_bytes()
    else:
        url = f"https://codeload.github.com/jdecked/twemoji/tar.gz/{COMMIT}"
        with urllib.request.urlopen(url, timeout=60) as response:
            data = response.read(16 * 1024 * 1024)
    if hashlib.sha256(data).hexdigest() != ARCHIVE_SHA256:
        raise ValueError("Twemoji archive SHA-256 mismatch")

    destination = Path(__file__).resolve().parents[1] / "assets" / "twemoji"
    destination.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        prefix = f"twemoji-{COMMIT}/assets/72x72/"
        entries = sorted(
            ("".join(chr(int(cp, 16)) for cp in Path(m.name).stem.split("-")), m)
            for m in archive.getmembers()
            if m.isfile() and m.name.startswith(prefix) and m.name.endswith(".png")
        )
        assert len(entries) == COUNT
        assert len({text for text, _ in entries}) == COUNT
        index = []
        vectors = bytearray()
        offsets = [0]
        for cell, (text, member) in enumerate(entries):
            svg_name = member.name.replace("/assets/72x72/", "/assets/svg/").removesuffix(".png") + ".svg"
            svg = archive.extractfile(svg_name).read(65537)
            assert len(svg) <= 65536
            assert not any(marker in svg for marker in [b"<image", b"<text", b"<use", b"href=", b"<filter"])
            encoded = subprocess.run(["zstd", "-q", "-19", "--zstd=wlog=16", f"--stream-size={len(svg)}", "-c"], input=svg, stdout=subprocess.PIPE, check=True).stdout
            vectors.extend(encoded)
            offsets.append(len(vectors))
            index.append((text.replace("\ufe0f", ""), cell))
        assert len({text for text, _ in index}) == COUNT
        assert len(vectors) <= 8 * 1024 * 1024
        (destination / "vectors.bin").write_bytes(struct.pack(f"<{COUNT + 1}I", *offsets) + vectors)
        (destination / "index.tsv").write_text(
            "".join(f"{text}\t{cell}\n" for text, cell in sorted(index)), encoding="utf-8"
        )
        license_file = archive.extractfile(f"twemoji-{COMMIT}/LICENSE-GRAPHICS")
        (destination / "LICENSE-GRAPHICS").write_bytes(license_file.read())
    print(f"{COUNT} emoji; vectors.bin {len(vectors)} compressed SVG bytes")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Rebuild the bundled Twemoji atlas; requires Pillow==11.3.0 (development only)."""

import argparse
import hashlib
import io
import sys
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request

from PIL import Image

COMMIT = "b6b55fef1e8636b540a6d016a4729ca8cdf2e60b"  # jdecked/twemoji v17.0.3
ARCHIVE_SHA256 = "705d79de1460e5e775f362f0d0f01fbe3ef8d65bf4648c490e4649704584f747"
COLUMNS, CELL, COUNT = 64, 32, 4009


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
        atlas = Image.new("RGBA", (COLUMNS * CELL, ((COUNT + COLUMNS - 1) // COLUMNS) * CELL))
        index = []
        for cell, (text, member) in enumerate(entries):
            with Image.open(archive.extractfile(member)) as original:
                assert original.size == (72, 72)
                glyph = original.convert("RGBA").resize((CELL - 2, CELL - 2), Image.Resampling.LANCZOS)
                atlas.paste(glyph, ((cell % COLUMNS) * CELL + 1, (cell // COLUMNS) * CELL + 1))
            index.append((text.replace("\ufe0f", ""), cell))
        assert len({text for text, _ in index}) == COUNT
        atlas.save(destination / "atlas.png", optimize=True)
        # Lossless; ~13% smaller than Pillow's best zlib output. Install with `cargo install oxipng`.
        if shutil.which("oxipng"):
            subprocess.run(["oxipng", "-o", "max", "--strip", "all", "-q", destination / "atlas.png"], check=True)
        else:
            print("oxipng not found; atlas.png left at Pillow compression", file=sys.stderr)
        (destination / "index.tsv").write_text(
            "".join(f"{text}\t{cell}\n" for text, cell in sorted(index)), encoding="utf-8"
        )
        license_file = archive.extractfile(f"twemoji-{COMMIT}/LICENSE-GRAPHICS")
        (destination / "LICENSE-GRAPHICS").write_bytes(license_file.read())
    print(f"{COUNT} emoji; {atlas.width}x{atlas.height}; {atlas.width * atlas.height * 4} decoded RGBA bytes")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Copy the pinned Phosphor SVGs used by the GPUI experiment (development only).

Reuses the version, URLs and SHA-256 pins from tools/generate-icons.py; every file is verified
before it is written. GPUI tints monochrome SVGs at draw time, so no atlas is needed here.

    python3 apps/gpui/assets/fetch-icons.py
"""

import hashlib
import importlib.util
from pathlib import Path
import sys

# tools/ is a Cargo workspace glob; a __pycache__ directory there breaks manifest loading.
sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("icons", ROOT / "tools" / "generate-icons.py")
icons = importlib.util.module_from_spec(spec)
spec.loader.exec_module(icons)

NAMES = [
    "arrow-bend-up-left", "arrow-down", "caret-down", "caret-right", "chat-centered-text", "chats",
    "check", "copy", "download-simple", "file", "file-image", "file-text", "gear", "hash",
    "megaphone-simple", "paper-plane-right", "pencil-simple", "push-pin", "speaker-high", "trash",
    "users", "x",
]


def main():
    destination = Path(__file__).resolve().parent / "icons"
    destination.mkdir(exist_ok=True)
    pins = {name: (asset, sha256) for name, asset, sha256 in icons.ICONS}
    for name in NAMES:
        asset, sha256 = pins[name]
        svg = icons.fetch(f"assets/{asset}")
        digest = hashlib.sha256(svg).hexdigest()
        if digest != sha256:
            raise ValueError(f"{asset} SHA-256 mismatch: {digest}")
        (destination / f"{name}.svg").write_bytes(svg)


if __name__ == "__main__":
    main()

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
    "check", "copy", "download-simple", "file", "file-image", "file-text", "folder-open", "gear",
    "hash", "link",
    "magnifying-glass", "megaphone-simple", "paper-plane-right", "pencil-simple", "plus-circle", "push-pin", "smiley", "speaker-high",
    "trash", "users", "x",
    "dots-three", "eye-slash", "lock-simple", "thread",
    "calendar-blank", "headphones", "microphone", "user-plus",
    "arrow-clockwise", "arrow-up", "gif",
    "game-controller",
    "arrow-right", "arrow-left", "phone", "image", "sparkle", "compass", "shield-warning", "monitor-arrow-up", "crown", "shopping-cart-simple", "chart-bar", "question",
    "star-fill", "fire",
    "star",
]


def main():
    destination = Path(__file__).resolve().parent / "icons"
    destination.mkdir(exist_ok=True)
    pins = {name: (asset, sha256) for name, asset, sha256 in icons.ICONS}
    for name in NAMES:
        asset, sha256 = pins[name]
        # Serein's own glyphs live in the repository, pinned the same way.
        if asset.startswith("repo:"):
            svg = (ROOT / asset.removeprefix("repo:")).read_bytes()
        else:
            svg = icons.fetch(f"assets/{asset}")
        digest = hashlib.sha256(svg).hexdigest()
        if digest != sha256:
            raise ValueError(f"{asset} SHA-256 mismatch: {digest}")
        (destination / f"{name}.svg").write_bytes(svg)


if __name__ == "__main__":
    main()

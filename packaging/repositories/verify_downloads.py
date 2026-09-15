"""Verify complete checksum coverage of downloaded native release packages."""

import hashlib
from pathlib import Path
import re
import sys


def verify(directory):
    directory = Path(directory)
    manifest = directory / "SHA256SUMS.txt"
    if manifest.is_symlink() or not manifest.is_file() or manifest.stat().st_size > 1024 * 1024:
        raise ValueError("missing, unsafe or oversized checksum manifest")
    checksums = {}
    for line in manifest.read_text().splitlines():
        match = re.fullmatch(r"([0-9a-fA-F]{64}) [ *](?:\./)?([A-Za-z0-9][A-Za-z0-9_.+~-]*)", line)
        if not match:
            raise ValueError("invalid checksum entry or unsafe filename")
        digest, name = match.groups()
        if name in checksums:
            raise ValueError(f"duplicate checksum entry: {name}")
        checksums[name] = digest.lower()
    packages = sorted(p for p in directory.iterdir() if p.name.endswith((".deb", ".rpm", ".pkg.tar.zst", ".flatpak")))
    if not packages or len(packages) > 100:
        raise ValueError("expected 1–100 native packages")
    for package in packages:
        if package.is_symlink() or not package.is_file() or package.stat().st_size > 2 * 1024**3:
            raise ValueError(f"unsafe or oversized package: {package.name}")
        if package.name not in checksums:
            raise ValueError(f"package missing from checksum manifest: {package.name}")
        digest = hashlib.sha256()
        with package.open("rb") as stream:
            while block := stream.read(1024 * 1024):
                digest.update(block)
        if digest.hexdigest() != checksums[package.name]:
            raise ValueError(f"package checksum mismatch: {package.name}")
    print(f"Verified {len(packages)} native release packages")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: verify_downloads.py INPUT_DIRECTORY")
    verify(sys.argv[1])

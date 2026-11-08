"""Package a reviewed local build; this script never downloads or executes creator code."""
import argparse
import hashlib
import json
from pathlib import Path

WASM_HEADER = b"\0asm\x01\0\0\0"
LICENSE_SECTION = "serein.license"


def uleb128(value):
    encoded = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            encoded.append(byte | 0x80)
        else:
            encoded.append(byte)
            return bytes(encoded)


def read_uleb128(data, offset):
    value = 0
    shift = 0
    while True:
        if offset >= len(data) or shift > 63:
            raise ValueError("malformed LEB128")
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value, offset
        shift += 7


def custom_sections(wasm):
    if not wasm.startswith(WASM_HEADER):
        raise ValueError("Wasm v1 module required")
    sections = []
    offset = len(WASM_HEADER)
    while offset < len(wasm):
        section_id = wasm[offset]
        offset += 1
        size, offset = read_uleb128(wasm, offset)
        end = offset + size
        if end > len(wasm):
            raise ValueError("section exceeds module")
        if section_id == 0:
            name_length, name_start = read_uleb128(wasm, offset)
            name_end = name_start + name_length
            if name_end > end:
                raise ValueError("custom section name exceeds section")
            name = wasm[name_start:name_end].decode("utf-8")
            sections.append((name, wasm[name_end:end]))
        offset = end
    return sections


def embed_license(wasm, license_bytes):
    """Append an inert custom section; the host ignores custom sections at runtime."""
    if not license_bytes:
        raise ValueError("license text must not be empty")
    existing = [payload for name, payload in custom_sections(wasm) if name == LICENSE_SECTION]
    if existing:
        if existing == [license_bytes]:
            return wasm
        raise ValueError(f"Wasm already contains a different {LICENSE_SECTION} section")
    name = LICENSE_SECTION.encode("utf-8")
    content = uleb128(len(name)) + name + license_bytes
    section = b"\0" + uleb128(len(content)) + content
    packaged = wasm + section
    parsed = [(name, payload) for name, payload in custom_sections(packaged) if name == LICENSE_SECTION]
    if parsed != [(LICENSE_SECTION, license_bytes)]:
        raise ValueError("license custom section did not round-trip")
    return packaged


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=Path)
    parser.add_argument("wasm", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument(
        "--license",
        type=Path,
        help="append this exact file as an inert serein.license Wasm custom section",
    )
    args = parser.parse_args()
    if args.manifest.stat().st_size > 16 * 1024 or args.wasm.stat().st_size > 4 * 1024 * 1024:
        parser.error("manifest or Wasm exceeds the package limits")
    wasm = args.wasm.read_bytes()
    if args.license:
        try:
            wasm = embed_license(wasm, args.license.read_bytes())
        except (OSError, UnicodeError, ValueError) as error:
            parser.error(f"cannot embed license: {error}")
    if len(wasm) > 4 * 1024 * 1024:
        parser.error("Wasm with license exceeds the package limit")
    package = {"manifest": json.loads(args.manifest.read_text(encoding="utf-8")), "wasm": list(wasm)}
    data = json.dumps(package, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    if len(data) > 16 * 1024 * 1024:
        parser.error("package exceeds 16 MiB")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(data)
    print(f"{args.output}: {len(data)} bytes; sha256={hashlib.sha256(data).hexdigest()}")


if __name__ == "__main__":
    main()

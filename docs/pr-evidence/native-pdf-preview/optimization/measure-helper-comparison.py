"""Linux-only offline PDF helper comparison (protocol v1 baseline / v2 optimized).

Usage: python3 measure-helper-comparison.py BINARY OUTPUT --protocol 1|2
       [--fixture PDF --pages N --target WIDTH HEIGHT]
One warmup then five samples per page. No desktop/live session is launched.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import statistics
import struct
import sys
import time


def measure(binary, document, page, pages, version, target):
    magic = f"SPDF000{version}".encode()
    request = magic + struct.pack("<II", page, len(document))
    if version == 2:
        request += struct.pack("<II", *target)
    request += document
    read_input, write_input = os.pipe()
    read_output, write_output = os.pipe()
    null = os.open(os.devnull, os.O_RDWR)
    descriptors = [read_input, write_input, read_output, write_output, null]
    actions = [
        (os.POSIX_SPAWN_DUP2, read_input, 0),
        (os.POSIX_SPAWN_DUP2, write_output, 1),
        (os.POSIX_SPAWN_DUP2, null, 2),
        *((os.POSIX_SPAWN_CLOSE, fd) for fd in descriptors),
    ]
    start = time.perf_counter()
    try:
        pid = os.posix_spawn(binary, [binary, "--serein-pdf-helper"], {}, file_actions=actions)
    except BaseException:
        for descriptor in descriptors:
            os.close(descriptor)
        raise
    reaped = False
    os.close(read_input)
    os.close(write_output)
    os.close(null)
    os.set_blocking(write_input, False)
    os.set_blocking(read_output, False)
    try:
        remaining = memoryview(request)
        response = bytearray()
        while True:
            left = 15 - (time.perf_counter() - start)
            if left <= 0:
                raise TimeoutError("PDF helper exceeded 15 seconds")
            ready, writable, _ = select.select(
                [read_output], [write_input] if write_input is not None else [], [], left
            )
            if writable:
                remaining = remaining[os.write(write_input, remaining):]
                if not remaining:
                    os.close(write_input)
                    write_input = None
            if ready:
                chunk = os.read(read_output, 65536)
                if not chunk:
                    break
                response.extend(chunk)
                assert len(response) <= 28 + 1536 * 1024 * 4
        waited, status, usage = os.wait4(pid, 0)
        reaped = True
        elapsed = time.perf_counter() - start
        assert waited == pid and os.waitstatus_to_exitcode(status) == 0
        assert response[:8] == magic
        code, count, rendered_page, width, height = struct.unpack("<IIIII", response[8:28])
        assert (code, count, rendered_page) == (0, pages, page)
        assert 0 < width <= 1536 and 0 < height <= 1536
        if version == 2:
            assert width <= target[0] and height <= target[1]
        assert width * height <= 1536 * 1024
        assert len(response) == 28 + width * height * 4
        return {
            "page": page, "width": width, "height": height,
            "wall_seconds": elapsed,
            "user_cpu_seconds": usage.ru_utime,
            "system_cpu_seconds": usage.ru_stime,
            "total_cpu_seconds": usage.ru_utime + usage.ru_stime,
            "peak_rss_KiB": usage.ru_maxrss,
        }
    finally:
        if write_input is not None:
            os.close(write_input)
        os.close(read_output)
        if not reaped:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            os.wait4(pid, 0)


if __name__ == "__main__":
    if not sys.platform.startswith("linux"):
        raise SystemExit("Linux wait4 ru_maxrss units are required")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=lambda value: Path(value).resolve(strict=True))
    parser.add_argument("output", type=Path)
    parser.add_argument("--protocol", type=int, choices=[1, 2], required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--pages", type=int, default=2)
    parser.add_argument("--target", type=int, nargs=2, default=[1536, 940])
    args = parser.parse_args()
    document = args.fixture.read_bytes()
    assert document.startswith(b"%PDF-") and 0 < len(document) <= 16 * 1024 * 1024
    assert 0 < args.pages <= 256 and all(0 < edge <= 1536 for edge in args.target)
    def run(page):
        return measure(str(args.binary), document, page, args.pages, args.protocol, args.target)
    warmup = [run(page) for page in range(args.pages)]
    samples = [run(page) for page in range(args.pages) for _ in range(5)]
    metrics = ["wall_seconds", "user_cpu_seconds", "system_cpu_seconds",
               "total_cpu_seconds", "peak_rss_KiB", "width", "height"]
    result = {
        "method": "posix_spawn, cleared env, bounded nonblocking pipe I/O, wait4",
        "protocol": args.protocol,
        "target": args.target if args.protocol == 2 else "v1 always uses maximum",
        "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        "fixture_sha256": hashlib.sha256(document).hexdigest(),
        "fixture_bytes": len(document),
        "warmup": warmup, "samples": samples,
        "median_by_page": {
            str(page): {metric: statistics.median(s[metric] for s in samples if s["page"] == page)
                        for metric in metrics}
            for page in range(args.pages)
        },
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["median_by_page"], indent=2))

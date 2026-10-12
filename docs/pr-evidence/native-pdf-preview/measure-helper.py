"""Linux-only offline measurement of the changed release PDF helper.

Usage: python3 measure-helper.py /absolute/path/to/changed/serein output.json
The baseline has no helper and must NOT be passed to this script.
"""

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


def measure(binary, document, page):
    request = b"SPDF0001" + struct.pack("<II", page, len(document)) + document
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
        pid = os.posix_spawn(
            binary, [binary, "--serein-pdf-helper"], {}, file_actions=actions
        )
    except BaseException:
        for descriptor in descriptors:
            os.close(descriptor)
        raise
    reaped = False
    os.close(read_input)
    os.close(write_output)
    os.close(null)
    try:
        # The controlled request fits in PIPE_BUF; this is not a large-input
        # pipe benchmark. The helper independently enforces its own deadline.
        assert len(request) <= os.fpathconf(write_input, "PC_PIPE_BUF")
        remaining = memoryview(request)
        while remaining:
            remaining = remaining[os.write(write_input, remaining):]
        os.close(write_input)
        write_input = None
        response = bytearray()
        while True:
            left = 15 - (time.perf_counter() - start)
            if left <= 0 or not select.select([read_output], [], [], left)[0]:
                raise TimeoutError("PDF helper response exceeded 15 seconds")
            chunk = os.read(read_output, 65536)
            if not chunk:
                break
            response.extend(chunk)
            assert len(response) <= 28 + 1536 * 1024 * 4
        waited, status, usage = os.wait4(pid, 0)
        reaped = True
        elapsed = time.perf_counter() - start
        assert waited == pid and os.waitstatus_to_exitcode(status) == 0
        assert response[:8] == b"SPDF0001"
        code, pages, rendered_page, width, height = struct.unpack(
            "<IIIII", response[8:28]
        )
        assert (code, pages, rendered_page) == (0, 2, page)
        assert 0 < width <= 1536 and 0 < height <= 1536
        assert width * height <= 1536 * 1024
        assert len(response) == 28 + width * height * 4
        return {
            "page": page,
            "width": width,
            "height": height,
            "wall_seconds": elapsed,
            "user_cpu_seconds": usage.ru_utime,
            "system_cpu_seconds": usage.ru_stime,
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
    binary = str(Path(sys.argv[1]).resolve(strict=True))
    fixture = (
        Path(__file__).resolve().parents[3]
        / "apps/desktop/tests/fixtures/pdf-preview.pdf"
    )
    document = fixture.read_bytes()
    assert len(document) == 2116
    warmup = [measure(binary, document, page) for page in range(2)]
    samples = [
        measure(binary, document, page) for page in range(2) for _ in range(5)
    ]
    metrics = [
        "wall_seconds", "user_cpu_seconds", "system_cpu_seconds", "peak_rss_KiB"
    ]
    result = {
        "method": "posix_spawn, cleared environment, bounded pipe response, wait4",
        "binary_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
        "fixture_sha256": hashlib.sha256(document).hexdigest(),
        "warmup": warmup,
        "samples": samples,
        "median_by_page": {
            str(page): {
                metric: statistics.median(
                    sample[metric] for sample in samples if sample["page"] == page
                )
                for metric in metrics
            }
            for page in range(2)
        },
    }
    Path(sys.argv[2]).write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["median_by_page"], indent=2))

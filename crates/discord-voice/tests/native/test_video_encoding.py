#!/usr/bin/env python3
"""Production FFmpeg shim timing/configuration fixtures; no GPU or capture access."""
import argparse
import os
from pathlib import Path
import shlex
import subprocess
import tempfile


def run(prefix):
    here = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix="serein-encode-fixture-") as directory:
        executable = Path(directory) / "video-encoding-test"
        subprocess.run([
            *shlex.split(os.environ.get("CC", "cc")), "-std=c11", "-Wall", "-Wextra", "-Werror",
            "-I" + str(prefix / "include"), "-I" + str(here.parents[1] / "src"),
            str(here / "video_encode_timing_test.c"), "-L" + str(prefix / "lib"),
            "-Wl,-rpath," + str(prefix / "lib"), "-lavcodec-serein", "-lavutil-serein",
            "-o", str(executable),
        ], check=True)
        subprocess.run([str(executable)], check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix", type=Path, required=True)
    run(parser.parse_args().prefix.resolve())

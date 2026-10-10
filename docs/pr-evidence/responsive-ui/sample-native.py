"""Sample a synthetic native preview; requires psutil and an active DISPLAY."""
import json
import os
import pathlib
import subprocess
import sys
import time

import psutil

binary, label, output = sys.argv[1:]
args = [binary, "--demo", "--page=markdown", "--reply", "--interactive",
        "--width=1120", "--height=760"]
log_path = pathlib.Path(output).with_suffix(".log")


def require_running(process):
    status = process.poll()
    if status is not None:
        raise RuntimeError(f"Preview exited with status {status}; see {log_path}")


with open(log_path, "w") as log:
    process = subprocess.Popen(args, env=dict(os.environ, WGPU_BACKEND="gl"),
                               stdout=log, stderr=log)
    try:
        native = psutil.Process(process.pid)
        time.sleep(3)
        require_running(process)
        initial_cpu = native.cpu_times()
        started = time.monotonic()
        samples = []
        for _ in range(15):
            time.sleep(1)
            require_running(process)
            samples.append({"elapsed_s": time.monotonic() - started,
                            "rss_bytes": native.memory_info().rss})
        require_running(process)
        final_cpu = native.cpu_times()
        elapsed = time.monotonic() - started
        cpu_seconds = ((final_cpu.user + final_cpu.system)
                       - (initial_cpu.user + initial_cpu.system))
        result = {
            "revision": label, "command": args, "warmup_s": 3,
            "duration_s": elapsed, "interval_s": 1,
            "process_cpu_one_core_percent": 100 * cpu_seconds / elapsed,
            "peak_rss_bytes": max(sample["rss_bytes"] for sample in samples),
            "settled_rss_bytes": samples[-1]["rss_bytes"],
            "children": [{"pid": child.pid, "rss_bytes": child.memory_info().rss}
                         for child in native.children(recursive=True)],
            "samples": samples,
            "executable_bytes": pathlib.Path(binary).stat().st_size,
        }
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
pathlib.Path(output).write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps({key: value for key, value in result.items()
                  if key not in ["command", "samples"]}))

"""Offline release demo RSS/CPU sampler; never opens devices or authenticated sessions."""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

import psutil

parser = argparse.ArgumentParser()
parser.add_argument("executable", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--xdotool", default="xdotool")
args = parser.parse_args()
env = dict(os.environ, WINIT_UNIX_BACKEND="x11", WAYLAND_DISPLAY="",
           VK_DRIVER_FILES="/usr/share/vulkan/icd.d/lvp_icd.json")
with args.output.with_suffix(".log").open("w") as log:
    app = subprocess.Popen([str(args.executable.resolve()), "--demo", "--demo-chat", "--demo-recorder"], env=env, stdout=log, stderr=log)
    try:
        time.sleep(3)
        window = subprocess.check_output([args.xdotool, "search", "--pid", str(app.pid)], env=env, text=True).splitlines()[0]
        subprocess.run([args.xdotool, "windowfocus", "--sync", window, "mousemove", "--sync", "--window", window, "500", "24", "sleep", "0.1", "click", "1"], env=env, check=True)
        time.sleep(8)
        process = psutil.Process(app.pid)
        process.cpu_percent(None)
        samples = []
        for _ in range(20):
            time.sleep(1)
            samples.append({"cpu_percent_one_core": process.cpu_percent(None), "rss_bytes": process.memory_info().rss, "children": [{"name": p.name(), "rss_bytes": p.memory_info().rss} for p in process.children(recursive=True)]})
        result = {"argv": ["--demo", "--demo-chat", "--demo-recorder"], "warmup_seconds": 11, "sample_interval_seconds": 1, "samples": samples,
                  "mean_cpu_percent_one_core": statistics.mean(s["cpu_percent_one_core"] for s in samples),
                  "peak_rss_bytes": max(s["rss_bytes"] for s in samples),
                  "settled_rss_bytes": statistics.median(s["rss_bytes"] for s in samples[-5:])}
        args.output.write_text(json.dumps(result, indent=2) + "\n")
    finally:
        app.terminate()
        try:
            app.wait(timeout=5)
        except subprocess.TimeoutExpired:
            app.kill()
            app.wait()

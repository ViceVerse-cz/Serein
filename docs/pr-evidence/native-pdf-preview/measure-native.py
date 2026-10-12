"""Linux X11 synthetic native capture and idle measurement.

Usage: python3 measure-native.py BINARY OUTPUT_DIRECTORY LABEL [--keep] [--light]
Both revisions require the additive --demo-pdf fixture. Never use a live session.
Requires Xvfb, xdotool and ImageMagick import; defaults to X display :78.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import time


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=lambda value: Path(value).resolve(strict=True))
parser.add_argument("output", type=Path)
parser.add_argument("label")
parser.add_argument("--keep", action="store_true")
parser.add_argument("--light", action="store_true")
parser.add_argument("--display", default=":78")
args = parser.parse_args()
assert args.label.replace("-", "").isalnum(), "use a simple file label"
args.output.mkdir(parents=True, exist_ok=True)
env = dict(
    os.environ,
    DISPLAY=args.display,
    GDK_BACKEND="x11",
    WINIT_UNIX_BACKEND="x11",
    XDG_CONFIG_HOME=str(args.output / "demo-config"),
    XDG_CACHE_HOME=str(args.output / "demo-cache"),
)
env.pop("WAYLAND_DISPLAY", None)


def stop(process):
    process.terminate()
    try:
        process.wait(5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def command(*values):
    return subprocess.check_output(values, env=env, text=True).strip()


with (args.output / f"{args.label}-native.log").open("w") as log:
    xserver = subprocess.Popen(
        [
            "Xvfb", args.display, "-screen", "0", "1440x1000x24",
            "-dpi", "96", "-nolisten", "tcp",
        ],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    app = None
    try:
        time.sleep(1)
        assert xserver.poll() is None, "Xvfb failed; choose a free display"
        # Keep the pointer outside the app to avoid hover/tooltips.
        command("xdotool", "mousemove", "1430", "990")
        app = subprocess.Popen(
            [str(args.binary), "--demo", "--demo-pdf"]
            + (["--demo-light"] if args.light else []),
            env=env, stdout=log, stderr=log,
        )
        (args.output / f"{args.label}-pid").write_text(str(app.pid))
        time.sleep(12)
        assert app.poll() is None, "demo exited before measurement"
        # Xvfb resets the pointer when its last client disconnects, including
        # the pre-launch xdotool above. Clear any initial centred hover after
        # the app is connected, then allow a repaint before capture/sampling.
        command("xdotool", "mousemove", "400", "140")
        command("xdotool", "mousemove", "1430", "990")
        time.sleep(1)
        windows = command(
            "xdotool", "search", "--onlyvisible", "--pid", str(app.pid)
        ).splitlines()
        window = max(
            windows,
            key=lambda value: int(command(
                "xdotool", "getwindowgeometry", "--shell", value
            ).split("WIDTH=")[1].splitlines()[0]),
        )
        (args.output / f"{args.label}-window").write_text(window)
        subprocess.run(
            ["import", "-window", window, str(args.output / f"{args.label}.png")],
            env=env, check=True,
        )
        samples = []
        for _ in range(50):
            stat = Path(f"/proc/{app.pid}/stat").read_text().split(") ")[1].split()
            status = Path(f"/proc/{app.pid}/status").read_text().splitlines()
            memory = {
                key: int(line.split()[1])
                for line in status for key in ["VmRSS:", "VmHWM:"]
                if line.startswith(key)
            }
            samples.append({
                "t": time.monotonic(), "ticks": int(stat[11]) + int(stat[12]),
                **memory,
            })
            time.sleep(0.1)
        clock = os.sysconf("SC_CLK_TCK")
        result = {
            "label": args.label, "pid": app.pid, "window": window,
            "clock_ticks": clock, "samples": samples,
            "settled_RSS_KiB": samples[-1]["VmRSS:"],
            "peak_sampled_RSS_KiB": max(s["VmRSS:"] for s in samples),
            "idle_cpu_percent_one_core": 100
            * (samples[-1]["ticks"] - samples[0]["ticks"]) / clock
            / (samples[-1]["t"] - samples[0]["t"]),
        }
        (args.output / f"{args.label}-samples.json").write_text(
            json.dumps(result, indent=2) + "\n"
        )
        print(json.dumps({k: v for k, v in result.items() if k != "samples"}), flush=True)
        if args.keep:
            while app.poll() is None:
                time.sleep(1)
    finally:
        if app is not None:
            stop(app)
        stop(xserver)

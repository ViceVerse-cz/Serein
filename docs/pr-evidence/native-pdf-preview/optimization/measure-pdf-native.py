"""Synthetic Linux/X11 PDF-card and page comparison.

Usage: python3 measure-pdf-native.py BINARY OUTPUT LABEL [--display :81]
One native --demo --demo-pdf process per run; never a saved/live session.
Requires Xvfb, xdotool and ImageMagick. Window-only, unscaled captures.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def stop(process):
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def sample(pid):
    rows = []
    for _ in range(50):
        stat = Path(f"/proc/{pid}/stat").read_text().split(") ")[1].split()
        status = Path(f"/proc/{pid}/status").read_text().splitlines()
        memory = {
            key: int(line.split()[1])
            for line in status for key in ["VmRSS:", "VmHWM:"]
            if line.startswith(key)
        }
        rows.append({
            "t": time.monotonic(), "ticks": int(stat[11]) + int(stat[12]),
            **memory,
        })
        time.sleep(0.1)
    clock = os.sysconf("SC_CLK_TCK")
    return {
        "clock_ticks": clock, "samples": rows,
        "settled_RSS_KiB": rows[-1]["VmRSS:"],
        "peak_sampled_RSS_KiB": max(row["VmRSS:"] for row in rows),
        "idle_cpu_percent_one_core": 100
        * (rows[-1]["ticks"] - rows[0]["ticks"]) / clock
        / (rows[-1]["t"] - rows[0]["t"]),
    }


def helpers(pid):
    """Inspect child identity only, never arbitrary process payloads."""
    found = []
    for task in Path(f"/proc/{pid}/task").iterdir():
        try:
            children = (task / "children").read_text().split()
        except FileNotFoundError:
            continue
        for child in children:
            try:
                argv = Path(f"/proc/{child}/cmdline").read_bytes().split(b"\0")
            except FileNotFoundError:
                continue
            if b"--serein-pdf-helper" in argv:
                found.append(int(child))
    return sorted(set(found))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=lambda value: Path(value).resolve(strict=True))
    parser.add_argument("output", type=Path)
    parser.add_argument("label")
    parser.add_argument("--display", default=":81")
    parser.add_argument("--light", action="store_true")
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    assert args.label.replace("-", "").isalnum()
    args.output.mkdir(parents=True, exist_ok=True)
    env = dict(
        os.environ, DISPLAY=args.display,
        GDK_BACKEND="x11", WINIT_UNIX_BACKEND="x11",
        XDG_CONFIG_HOME=str(args.output / f"{args.label}-config"),
        XDG_CACHE_HOME=str(args.output / f"{args.label}-cache"),
    )
    env.pop("WAYLAND_DISPLAY", None)

    def command(*values):
        return subprocess.check_output(
            values, env=env, text=True, timeout=15
        ).strip()

    with (args.output / f"{args.label}-native.log").open("w") as log:
        xserver = subprocess.Popen(
            ["Xvfb", args.display, "-screen", "0", "1440x1000x24",
             "-dpi", "96", "-nolisten", "tcp"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        app = None
        try:
            time.sleep(1)
            assert xserver.poll() is None, "Choose an unused X display"
            app = subprocess.Popen(
                [str(args.binary), "--demo", "--demo-pdf"]
                + (["--demo-light"] if args.light else []),
                env=env, stdout=log, stderr=log,
            )
            (args.output / f"{args.label}-pid").write_text(str(app.pid))
            time.sleep(12)
            assert app.poll() is None, "Synthetic demo exited"
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
            geometry = command("xdotool", "getwindowgeometry", "--shell", window)
            assert "WIDTH=1120\nHEIGHT=760" in geometry, geometry
            command("xdotool", "windowfocus", window)
            command("xdotool", "mousemove", "400", "140")
            command("xdotool", "mousemove", "1430", "990")
            time.sleep(1)
            for state in ["card", "page"]:
                if state == "page":
                    # Coordinates of Preview in the shared 1120x760 fixture.
                    command("xdotool", "mousemove", "--window", window, "671", "496")
                    command("xdotool", "click", "1")
                    command("xdotool", "mousemove", "1430", "990")
                    time.sleep(3)
                assert app.poll() is None
                active = helpers(app.pid)
                assert not active, f"Helper did not finish: {active}"
                command(
                    "import", "-window", window,
                    str(args.output / f"{args.label}-{state}.png"),
                )
                result = {
                    "label": args.label, "state": state,
                    "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                    "helpers_after_settle": active, **sample(app.pid),
                }
                (args.output / f"{args.label}-{state}-samples.json").write_text(
                    json.dumps(result, indent=2) + "\n"
                )
                print(json.dumps({key: value for key, value in result.items()
                                  if key != "samples"}), flush=True)
            if args.keep:
                while app.poll() is None:
                    time.sleep(1)
        finally:
            if app is not None:
                stop(app)
            stop(xserver)

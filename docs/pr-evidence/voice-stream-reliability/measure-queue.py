#!/usr/bin/env python3
"""Offline native queue measurement. Requires a Rust/GStreamer >=1.20 Docker container
with this output directory mounted at /audit (2 CPUs/2 GiB is sufficient).
Run from the repository: python3 SCRIPT --output target/native-gst-audit.
No host Cargo build, account, capture, or repository dependency change is made.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import statistics
import subprocess

BASELINE = "074ba3a158b72ddb9b7bc327fba64ca4b93975e2"
MODULE = "crates/platform/src/video/live_gst.rs"
MANIFEST = """[package]
name = "native-gst-audit"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
gstreamer = "=0.25.3"
gstreamer-app = "=0.25.2"
gstreamer-video = "=0.25.3"
"""
SHIMS = """#![allow(dead_code)]
pub const MAX_ACCESS_UNIT: usize = 2 * 1024 * 1024 + 64 * 1024;
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
pub const INVALID: &str = "The video could not be decoded safely.";
pub const BUSY: &str = "The video decoder is falling behind.";
pub const UNSUPPORTED: &str = "This video format or codec is not supported on this system.";
pub struct LiveFrame { pub width: u32, pub height: u32, pub rgba: Vec<u8> }
pub type LiveSink = Box<dyn Fn(LiveFrame) + Send + Sync>;
pub fn check_dimensions(width: u32, height: u32) -> Result<(), &'static str> {
    if width == 0 || height == 0 || width > 1920 || height > 1920 || u64::from(width) * u64::from(height) > 1920 * 1080 { return Err(INVALID); }
    Ok(())
}
#[path = "baseline.rs"] mod baseline;
#[path = "live_gst.rs"] mod after;
"""
MEASUREMENT = """
#[cfg(test)]
mod retention_measurement {
    use super::*;
    #[test]
    fn queue_retention() {
        let mut decoder = H264Decoder::new(Box::new(|_| {})).expect("Native H.264 pipeline");
        decoder.pipeline.set_state(gst::State::Paused).unwrap();
        let mut payload = vec![0xff; MAX_ACCESS_UNIT];
        payload[..5].copy_from_slice(&[0, 0, 0, 1, 0x0c]);
        payload[MAX_ACCESS_UNIT - 1] = 0x80;
        let mut accepted = 0;
        for _ in 0..32 { accepted += usize::from(decoder.decode(&payload).is_ok()); }
        let items = decoder.appsrc.property::<u64>("current-level-buffers");
        let bytes = decoder.appsrc.current_level_bytes();
        assert_eq!(accepted, EXPECTED);
        assert_eq!(items, EXPECTED);
        assert_eq!(bytes, EXPECTED * MAX_ACCESS_UNIT as u64);
        println!("LABEL accepted={accepted} items={items} bytes={bytes}");
    }
}
"""

def capture(command):
    return subprocess.check_output(command, text=True, stderr=subprocess.STDOUT)

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--container", default="serein-native-gst-audit")
parser.add_argument("--baseline", default=BASELINE)
args = parser.parse_args()
repo = Path(capture(["git", "rev-parse", "--show-toplevel"]).strip())
output = args.output.resolve()
(output / "src").mkdir(parents=True, exist_ok=True)
(output / "Cargo.toml").write_text(MANIFEST)
(output / "src/lib.rs").write_text(SHIMS)
sources = {"baseline": capture(["git", "show", f"{args.baseline}:{MODULE}"]), "after": (repo / MODULE).read_text()}
for label, source in sources.items():
    path = output / "src" / ("baseline.rs" if label == "baseline" else "live_gst.rs")
    extra = MEASUREMENT.replace("EXPECTED", "32" if label == "baseline" else "4").replace("LABEL", label.upper())
    path.write_text(source + extra)
    assert path.read_text()[:-len(extra)] == source
prefix = ["docker", "exec", "-w", "/audit", args.container]
metadata = {"baseline": args.baseline, "source_sha256": {label: hashlib.sha256(source.encode()).hexdigest() for label, source in sources.items()}}
metadata["toolchain"] = capture(prefix + ["rustc", "--version"]).strip()
metadata["gstreamer"] = capture(prefix + ["pkg-config", "--modversion", "gstreamer-1.0"]).strip()
metadata["platform"] = capture(prefix + ["uname", "-sm"]).strip()
metadata["container_image"] = capture(["docker", "inspect", "--format", "{{.Config.Image}}", args.container]).strip()
if not (output / "Cargo.lock").exists():
    (output / "lock-generation.log").write_text(capture(prefix + ["cargo", "generate-lockfile"]))
metadata["lock_sha256"] = hashlib.sha256((output / "Cargo.lock").read_bytes()).hexdigest()
(output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
(output / "release-build.log").write_text(capture(prefix + ["cargo", "test", "--locked", "--release", "-j2", "--no-run"]))
command = prefix + ["cargo", "test", "--locked", "--release", "-j2"]
(output / "release-warmup.log").write_text(capture(command + ["--", "--nocapture", "--test-threads=1"]))
samples = {"BASELINE": [], "AFTER": []}
for run in range(1, 6):
    raw = capture(command + ["queue_retention", "--", "--nocapture", "--test-threads=1"])
    (output / f"release-run-{run}.log").write_text(raw)
    for label, accepted, items, retained in re.findall(r"(BASELINE|AFTER) accepted=(\d+) items=(\d+) bytes=(\d+)", raw):
        samples[label].append({"accepted": int(accepted), "items": int(items), "bytes": int(retained)})
    print(f"Run {run}: " + " | ".join(line for line in raw.splitlines() if " accepted=" in line), flush=True)
assert all(len(rows) == 5 for rows in samples.values()), samples
medians = {label: {key: statistics.median(row[key] for row in rows) for key in ["accepted", "items", "bytes"]} for label, rows in samples.items()}
(output / "release-summary.json").write_text(json.dumps({"metadata": metadata, "samples": samples, "medians": medians}, indent=2) + "\n")
print(json.dumps(medians, sort_keys=True))

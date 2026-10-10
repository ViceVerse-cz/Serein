#!/usr/bin/env python3
"""Compile the production updater with an egui-only eframe boundary, offline.

This does not exercise the desktop renderer, an installer or live update traffic.
Set CARGO_HOME/CARGO_TARGET_DIR to reusable writable locations before running.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    mode = sys.argv[1] if len(sys.argv) == 2 else "test"
    if mode not in {"test", "clippy"} or len(sys.argv) > 2:
        raise SystemExit("Usage: updater-scope.py [test|clippy]")
    repository = Path(__file__).resolve().parents[3]
    with tempfile.TemporaryDirectory(prefix="serein-updater-scope-") as directory:
        root = Path(directory)
        (root / "src").mkdir()
        (root / "eframe/src").mkdir(parents=True)
        (root / "Cargo.toml").write_text("""[package]
name = "serein-updater-bug-hunt"
version = "0.1.0"
edition = "2024"
[workspace]
[features]
demo = []
[dependencies]
eframe = { path = "eframe" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
md4 = "=0.10.2"
url = "2"
sha2 = "=0.10.9"
semver = "=1.0.28"
zip = { version = "=4.6.1", default-features = false, features = ["deflate-flate2"] }
getrandom = "0.4"
reqwest = { version = "=0.13.5", default-features = false, features = ["rustls", "json", "stream", "gzip"] }
tokio = { version = "1.50", features = ["rt-multi-thread", "macros", "sync", "time", "net", "io-util", "fs"] }
ui = { path = """ + json.dumps((repository / "crates/ui").as_posix()) + " }\n")
        # Preserve versions from the production lock; only the two harness packages differ.
        (root / "Cargo.lock").write_bytes((repository / "Cargo.lock").read_bytes())
        (root / "src/lib.rs").write_text(
            "#![forbid(unsafe_code)]\n#![allow(dead_code)]\n#[path="
            + json.dumps((repository / "apps/desktop/src/updater.rs").as_posix())
            + "]\nmod updater;\n")
        (root / "eframe/Cargo.toml").write_text("""[package]
name = "eframe"
version = "0.0.0"
edition = "2024"
[dependencies]
egui = { git = "https://github.com/emilk/egui", rev = "8f6d3d6ed99cb24d2e14c43951803d2868db40b1", default-features = false, features = ["default_fonts"] }
""")
        (root / "eframe/src/lib.rs").write_text("pub use egui;\n")
        environment = dict(os.environ)
        environment.setdefault("CARGO_TARGET_DIR", str(root / "target"))
        environment.setdefault("CARGO_INCREMENTAL", "0")
        environment.setdefault("CARGO_PROFILE_DEV_DEBUG", "0")
        environment.setdefault("CARGO_PROFILE_TEST_DEBUG", "0")
        command = ["cargo", mode, "--offline", "--manifest-path", str(root / "Cargo.toml"), "--lib"]
        if mode == "clippy":
            command.extend(["--tests", "--", "-D", "warnings"])
        subprocess.run(command, env=environment, check=True)


if __name__ == "__main__":
    main()

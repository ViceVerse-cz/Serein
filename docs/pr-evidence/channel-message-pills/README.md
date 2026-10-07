# Channel and message pills â€” issue #576

Baseline: `38d919d74054b885b992ce607f0bfa93ab449696`; final runtime:
`78de8d15`. The original checkout's
untracked `community-extensions/` was preserved in a separate checkout. Both
comparison builds use the new synthetic `channel-links` fixture and the same
one-edge Cargo.lock repair: gpu-allocator 0.28.0 selects windows 0.62.2, matching
wgpu-hal 30.0.1. The untouched lockfile cannot build that Windows renderer because
it selects incompatible windows 0.61.3 types. No other baseline UI code changed.

## Reproduce the render

Use pinned Rust 1.98.1 and separate baseline/changed worktrees. In each worktree:

```powershell
cargo build --release --locked -p serein --features demo --example profile_preview
$preview = Join-Path $env:CARGO_TARGET_DIR 'release/examples/profile_preview.exe'
& $preview --demo --page=channel-links --width=1120 --height=900 --output=target/pills-dark.png
& $preview --demo --page=channel-links --width=720 --height=900 --light --output=target/pills-light-narrow.png
```

Set `CARGO_TARGET_DIR` to an absolute build-cache directory first (or use
`target/release/examples/profile_preview.exe` without that variable). Serialize
builds that share a target directory, and preserve each comparison executable
before building the other revision.

The fixture contains all four channel kinds and four message-link variants,
unknown destinations, a long Czech/Japanese post title, a named link, code and a
hidden spoiler. It uses no service adapters, credentials or live account data.
Images are eframe/WGPU framebuffer exports at 125% display scale, not OS window
captures. The native Computer Use helper failed to connect to its pipe with
`os error 2`; Orca was not installed. Native pointer/keyboard and screen-reader
operation remain unverified. Synthetic egui input/accessibility-tree tests are
separate evidence.

Both dark/wide and light/narrow pairs were inspected. The corrected capture has
uniform icon/text backgrounds, correct channel-type and message suffix icons,
the forum/post breadcrumb and a synthetic foreign-server avatar. Long multilingual
labels wrap inside the message column. Named links and hidden spoilers retain
their existing behavior.

## Process measurements

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), 33,410,678,784 bytes
physical RAM; NVIDIA RTX 5070 Ti and AMD integrated graphics are available. The
preview uses the existing WGPU renderer; its selected GPU/backend was not logged.

Run `sample-process.ps1 -Executable <absolute-preview-path> -Output <json-path>`
once per build. It launches only `--demo`, warms up for five seconds, then samples
21 times at approximately one-second intervals using actual elapsed wall time.
CPU is the process CPU-seconds delta divided by elapsed seconds (one-core
percentage); working set and private bytes come from Windows process counters.
This is a static idle fixture. No scripted scrolling or native input is claimed.
Compiler activity on the shared machine introduces noise. Frame latency,
startup latency and live Discord performance are unmeasured.

| Process metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| CPU, one-core percentage | 0.00% | 0.31% | +0.31 percentage points |
| Peak / settled working set | 196,276,224 B | 198,307,840 B | +2,031,616 B / +1.04% |
| Settled private bytes | 414,261,248 B | 415,698,944 B | +1,437,696 B / +0.35% |

Settled means the mean of the last five samples. Baseline elapsed sample time was
20.536 s; after was 20.252 s. One sample series per build supports no performance
improvement claim; raw counters and summary are committed alongside this file.

Standard packages use `cargo xtask package`, without demo/developer features and
with voice included. Baseline and changed `dist` directories are kept separate.
Package bytes are the sum of files; compressed bytes use PowerShell
`Compress-Archive -Path dist/* -CompressionLevel Optimal`. No package executable
is launched with a saved session.

Baseline packaging passed. Changed production packaging is still in progress.
NSIS is unavailable locally, so packaging produces an unsigned directory rather
than an installer executable. The standard build emitted the existing OpenH264
duplicate-object debug-info linker warning.

## Verification limits

The unchanged baseline UI suite has eight failures: six tests require empty
platform output commands, and two emoji tests expect unescaped underscore
labels. The task suite reproduces those failures. Native OS input/capture,
macOS/Linux runtime behavior and live Discord compatibility are unverified.
These prevent treating this change as ready to merge.

`cargo xtask check` reached 1,154 passed, 8 baseline failures, 23 ignored. The
changed UI suite had 414 passed, 8 baseline failures, 5 ignored. After the final
background-only correction, all seven focused pill tests and strict workspace
Clippy passed again. Formatting, production-only checking and policy checks also
passed. See `checks.json` for the exact baseline failure names.

# Voice and streaming reliability audit — October 3, 2026

Baseline: `074ba3a158b72ddb9b7bc327fba64ca4b93975e2`, fetched `origin/main`.
The task started without tracked changes. Unrelated untracked
`community-extensions/` content was preserved. No dependency or feature change.
No UI change; screenshots are not applicable.

## Native Linux pressure workload

The exact baseline/current Linux decoder modules are extracted into an ignored
standalone harness. Only surrounding type/limit shims and measurement tests are
added; `source-identity.log` verifies the production module prefix. This exercises
the real native appsrc and decoder admission, rather than a mock queue. It does
not build the complete Linux client or exercise desktop capture/GPU integration.

Linux aarch64, Debian 12 container, GStreamer 1.22.0, Rust 1.98.1;
GStreamer Rust crates 0.25.3/app 0.25.2/video 0.25.3. Container limited to two
CPUs and 2 GiB. Both modules use the same harness and native runtime.
One release warmup runs all five tests; five serial measured runs each offer
32 Annex-B filler access units of 2,162,688 bytes to a paused native pipeline.

All runs retain 32 units / 69,206,016 bytes before and four units / 8,650,752 bytes
after. This is an 87.5% reduction in queued compressed payload for this stalled
workload. It is not RSS, GPU memory, decode throughput or live playback latency.
Consumed native buffers, codec surfaces and outer worker queues are separate.

Reproduce from the repository root (Docker plus network for public build tools):

```sh
mkdir -p target/native-gst-audit
docker run -d --name serein-native-gst-audit --cpus=2 --memory=2g \
  -v "$PWD/target/native-gst-audit:/audit" -w /audit \
  rust:1.98.1-slim-bookworm@sha256:ff521445a372125ed4f76e1453a1f8098f2d05332d1601d30db1c1f62757e730 sleep 3600
docker exec serein-native-gst-audit apt-get update
docker exec serein-native-gst-audit apt-get install -y --no-install-recommends \
  pkg-config libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-bad gstreamer1.0-libav gstreamer1.0-tools
python3 docs/pr-evidence/voice-stream-reliability/measure-queue.py --output target/native-gst-audit
docker stop serein-native-gst-audit
```

The script pins direct harness dependencies and records its generated lock hash;
the repository lockfile is unchanged. Package-manager versions may change on a
later rerun. Raw samples and source/runtime identities are in
`release-summary.json` and `release-run-*.log`. The container was stopped.
Strict release harness Clippy passed after the final source copy; that output
was streamed through the task tools rather than saved in a raw artifact.

## Behavior and verification

`checks.txt` contains selected actual output, including failures:

- `cargo test --locked -p discord-voice --lib`: 60 passed, three ignored.
  Uses synthetic MLS keys, local WebSockets/UDP and device-free media. The new
  rekey regression withholds Execute/new Welcome after delivery while continuing
  heartbeat ACKs. Baseline transport hangs beyond the test's 45-second deadline;
  corrected sender/viewer return the existing timeout errors after 30 seconds.
- The jitter regression queues successors out of order across a four-packet gap,
  including sequence wrap. Recovery now preserves them after three concealment
  packets. A far-forward resync also resets the prior loss count.
- A real oversized local UDP datagram exercises error/outage tracking, followed
  by successful delivery. Real send-buffer saturation remains unmeasured.
- `cargo test --workspace --exclude ui --locked` passed, including desktop tests.
  Workspace strict Clippy and formatting passed during `cargo xtask check`.
  Production/no-default-feature check and policy checks passed separately.
- `cargo xtask check` fails in existing UI tests and aborts on
  `composer_tests::download_cancel_remains_visible_without_a_text_composer`.
  That exact abort reproduces in a detached, unchanged baseline worktree.
  No unrelated UI repairs were included; this prevents a green full check.

Standard voice-inclusive macOS package sizes and reproduction are recorded in
`package-measurements.json` and [the performance notes](../../performance.md).
Live Linux portal/capture, physical audio, network behavior and official Discord
viewer error 2012 remain unverified. No account, microphone, camera or desktop
capture was used. Ordinary offline demo idle sampling cannot exercise these
media paths; active CPU/RSS, audio callback timing and A/V latency are unmeasured.

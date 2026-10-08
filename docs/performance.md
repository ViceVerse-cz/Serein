# Performance findings

Recent synthetic/offline measurements are workload-specific. They do not establish live Discord
performance, universal device results or application-wide memory bounds. The raw PR screenshot,
log and per-run evidence archive has been removed; the summaries below retain the useful results.

## Message-link chips — October 8, 2026

Compared on Fedora Linux 43, x86_64, Rust 1.98.1, with the standard voice-enabled release package
(`lto = "fat"`, `codegen-units = 1`). Package bytes are the executable, the RPM installed-size
field, and the compressed RPM. The reducer is `replay-bench`: one warmup, then five runs, median
elapsed time, retained timeline unchanged at 331,992..332,477 estimated bytes and 500 records.
The idle sample is the native `--demo --demo-message-links` window at 1280×1000 on Xvfb through
lavapipe (`WGPU_BACKEND=vulkan`), 21 samples at 1 second. CPU is percent of one logical core from
`/proc` ticks. RSS is `VmRSS`. No child processes. These runs do not measure frame time, startup,
GPU memory, or live Discord.

| Workload | Before | After | Result |
| --- | ---: | ---: | ---: |
| Reducer replay median | 87.063 ms | 89.386 ms | +2.3 ms, inside the before-run spread |
| Standard executable | 87,096,456 B | 87,125,128 B | +28,672 B |
| Installed RPM | 91,628,399 B | 91,657,071 B | +28,672 B |
| Compressed RPM | 46,215,813 B | 46,226,682 B | +10,869 B |
| Demo idle mean CPU | 0.100% | 0.100% | no visible change |
| Demo idle RSS | 233.230 MiB | 236.266 MiB | +3.0 MiB, one noisy sample |

The before reducer spread was 85.584–94.617 ms, so the median difference is not a regression.
The after idle sample followed one same-channel click and a 10 second settle while other builds
were running; the before sample was an untouched window. Neither sample supports an idle
performance claim. Startup latency, full-frame p95, and other platforms were unmeasured.

## Voice default-device polling — October 7, 2026

An offline probe compared creating a fresh PulseAudio client for every metadata poll with reusing
one client. Across five measured runs after warmup, 300 polls created 300 clients and left 300 peer
sockets connected in the fresh-client pattern; reuse created one client and left one socket. The
probe used synthetic socket pairs and a 2 ms pause per query. This identifies dependency resource
retention in the polling pattern; it does not establish application socket behavior or whole-process
memory use.

Both standard voice-enabled macOS packages were the same size: executable 68,180,992 B, installed
app 74,201,013 B, distribution 74,265,778 B. ZIP sizes differed by 37 B (48,039,907 vs 48,039,944 B),
which is packaging variation, not a runtime change. No Discord call, microphone, physical device,
callback latency, frame timing or CPU/RSS comparison was measured.

## CPU and memory audit — October 2, 2026

Release workloads compared a 100,000-event synthetic timeline, cursor scans, and synthetic video
frame ownership on an Apple M1 with Rust 1.98.1:

| Workload | Before | After | Result |
| --- | ---: | ---: | ---: |
| Reducer replay median | 153.985 ms | 53.504 ms | −65.25% |
| Full-history cursor query batch | 263.428 ms | 6.948 ms | −97.36% |
| Cursor batch with deleted tail rows | 341.132 ms | 16.075 ms | −95.29% |
| Coalesced 1080p frame peak RSS | 34.406 MiB | 26.516 MiB | −22.93% |
| Standard executable | 62,006,096 B | 62,006,112 B | +16 B |
| Installed bundle | 68,016,525 B | 68,016,541 B | +16 B |

The reducer retained 500 rows in both builds. Cursor timings isolate hot lookup work and do not
predict whole-UI gains. Frame conversion time was effectively unchanged; the lower peak RSS came
from reusing the undisplayed pending frame. With uploads every third frame, peak RSS was unchanged.

A focused native idle sample showed mean CPU of 1.180% before and 1.275% after, and settled RSS of
108.766 MiB and 111.484 MiB. This small, noisy sample supports no idle-performance improvement
claim. Startup latency, full-frame p95, GPU memory, live traffic and other platforms were unmeasured.

These workloads can be repeated with the pinned toolchain using `cargo replay` for the reducer and
the existing ignored client-core, desktop-frame and replay-soak workloads for detailed memory work.
The delivery skill documents how to compare a task baseline with the changed build. Do not compare
results from different machines or claim live-client behavior from synthetic fixtures.

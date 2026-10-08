# Performance findings

Recent synthetic/offline measurements are workload-specific. They do not establish live Discord
performance, universal device results or application-wide memory bounds. The raw PR screenshot,
log and per-run evidence archive has been removed; the summaries below retain the useful results.

## Animation frame retention — October 8, 2026

Decoded GIF and animated-avatar frames were the largest bounded RAM consumer. Each frame is held as
RGBA, so one 498x280, 40-frame GIF is about 22 MB and a 128 px, 40-frame avatar about 2.6 MB. Inline
GIFs kept their frames after scrolling away until a 128 MiB pool filled. Animated avatars and banners
kept theirs in another 128 MiB pool, although avatars only play on hover or in an open profile.

Frames not played for 5 s (scrolled away, not hovered, or shown in an unfocused window) are now
released while the still texture stays. Hover-only avatars keep frames only when they are about to
play. Released artwork asks the worker for frames again only once it can play; the encoded source comes from
the disk cache, so this costs a re-decode, not a download.

The ignored `ui` workload `animation_memory_workload` (release build, Apple M1, Rust 1.98.1, no window,
GPU, network or account) scrolls past 12 GIF embeds with 20 of 60 animated avatar rows on screen, then
settles for 6 s. Retained bytes were identical across three runs per revision (six on the change):

| Metric | Base | Change |
| --- | ---: | ---: |
| Peak retained decoded frames | 237.3 MiB | 111.7 MiB |
| Settled retained decoded frames | 237.3 MiB | 46.3 MiB |
| Peak process RSS | 265.7–265.8 MiB | 145.2–157.1 MiB (5 of 6 runs ≤ 145.4) |

Retained bytes come from the pools' own accounting. RSS did not fall after settling, because the
macOS allocator keeps freed ~0.5 MB blocks resident for reuse. Later decodes reuse them rather than
growing the process. GPU playback textures, which were also released, were not measured, and the
real app's scroll speed and media mix will differ. Standard macOS package: executable
68,526,000 B (+16,416), installed app 74,564,335 B (+16,416), `ditto` ZIP 48,183,136 B (+3,269).

## Active-call repaint cadence — October 8, 2026

A code audit of the UI, desktop wiring, client state and network/voice crates found one
always-on cost: while any call was active, `logic()` requested a repaint every 50 ms, running the
whole UI pass at 20 Hz even though speaking, notices, remote video, devices, hotkeys, screen share
and deadlines each already wake the UI themselves. The request is now a 1 s heartbeat.

The synthetic `--demo --demo-call` fixture (no credentials, no audio device, no network) was sampled
on an Apple M1 (16 GiB, macOS 27.0, Rust 1.98.1) with release `--features demo` builds of the base
commit and the change. Each sample waited 8 s, then summed process CPU time and polled RSS every
0.5 s for 30 s; base and change were alternated.

| Workload (30 s) | Base | Change |
| --- | ---: | ---: |
| Active-call fixture, CPU, 3 runs each | 6.27%, 5.77%, 6.17% | 0.50%, 0.50%, 0.50% |
| Active-call fixture, peak RSS | 126.3–126.5 MiB | 126.3–126.4 MiB |
| Plain `--demo` idle, CPU | 0.000% | 0.000% |

The 0.50% remaining is the 1 s heartbeat plus the call timer. CPU time has 10 ms resolution, so
treat the figures as approximate. They cover one fixture and display, not a live call: remote video,
screen share, audio threads and GPU work were not measured. A `footprint`/`heap` look at the idle demo
showed 68 MB physical footprint and 11.6 MB of live heap, so no idle-memory regression was found.

The same change set also avoids work that was not benchmarked, so no speedup is claimed for it:
permission decisions are no longer discarded when an event leaves the guild and channel records
equal or when an unrelated channel is removed, notification/read-state lookups use the indexed
channel map instead of a linear scan, and the composer thumbnail reads at most 64 MiB (the decode
allocation limit) instead of up to the 500 MB upload limit plus a second copy.

Standard no-default-features macOS packages built from both revisions: executable 68,509,584 B in
both, installed app 74,547,919 B in both, `ditto` ZIP 48,179,547 vs 48,179,867 B (+320 B).

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

## Opt-in Voice Messages — October 5, 2026

Compared baseline `6313e271e9c34b39069eb20787afb1476741b21c` with runtime source
`9b44a4dfbe6d1702f36b20b0877bd24610cd9fa8`. Both use pinned Rust 1.98.1, ordinary
fat LTO and the locked dependencies; no new dependency versions or system runtime
requirements were introduced. Standard packages include voice and disable
default/demo features; both pass the 225-file Debian smoke check.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable | 85,597,872 B | 86,025,264 B | +427,392 B / +0.50% |
| Installed regular files | 90,111,993 B | 90,539,385 B | +427,392 B / +0.47% |
| Complete compressed DEB | 44,266,736 B | 44,328,696 B | +61,960 B / +0.14% |
| Mean idle CPU, one logical core | 0.00% | 0.00% | 0.00 percentage points |
| Sampled peak RSS | 242,778,112 B | 251,494,400 B | +8,716,288 B / +3.59% |
| Settled RSS | 242,778,112 B | 251,363,328 B | +8,585,216 B / +3.54% |

Native idle sampling uses release `--no-default-features --features demo`,
`--demo --demo-chat --demo-recorder`, a neutral channel-header click, eleven seconds
of total warmup and twenty one-second psutil CPU/RSS samples per revision. The
baseline ignores the new recorder fixture flag; after enables its synthetic
contribution with no dialog/capture active. Settled RSS is the final five samples'
median. Host: Ubuntu 26.04.1 / Linux 7.0.0-34, AMD Ryzen 5 7535U (12 logical CPUs),
15,369,355,264 bytes RAM, Xvfb/X11 1120×760 scale 1, WGPU/Vulkan llvmpipe
(LLVM 21.1.8), with Mesa lavapipe forced. No builds or helper children ran during
sampling. This one launch pair reports an 8.19-MiB settled RSS increase but cannot
separate integration cost from allocator/driver variation. No improvement is claimed.
Startup peaks, frame latency, GPU memory, physical microphone/DSP cost and live
Discord delivery are unmeasured.

One clip is bounded to 120 seconds / 8 MiB, with an eight-frame / 30,720-sample-byte
PCM queue, at most 6,000 amplitude bytes and a replaceable 64-bin UI snapshot.
Device-free tests validate Opus/Ogg decoding, duration trimming, controls and
bounds; synthetic HTTP tests validate the single-send metadata path. These are
component checks, not physical audio or live-account compatibility evidence.

The [evidence README](pr-evidence/voice-messages/README.md) gives reproduction
commands and native dark/light inspection. [Raw measurements and package hashes](pr-evidence/voice-messages/measurements.json)
include every sample and both release executable hashes. `cargo xtask check`,
extension/SDK checks and standard Linux packaging pass.

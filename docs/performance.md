# Performance findings

Recent synthetic/offline measurements are workload-specific. They do not establish live Discord
performance, universal device results or application-wide memory bounds. The older upstream PR screenshot,
log and per-run evidence archive has been removed; the summaries below retain the useful results.

## Long-session live memory inspection — October 9, 2026

Read-only `vmmap -summary`, `footprint` and `heap -s` inspected the owner's already-running
macOS process after 11 hours 33 minutes. No messages, calls, microphone use, UI interaction,
restart, payload dump or credential access occurred. Host: Apple M1 Pro, 16 GiB RAM,
macOS 27.0.1 (26A434). The installed executable's timestamp was October 7;
its source commit and enabled features were unavailable. It therefore appears to predate
main's October 8 idle-animation cleanup (`1b3e4a7b`), but this is not verified build provenance.

Physical footprint was 382.0 MiB, with a lifetime peak of 528.6 MiB. Ordinary `ps` RSS was
about 70.5 MiB; compressed/swapped private pages explain much of the discrepancy. The
697.2 MiB resident-region sum includes shared code/library pages and is not the app's
private footprint. The roughly 390 GiB virtual total is mostly reserved guard address
space, not RAM consumption.

| Allocation/category | Live snapshot | Interpretation |
| --- | ---: | --- |
| Allocated heap across malloc zones | 215.3 MiB | Includes Rust buffers and native objects; not all message state |
| Heap dirty/swapped space beyond allocated bytes | 42.7 MiB | Allocator fragmentation/free space, as reported by `vmmap` |
| Owned graphics backing | 84.4 MiB | Driver/GPU resources charged to the process |
| IOSurface | 26.9 MiB | Shared graphics surfaces |
| IOAccelerator graphics | 1.6 MiB | Additional mapped graphics resources |

These categories leave roughly 11 MiB for other footprint charges. Heap inspection found
205.7 MiB of untyped allocations, including hundreds of blocks around 208–800 KiB; this
is consistent with decoded image/frame sizes, not proof of their owner. Allocation stack
logging was not enabled, so exact attribution to Rust types/functions is unavailable.
One snapshot and a lifetime peak do not demonstrate a leak or a growth rate. Aggregate
sizes only are recorded here; no account data or heap payloads are published.
A second `footprint` sample at 11 hours 54 minutes reported 362 MiB (about 20 MiB less),
with graphics categories unchanged. The original production executable was still running;
no fix had been installed. Host builds were active, and app interaction was not controlled,
so this is an observation of ordinary variation, not an improvement from this PR.

Current main additionally keeps inline still textures in a 96-MiB / 384-item pool until
capacity eviction. The change expires renditions not painted for 60 seconds, using the
existing maintenance wakeup. Visible renditions refresh their deadline, while viewer-close
and five-second unplayed-frame cleanup retain their existing behavior. The pool's byte
accounting describes retained texture/frame resources, not whole-process memory.

Compared baseline `481a0c41` with this change on the same host, pinned Rust 1.98.1,
locked dependencies and release profile (fat LTO). The ignored `ui` library workload
`still_memory_workload` admits 160 synthetic 512×288 stills through the media library,
consumes upload deltas, then advances the maintenance clock by 61 seconds without painting.
It has no window, GPU, network or account. One warmup and three direct test-binary runs per
revision produced identical retained-byte counts:

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Peak retained inline texture bytes | 94,371,840 (90 MiB) | 94,371,840 (90 MiB) | 0 |
| Retained inline texture bytes after expiry | 94,371,840 (90 MiB) | 0 | −94,371,840 (−100%) |

These are texture-size accounting values, not measured GPU allocations or process RSS.
Regression tests additionally verify a renderer free delta, the exact expiry boundary,
painting renewal, reload eligibility and viewer-close cleanup. Native desktop automation
was not exposed in this session, so a scripted native populated-cache comparison was not
obtained. Native GPU reclamation, before/after process footprint, day-long soak, frame
latency and other platforms remain unmeasured. The production app was not replaced.

Both revisions passed standard `cargo xtask package` with voice included, no default
features and the locked release profile. Each app was saved separately; installed bytes
sum all app files, and distribution ZIPs use `ditto -c -k --sequesterRsrc --keepParent`.

| Package metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 68,657,520 B | 68,657,536 B | +16 B (+0.000023%) |
| Installed app | 74,695,855 B | 74,695,871 B | +16 B (+0.000021%) |
| Distribution ZIP | 48,255,410 B | 48,255,390 B | -20 B (-0.000041%) |

These package deltas are negligible. Both packages were locally ad-hoc signed, not notarized.

## Allocation-stack follow-up and idle artwork — October 10, 2026

The owner's restarted current-main build (`0d9c6772`, optimized release with
matching Rust symbols and `MallocStackLogging=lite`) was inspected after normal
server, image, profile, call and streaming activity. No agent-driven messages,
calls or microphone capture were performed. At roughly four minutes the process
footprint was 290.4 MiB (503.3 MiB peak), with 67.0 MiB live malloc allocations.
Graphics regions included 150.6 MiB owned unmapped backing, about 27 MiB IOSurface
and 1.6 MiB IOAccelerator. The malloc-zone report also included 69.6 MiB dirty
fragmentation/slack and 16.2 MiB profiling-tool data. These categories use
different accounting rules; they must not be added to derive footprint.

Allocation stacks identified 18.6 MiB for `ui::fonts::install_cjk` and 16.0 MiB
for epaint's CPU glyph atlas. The two large worker groups from the earlier
October 7 binary were absent. The previous 501.3-MiB and current 290.4-MiB live
captures had different activity and duration: they are diagnostic snapshots,
not a controlled improvement benchmark. Stack logging adds observer overhead.
Only addresses/stacks and summaries were captured, never allocation contents;
the sorted allocation report was capped at 16 MiB.

The leak scanner flagged 40,016 bytes in 440 blocks. Most belonged to macOS
AppIntents/XPC cycles; about 13 KiB traced to RNNoise's easyfft/generic_singleton
thread-local FFT planner cache. Its dependency deliberately uses `Box::leak`;
this small per-worker retention remains and is not fixed by artwork expiry.

Avatar/banner/activity/sticker/picker textures and custom/high-resolution emoji
textures previously stayed until their item/byte pools filled. They now expire
after 60 seconds without use, using the existing host maintenance deadline.
Visible and cached composer artwork refreshes the deadline. Composer layout only
reads cached still textures; it neither refreshes nor requests clipped assets.
Timeline custom/shared artwork and jumbo Unicode emoji resolve only for visible
slots, so leading overscan rows cannot retain or repeatedly reload them. Expired
artwork uses the normal bounded worker/disk-cache path when painted again.
Existing 64-MiB artwork and 16-MiB emoji ceilings are unchanged.

The ignored `avatars::tests::idle_artwork_workload` compares optimized release
unit-test builds with the pinned Rust 1.98.1 toolchain and lockfile on macOS
27.0.1 / Apple M1 Pro / 16 GiB RAM. The new harness was first run on unmodified baseline source before applying
the runtime change. The same workload submits 932 synthetic
64×64 RGBA textures without draining the existing 128-request batch, so only
128 avatar textures are admitted. It advances the maintenance clock by 61
seconds, with one warmup and three measured runs per build. All runs agree.
This measures retained texture payload accounting without a window or network,
not GPU allocation, process RSS, peak decoding memory or latency. Release samples
are reused from initial implementation `2b60dd8c`; the review follow-up changes
layout/paint access, not expiry accounting. One run of the final debug test binary
confirmed the same 2,097,152 B peak and zero settled payload. A redundant optimized
UI-test rebuild was stopped during compilation; it produced no new release sample.
Separate UI
regressions cover both cache pools, clipped/visible artwork, reloads and cached
composer image painting. Native before/after process and GPU measurements are
unmeasured; attempted standard-package `--demo` processes exited before a valid
native sample could be captured. Their exit cause was not established and no
zero-RSS result is used as evidence.

| Texture payload metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Peak admitted payload | 2,097,152 B | 2,097,152 B | 0 B |
| Payload after idle maintenance | 2,097,152 B | 0 B | -2,097,152 B / -100% |
| Retained cache entries after idle maintenance | 128 | 0 | -128 |

GPU drivers and allocators may reserve freed resources; these payload reductions
are not an equal whole-process RAM reduction guarantee. Returning after expiry
can re-decode cached artwork or fetch it again after a disk-cache miss. Font
storage and the shared Unicode emoji atlas retain their existing lifetimes.

The standard voice-inclusive package was rebuilt without demo/developer features.
The preserved baseline is the verified PR #618 package at `c16cb2df`, whose source
tree matches starting main `0d9c6772`; its executable hash was checked against
the baseline copy before comparison. This is a reused baseline, not a simultaneous
fresh package build. Both use the pinned release profile and lockfile.

| Package metric | Baseline | After | Delta | Method |
| --- | ---: | ---: | ---: | --- |
| Executable | 68,657,536 B | 68,657,536 B | +0 B | Mach-O file length |
| Installed bundle | 74,695,871 B | 74,695,871 B | +0 B | Sum of bundle file lengths |
| ZIP | 48,255,390 B | 48,261,871 B | +6,481 B / +0.0134% | ditto -c -k --sequesterRsrc --keepParent |

The ZIP difference is small compression/signing noise, not a meaningful package
improvement. The local package is ad-hoc signed, not notarized. Full workspace
checking passes (1,234 tests, zero failures), including formatting, strict Clippy
and policy checks. Linux/Windows runtime behavior and a controlled live-account
soak after this new artwork change remain unmeasured.

## Gateway resume fallback — October 8, 2026

Compared baseline `2164f52e` with implementation `2ebc48a7` on Windows 11 x64,
AMD Ryzen 7 7800X3D (16 logical CPUs), 32 GB RAM, Rust 1.98.1 and the locked
dependencies. The recovery change adds one fixed-size failure counter, with no
new queue or persistence. Synthetic localhost tests now return to the original
gateway after three failed resume attempts; the baseline keeps selecting the
failed resume route. Manual recovery and successful resumes preserve their
existing behavior. These are protocol-policy checks, not live recovery timings.

Both revisions passed `cargo xtask package` with voice included, release fat LTO
and no default features. Each output directory was retained separately. Installed
bytes are the sum of all files in `dist`; archives use PowerShell
`Compress-Archive -CompressionLevel Optimal` over that directory.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 86,008,832 B | 86,010,368 B | +1,536 B (+0.0018%) |
| Full portable directory | 90,199,948 B | 90,201,484 B | +1,536 B (+0.0017%) |
| Portable ZIP | 50,002,123 B | 50,002,724 B | +601 B (+0.0012%) |

These are unsigned Windows portable packages. NSIS was unavailable, so no local
installer was produced. Both builds emitted the same OpenH264 duplicate-object
debug-info linker warning; packaging exited successfully.

The release reducer control used `cargo replay`, then one warmup and five direct
`replay-bench` runs per revision. It processed 100,000 events and retained 500
records / 331,992–332,477 estimated timeline bytes in both cases. Median times
were 159.8291 ms before and 117.5277 ms after. The five samples were
173.1020/159.8291/125.8932/152.4096/181.1192 ms and
128.2427/118.3857/106.1182/109.4493/117.5277 ms respectively. Other builds shared
the host, and the replay executables were byte-identical: this workload does not
include the changed gateway transport. The 42.3014 ms difference is not evidence
of a speed improvement. Process RSS, UI timing and live network recovery were
not measured.

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

## Window geometry minimum — October 7, 2026

Baseline `c5e50e77` and fixed `c5ace4dd` were built on Windows 11 Home build
26200, Ryzen 7 7800X3D, 32 GiB RAM, Rust 1.98.1. Both standard voice-enabled
packages used `cargo xtask package` with no default or demo features. The installed
size sums all 216 files in `dist`; .NET `ZipFile.CreateFromDirectory` compressed
each complete directory with the same default settings. NSIS was unavailable, so
these measurements cover the package directory and ZIP, not an installer binary.

| Metric | Baseline | Fixed | Delta |
| --- | ---: | ---: | ---: |
| Release executable | 85,922,304 B | 85,922,304 B | 0 B |
| Installed package | 90,113,420 B | 90,113,420 B | 0 B |
| Distribution ZIP | 49,970,177 B | 49,970,277 B | +100 B (+0.00020%) |

For native idle samples, each source was also built with
`cargo build --release --locked -p serein --no-default-features --features demo`
and run with `--demo`.
Both runs used the same 1120×760 synthetic fixture, Windows DPI 120 (125% scale),
the default DX12 renderer and no child processes. After a 15-second warmup,
`Get-Process` sampled cumulative CPU time and private bytes every second for ten
seconds, with no further interaction. CPU is the mean one-core percentage; settled
private memory is the median of the final five readings.

| Native demo metric | Baseline | Fixed | Delta |
| --- | ---: | ---: | ---: |
| Idle CPU, 10 samples | 0% | 0% | Below sample resolution |
| Peak private memory | 192,835,584 B | 192,643,072 B | -192,512 B (-0.10%) |
| Settled private memory | 192,835,584 B | 192,643,072 B | -192,512 B (-0.10%) |

The tiny ZIP and memory differences do not establish a performance improvement.
Startup latency and frame timing were not measured. Demo ignores persisted geometry;
the offline window-geometry check covers restoration of undersized saved values.

## Window minimum and restoration (PR #583): Windows integration evidence - October 8, 2026

Fresh standard Windows x64 voice-enabled packages compare main `1b3e4a7b` with `371632f1` (measured 2026-10-08). Baseline/current file counts: 216/216.

| Metric | Main `1b3e4a7b` | Current integration | Delta |
| --- | ---: | ---: | ---: |
| Standard executable | 86,008,832 B | 86,009,344 B | +512 B (+0.0006%) |
| Installed directory | 90,199,948 B | 90,200,460 B | +512 B (+0.0006%) |
| Distribution ZIP | 50,001,798 B | 50,002,093 B | +295 B (+0.0006%) |

Method: `cargo xtask package`, Rust 1.98.1, standard release flags without demo; Windows 11 build 26200, Ryzen 7 7800X3D, 32 GiB RAM. Runtime workspace artifacts were invalidated before each feature build. Installed bytes sum every file in `dist`; ZIP uses whole-directory .NET Optimal compression. NSIS was unavailable, so no installer executable was built.

Current native CPU, memory, frame/startup latency and affected-device behavior remain unmeasured because the native automation bridge is unavailable. Package size and synthetic reducer timing do not establish live Discord performance.

### Window minimum after monitor changes — review follow-up (2026-10-08)

Fresh standard Windows x64 voice-enabled packages compare `666d7d4a` with this
review fix. Both use Rust 1.98.1, the pinned lockfile, standard release flags
without demo, Windows 11 build 26200, Ryzen 7 7800X3D and 32 GiB RAM. Each package
contains 216 files. Installed bytes sum all files in `dist`; ZIP uses the complete
directory with .NET `ZipFile.CreateFromDirectory`, Optimal compression.

| Metric | Before | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable | 86,009,344 B | 86,011,904 B | +2,560 B (+0.0030%) |
| Installed directory | 90,200,460 B | 90,203,020 B | +2,560 B (+0.0028%) |
| Distribution ZIP | 50,002,090 B | 50,003,472 B | +1,382 B (+0.0028%) |

Both `cargo xtask package` runs passed; NSIS is unavailable, so no installer
executable was produced. The final `cargo xtask check` passed formatting, strict
workspace Clippy, workspace tests, demo compilation and policy checks. Four
focused application-settings tests passed, including small displays, scale and
decoration bounds for the recalculated minimum.

Native capture/interaction remains unavailable (the native helper reports a
missing pipe). Moving a real window between monitors, native CPU/memory, startup
and frame latency were not measured. These package measurements and synthetic
tests do not establish native multi-monitor or live Discord behavior.
## Watched-stream recovery — October 8, 2026

Standard Windows x64 voice-enabled packages (`cargo xtask package`, no demo feature,
Rust 1.98.1) compared baseline `1b3e4a7b` and fixed runtime `5b446a45` on Windows 11
build 26200, Ryzen 7 7800X3D, 32 GiB RAM. Executable size stayed 86,008,832 B;
the 216-file installed directory stayed 90,199,948 B. Whole-directory .NET Optimal
ZIP size changed from 50,001,798 B to 50,001,523 B (-275 B), packaging variation.
NSIS was unavailable, so no installer binary was built. Native CPU/memory/frame
measurements and live stream continuity remain unmeasured because the native
automation bridge is unavailable. The lifecycle regression checks retained worker
ownership during recovery; it does not measure network quality or throughput.

## Channel and message pills — October 7, 2026

Compared baseline `38d919d7` with runtime `78de8d15` on Windows 11 Home
10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), 33,410,678,784 bytes RAM,
Rust 1.98.1. Both use the same new synthetic channel-links fixture and the
same Windows allocator lockfile correction, required to compile the baseline.
The release preview uses WGPU with the demo feature and 125% display scale;
the selected GPU/backend was not logged.

After five seconds warmup, 21 process-counter samples were taken about one
second apart per build. This was a static idle fixture, with no scripted input.
Baseline elapsed sample time was 20.536 s and after was 20.252 s. Settled memory
is the mean of the final five samples.

| Metric | Baseline | After | Delta | Method |
| --- | ---: | ---: | ---: | --- |
| CPU, percentage of one core | 0.00% | 0.31% | +0.31 percentage points | Windows process CPU delta / actual elapsed time |
| Peak / settled working set | 196,276,224 B | 198,307,840 B | +2,031,616 B / +1.04% | Windows WorkingSet64; maximum / mean of last five samples |
| Settled private bytes | 414,261,248 B | 415,698,944 B | +1,437,696 B / +0.35% | Windows PrivateMemorySize64; mean of last five samples |

One series per build on a shared machine with compiler activity is observational;
it supports no performance improvement claim. Native frame/startup latency and
live-service performance were not measured. Both standard voice-inclusive release
packages passed, without demo/developer features:

| Package metric | Baseline | After | Delta | Method |
| --- | ---: | ---: | ---: | --- |
| Executable | 85,449,728 B | 85,464,064 B | +14,336 B / +0.017% | serein.exe file length |
| Installed files | 89,623,341 B | 89,637,677 B | +14,336 B / +0.016% | Sum of all dist file lengths |
| ZIP distribution | 49,775,655 B | 49,781,438 B | +5,783 B / +0.012% | Compress-Archive, Optimal; ZIP file length |

Installed bytes sum all files in `dist`; ZIPs use PowerShell `Compress-Archive`
with Optimal compression. NSIS was unavailable, so no installer executable was
generated locally. Both unsigned packages emitted the existing OpenH264
duplicate-object debug-info linker warning.

Inspected dark/wide and light/narrow images are eframe framebuffer exports. The
native computer-use helper was unavailable, so OS screenshots, native input and
screen-reader operation remain unverified. Workspace checking reached 1,154
passing tests, eight failures also reproduced on the baseline, and 23 ignored
tests. Seven focused pill tests and strict Clippy passed after the final visual
correction. Commands, raw counters, baseline failure names and image provenance
are in [the evidence directory](pr-evidence/channel-message-pills/README.md).

### Review follow-up package

The standard voice-inclusive package was rebuilt at `7d9c0d05` after the review
fixes, using the same Windows host, pinned toolchain, no demo/developer features,
and compression method. It passed; NSIS remains unavailable locally. Comparison
with the same `38d919d7` baseline:

| Package metric | Baseline | After review | Delta | Method |
| --- | ---: | ---: | ---: | --- |
| Executable | 85,449,728 B | 85,464,576 B | +14,848 B / +0.017% | serein.exe file length |
| Installed files | 89,623,341 B | 89,638,189 B | +14,848 B / +0.017% | Sum of all dist file lengths |
| ZIP distribution | 49,775,655 B | 49,781,661 B | +6,006 B / +0.012% | Compress-Archive, Optimal; ZIP file length |

Raw sizes and the executable hash are in
[review-package.json](pr-evidence/channel-message-pills/review-package.json).
The process samples and screenshots above remain evidence for `78de8d15`; they
were not repeated for the review fixes and support no claim about changed CPU
or memory usage. Full workspace checking at `f9e568c9` reached 1,157 passes,
the same eight baseline failures, and 23 ignored tests. After the final Chinese
label correction, all ten focused pill tests, formatting and strict workspace
Clippy passed. Native input/capture remain unverified; merge blockers persist.

### Reply message-link preview follow-up (2026-10-08)

Compared `d2299412` with the reply-preview fix, using the new synthetic
`channel-link-replies` fixture in both builds. The Windows host, WGPU renderer,
125% scale and five-second warmup match the previous section. Each build has one
series of 21 process-counter samples over about 20 seconds. Baseline and changed
preview executables were rebuilt and preserved separately. Concurrent builds
and OS residency add noise; these idle results are not a performance improvement
claim and do not measure frame latency or live Discord behavior.

| Metric | Baseline | After | Delta | Method |
| --- | ---: | ---: | ---: | --- |
| CPU, one-core percentage | 0.692% | 0.000% | -0.692 percentage points | Process CPU delta / elapsed time |
| Peak working set | 200,089,600 B | 213,258,240 B | +13,168,640 B / +6.58% | WorkingSet64 maximum |
| Settled working set | 200,089,600 B | 213,257,421 B | +13,167,821 B / +6.58% | WorkingSet64 mean of last five samples |
| Settled private bytes | 416,260,096 B | 416,219,136 B | -40,960 B / -0.010% | PrivateMemorySize64 mean of last five samples |

Working set increased while private bytes were approximately flat. Raw samples,
the inspected light/dark framebuffer pairs and reproduction details are in
[the reply evidence](pr-evidence/channel-message-pills/README.md#reply-message-links-2026-10-08).
Native input and OS screenshot capture remain unavailable; a successful
framebuffer export does not establish them.

The fresh standard voice-inclusive package passes without demo/developer
features. Compared with the historical `7d9c0d05` package, executable and
installed payload grow by 512 B; ZIP grows by 214 B. The runtime source at
`7d9c0d05` is identical to pre-fix `d2299412` (the intervening changes are four
documentation files), but this is not a fresh simultaneous package baseline.

| Package metric | Historical baseline | Reply fix | Delta | Method |
| --- | ---: | ---: | ---: | --- |
| Executable | 85,464,576 B | 85,465,088 B | +512 B / +0.0006% | serein.exe file length |
| Installed files | 89,638,189 B | 89,638,701 B | +512 B / +0.0006% | Sum of dist file lengths |
| ZIP distribution | 49,781,661 B | 49,781,875 B | +214 B / +0.0004% | Compress-Archive, Optimal; ZIP file length |

[reply-package.json](pr-evidence/channel-message-pills/reply-package.json) records
the new measurements and executable hash. NSIS remains unavailable; this is an
unsigned directory and ZIP. Final serial workspace checking reached 1,158
passes, eight recorded baseline UI failures and 23 ignored tests. Formatting,
strict workspace Clippy and the independent policy check passed.

## Attachment picker and draft removal — October 10, 2026

Starting from `b6d608bb`, with upstream `0d9c6772` merged, four offline regressions
fail before the attachment fixes and pass afterwards. The complete upload module's
15 tests and one headless tray/card interaction fixture pass. Current upload,
model/core/session/protocol/API and picker/HEIC source is rebuilt against cached
external dependencies; the tray fixture extracts current methods and uses cached
UI helpers. Preview cancellation returns its job/byte permits and preserves the
previous upload; no queue, resource budget or persistence is added. These are
scoped correctness/resource checks, not full application or performance evidence.
Current release package sizes, native screenshots, CPU/RSS and frame latency are
unmeasured: the shared Cargo build-cache lock is not writable, disk space prevents
a fresh full cache, and native window capture tools are unavailable.

## Runtime resource cleanup — October 8, 2026

Historical evidence from the original combined PR, separated by scope. The recorded revision labels, hashes and measurements are unchanged; these results do not validate the new branch heads.

Against `98158b4`, queued updater results retained staged directories or inert helpers after app closure. Results now retain cleanup ownership until accepted. The remaining staged-directory count changes from 1 to 0; the helper exits within the fixture’s three-second bound. Registered-game normalization also preserves its 256-byte ceiling. See [the resource evidence](pr-evidence/updater-resource-hunt/README.md). Full executable/package size, CPU/RSS and native integration remain unmeasured.
## Repository-wide lifecycle and state bug hunt — October 5, 2026

Compare the verified local starting commit `b6c32b6ddfa3bef63f01bc9b101de5aa20203017`
with the repository-wide bug-hunt fixes. The baseline and after binaries were built
from isolated worktrees with locked dependencies. Testing used synthetic offline data
and did not require an account, physical capture device or GPU. Raw samples, artifact
identities and environment details are in the
[bug-hunt measurements](pr-evidence/repository-bug-hunt/measurements.json).

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable, bytes | 85,746,216 | 85,796,648 | 50,432 / +0.059% |
| Installed regular files, bytes | 120,976,132 | 121,026,564 | 50,432 / +0.042% |
| Compressed DEB, bytes | 70,795,724 | 70,809,936 | 14,212 / +0.020% |
| Reducer replay median, 100,000 events | 79.729 ms | 80.958 ms | 1.229 ms / +1.54% |
| Appearance idle mean CPU, one core | 360.955% | 347.380% | -13.575 percentage points / -3.76% |
| Appearance idle sampled peak RSS | 192.594 MiB | 194.070 MiB | 1.477 MiB / +0.77% |

The environment was Debian 13.7 on Linux 6.18.44 x86_64, Intel Xeon Platinum
8573C, five visible logical CPUs, 18,882,699,264 bytes RAM and Rust 1.98.1.
The standard `cargo xtask package` build used the normal release profile and
passed the 248-file DEB smoke covering its executable, desktop entry, metadata,
ownership, content and host shared-library closure. Installed size sums regular
file content and excludes filesystem allocation and symlink overhead.

The reducer benchmark replays 100,000 deterministic gateway events into a fresh
process. After one warmup per revision, five baseline/after pairs alternated order.
Baseline samples ranged from 73.512 to 111.762 ms and after samples from 75.801 to
94.434 ms. Both revisions retained 500 records with an estimated timeline size of
331,992 to 332,477 bytes. The overlapping ranges and 1.54% median difference are
consistent with run-to-run noise, so no reducer regression or improvement is
claimed.

The native idle check used the release `profile_preview` example with demo data on
the Appearance page at 1120×760 under Xvfb and Mesa lavapipe Vulkan. Each revision
warmed for eight seconds, settled for three seconds after pointer movement, then
recorded twenty one-second CPU and RSS samples. Neither process spawned children.
After mean CPU was 13.575 percentage points lower and sampled RSS was 1,548,288
bytes higher. This single software-renderer pair does not establish a performance
change; frame timing, GPU memory and active media workloads remain unmeasured.

`cargo xtask check` passed the full workspace tests, strict all-target Clippy,
documentation, no-default-feature build and policy checks. Focused GStreamer,
voice, packaging and repository-script tests also passed, followed by the standard
package smoke. Native Windows/macOS builds, physical GPU and capture devices, live
accounts, live Wayland compositor events and extended soak testing remain
unverified.

## Responsive native controls — October 5, 2026

Historical evidence from the original combined PR, separated by scope. The recorded revision labels, hashes and measurements are unchanged; these results do not validate the new branch heads.

[Raw samples and source hashes](pr-evidence/responsive-ui/measurements.json) and
[reproduction scripts](pr-evidence/responsive-ui/README.md) record a fresh native
before/after comparison. Environment: Debian 13 x86-64, Xeon Platinum 8573C,
five visible CPUs, 17 GiB RAM, Rust 1.98.1, eframe/wgpu under Xvfb with
`WGPU_BACKEND=gl` and Mesa software output. Both builds use the same isolated
native preview, `dev` profile with optimization level 1 and no debug info, actual
UI code, synthetic long-name reply fixture, dark appearance, 1120x760 pixels
and 100% zoom. Tray/startup availability is fixed to false; the helper contains
no service or capture adapters. These are development preview measurements,
not standard release/package measurements.

Compilation stopped before sampling. One run per revision warms up for three
seconds, then samples process RSS once per second for 15 seconds and computes
aggregate CPU time over that interval. Neither process had children.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Idle CPU, percentage of one core | 0.00% | 0.00% | Below process-timer resolution |
| Post-warmup sampled peak/settled RSS | 192,663,552 bytes | 191,758,336 bytes | -905,216 bytes / -0.47% |
| Isolated development preview executable | 83,176,272 bytes | 83,183,560 bytes | +7,288 bytes / +0.009% |

The small RSS difference is noise from one short paired sample; no memory or
speed improvement is claimed. Startup peak, frame/startup latency, sustained
load, leak soaks and GPU memory remain unmeasured. Native screenshots were
inspected at 1120x760/100% dark and 760x520/150% light/dark, including scrolling
to the encoding card and the last Add Friend action. The reply/edit regression
test also verifies that context controls leave the message input inside the
viewport. All 417 UI library tests pass (five benchmark tests ignored), and
strict UI Clippy across all targets passes.

The default workspace-check/package commands hit the inherited read-only debug
cache. Retrying the workspace check and standard release package in an owned
target directory reaches the native dependency build and fails because
`glib-sys` cannot find `glib-2.0.pc`.
The environment also lacks GTK4/WebKit6 development/runtime dependencies.
Standard release executable, installed-package and compressed-distribution
sizes remain unavailable; complete workspace/packaging validation is a CI
requirement. The development preview sizes above must not be treated as shipped
package sizes.

## Codebase bug hunt — October 5, 2026

Baseline `8a369cc3c71c18d97065dde8197c16d90970ed0a`; see
[reproduction and raw samples](pr-evidence/codebase-bug-hunt/README.md).
Debian 13 x86_64, kernel 6.18.44, five visible CPUs, 18,882,699,264 bytes RAM
and Rust 1.98.1. Standard release `replay-bench` binaries were rebuilt from each
revision. Each ran once for warmup and five measured times with alternating pair
order, without concurrent compilation or preview processes. Both replay 100,000
synthetic events and retain 331,992–332,477 estimated timeline bytes / 500 records.

The isolated native Appearance preview uses actual UI code with `ui/demo`,
opt-level 1/debug 0, Xvfb, eframe/wgpu and requested GL software rendering
(`WGPU_BACKEND=gl`, `LIBGL_ALWAYS_SOFTWARE=1`; exact adapter not instrumented).
Both use 1120 × 760 dark at 100% zoom, a six-second warmup including 100 scroll
events, then 15 one-second process RSS samples. Aggregate idle CPU is the
percentage of one core over that interval. Neither preview had children. The
new synthetic fixture requests Serif after session reset; baseline silently
retains Inter, while the fixed registry applies Serif.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release reducer median, five runs | 83.116483 ms | 88.286294 ms | +5.169811 ms / +6.22% |
| Release replay executable | 1,510,296 bytes | 1,509,592 bytes | −704 bytes / −0.047% |
| Auxiliary UI sampled peak/settled RSS | 161,026,048 bytes | 161,624,064 bytes | +598,016 bytes / +0.37% |
| Auxiliary UI idle CPU, one core | 0.00% | 0.00% | Below process-timer resolution |
| Auxiliary UI executable | 89,526,776 bytes | 89,530,560 bytes | +3,784 bytes / +0.0042% |
| Full desktop release / installed / compressed package | Unavailable | Unavailable | Missing `glib-2.0.pc` |

Reducer ranges overlap widely: 80.75–123.18 ms before and 82.21–142.91 ms after.
The single short UI pair and five replay samples do not establish a performance
improvement or a statistically significant regression. Preview binary sizes
must not be treated as shipped desktop/package sizes. Frame/startup latency,
GPU memory, sustained-load/leak soaks and physical media-device cost remain
unmeasured. Actual voice tests pass with two Linux OpenH264 runtime versions;
those checks measure correctness, not encoding performance.

The thumbnail fix admits at most two jobs and 32 MiB of source bytes before
copying/spawning, retaining permits through actual decoder completion. Failed
placeholder attempts cap at 2,048; tracked credential-removal IDs cap at eight. These are resource ceilings, not
process RSS or proof that all application leaks are absent. Standard workspace
and package checks were attempted and remain blocked at missing GLib development
metadata; the license-policy command also lacks `cargo-deny`.

## Runtime bug hunt — October 7, 2026

Compared baseline `8003ae068679fecb67cd02c4bc5e6fbedf37ffee` with this task's
client-core search corrections. The measured after build uses the working tree
before adding this documentation; the source blob IDs and binary hashes are in
[the raw measurements](pr-evidence/runtime-bug-hunt/measurements.json).
Both actual `replay-bench` binaries use Rust 1.98.1, the pinned lockfile, normal
release fat LTO and no optional features. The baseline uses a detached worktree;
both builds share the same writable Cargo cache and target directory, with the
baseline binary copied aside before building the changed sources.

Host: Linux x86-64, Intel Xeon Platinum 8573C, five exposed logical CPUs and
17 GiB exposed RAM. After one warmup per binary, five measured runs per binary
use alternating pair order. No task compiler or native fixture process runs
during sampling; variation from the shared host remains. The existing workload
applies 100,000 synthetic message events, checks the 500-record / 4 MiB timeline
bounds and verifies logout releases retained timeline data.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Reducer elapsed median (five runs) | 99.675228 ms | 95.818134 ms | −3.857094 ms (−3.87%); noisy |
| Reducer elapsed sample range | 94.165187–106.522434 ms | 87.880852–107.966072 ms | Overlapping ranges |
| Retained timeline estimate | 331,992–332,477 B / 500 records | 331,992–332,477 B / 500 records | Unchanged |
| Release replay executable | 1,511,512 B | 1,516,760 B | +5,248 B (+0.3472%) |
| Standard voice-enabled app, installed and compressed package | Unmeasured | Unmeasured | Missing `glib-2.0.pc` prevents both builds |

This workload does not exercise permission changes with an open search or
desktop credential/cache failure handling. Its timing is a general reducer
regression check, with no performance improvement claim. The executable size
is for the reducer tool, not the installed application. Native UI CPU/RSS,
startup, search latency and application-wide leak detection are unmeasured.
The full workspace check and standard package attempt stop at missing GLib
development metadata. Native before/after screenshots for search retirement
and account cleanup status are consequently unavailable.

## PR #567 upstream conflict resolution — October 7, 2026

Compared the verified release reducer from feature head
`11abe5a3700cfe0ec734808931d3791922d98b12` with the merged working tree including
upstream `20230cb91b0117fe4c888d70867fa600704dcad4`. The subsequent upstream
documentation cleanup `a76e030d749ef558e687bbff6dc592a7dddec244` changes none of
the measured runtime sources. Both use Rust 1.98.1, normal
fat LTO and no optional features; the merged build uses the updated upstream
lockfile. Separate binary paths preserve the baseline. Complete reducer source
blob IDs, executable hashes, warmups and samples are in
`docs/pr-evidence/pr567-conflict-resolution/measurements.json`.

Linux / Intel Xeon Platinum 8573C, five exposed logical CPUs, 17 GiB exposed RAM;
one warmup and five measured runs per binary, with alternating pair order and no
active task compiler during sampling. The workload replays 100,000 synthetic
events and verifies the bounded 500-record timeline and logout release.

| Metric | Baseline | Merged | Delta / method |
| --- | ---: | ---: | --- |
| Reducer median | 125.050355 ms | 144.784515 ms | +19.734160 ms (+15.78%); wide, overlapping sample ranges |
| Reducer sample range | 83.038196–158.760550 ms | 122.432188–168.732948 ms | Five samples each; shared-host scheduling noise |
| Retained timeline estimate | 331,992–332,477 B / 500 records | 331,992–332,477 B / 500 records | Unchanged; not process RSS |
| Release reducer executable | 1,516,760 B | 1,527,472 B | +10,712 B (+0.71%); not the application package |
| Standard voice-enabled app / installed / compressed package | Unmeasured | Unmeasured | Both full check and package attempts stop at missing `glib-2.0.pc` |

The measured median increased; the large timing spread prevents attributing that
change to the merge. No performance improvement is claimed. This reducer does
not exercise native audio, upload previews, renderer GPU routing or UI latency.
Full native GUI screenshots, process CPU/RSS, physical hardware encoding and
live service compatibility remain unverified. Offline audio and upload regressions
exercise their actual bounded production helpers without opening devices.

## Theme command deadline regression

The local bug hunt based on `6451d97` fixes Linux theme detection retaining a
worker when a descendant keeps the helper's stdout open after its parent exits.
The existing incremental process reader now bounds stdout and helper completion
together. With the same inert one-second descendant and 50-ms command deadline,
the original regression fails and the fixed regression passes.

| Metric | Before | After | Method |
| --- | ---: | ---: | --- |
| Fixture process elapsed median | 1013.038 ms | 60.032 ms | One warmup and five samples per binary |
| Sample range | 1008.951–1021.818 ms | 57.558–63.705 ms | Counterbalanced order, shared Linux host |

[Evidence and raw samples](pr-evidence/theme-command-timeout/README.md) record
the pinned toolchain, current-source checks and limits. Elapsed time includes
test startup and demonstrates bounded helper completion; it does not measure
whole-app theme latency, CPU or RSS. Full executable and installed/compressed
package sizes remain unmeasured because workspace check/package attempts stop
at the existing build-cache permission error.

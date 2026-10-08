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

## Arabic/Hebrew message layout and logical selection — October 2, 2026

Standard source `73aa31e1a81629d3eebf901eac70ed881a6dd462` is compared with
main `1107d9045fb9d98980d6d8e9987c96a362b4f9ab`. The native pair uses the preserved
FAT demo from renderer source `5eb13e07`; the later change adds nine other-language
limit notices and a lookup regression, with runtime rendering, i18n code and the
English/Czech catalogs byte-identical. Fresh actual wide dark and narrow light
English captures on `73aa31e1` are byte-identical to the reviewed `5eb` pixels.
The common CPU layout fixture compares original `47a81035` with `73aa31e1`.
Raw samples, commands, source identities, binary hashes and capacity limits are in
[`rtl-message-layout/measurements.json`](https://github.com/ViceVerse-cz/Serein/blob/7d495ed350e165bb7bfd3165b88ee6429e544bcf/docs/pr-evidence/rtl-message-layout/measurements.json).

Environment: macOS 27.0 (26A428), Apple M1 MacBookAir10,1 / 16 GiB, Rust 1.98.1,
locked dependencies, two build jobs.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable, bytes | 62,154,064 | 62,252,688 | +98,624 / +0.1587% |
| Installed package, bytes | 68,164,493 | 68,266,274 | +101,781 / +0.1493% |
| Distribution ZIP, bytes | 43,322,199 | 43,388,480 | +66,281 / +0.1530% |
| Native idle CPU, ten-sample median | 0.0% | 0.0% | 0 percentage points |
| Sampled peak RSS, KiB | 124,464 | 124,832 | +368 / +0.2957% |
| Settled RSS, KiB | 124,416 | 124,784 | +368 / +0.2958% |
| CPU layout, 200 frames at 360 px, ms | 27.298334 | 26.033583 | -1.264751 / -4.633% |
| CPU layout, 200 frames at 900 px, ms | 26.678542 | 25.596833 | -1.081709 / -4.055% |

Both standard packages use the unchanged default FAT release profile and include
voice. The xtask internally uses `--no-default-features` to omit development data.
The final package has 207 regular files versus 206 before, including the additional
epaint patch notice. Deep/strict local ad-hoc signature verification passed.
Installed size sums all regular files; ZIP uses identical
`ditto -c -k --sequesterRsrc` over complete portable contents without an enclosing
directory. Workspace artifacts across worktree IDs were inspected and cleared;
the log confirms all twelve runtime workspace crates compiled from `73aa31e1`.

Both native executables use `cargo build --release --locked -p serein --features demo`,
default features and FAT LTO, without instrumentation. Both run `--demo --demo-chat`
at 1120×760 logical pixels, 2× display scale, Metal. A five-second warmup precedes
ten one-second process `ps` samples; settled RSS is the last-five median. All team
compilers, tests, replays and other apps were held during the pair. The +368 KiB
difference is small idle variation; no memory improvement is claimed. These
samples measure inactive English/common chat overhead, not Arabic rendering.
The source distinction above is retained; other-language RSS is unmeasured.

The CPU component harness uses the exact same five messages at 360 and 900 logical
pixels: long Arabic, mixed Arabic/Latin/digits/link/bold, Hebrew/Latin/digits,
Arabic marks with a newline, and forty repetitions of formatted Latin text.
Parsing and setup are outside timing. Each width has ten warmup frames and five
measured batches of 200 complete egui layout passes. The fixture body is
byte-identical, SHA256 `84a246bf25a840dcea103a64b54ddf56935691bbe2ef95a8a14fcb05f1c35846`.
The baseline receives only this manual test fixture; the changed source includes
it in the ignored test `markdown::tests::rtl_message_layout_benchmark`. Both
harnesses were built with `CARGO_PROFILE_RELEASE_LTO=thin CARGO_BUILD_JOBS=2 cargo
test --release --locked --offline -p ui --lib --no-run --message-format=json`,
then run directly with that exact test, `--ignored --nocapture --test-threads=1`.
This process-only thin-LTO override does not change repository profiles or the
default FAT shipping/native measurements. No compiler or native app was active
during the component pair. Small cached-workload timing differences do not establish
a general speed improvement and exclude cold layout, GPU frame latency and input.

The bounded RTL path admits 8 KiB / 512 spans / 128 rows per run, with 32 entries / 8 MiB
and 4 MiB per cache entry. Selection stages at most 4,096 current-pass shared
references / 4 MiB of actual referenced capacities plus a separate 4 MiB copy
buffer; all references drop after the pass. Oversized or indivisible content
shows an explicit localized preview limit. Stored message/draft/edit text remains
logical. No live account, upload, message, call, microphone or camera was used.


Current main integration `c36b585d8e4868b50194aec90ec202b59238f6f9` separately
passes the fresh full check (383 UI / 168 desktop tests plus strict lint and policy)
and standard package. Exact main `71ebbc1c0393a0ba4f4e6c93ae9b7b0bd3e06d35`
was built independently with the same default FAT standard command; both builds
freshly compiled all twelve runtime workspace crates after inspected cache pruning
and passed deep/strict local signature verification. The RTL renderer, selection,
font and manifest/lock files remain byte-identical to 73. The historic native and
component results above keep their source identities; no new native/component
measurement is implied by this package integration.

| Current standard package | Main 71 | Integrated c36 | Delta |
| --- | ---: | ---: | ---: |
| Executable, bytes | 62,269,488 | 62,368,128 | +98,640 / +0.1584% |
| Installed, bytes | 68,279,917 | 68,381,714 | +101,797 / +0.1491% |
| ZIP, bytes | 43,363,423 | 43,429,267 | +65,844 / +0.1518% |

The additional regular file is the epaint patch notice (207 versus 206). Existing
license texts are unchanged; PROVENANCE.md gains the intentional minimal-vendor
source/patch entry. Raw hashes, source proof and integration size records are
included separately in the linked measurement JSON.


### Current RTL and scalable-emoji integration (October 2, 2026)

Fresh integrated source `4a7e92ef7fab758a6550b14826629ef567d76e81` includes
main e74d's scalable-emoji SVG worker, dependencies and bundled licenses. The
comparison below uses exact main71 and measures this aggregate package, rather
than isolated RTL cost. Earlier source 73/c36 shipping sizes, source 5eb native
samples and source 73 CPU component measurements retain their original pins.

| Current aggregate shipping metric | Exact main71 | Source 4a7 | Delta |
| --- | ---: | ---: | ---: |
| Executable | 62,269,488 B | 67,212,176 B | +4,942,688 B / +7.9376% |
| Installed package | 68,279,917 B | 73,298,357 B | +5,018,440 B / +7.3498% |
| Distribution ZIP | 43,363,423 B | 47,633,110 B | +4,269,687 B / +9.8463% |

The standard voice-enabled package uses the repository default FAT-LTO profile;
xtask omits development data with its internal `--no-default-features`. All twelve
runtime workspace crates compiled fresh after inspected workspace-name release
cache invalidation across worktree IDs. Packaging finished in 11m42s and passed
deep/strict local ad-hoc signature verification. The 221 regular files versus
206 before include incoming SVG dependencies/licenses and the epaint patch
notice; existing license texts remain unchanged. Installed bytes sum regular
files and both archives use `ditto -c -k --sequesterRsrc` over complete contents.

Current source passes 15 RTL, seven selection and the actual high-DPI inline
emoji regression, plus the full check (385 UI / 170 desktop), formatting, strict
Clippy and policy. Arabic/Hebrew mixed messages keep the inline atlas and queue
no vector requests, while standalone jumbo emoji retains the incoming vector
path. All seven RTL/shaper/selection/font files and the complete `show_rtl`
method are byte-identical to historical source 73. No new native, GPU or CPU component
measurement is inferred from this integration; the historical measurements
remain explicitly source-pinned. Current hashes, inventory, build provenance
and source equivalence are recorded separately in
[`rtl-message-layout/measurements.json`](https://github.com/ViceVerse-cz/Serein/blob/7d495ed350e165bb7bfd3165b88ee6429e544bcf/docs/pr-evidence/rtl-message-layout/measurements.json).

## RTL message layout (PR #531): Windows integration evidence - October 8, 2026

Fresh standard Windows x64 voice-enabled packages compare main `1b3e4a7b` with `ea64b31e` (measured 2026-10-08). Baseline/current file counts: 216/217; the additional file is the epaint patch notice. Both Chinese layout-limit notices are included in the measured source.

| Metric | Main `1b3e4a7b` | Current integration | Delta |
| --- | ---: | ---: | ---: |
| Standard executable | 86,008,832 B | 86,181,376 B | +172,544 B (+0.2006%) |
| Installed directory | 90,199,948 B | 90,374,954 B | +175,006 B (+0.1940%) |
| Distribution ZIP | 50,001,798 B | 50,061,897 B | +60,099 B (+0.1202%) |

Method: `cargo xtask package`, Rust 1.98.1, standard release flags without demo; Windows 11 build 26200, Ryzen 7 7800X3D, 32 GiB RAM. Runtime workspace artifacts were invalidated before the feature build. Installed bytes sum every file in `dist`; ZIP uses whole-directory .NET Optimal compression. NSIS was unavailable, so no installer executable was built.

Current native CPU, memory, frame/startup latency and affected-device behavior remain unmeasured because the native automation bridge is unavailable. Package size and synthetic reducer timing do not establish live Discord performance.

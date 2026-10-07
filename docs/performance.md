# Performance findings

Recent synthetic/offline measurements are workload-specific. They do not establish live Discord
performance, universal device results or application-wide memory bounds. The raw PR screenshot,
log and per-run evidence archive has been removed; the summaries below retain the useful results.

## Watched-stream quality request — October 8, 2026

Windows 11 Home build 26200, Ryzen 7 7800X3D, 32 GiB RAM, 100% system scale.
The standard voice-enabled `cargo xtask package` compared `a76e030d` with
`26bc3c91`; no installer executable was made because `makensis` is absent.
The compressed size uses .NET `ZipFile.CreateFromDirectory` with optimal compression
on the same 216 installed files in each build.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable | 85,932,032 B | 85,934,080 B | +2,048 B (+0.0024%) |
| Installed package | 90,123,148 B | 90,125,196 B | +2,048 B (+0.0023%) |
| Compressed distribution | 49,976,600 B | 49,979,377 B | +2,777 B (+0.0056%) |
| Offline demo idle CPU, mean of one core | 2.4% | 1.5% | −0.9 percentage points; noisy |
| Offline demo peak private bytes | 166,727,680 B | 166,727,680 B | 0 B |
| Offline demo settled median private bytes | 166,727,680 B | 166,694,912 B | −32,768 B; noise |

The process sample used matched release `--no-default-features --features demo`
builds, `--demo --demo-voice`, a hidden native window, DX12 requested through
`WGPU_BACKEND`, a 15-second warmup and ten one-second samples; neither process
spawned a child. The synthetic fixture does not receive video or exercise the
new signaling. No decoded frame rate, stream bandwidth, live quality, GPU memory
or other operating systems were measured; the idle CPU difference is not an
improvement claim.

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

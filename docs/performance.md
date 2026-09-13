# Notification sound replacement — September 13, 2026

Baseline: `6d9e32222d1e3bd4d4edfd01f30854033788b11f` (synthesized mono cues).
After: embedded owner-supplied MP3 cues, decoded to stereo on the existing worker.

Windows x64, Ryzen 7 7800X3D (16 logical processors), approximately 32 GiB RAM,
Rust 1.98.1. No Cargo builds ran during the recorded timing samples.

| Cue preparation at 48 kHz | Baseline median | After median | Delta |
| --- | ---: | ---: | ---: |
| New message | 0.108364 ms | 0.329464 ms | +0.221100 ms |
| Current channel | 0.103316 ms | 0.264723 ms | +0.161407 ms |
| Incoming ring | 0.343465 ms | 3.877686 ms | +3.534221 ms |

Method: isolated copies of each revision's `samples` function, using the existing
release Symphonia/Opus dependencies for the new decoder; compiled with `rustc -O
-C lto=thin`. One warmup per cue, then five batches of 100 preparations with
`std::hint::black_box`, measured by `Instant`; table shows the median batch time
divided by 100. The incoming ring changes from 0.8 seconds mono to approximately
four seconds stereo, so this is a changed-workload comparison, not a decoder
speed comparison. These timings exclude device startup and playback and do not
measure UI latency, native process CPU/RSS, or live Discord behavior.

The encoded assets total 106,608 bytes. Source decoding and sample-rate conversion
run outside UI/audio callbacks; the callback copies prepared samples and tracks
the final device playback timestamp. Memory ceilings are documented in
[storage-policy.md](storage-policy.md).

# Empty-channel welcome — September 13, 2026

Baseline: `7eb23fa` with the same new offline empty-channel fixture injected for
the preview only. Both previews were built with
`cargo build --release --locked -p serein --features demo` and launched with
`--demo --demo-empty-channel`. Standard packages exclude that fixture.

Ubuntu 26.04.1 x64, Ryzen 5 7535U (12 logical CPUs), 14 GiB usable RAM,
Rust 1.98.1, eframe/wgpu, default dark palette, 1× scale, 1120×760.
The comparison used an isolated Xvfb 21.1.22 display with hardware presentation
unavailable, rather than the owner's interactive desktop. No builds ran during
sampling. The window was resized to 1120×760 after three seconds, then left
untouched for five more seconds before one ten-second sample (11 readings at
one-second intervals). Both windows were unfocused, with no caret animation.

| Process metric | Baseline | Welcome | Delta |
| --- | ---: | ---: | ---: |
| Idle CPU, one core = 100% | 0.0% | 0.0% | 0.0 percentage points |
| Settled RSS | 255,496 KiB | 241,488 KiB | −14,008 KiB (−5.48%) |
| Peak RSS through sample end | 255,496 KiB | 241,488 KiB | −14,008 KiB (−5.48%) |

CPU comes from `/proc/<pid>/stat` user/system tick deltas over the actual sample
duration; no CPU ticks were observed in either idle interval. Settled RSS is the
median of the last five `VmRSS` readings, and peak RSS is `VmHWM`. Neither process
had children; the shared Xvfb server is test infrastructure and is excluded.
Startup/close frame diagnostics showed nine callbacks and zero timeline reflows
for each run. This single pair is noisy and does not establish a memory
improvement or physical-GPU performance. Earlier interactive-desktop samples
were discarded after external input changed the scene. Startup latency and p95
frame latency remain unmeasured. Standard executable/installed/compressed package
sizes are recorded in the task PR, using the built packages.

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

## Title-strip dragging — September 13, 2026

Baseline: `7eb23fa`, built in a detached worktree. After: the title-strip press handling
and nonselectable caption text from `fix/titlebar-drag`, on that same baseline.
Windows x64, Ryzen 7 7800X3D, approximately 32 GiB RAM, Rust 1.98.1.
Both builds use `cargo xtask package`, including voice, without demo/developer-session features.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 70,288,896 | 70,289,920 | +1,024 (+0.0015%) |
| Installed package bytes | 76,691,365 | 76,692,825 | +1,460 (+0.0019%) |
| ZIP bytes | 44,629,357 | 44,629,786 | +429 (+0.0010%) |

One package per revision; installed size sums files, ZIP uses PowerShell `Compress-Archive`.
Both package file lists match. Sizes were captured before adding this measurement note;
no runtime source changes followed. The title strip now requests a native drag on the
initial primary-button press instead of waiting for a movement threshold. Synthetic input
tests verify command timing and caption-button isolation, not actual OS movement.
Native CPU, RSS, frame timing and drag latency are unmeasured: native computer-control APIs
are disabled in this session and the Orca CLI is absent. No runtime speed claim is made.

# Friends-home derived rows - September 14, 2026

Baseline: `b30b41ae24517ff1fdbd4efe288b9781281645e4`, the fetched main revision
at implementation start. After: that baseline plus `fix/friends-home-idle`.
The installed nightly `1.0.0-nightly.20260914.16` maps to release source
`ee8c246f5dbd40b31e80d00a2967ed931f05e787`; it does not contain the report ZIP's
friends-home cache. The installed client was not updated or used for these tests.
The ZIP was not applied wholesale: it also contained unrelated older source.

Friends Online/All now reuse a bounded filtered, sorted ID list. Relationship
changes invalidate it; Online additionally tracks online eligibility and gateway
connection state. Visible rows resolve current profiles and activities every
paint. Rail unread aggregation and folder row construction are reused on idle
wakes. The caret, Windows badge wake, VSync, DM lookup/order, 15-chat rail cap,
muted-guild visibility and existing action/confirmation paths are unchanged.
Cold Online filtering still scans the bounded presence list. No presence index,
protocol change, persistence migration or release optimization setting was added.

## Reproducible synthetic release workload

Windows 11 Home 10.0.26200 x64, Ryzen 7 7800X3D (16 logical processors),
33,410,678,784 bytes usable RAM (31.1 GiB), Rust 1.98.1. Both revisions use
the locked release profile, thin LTO, one codegen unit and default UI features.
`crates/ui/examples/friends_idle.rs` is identical on both revisions. It extends
the existing offline fixture to 4,000 friends, with the original 16 presence
records and seven Online rows, and runs `MessagingUi::show` in egui at 1120x760,
1x scale, default dark style. It asserts the exact Online count and no commands.
Five warmup frames precede 200 timed frames in each process; one process warmup
per revision precedes five alternating before/after pairs. No Cargo builds ran
during measurement. Build once with
`cargo build --release --locked -p ui --example friends_idle`, copy each executable
aside, then invoke those executables directly.

| Metric | Baseline median | After median | Delta |
| --- | ---: | ---: | ---: |
| 200 synthetic egui frames | 43.963 ms | 18.721 ms | -25.242 ms (-57.4%) |

Raw baseline runs: 44.786, 43.116, 43.197, 43.963, 45.158 ms.
Raw after runs: 18.311, 19.112, 20.175, 18.721, 18.605 ms.
This isolates repeated UI work, including egui output checks; it excludes native
event-loop timing, renderer/GPU presentation, tessellation, account startup and
process memory. It is not a native idle-CPU or p95-frame-latency measurement, nor
a benchmark of worst-case presence/navigation cardinality or live Discord.

## Reducer and standard package checks

On the same host, build the locked release `replay-bench` once per revision,
then invoke the two retained executables: one process warmup each, followed by
five alternating measured pairs with no concurrent Cargo builds. Each run applies
100,000 synthetic message events. Median baseline 44.7470 ms, after 42.9669 ms
(-1.7801 ms, -4.0%); ranges overlap, so this is not a reducer speedup claim.
Both retain 500 timeline records and 236,992..237,477 estimated timeline bytes,
not process RSS. Baseline runs: 41.7816, 44.7470, 46.2731, 45.2156, 43.4238 ms.
After runs: 45.0100, 42.9669, 42.9716, 41.7902, 42.0629 ms.

Standard packages use `cargo xtask package` (release, voice included, no demo or
developer-session features), with separate before/after `dist` directories.

| Package metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 70,444,544 | 70,472,704 | +28,160 (+0.0400%) |
| Full portable package bytes | 74,508,095 | 74,536,255 | +28,160 (+0.0378%) |
| ZIP bytes | 42,649,871 | 42,661,631 | +11,760 (+0.0276%) |

One package per revision, 186 matching file paths; full package size sums all
files, and ZIP uses PowerShell `Compress-Archive -CompressionLevel Optimal` on
each `dist` directory. `makensis` was absent, so installer size is unmeasured;
these are unsigned portable packages, not published or installed builds.

## Native sampling and limits

`scripts/frame-sample.ps1` accepts an exact prebuilt executable and launches only
`--demo --demo-friends --demo-frame-sample=8,15` (requires `--features demo`).
The fixture selects Friends Online and requests Search focus. Two bounded JSON
markers bracket the sample after warmup; callback wall-time buckets exclude
warmup and stop at the complete marker. They end at `FrameMetrics::finish`, before
tessellation/presentation. The script records binary hash, revision, profile,
actual elapsed time, CPU-time delta (one core = 100%), sampled peak and final
working-set/private bytes, focus/input counts, viewport size and scale. It rejects
disturbed/unfocused samples, changed marker geometry and delayed marker receipt.
It only closes its own spawned process. Run five matching pairs separately for
debug and release; never compare lifetime buckets to a shorter idle window.

Native measurements remain unavailable here. A debug baseline with identical
sample-only instrumentation built successfully, but a 3 s warmup / 3 s smoke run
timed out after 66 s without completing a sample. The native control pipe was
unavailable (`os error 2`) and the Orca CLI absent, so window focus/rendering could
not be verified. Demo mode has no live badge timer, and unfocused egui caret
rendering does not keep requesting frames; the sampler deliberately adds no
timer to disguise that distinction. The failed sample is discarded. Native
debug/release idle CPU, peak/settled process memory, p95 and first-paint latency
are unmeasured, and there is no claim of a production CPU improvement.

The supplied report's 82.744% to 43.028% CPU comparison is not reused: its baseline
was ten seconds at about 144 seconds uptime, versus eight seconds warmup plus
15 seconds afterward without verified navigation/focus. Its frame buckets also
covered different process lifetimes, including splash/READY. It cannot establish
an equivalent-workload speedup. READY apply and work after `FrameMetrics::finish`
remain outside this fix. Synthetic regression checks do not prove live service
compatibility; rollout still requires owner-controlled native verification.

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
later main integration is outside this comparison. The title strip requests a native drag on the
initial primary-button press instead of waiting for a movement threshold. Synthetic input
tests verify command timing and caption-button isolation, not actual OS movement.
Native CPU, RSS, frame timing and drag latency are unmeasured: native computer-control APIs
are disabled in this session and the Orca CLI is absent. No runtime speed claim is made.

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

## Channel shortcut restore - September 13, 2026

Baseline: `a90f0759ada23206809dc5374aef3e472875571a`. After: that revision plus
the shortcut restore fix on `fix/channel-shortcut-restore`. Windows x64,
Ryzen 7 7800X3D (16 logical processors), 31.1 GiB usable RAM, Rust 1.98.1.
Both use `cargo xtask package`, including voice, without demo/developer-session
features, built sequentially in the same worktree with baseline output copied aside.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 70,294,016 | 70,294,016 | 0 (0%) |
| Installed package bytes | 76,702,334 | 76,702,726 | +392 (+0.0005%) |
| ZIP bytes | 44,632,436 | 44,632,874 | +438 (+0.0010%) |

One package per revision; installed size sums files, ZIP uses PowerShell
`Compress-Archive`, and package file lists match. Sizes precede this measurement
note. The fix retains one pending restore flag until the existing bounded worker
has queue space, with no timer, worker, queue expansion or database migration.
The synthetic queue/SQLite check verifies recovery after all 16 slots are occupied;
it is not a timing benchmark. Native CPU, RSS and restore latency are unmeasured
because native computer-control APIs are disabled and the Orca CLI is absent.
No runtime speed or memory improvement is claimed.
## Video orientation and fullscreen controls - September 13, 2026

Baseline: `a90f0759ada23206809dc5374aef3e472875571a`. After: that revision plus
the video orientation, context-menu, fullscreen and seek-buffering changes on
`fix/video-player-controls`. Both packages were built sequentially in the same
detached worktree, with the baseline output copied aside before the second build.
Windows x64, Ryzen 7 7800X3D (16 logical processors), approximately 32 GiB RAM,
Rust 1.98.1. Both use `cargo xtask package`, including voice, without demo or
developer-session features.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 70,294,016 | 70,314,496 | +20,480 (+0.0291%) |
| Installed package bytes | 76,702,334 | 76,723,260 | +20,926 (+0.0273%) |
| ZIP bytes | 44,632,434 | 44,636,533 | +4,099 (+0.0092%) |

One package per revision; installed size sums all files, and ZIP size uses
PowerShell `Compress-Archive`. Package file lists match. Measurements precede
this performance note and the final playback-visibility documentation clarification.
Fullscreen reuses the existing decoder session and texture.
The offline UI check verifies stable seek range during loading and fullscreen
commands; the native Windows decoder check verifies upright rows and four track
rotations. Neither measures native UI performance.

Native CPU, RSS, frame timing and fullscreen transition latency are unmeasured:
native computer-control APIs are disabled in this session and the Orca CLI is
absent. No runtime speed or memory improvement is claimed.

## Cross-server emoji and information cards - September 14, 2026

Baseline: `5c45721989234737ef99bf13f71385feb91be8b8`. After: `7da50d5` on
`feat/cross-server-emoji`. Windows 11 Home 10.0.26200, Ryzen 7 7800X3D
(16 logical processors), 33,410,678,784 bytes usable RAM, Rust 1.98.1. Both
standard packages use `cargo xtask package`, including voice, without demo or
developer-session features. Separate worktrees retain separate `dist` outputs;
builds ran sequentially with the same Cargo release target.

| Package metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 70,443,008 | 70,501,376 | +58,368 (+0.0829%) |
| Full portable package bytes | 76,862,594 | 76,922,488 | +59,894 (+0.0779%) |
| ZIP bytes | 44,681,625 | 44,701,148 | +19,523 (+0.0437%) |

One package per revision, 227 files each; package size sums all files, and ZIP
uses PowerShell `Compress-Archive -CompressionLevel Optimal`. These sizes precede
this performance note and the native evidence images. `makensis` was unavailable,
so these are portable package measurements, not NSIS installer sizes.

Native comparison uses `cargo build --release --locked -p serein --features demo`
and explicit `--demo --demo-emoji` at 1120x760, 1x display scale, dark appearance.
The empty picker search is focused in the initial synthetic fixture. After five
seconds of warmup, PowerShell samples the demo process eleven times at one-second
intervals. CPU is the process CPU-time delta divided by actual elapsed time, with
one core equal to 100%; settled working set/private bytes use the median of the
last five readings, and peak working set is the OS lifetime process high-water
mark. No task build runs during sampling. The configured renderer is wgpu;
the actual adapter/backend is not logged. Available host GPUs are an RTX 5070 Ti
and AMD integrated graphics.

| Native process metric | Baseline | After |
| --- | ---: | ---: |
| Idle CPU, one core = 100% | 14.991% | Not measured |
| Settled working set bytes | 176,566,272 | Not measured |
| Settled private bytes | 398,360,576 | Not measured |
| Lifetime peak working set bytes | 197,861,376 | Not measured |

The baseline interval was 10.110 seconds, with no child processes. The changed
demo release also built successfully, but the user stopped Computer Use with
physical Escape before its screenshot or process sample. No further native
control was attempted. A paired CPU/memory comparison, startup latency, and p95
frame latency therefore remain unmeasured; no runtime improvement is claimed.

The baseline synthetic reducer replay used one warmup and five direct runs of
the release `replay-bench`: 48.4728, 48.1248, 44.2041, 44.3448, and 45.3094 ms
(median 45.3094 ms), retaining 236,992-237,477 estimated bytes / 500 records.
The changed replay was not run. This workload does not measure emoji interaction
latency, process RSS, or live Discord behavior.

## Large account READY startup - September 14, 2026

Baseline: `3cb739a4d679721d272f7182f82f82d6db138da1`. After:
`0661fe27afcb52b2ba691335503823eeec8629d1` on `fix/ready-large-accounts`.
Windows 11 Home 10.0.26200, Ryzen 7 7800X3D, 33,410,678,784 bytes RAM,
Rust 1.98.1 x86_64-pc-windows-msvc. Both standard release packages use
`cargo xtask package`, including voice, without demo or developer-session features.
Separate worktrees preserve separate `dist` outputs. Build target reuse was serialized;
stale affected workspace release artifacts were cleared before the successful changed build.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 70,501,376 | 70,543,872 | +42,496 (+0.0603%) |
| Full portable package bytes | 74,564,927 | 74,607,423 | +42,496 (+0.0570%) |
| ZIP bytes | 42,651,848 | 42,669,470 | +17,622 (+0.0413%) |
| Synthetic 100,000-event reducer replay, median ms | 45.0037 | 42.6369 | -2.3668 (-5.2591%) |
| Retained timeline estimated bytes / records | 236,992-237,477 / 500 | 236,992-237,477 / 500 | Unchanged |

One package per revision, matching 186-file lists; installed size sums all files.
ZIP uses `Compress-Archive -LiteralPath dist -CompressionLevel Optimal`.
`makensis` was unavailable, so these are unsigned portable packages, not NSIS installers.
Both package builds passed with the same nonfatal OpenH264 LNK4255 linker warning.
Sizes precede this performance-note-only commit.

Replay uses `cargo replay` to build, then the preserved release executable directly:
one warmup and five measured runs per revision, with no concurrent task build during
the measured runs. Baseline runs: 46.0115, 44.0077, 46.0564, 41.7041, 45.0037 ms.
After runs: 42.6369, 42.1586, 42.2748, 43.6456, 43.2793 ms. These small samples on a
shared workstation are noisy; the lower observed median is not a claimed runtime
improvement. This existing workload measures a synthetic message reducer, not large-account
startup time, process RSS, UI frame latency, or live Discord compatibility.

Separate offline regressions admit 70, 96, and 200 guilds with 100 channels each and
transfer a prepared 200-guild / 20,000-channel snapshot above 4 MiB through the actual
desktop FIFO into authenticated state. They verify permissions, subsequent event order,
optional-data warning behavior, and queue reservation release; they are correctness checks,
not startup benchmarks.

Native before/after screenshots, startup latency, peak/settled app memory, idle CPU and
p95 frame time remain unmeasured: the Computer Use native pipe returned OS error 2 and
the Orca CLI is not installed. The egui warning-render test is not native visual evidence.
No owner-account or live load test was performed. Account budgets are finite component
allocation estimates (128 MiB navigation/permission and 64 MiB permission sub-budget),
not whole-process memory guarantees; decoding and old/new state replacement add peak memory.

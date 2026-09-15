# Theme editor readability - September 15, 2026

Baseline: `235cf01`, reusing the verified `ae36f54` package because intervening
commits changed documentation only. After: `1f5349b`. Standard Windows x64
`cargo xtask package`, pinned Rust 1.98.1 MSVC, locked dependencies, voice included.
The baseline distribution was copied to its own directory before the serial
after build in the owned package worktree, reusing the same Cargo target.
The root `dist` was untouched. Both packages contain 186 files. `makensis` was
unavailable; the portable package passed with the nonfatal OpenH264 LNK4255 warning.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 71,029,760 | 71,039,488 | +9,728 (+0.014%) |
| Full portable package, bytes | 75,095,823 | 75,105,551 | +9,728 (+0.013%) |
| ZIP, PowerShell Compress-Archive Optimal, bytes | 42,850,464 | 42,856,609 | +6,145 (+0.014%) |

Each ZIP contains its package's `dist/*`; full size sums all files. Native UI
CPU, memory, and frame-time samples remain unavailable because OS window
capture/control is disabled and Orca is absent. No runtime performance gain is
claimed. Inspected synthetic debug framebuffer comparisons and their exact
fixture are documented in `docs/pr-evidence/theme-editor`; these do not establish
native OS interaction or live Discord compatibility.

# Compact theme gallery - September 15, 2026

Baseline: `cf4bcc2`. After: `ae36f54`. Both standard Windows x64 portable
packages include voice and use pinned Rust 1.98.1 MSVC with locked
`cargo xtask package`. Builds ran serially in the owned package worktree with
the same Cargo target; the baseline distribution was copied to a separate
directory before building the after revision. The root `dist` was untouched.
Both packages contain 186 files. `makensis` was unavailable; no installer was
built. The OpenH264 LNK4255 linker warning was nonfatal.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 70,965,248 | 71,029,760 | +64,512 (+0.091%) |
| Full portable package, bytes | 75,031,311 | 75,095,823 | +64,512 (+0.086%) |
| ZIP, PowerShell Compress-Archive Optimal, bytes | 42,835,573 | 42,850,464 | +14,891 (+0.035%) |

Each ZIP contains the corresponding `dist/*`; package size sums every file.
This is a size comparison, not a UI speed or memory result. Matched release
CPU, memory, and frame-time measurements remain unavailable because native
window capture/control is disabled and Orca is absent. The inspected synthetic
debug egui/WGPU renders under `docs/pr-evidence/theme-gallery` separately cover
layout; they are not native OS screenshots or live Discord evidence.

# Theme card covers and local editing - September 15, 2026

Baseline: `c36b5a2` on `feat/theme-maker`; intervening `e925b0b` changed only
this performance note. After: `b8ee526`. Both Windows x64 portable packages used
the pinned Rust 1.98.1 MSVC toolchain, locked `cargo xtask package`, and voice in
the release build. Builds used separate worktrees and Cargo targets; the root
`dist` was untouched. Both packages contain 186 files. The baseline package was
retained from the prior theme-maker measurement; the after package was built
for this change. `makensis` was unavailable, so no installer was produced.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 70,922,240 | 70,965,248 | +43,008 (+0.061%) |
| Full portable package, bytes | 74,988,303 | 75,031,311 | +43,008 (+0.057%) |

The worker bounds each selected cover to a 2 MiB static image and shrinks its
decoded card image to at most 640 x 360. Native UI CPU, memory, frame timing,
and before/after screenshots remain unmeasured because desktop window capture
is unavailable in this session. Package sizes and synthetic tests are separate
from installed-client visual or live Discord evidence.

# Theme maker and continuous image surfaces - September 15, 2026

Baseline: branch fork `aec1f19a10a045d3607de995f65723c7f749be66`.
After: `c36b5a2` on `feat/theme-maker`. Windows x64, pinned Rust 1.98.1
MSVC, locked release `cargo xtask package` with voice included. Each revision
used an isolated worktree and Cargo target directory; neither build touched
the existing `dist` or release executable. Both unsigned portable packages
contain 186 files. `makensis` was unavailable, so no installer was built.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 70,661,120 | 70,922,240 | +261,120 (+0.37%) |
| Full portable package, bytes | 74,727,183 | 74,988,303 | +261,120 (+0.35%) |
| ZIP, PowerShell Compress-Archive Optimal, bytes | 42,733,973 | 42,824,299 | +90,326 (+0.21%) |

ZIP each `dist/*` with `Compress-Archive -CompressionLevel Optimal`; measure
the executable and sum all files under `dist`. The size increase is measured,
but native demo CPU, memory, and frame timing were unavailable because desktop
window capture/control is unavailable in this session. Synthetic tests and
package sizes do not prove the installed live app's visual result.

# Thread participant loading — September 15, 2026

Baseline: `aec1f19a10a045d3607de995f65723c7f749be66`. After: that revision plus
`fix/thread-member-list`. macOS 27.0 (26A428), Apple M1 Pro, 16 GiB RAM,
pinned Rust 1.98.1 aarch64-apple-darwin. Both standard voice-enabled packages
use `cargo xtask package` (locked release, no default features). Builds ran
serially; separate copied package directories preserve the outputs.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 64,999,792 | 65,041,520 | +41,728 (+0.0642%) |
| Full installed package, bytes | 70,960,222 | 71,001,950 | +41,728 (+0.0588%) |
| ZIP (Deflate level 6), bytes | 42,812,510 | 42,821,470 | +8,960 (+0.0209%) |
| Synthetic reducer median, ms | 42.022708 | 41.725500 | -0.297208 (-0.71%) |

One package per revision. Installed size sums all file lengths under `dist`;
ZIP uses Python `zipfile.ZIP_DEFLATED`, compression level 6, on those same files.
These measurements precede this documentation-only note and screenshot delivery.

For each revision, `cargo replay` builds the workload; its preserved executable
then runs once to warm up and five times for measurement, with no concurrent task
build during sampling. Baseline samples (ms): 42.059334, 41.580417, 41.680334, 42.050667, 42.022708.
After samples (ms): 41.9055, 41.592916, 41.557208, 42.451125, 41.7255.
Both retain 500 records / 236,992–237,477 estimated timeline bytes. This generic
100,000-event reducer does not exercise the thread REST request or measure UI
latency, process RSS or live Discord behavior. Small shared-workstation samples
are noisy; no speed improvement is claimed.

The new read retains the existing 100-member / 128-KiB People budget, caps wire
input at 512 KiB, and uses one cancellable task with the existing REST permits
and bounded event queue. There is no per-frame network work or persistent cache.
Native screenshots use separate `--features demo` builds, explicitly launched
with `--demo`, selecting the same existing Introductions thread fixture. The
baseline shows unavailable; the changed fixture receives its synthetic rows.
No owner-controlled live compatibility, endpoint latency, native CPU/RSS or p95
frame measurement was run.

# Last-viewed server channel - September 14, 2026

Baseline: `ff3d711a91e0b3ae6de4c6aadbcce156264152fb`. After:
`1ae551548f1f0e66e8b27172edb1e279eecce1fa`. The baseline package and replay
were built from `7e7dcd14295dbd2626b7b6f71e9f639e28ca10aa`, whose Git tree
matches the baseline exactly (`ac90a66b3a8195fbdd27a4d777104e88ef160479`).
Separate worktree `dist` directories preserve both standard voice-enabled
release packages; neither uses an installed or authenticated client.

Windows 11 Home 10.0.26200 x64, Ryzen 7 7800X3D, 33,410,678,784 bytes usable
RAM, pinned Rust 1.98.1 MSVC. Both used `CARGO_BUILD_JOBS=2`, the same Cargo
target directory (serial builds), and `cargo xtask package` (locked release,
no default features, voice included). Both portable packages contain 186 files.
`makensis` was unavailable, so these are unsigned portable packages, not NSIS
installers. The existing OpenH264 LNK4255 warning was nonfatal on both builds.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 70,523,392 | 70,525,952 | +2,560 (+0.004%) |
| Full portable package, bytes | 74,586,943 | 74,589,503 | +2,560 (+0.003%) |
| ZIP, PowerShell Compress-Archive Optimal, bytes | 42,681,137 | 42,682,443 | +1,306 (+0.003%) |
| Synthetic 100,000-message reducer median, ms | 46.6756 | 41.6362 | -5.0394 (-10.8%; noisy) |
| Retained timeline, estimated bytes | 236,992..237,477 | 236,992..237,477 | unchanged |
| Retained message records | 500 | 500 | unchanged |

Build the workload once per revision with `cargo replay`, then invoke the
resulting `release/replay-bench.exe` directly: one warmup, five measured runs,
with no concurrent Cargo build during measurement. Baseline warmup: 43.3129 ms;
samples: 48.1569, 46.6756, 41.7873, 47.9139, 43.6477 ms. After warmup:
47.2663 ms; samples: 45.8912, 41.6362, 44.2464, 41.2978, 41.2562 ms.
This generic reducer does not exercise server clicks; the timing difference is
not evidence of a navigation speedup. ZIP each package with
`Compress-Archive -Path dist/* -DestinationPath <separate-output.zip> -CompressionLevel Optimal`;
measure executable length and sum all files under `dist`.

Server clicks now select through the existing history/resident-window path.
Remembered server/channel IDs add at most 16 KiB vector payload and a fixed
header; visits scan at most 1,024 entries. Cold/invalid remembered selections
scan existing bounded navigation to choose an accessible fallback. There is no
per-frame work, timer, persistence or new network endpoint for this memory.
Focused reducer and synthetic egui pointer tests cover restoration, repeated
click no-op, revoked/deleted fallback, voice preview, logout and memory bounds.
`cargo xtask check` passed. Native screenshots and interaction CPU/memory/p95
were unavailable: Orca CLI is absent and the Windows Computer Use native pipe
fails with OS error 2. These tests are not native visual or live Discord proof.

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


## 2026-09-15: gallery preview, customization and selection

| Metric / method | Baseline `fd0cf4e` | After `e452b0f` | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 71,039,488 | 71,050,240 | +10,752 (+0.015%) |
| Full portable package, bytes | 75,105,551 | 75,116,303 | +10,752 (+0.014%) |
| ZIP, Compress-Archive Optimal, bytes | 42,856,609 | 42,860,670 | +4,061 (+0.009%) |
| Native release UI CPU, memory, frame time | Unmeasured | Unmeasured | Unmeasured |

Standard voice-enabled `cargo xtask package` passed on Windows x64 with pinned Rust
1.98.1 MSVC and locked dependencies. One package per revision, 186 files each; package
size sums all files. Baseline reuses the verified `1f5349b` package, since intervening
commits through `fd0cf4e` contain documentation only. It was preserved separately
before the after builds in the owned package worktree; root `dist` was untouched.
ZIP uses Compress-Archive Optimal on each `dist` directory. These measurements cover
full-app gallery preview, bundled customization and theme selection together.
The final release build took 3m 16s. OpenH264 LNK4255 was nonfatal. `makensis` is absent,
so packaging produced an unsigned portable distribution, not an NSIS installer.

No UI speed or memory improvement is claimed. Matched native release CPU, memory and
frame-time measurements remain unavailable because native desktop capture/control is
disabled and Orca is absent. The inspected offline debug framebuffer renders and
behavioral tests do not establish installed-client visuals or live interoperability.


### Back button outline follow-up

`96a3a05` (verified `e452b0f` code/package) versus `310ad5e`, same Windows
voice-enabled release command, toolchain, package worktree and ZIP method above.
The baseline distribution was preserved separately before rebuilding. Both packages
contain 186 files, a 71,050,240-byte executable and 75,116,303 total bytes (no change).
The ZIP changed from 42,860,670 to 42,860,657 bytes (-13 bytes, below 0.001%). This
compression difference is not a performance improvement. Packaging passed in 3m 15s;
NSIS remains unavailable. Native UI timing/memory limitations above still apply.

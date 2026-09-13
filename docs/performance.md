# Initial performance evidence

## Windows camera selection and DirectShow fallback — September 13, 2026

Baseline `67d82443f1653975f8a0f9132c31c8139f4be052` has identical runtime sources
and dependency manifests to the verified `438278d` package retained from the
keybinds task (`git diff 438278d 67d8244 -- apps crates Cargo.toml Cargo.lock` was
empty before edits). Reused that verified package; no old unverified executable
was treated as a baseline. Windows 11 Home 10.0.26200, Ryzen 7 7800X3D, Rust
1.98.1 MSVC. Standard release packages include voice, exclude demo/developer
features, and are unsigned. Separate full package copies and one PowerShell
`Compress-Archive` per revision; measurements precede this report.

| Metric (bytes) | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 68,877,824 | 68,986,368 | +108,544 (+0.158%) |
| Full package | 142,036,909 | 142,148,872 | +111,963 (+0.079%) |
| Compressed package | 82,130,844 | 82,171,135 | +40,291 (+0.049%) |

The explicit read-only Windows enumeration check found three registered virtual
cameras without activating any source. This is device-discovery evidence, not a
capture benchmark. Synthetic tests cover selector interaction, disabled/demo
discovery guards, RGB format/buffer bounds and aspect-preserving conversion.
Native screenshots and UI interaction sampling remain unavailable because the
Orca CLI is absent and native computer APIs are disabled. Actual capture, live
Discord delivery, idle/active CPU and memory, display/renderer metrics and frame
timing were not measured. No capture-performance improvement is claimed.

## Windows automatic startup — September 12, 2026

Baseline `073f0d23adb85c37b9a3779bd30da05ff05d3480`; Windows 11 Home
10.0.26200, Ryzen 7 7800X3D (16 logical processors), 33,410,678,784 bytes RAM,
Rust 1.98.1 MSVC. Both standard `cargo xtask package` builds include voice and
exclude demo/developer features. Separate clean `dist` directories; one
PowerShell `Compress-Archive -CompressionLevel Optimal` per revision. Measurements
precede this report's addition to the bundled documentation. The existing Windows
dependency gains its registry API feature; no dependency is added.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 62,928,384 | 62,967,296 | +38,912 (+0.062%) |
| Installed package file bytes | 69,153,081 | 69,193,907 | +40,826 (+0.059%) |
| ZIP bytes | 41,627,780 | 41,645,300 | +17,520 (+0.042%) |

One separate release `--features demo` launch per revision, `--demo
--demo-settings=appearance` before and `--demo --demo-settings=general` after,
five-second warmup, then 50 process samples at requested 200 ms intervals
(10.921 / 10.524 seconds elapsed). No input during sampling; same viewport/theme
as the screenshots. Windows process counters, not GPU memory or frame timing:

| Idle settings metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Sampled peak / settled working-set bytes | 171,323,392 | 168,386,560 | -2,936,832 |
| Sampled peak / settled private bytes | 398,921,728 | 397,230,080 | -1,691,648 |
| Process CPU seconds over sample | 0.015625 | 0 | -0.015625 |

This is a single noisy sample with different settings-page contents; baseline
sampling overlapped compilation. CPU is near counter resolution, so these numbers
do not establish an improvement. Startup peaks, GPU/helper-process memory, p95
frame times and native interaction latency are not measured.

Startup registration has no polling loop: one bounded off-thread load, then an
operation only after an explicit setting change. Registry round-trip tests use
random synthetic keys outside Windows startup locations. No sign-out/sign-in or
saved-account launch was performed; startup p95 and live compatibility are unmeasured.

Native framebuffer evidence uses the existing `profile_preview --demo` example,
extended only with settings-page selection and a one-second wait for modal fade-in.
Before is Appearance (startup absent), after is General, using the same 1120×760
logical viewport, Windows 125% scale, default dark theme and wgpu renderer.
The Computer Use native pipe is unavailable, so native keyboard/pointer automation
and screen-reader validation remain unverified; synthetic UI pointer tests cover
both dark/light at 760 and 1120 logical pixels. The native tray test separately
verifies late registration after minimizing, restore, restart, Quit and cleanup.

## Invite input alignment — September 12, 2026

Baseline `a2ad207ca9225ef8973ab3e865ad14a1d5083e0e`, Windows 11 Home
10.0.26200, Rust 1.98.1 MSVC. One standard locked `cargo xtask package` build
and PowerShell `Compress-Archive` per revision; voice included, no demo/developer
features. Separate `target/invite-input-{baseline,after}` snapshots exclude the
unrelated obsolete `dist/voice` directory, which was preserved. Measurements precede
this report's addition to bundled documentation.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 62,853,632 | 62,854,656 | +1,024 |
| Installed package file bytes | 73,053,443 | 73,054,467 | +1,024 |
| ZIP bytes | 43,464,705 | 43,465,087 | +382 |

Only text-field presentation changes; no dependency, protocol, storage or input-limit
change. Synthetic egui tests measure centering, padding, height and focus outline in
dark/light themes at 240/490-point widths with empty and typed input. Native capture
and interaction remain unavailable (no native app surface), so screenshots, CPU/RSS
and frame timings are unmeasured. No runtime performance improvement is claimed.

## Standalone Join Server dialog — September 12, 2026

Baseline `9bbdd07b2a545b554ee164fd862cda79b86608f7` versus the standalone join
dialog, Windows 11 Home 10.0.26200, Rust 1.98.1 MSVC, locked release profile.
Both `cargo xtask package` builds include voice without demo/developer features.
One build and PowerShell `Compress-Archive` per revision; installed size sums file
lengths. Separate snapshots under `target/join-server-{baseline,after}-install`
contain the standard generated package, excluding an unrelated obsolete `dist/voice`
sub-package (preserved in place). These measurements precede this report's addition
to bundled documentation.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 62,809,600 | 62,850,048 | +40,448 (+0.0644%) |
| Installed package bytes | 73,002,844 | 73,043,960 | +41,116 (+0.0563%) |
| ZIP bytes | 43,448,863 | 43,459,289 | +10,426 (+0.0240%) |

No new dependencies or persistent storage. One 512-character/2048-byte dialog draft
reuses the bounded invite cache and single in-flight lookup/write guards. Opening or
repainting never sends a join; checking and confirming are separate explicit actions.
Synthetic egui click tests cover wide/dark and narrow/light layouts, preview failure,
retry, duplicate suppression and session reset. Native capture/control is unavailable
in this session (no native apps surface), so screenshots, CPU/RSS and frame timings
remain unmeasured. No live Discord join was performed; no native performance claim.

## Profile and status menu — September 12, 2026

Baseline `96d94285531f7ede3780187c858b786dd189bb2d` versus the profile/status menu
implementation. Windows 11 Home 10.0.26200, Rust 1.98.1 x86_64-pc-windows-msvc,
locked release profile. Both `cargo xtask package` builds include voice and omit
demo/developer features. Before/after packages are retained in separate directories.
Package measurements precede this report's addition to the bundled documentation.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Packaged executable bytes | 62,018,048 | 62,072,320 | +54,272 (+0.0875%) |
| Installed package logical bytes | 66,503,839 | 66,559,935 | +56,096 (+0.0843%) |
| ZIP bytes, PowerShell Compress-Archive | 39,643,012 | 39,666,084 | +23,072 (+0.0582%) |

One build/archive per revision; totals sum regular file lengths. No dependency or
storage schema change. One bounded 128-character/512-byte draft and replaceable
presence value reuse the existing five-second Gateway publication cadence.

Native CPU, process memory, frame/startup timings and visual inspection remain
unmeasured: Computer Use failed with `Computer Use native pipe is unavailable ...
(os error 2)`, and Orca CLI is not installed. Headless egui tests verify synthetic
interaction/layout only; they do not establish native visual or performance results.
No live Discord, call or microphone test was run. Build/archive logs and packages
are local under `E:/codex-builds/rustcord-profile-status-menu/target` and `dist`.

## Signing and lint repair — September 12, 2026

Baseline `ff5c125` versus runtime commit `f3f8869`, macOS 27.0 (26A428), Apple
M1 Pro, 16 GiB RAM, Rust 1.98.1, locked release profile. Both standard
`cargo xtask package` builds include voice, without demo/developer features.
Separate worktrees and package directories share a sequential Cargo target cache.
Package measurements precede this report; neither package is Apple-notarized.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Packaged executable bytes | 57,812,096 | 57,812,096 | 0 |
| Installed package logical bytes | 62,377,684 | 62,378,917 | +1,233 (+0.0020%) |
| Package ZIP bytes | 38,524,982 | 38,524,075 | -907 (-0.0024%) |
| `size_of::<server_actions::Event>()`, bytes | 528 | 64 | -464 (-87.88%) |
| `size_of::<client_core::Event>()`, bytes | 544 | 352 | -192 (-35.29%) |
| Synthetic reducer median, ms | 40.9930 | 41.1411 | +0.1481 (+0.36%) |

The successful invite payload moves into one `Box`; errors remain inline and
queue admission still counts the full channel/message allocation. Type sizes are
for this target/toolchain, not total process memory. Formatting, equivalent boolean
simplification, and demo-only helper gating preserve application behavior.

Reducer: one warmup plus five direct `replay-bench` runs per revision, with no
concurrent compilation. Baseline milliseconds: 40.844708, 40.727459, 41.006500,
41.145250, 40.993000. After: 41.141125, 40.425292, 40.365000, 41.208666,
41.195000. Both retain 500 records / 236,992..237,477 estimated timeline bytes
after 100,000 synthetic message events. This workload does not send invites;
the small timing difference is within sample variation, not an invite-performance claim.

Installed size sums regular file lengths; ZIP uses the release workflow's
`ditto -c -k --sequesterRsrc` command. Documentation and archive metadata affect
these small package deltas. No UI, RSS, frame-time, live-account, microphone or
Apple signing/notarization performance claim is made.

## Server menu layout revision - September 11, 2026

Baseline `3002847` versus implementation `1417dad`, Windows 11 Home, AMD Ryzen
7 7800X3D (16 logical processors), 31.1 GiB visible RAM, Rust 1.98.1. Both built
with `cargo build --release --locked -p serein --features demo` (voice included),
with separate copies of the resulting executables retained for sampling.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release demo executable, bytes | 61,899,776 | 61,935,104 | +35,328 (+0.0571%) |
| Demo peak/last private bytes | 395,509,760 | 395,636,736 | +126,976 (+0.0321%) |
| Process CPU seconds over ten samples | 0 | 0 | Below counter resolution |

Each executable was launched with `--demo`, with an eight-second warmup followed
by ten one-second samples of `Process.PrivateMemorySize64` and
`TotalProcessorTime`. Actual sampling durations were 10.0945s and 10.0783s;
private bytes stayed flat within each run. No compilation ran during sampling.
The configured renderer is WGPU and initial viewport is 1120x760; actual display
scale and renderer adapter were not inspected. This is a single default-demo
idle comparison, not a menu interaction, frame latency, or startup benchmark.
The small memory difference is not evidence of a meaningful regression.

Standard package/ZIP sizes are unavailable: baseline `cargo xtask package` fails
at `apps/desktop/src/main.rs:468`, where `demo_members` is referenced without the
`demo` feature that defines it. Demo executable size is not shipping package
size. Native menu interaction/capture is blocked by the unavailable Computer Use
native pipe (Windows error 2); no native visual or live Discord claim is made.

## Server dropdown - September 11, 2026

Baseline `88d0c11` versus runtime commit `768d357`, Windows 11 Home, AMD Ryzen
7 7800X3D (16 logical processors), 31.1 GiB visible RAM, Rust 1.98.1. Both isolated
worktrees passed `cargo xtask package`, including standard voice support. Packages
below are measured at those commits, before this measurement report was added.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 61,799,424 | 61,864,448 | +65,024 (+0.1052%) |
| Installed package, bytes | 66,264,678 | 66,331,261 | +66,583 (+0.1005%) |
| PowerShell ZIP, bytes | 39,556,327 | 39,577,975 | +21,648 (+0.0547%) |
| Synthetic reducer median, ms | 41.1795 | 39.3013 | -1.8782 (-4.56%) |
| Demo peak/last private bytes | 396,316,672 | 395,911,168 | -405,504 (-0.10%) |
| Demo CPU seconds over 10 samples | 0.015625 | 0.265625 | +0.250000 |

Reducer: locked release `replay-bench`, one warmup and five direct executable runs
per revision, 100,000 synthetic events each. Both retain 500 records and
236,992..237,477 estimated timeline bytes. The workload does not exercise invite
or leave requests; the timing difference is not a server-action speedup. Separate
package outputs were retained. Shared-target replay reuse was detected and the
changed core/benchmark were rebuilt before collecting the reported changed runs;
the dependency file includes the new server-actions module.

Process sampling: launch each packaged executable with `--demo`, eight-second
warmup, ten samples at one-second intervals, `Process.PrivateMemorySize64` and
`TotalProcessorTime`. No compilation ran during these samples. Each retained
sample was flat in private bytes. The changed process's first sampling attempt
ended early and was excluded; a repeat survived all ten samples with no stderr.
The cause of that first exit was not established. CPU represents about 0.0098%
versus 0.1660% of the 16-logical-processor machine over the nominal ten seconds;
this short, single-run observation is noisy and not evidence of a regression or
improvement. wgpu is configured; actual backend, viewport, display scale,
occlusion, GPU/child memory and rendered state were not verified because the
native Computer Use pipe was unavailable (Windows error 2). Native dropdown
interaction timing and p95 frame/startup latency remain unmeasured.

The feature adds no dependency, polling or persistent storage. One pending write,
one bounded invite result and one dialog are retained. Native screenshots and
live Discord invite/leave interoperability remain unverified. Workspace tests,
strict all-target/all-feature Clippy and policy checks passed. The full check
stops at existing formatting differences, reproduced on the baseline. CI also
rejects existing MPL license coverage for symphonia-common, symphonia-format-ogg
and symphonia-codec-vorbis; this task changes no dependency or license policy.

## Embed image gallery — September 11, 2026

These measurements predate `b2cbb5d`, which made voice standard and removed license
coverage checks from packaging. The historical text/voice split and packaging
failures below do not describe the current standard package.

Baseline `4bbec5a` versus final runtime sources on `fix/embed-image-gallery`, Rust
1.98.1, locked release builds. Baseline and changed packages were copied to separate
evidence directories immediately after building. Text packages pass. The initial
gallery voice build and baseline compile, but packaging fails on missing
exact-version license texts for
`openh264-sys2 0.9.8` and `openh264 0.9.8`, also reproduced on the clean baseline.
Complete voice installed/archive sizes are therefore unavailable. The final voice
rebuild was interrupted when delivery switched to fast local
mode; final-source voice release verification remains incomplete. No dependencies
changed. Installed/ZIP totals include docs and licenses, exclude the sibling voice
directory, and predate this final measurement addendum. ZIP uses Python `zipfile`,
DEFLATE level 6, sorted relative file paths.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 55,205,376 | 55,207,424 | +2,048 (+0.004%) |
| Text installed package, bytes | 60,176,513 | 60,181,544 | +5,031 (+0.008%) |
| Text ZIP, bytes | 36,100,777 | 36,102,099 | +1,322 (+0.004%) |
| Maximum sampled working set, bytes | 156,225,536 | 166,957,056 | +10,731,520 (+6.87%) |
| Settled working set, bytes | 156,225,536 | 166,957,056 | +10,731,520 (+6.87%) |
| Settled private memory, bytes | 386,510,848 | 394,907,648 | +8,396,800 (+2.17%) |
| Sampled CPU, percent of one core | 0.305 | 0.000 | -0.305 percentage points |

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), 33,410,678,784 bytes
usable RAM; configured wgpu renderer and default 1120 x 760 logical viewport. Actual
adapter/backend, display scale, GPU allocations, helper-process memory and frame
timing were not measured. One fresh text-only `--demo` process per reported build,
five-second warmup, ten one-second PowerShell samples of WorkingSet64 and
PrivateMemorySize64; settled is the median of the last three samples. CPU uses
TotalProcessorTime delta over measured wall time (10.261/10.181 seconds). Each
process stayed alive throughout sampling and was then stopped. The original fixture
has one embed image; the changed fixture has three. Background desktop/build activity
was uncontrolled. These short idle samples are noisy, show increased sampled memory,
and are not an improvement claim or a substitute for scrolling/gallery interaction
measurements. Zero sampled CPU does not imply zero CPU cost.

Native screenshot/scripted interaction evidence is blocked by the absent Computer Use
native pipe (`os error 2`) after retry. No p95 latency or live Discord claim is made.

## Own profile editor - September 11, 2026

Baseline `22e2283` and final runtime sources on `feat/profile-edit`, built from
separate source worktrees with the locked Rust 1.98.1 toolchain. Both text release
packages pass; both voice executables compile, but voice packaging fails on the
same missing exact-version `openh264-sys2 0.9.8` and `openh264 0.9.8` license texts.
Complete voice installed/archive sizes therefore remain unavailable. No dependency
versions changed. Text totals include staged documentation/licenses, exclude the
sibling voice directory and archives, and predate this final measurement addendum.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 55,044,608 | 55,143,936 | +99,328 (+0.18%) |
| Text installed package, bytes | 59,967,663 | 60,074,259 | +106,596 (+0.18%) |
| Text ZIP level 6, bytes | 36,017,303 | 36,059,832 | +42,529 (+0.12%) |
| Voice executable, bytes | 61,060,608 | 61,163,520 | +102,912 (+0.17%) |
| Maximum sampled working set, bytes | 166,354,944 | 167,079,936 | +724,992 (+0.44%) |
| Settled working set, bytes | 166,354,944 | 167,079,936 | +724,992 (+0.44%) |
| Settled private memory, bytes | 394,178,560 | 397,459,456 | +3,280,896 (+0.83%) |
| Idle CPU, percent of one core | 0.0000 | 0.0000 | +0.0000 |
| Reducer median, ms | 40.9315 | 40.2442 | -0.6873 (-1.68%) |

Windows 11 Home, Ryzen 7 7800X3D (16 logical CPUs), 33,410,678,784 bytes usable RAM,
wgpu renderer; actual adapter/backend and GPU memory were not measured. Both text
builds ran as fresh native `--demo` processes, at the default 1120 x 760 logical
viewport and 125% display scale: baseline `--demo-settings=account` versus changed
`--demo-settings=profile`. After ten seconds warmup, five one-second PowerShell
process samples measured WorkingSet64, PrivateMemorySize64 and TotalProcessorTime.
Settled memory is the median of the last three samples; peak means maximum sampled,
not lifetime peak. Idle CPU is the process-time delta divided by sampled wall time,
as percent of one logical core. Neither short idle sample registered a CPU increment.
This does not imply zero CPU cost. There were no scripted edits or helper-process
measurements; background desktop activity was uncontrolled.

Reducer: separate release targets for each revision, `cargo replay` warmup followed
by five direct executable runs, 100,000 synthetic events, 500 records and
236,992..237,477 estimated retained timeline bytes on both. The small timing and
memory differences are noisy snapshots, not an improvement claim. Text archives use
Python zipfile, sorted relative names and DEFLATE level 6. Frame/startup percentiles,
editing latency, GPU allocation, real-account compatibility and voice runtime cost
remain unmeasured. [Raw samples and package counts](pr-evidence/profile-edit/metrics.json)
and [native screenshot procedure](pr-evidence/profile-edit/README.md) accompany the PR.

## September 11 — outgoing screen sharing

Same host: macOS 27.0 (26A428), Apple M1 Pro, 16 GiB RAM, wgpu Metal, built-in 3024×1964 Retina display, default reading scale. Baseline is the verified `609f8bf` release package; changed packages come from `cargo xtask package` and `cargo xtask package-voice` at feature commit `fb3736d`, before merging unrelated main UI changes from `afac0ee`. Full package means the app bundle and the four license/readme/notice files staged by xtask; stale pre-existing ZIPs and the sibling voice directory are excluded. Gzip level 6 archives use the same Python tarfile procedure on both outputs.

| Metric / method | Baseline `609f8bf` | Screen-sharing branch | Delta |
|---|---:|---:|---:|
| Text executable, bytes | 48,494,496 | 48,532,208 | +37,712 (+0.08%) |
| Text installed package, bytes | 49,645,543 | 49,696,554 | +51,011 (+0.10%) |
| Text gzip distribution, bytes | 31,319,339 | 31,345,030 | +25,691 (+0.08%) |
| Voice executable, bytes | 52,318,592 | 53,702,000 | +1,383,408 (+2.64%) |
| Voice installed package, bytes | 53,784,318 | 55,292,726 | +1,508,408 (+2.80%) |
| Voice gzip distribution, bytes | 33,159,474 | 33,672,043 | +512,569 (+1.55%) |
| Reducer median, ms | 37.742 | 37.772 | +0.030 (+0.08%) |
| Voice preview mean CPU, % | 0.18 | 0.24 | +0.06 |
| Voice preview settled RSS, KiB | 143456 | 141152 | -2304 |
| Voice preview maximum sampled RSS, KiB | 159584 | 141152 | -18432 |

Idle measurements use separate `--demo --demo-voice` processes at the same 1120×760 viewport, 30-second launch warmup, then ten one-second `ps` CPU/RSS samples without a competing compiler. Settled RSS is the median of the final three samples; maximum RSS is only the maximum of those ten samples, not a process lifetime peak. The preview performs no capture, encoding or network activity. No screen-capture helper is started in this workload. Other desktop activity is uncontrolled, and the small CPU/RSS/reducer changes are noise, not an improvement claim.

Reducer comparison: release `cargo replay`, then one warmup and five direct `target/release/replay-bench` runs per revision; 100,000 synthetic events, 500 retained records and 236,992–237,477 estimated retained bytes on both. Package counts include the documentation snapshot staged during measurement; later evidence-only prose edits may change a subsequent package by a few KiB. Raw samples and byte counts are in [metrics.json](pr-evidence/screen-sharing/metrics.json). UI frame-time percentiles, encoder throughput, actual sharing CPU/RAM, Windows runtime costs and live network quality remain unmeasured. Output presets cap at 1080p60 and target 4–16 Mbps; they are not achieved-throughput claims.


## Game IPC Rich Presence - September 11, 2026

Baseline `9d4b222` and final runtime sources on `fix/game-presence-ipc`; isolated source
worktrees and separate `dist` package directories. Both `cargo xtask package` (text-only)
and `cargo xtask package-voice` passed with the locked Rust 1.98.1 toolchain, release/thin LTO,
one codegen unit. No new crate versions; platform now uses the existing Tokio dependency
and Windows security features instead of process enumeration. Packages remain unsigned.
The existing upstream realfft license-evidence warning is unchanged.

| Metric / method | Baseline | After | Delta |
| --- | --- | --- | --- |
| Text executable (bytes) | 54,588,416 | 54,823,424 | +235,008 (+0.43%) |
| Text installed package (bytes) | 59,454,084 | 59,694,309 | +240,225 (+0.40%) |
| Text ZIP level 9 (bytes) | 35,184,437 | 35,272,127 | +87,690 (+0.25%) |
| Voice executable (bytes) | 59,689,984 | 59,923,456 | +233,472 (+0.39%) |
| Voice installed package (bytes) | 65,875,376 | 66,114,065 | +238,689 (+0.36%) |
| Voice ZIP level 9 (bytes) | 37,800,226 | 37,887,362 | +87,136 (+0.23%) |
| Demo median working set (bytes) | 167,661,568 | 166,600,704 | -1,060,864 (-0.63%) |
| Demo median private bytes (bytes) | 396,267,520 | 396,058,624 | -208,896 (-0.05%) |
| Demo idle CPU, one logical core (percent) | 0.172 | 0 | -0.172 (-100.00%) |
| 100k-event reducer replay median (ms) | 42.885 | 42.087 | -0.799 (-1.86%) |

Environment: Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs),
33,410,678,784 bytes usable RAM; wgpu renderer, actual adapter/GPU allocation unmeasured.
Native fixture: `--demo --demo-settings=activity --demo-game-activity`, default 1120x760
logical viewport, captured at 1400x950 pixels / 125% display scale. Each final build had a
fresh process, ten-second warmup, then ten `Get-Process` WorkingSet64/PrivateMemorySize64/
TotalProcessorTime samples at one-second intervals. CPU is elapsed process time divided
by elapsed wall time, expressed as a percentage of one logical core. No helper children
were observed. Screenshots capture the actual application with Win32 PrintWindow because
the supplied native computer-use pipe was unavailable. Images were inspected; no clipping
in the changed card. Headless egui tests cover dark/light and 760/1120 logical widths.
Native keyboard/accessibility and narrow/light screenshots were not exercised.

Package totals include all installed docs/licenses (670 text / 912 voice files) at package
creation, before this final measurement addendum; PR evidence is excluded by packaging.
ZIPs use Python zipfile, DEFLATE level 9 and sorted relative file paths. Executable sizes
are exact; do not treat small noisy memory/CPU/reducer differences as an improvement.

For each revision, `cargo replay` built the release benchmark, then the produced executable
ran once for warmup and five times for the reported median. Both retained 500 records and
236,992..237,477 estimated timeline bytes. The reducer fixture does not exercise game IPC,
process scanning or live Discord. Demo mode also never binds IPC. These samples establish
package cost and unchanged synthetic workload bounds, not live IPC/Gateway latency, p95
startup/frame time, microphone/voice performance or universal game compatibility.

[Raw samples](pr-evidence/game-presence-ipc/measurements.json) accompany the screenshot pair.
Runtime IPC limits: eight clients, 16 KiB frames, 16 pending typed updates, five-second
partial-frame/write deadlines and existing five-second Gateway update spacing. Native Windows
tests prove roundtrip, contention and release with blocked writers; Unix implementation
is unverified on this host. Full verification remains blocked by the inherited formatting/
Clippy findings and baseline-reproduced UI test crash described in `docs/progress.md`.

## Emoji size and loading flicker - September 11, 2026

Baseline `bd7d26ae0a4332a78617e2a6a6d8402b0a81cfd2`, compared with
`fix/emoji-size-flicker`. Both locked text-only and optional-voice release packages
built successfully with Rust 1.98.1, thin LTO, one codegen unit, wgpu, on Windows
11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), 33,410,678,784 bytes RAM.
Separate before/after package copies prevent baseline overwrite. Installed/ZIP
figures include bundled docs/licenses at measurement time, before these final
evidence updates; text packages exclude `dist/voice`. ZIPs use PowerShell
`Compress-Archive` default Optimal compression. No dependency/assets were added.

| Metric | Baseline | After | Delta |
| --- | --- | --- | --- |
| Text executable, bytes | 54,099,456 | 54,103,552 | +4,096 (+0.0076%) |
| Text installed package, bytes | 55,242,006 | 55,247,680 | +5,674 (+0.0103%) |
| Text ZIP, bytes | 33,403,603 | 33,406,136 | +2,533 (+0.0076%) |
| Voice executable, bytes | 59,205,632 | 59,209,728 | +4,096 (+0.0069%) |
| Voice installed package, bytes | 60,651,489 | 60,657,163 | +5,674 (+0.0094%) |
| Voice ZIP, bytes | 35,594,103 | 35,596,216 | +2,113 (+0.0059%) |
| Text working set, sampled peak/final, bytes | 161,533,952 | 166,764,544 | +5,230,592 (+3.24%) |
| Text private bytes, sampled peak/final | 393,166,848 | 396,890,112 | +3,723,264 (+0.95%) |
| Text CPU seconds during 10 s observation | 0 | 0.15625 | +0.15625 s |

Process method: launch each copied text executable with `--demo --demo-chat`
and `Start-Process -WindowStyle Hidden`; warm up 8 seconds, then sample
`Get-Process` WorkingSet64/PrivateMemorySize64/CPU ten times at one-second intervals.
One process run per revision; no scripted input. CPU after corresponds to 1.56%
of one core over that short interval (not a whole-machine percentage). The
sampled peak excludes startup. These are noisy, short synthetic idle observations,
not evidence of an emoji-rendering performance improvement. No extra emoji cache
or worker was added; its existing limits remain unchanged.

Native automation was unavailable (`orca` absent; `@oai/sky` native pipe missing,
Windows error 2). Thus rendered viewport/display scale, light/dark/narrow visual
checks, interactive typing/scrolling samples and helper-process verification
remain unverified. The configured initial viewport is 1120×760 logical pixels.
Frame/startup p95, GPU memory and voice-call usage were not measured. Focused
offline egui checks verify cold/ready geometry and editing; no screenshots or
live Discord compatibility are claimed. See `docs/progress.md` for baseline
Clippy/UI-runner blockers.

## User context menu - September 11, 2026

Baseline `ea68e0e9afaa822e64e6bea1e144d48816aab339`, compared with integrated code `de73c85`.
The after column includes the pending-upload UI merged from `d8cb031`; it cannot
isolate this menu's overhead. Before that merge, the menu-only text/voice executables
grew by 56,832 / 57,856 bytes. Reducer samples predate the UI-only integration;
client-core is unchanged by that merge.
Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical cores), 31.1 GiB visible RAM,
Rust 1.98.1, locked release, thin LTO/one codegen unit, wgpu. Both
`cargo xtask package` and `cargo xtask package-voice` passed on both revisions.
Final releases use the isolated `E:/codex-builds/rustcord-user-menu-target`; source
builds replaced stale artifacts encountered in the shared build cache. No new dependencies.

| Metric | Baseline | After | Delta |
| --- | --- | --- | --- |
| text executable, bytes | 53,986,304 | 54,098,432 | +112,128 (+0.208%) |
| text installed package, bytes | 55,106,039 | 55,227,192 | +121,153 (+0.220%) |
| text ZIP (DEFLATE 9), bytes | 33,243,781 | 33,289,588 | +45,807 (+0.138%) |
| voice executable, bytes | 59,091,968 | 59,205,120 | +113,152 (+0.191%) |
| voice installed package, bytes | 60,317,494 | 60,439,743 | +122,249 (+0.203%) |
| voice ZIP (DEFLATE 9), bytes | 35,345,246 | 35,390,825 | +45,579 (+0.129%) |
| Reducer median, 100,000 events | 39.3572 ms | 38.4854 ms | -0.8718 ms (-2.22%); noisy |
| Idle CPU, % of one logical core | 2.275 | 0.000 | -2.275 |
| Median working set, MiB | 157.254 | 157.199 | -0.055 |
| Peak sampled working set, MiB | 159.766 | 157.199 | -2.566 |
| Median private bytes, MiB | 375.941 | 375.672 | -0.270 |

Package samples include all declared shipped files (65 text / 92 voice), measured
before this performance addendum; the baseline's stale, untracked
`docs/agent-orchestration.md` was excluded from its text archive for identical file sets.
The text ZIP remains above the initial 30 MiB compressed target.
One reducer warmup plus five runs per revision, direct release `replay-bench.exe`:
baseline 40.1823, 39.3572, 40.3051, 36.3527, 36.1438 ms; after 43.8780, 36.7993,
41.1610, 38.4854, 37.9227 ms. Both retain 500 messages / 236992..237477 estimated
bytes. This measures the synthetic reducer, not UI latency or RSS; overlapping
ranges do not establish a speed improvement.

Native process samples: one fresh text-only `--demo` process per revision, 10-second
warmup, 20 samples at requested 500 ms intervals (10.303 / 10.285 actual seconds),
System.Diagnostics.Process working set/private bytes and TotalProcessorTime delta.
No scripted input because native Computer Use was unavailable; default requested
1120x760-point viewport, actual display scale/occlusion unverified. No auth/audio
helpers were started. GPU memory, startup/frame p95 and menu-interaction memory
remain unmeasured. Builds and other desktop activity were present, so these short
single-process samples are noisy. The observed CPU/memory differences are recorded,
not attributed to menu rendering or represented as a performance improvement.
Working-set samples exceed the original 80 MiB settled-idle target on both revisions.


## Combined DM/group activity ordering - September 11, 2026

Baseline `fd20dc9`, compared with this change on Windows 11 Home 10.0.26200,
Ryzen 7 7800X3D, 31.1 GiB visible RAM, Rust 1.98.1; locked release builds,
thin LTO, one codegen unit, wgpu. Text and optional voice packages were built
separately with `cargo xtask package` and `cargo xtask package-voice`.
Installed bytes include all package files; ZIP uses Python zipfile DEFLATE level 9.
One package sample per variant, before this documentation addendum.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 51,146,752 | 51,195,392 | +48,640 |
| Text installed bytes | 51,843,054 | 51,891,694 | +48,640 |
| Text ZIP bytes | 31,772,864 | 31,790,250 | +17,386 |
| Voice executable bytes | 54,500,864 | 54,549,504 | +48,640 |
| Voice installed bytes | 55,405,357 | 55,453,997 | +48,640 |
| Voice ZIP bytes | 33,149,123 | 33,165,994 | +16,871 |
| 100,000-event reducer replay median ms | 42.1050 | 40.3613 | -1.7437 (-4.1%) |
| Retained timeline estimated bytes | 236,992..237,477 | 236,992..237,477 | unchanged |

Replay: `cargo build --release --locked -p replay-bench`, then one direct executable
warmup and five measured runs per revision; 500 retained records. This small noisy
difference is not a speedup claim or a measurement of sidebar sorting, RSS or frames.
Sorting reuses bounded navigation/activity metadata and allocates no additional cache.
Native before/after CPU, memory and frame comparisons remain unmeasured: the owner
stopped Computer Use with Escape before the matched visual capture completed.

## Inline message spoilers - September 10, 2026

Baseline main b92a082a4b06c480fe6509252712f6513cd7dc62 / PR #35. Its verified text/voice
executables were copied before edits and kept separately; the complete package directories
were measured again. Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs),
31.1 GiB visible RAM, Rust 1.98.1, release thin LTO / one codegen unit / wgpu.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,982,976 | 49,995,264 | +12,288 (+0.025%) |
| text installed bytes | 50,534,234 | 50,552,906 | +18,672 (+0.037%) |
| text ZIP bytes | 31,263,624 | 31,275,003 | +11,379 (+0.036%) |
| voice executable bytes | 53,336,576 | 53,348,864 | +12,288 (+0.023%) |
| voice installed bytes | 54,111,170 | 54,129,279 | +18,109 (+0.033%) |
| voice ZIP bytes | 32,640,183 | 32,647,306 | +7,123 (+0.022%) |

Both unsigned Windows packages passed. One measurement each, ZIP DEFLATE level 9; text
excludes nested voice. File counts remain 50/96. No dependencies, assets or notices changed.
Installed totals describe staged docs before this measurement addendum, not an extra repack.
Measured SHA256: text 4E786EB9AAA322EDCD0ED2DEF1C8D416233326F510BB3DFA310C64FCF202B132;
voice 76879F8C79E971306F4B0BC868E78994142F34527DAB4943D4403337EC629EED.

The parser keeps the existing 8192-byte/128-line, 512-event and 16-depth limits and adds a
32-region ceiling represented by one u32 reveal mask per revealed message. Exact reveal
snapshots remain pruned to the active 500-row / 4-MiB message window; the existing parsed/source
cache remains 64 entries / 1 MiB with span allocation accounting. Hidden regions are skipped
before text layout, link/reference interaction or emoji requests. No worker, timer or disk
state was added. UI rendering is the changed workload; ordinary reducer replay would not
measure it. Native before/after screenshots, idle CPU/RSS/GPU and frame/startup timing remain
unmeasured because desktop automation is owner-paused. Headless keyboard, pointer, selection,
light/narrow and dark/wide tests are synthetic behavior checks, not native performance or
screen-reader evidence. No runtime-speed or memory improvement is claimed.

## Incoming typing indicators - September 10, 2026

Baseline main dff09752cb501aa5243334cd27f87a2c8c514db9 has the exact tested source tree of
reply-target PR #34. Its text/voice executables were separately copied and hash-verified before
root integration edits; package directories and the baseline replay executable were preserved.
Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), 31.1 GiB visible RAM,
Rust 1.98.1, release thin LTO / one codegen unit / wgpu; text and optional voice built separately.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,956,352 | 49,982,976 | +26,624 (+0.053%) |
| text installed bytes | 50,502,356 | 50,534,234 | +31,878 (+0.063%) |
| text ZIP bytes | 31,255,214 | 31,263,624 | +8,410 (+0.027%) |
| voice executable bytes | 53,314,560 | 53,336,576 | +22,016 (+0.041%) |
| voice installed bytes | 54,083,337 | 54,111,170 | +27,833 (+0.051%) |
| voice ZIP bytes | 32,627,919 | 32,640,183 | +12,264 (+0.038%) |
| initial ordinary replay median, ms / 100,000 events | 36.6831 | 38.9649 | +2.2818 (+6.22%) |
| follow-up interleaved replay median, ms / 100,000 events | 38.4538 | 37.1408 | -1.3130 (-3.41%) |
| retained timeline bytes / 500 rows | 228,992..229,477 | 228,992..229,477 | 0 |

Both unsigned Windows packages passed. One package measurement each, ZIP DEFLATE level 9,
text excluding nested voice; file counts remain 50/96 and no dependencies/notices changed.
Installed totals describe staged docs before this measurement addendum, not an extra repack.
Measured executable SHA256: text B22910243E7A9A34FE06F6887E9888284CF19998147FBCF5730ADC127AE53CC1;
voice 37409800846E9AC55A9B8DB7D07EDFB29A122C9505BC6FBB0F95F1F5D8121DFE.

Replay uses one warmup and five direct executable runs per revision. The initial +6.22%
prompted one interleaved baseline/after comparison after both package builds had finished,
also with one warmup each and five measured pairs. Its direction reversed; this short noisy
workload does not establish a speed improvement or stable regression. Follow-up baseline
times: 36.8714, 39.9938, 38.4538, 37.2671, 38.7337 ms; after: 37.1408, 36.4049, 37.3242,
38.0731, 36.9023 ms. This measures the synthetic ordinary reducer, not typing throughput,
process RSS, UI latency or Discord compatibility.

Typing adds eight fixed identity/deadline slots to State, an eight-entry coalescing gate and
a separate eight-slot fixed-payload inbox. No disk cache, worker, dependency or polling timer
was added. The UI uses at most three retained names of 40 characters each plus a remainder,
and schedules only the next displayed expiry. Native before/after screenshots, idle CPU,
RSS/GPU, p95 frame/startup timing and accessibility remain unmeasured while desktop automation
is owner-paused. Headless rendering checks are not native or live-account evidence.

## Saved reading and layout preferences - September 10, 2026

Baseline dfe9e3f8a657665009ac89bfadb5e1be7cb3f064 (PR #31) unsigned Windows text/voice
executables were copied and hash-verified before edits; their installed/ZIP size reports
were retained. Same Windows 11 Home 10.0.26200 / Ryzen 7 7800X3D (16 logical CPUs) /
about 31 GiB RAM / Rust 1.98.1 / release thin LTO, one codegen unit, wgpu.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,840,128 | 49,855,488 | +15,360 (+0.031%) |
| text installed bytes | 50,349,976 | 50,370,977 | +21,001 (+0.042%) |
| text ZIP bytes | 31,194,530 | 31,205,377 | +10,847 (+0.035%) |
| voice executable bytes | 53,196,800 | 53,213,184 | +16,384 (+0.031%) |
| voice installed bytes | 53,929,436 | 53,951,446 | +22,010 (+0.041%) |
| voice ZIP bytes | 32,570,631 | 32,576,903 | +6,272 (+0.019%) |

Both unsigned Windows packages passed. One size measurement each; ZIP DEFLATE level 9;
text excludes the voice folder. File counts remain 50/96; no dependencies or notices added.
Installed totals describe staged docs before this measurement addendum; no PR evidence is bundled.

SHA256 of measured executables:

- text: 4BC461415BC3814CD206194BBAC3D5C40E60DB7B0D64540FAF74C861A2FD3D72
- voice: 357F7BED34136B24174BC91EB0DCE675DC975C4225BB734E5F96461AF1DAED57

Settings add one fixed-size model record, one latest pending snapshot, and at most one
queued write on the existing 16-command / 16-result worker. The 300 ms debounce runs only
while a user change is pending; a failed/full queue stops retrying until another edit or
explicit Retry saving. No new worker, dependency, service request or periodic idle repaint.
SQLite schema 8 adds one scalar singleton within the existing 64 MiB database ceiling.

This changes settings persistence and native layout, not the message reducer workload.
No reducer speed claim is made. Native before/after screenshots, CPU/RSS/GPU use, frame
timing and scale-change latency remain unmeasured because desktop automation is owner-paused.
Headless geometry/control/anchor tests are synthetic and cannot establish native accessibility
or runtime performance. No native resource-performance improvement is claimed.

## Recently visited conversation windows - September 10, 2026

Baseline b4c66ac0c8f226bdfdc22d904fbde8a239975dcb (PR #29) packages were copied and hash-verified.
The baseline reducer executable and navigation harness were built and copied before core edits.
Same Windows 11 Home 10.0.26200 / Ryzen 7 7800X3D (16 logical CPUs) / about 31 GiB RAM /
Rust 1.98.1, release thin LTO, one codegen unit, wgpu.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,824,256 | 49,840,128 | +15,872 (+0.032%) |
| text installed bytes | 50,326,094 | 50,349,976 | +23,882 (+0.047%) |
| text ZIP bytes | 31,186,721 | 31,194,530 | +7,809 (+0.025%) |
| voice executable bytes | 53,180,416 | 53,196,800 | +16,384 (+0.031%) |
| voice installed bytes | 53,905,027 | 53,929,436 | +24,409 (+0.045%) |
| voice ZIP bytes | 32,560,106 | 32,570,631 | +10,525 (+0.032%) |

Both unsigned Windows packages passed. One measurement each; ZIP DEFLATE level 9; text
excludes the voice folder. File counts remain 50/96, with no dependency or notices changes.
Installed totals describe staged docs before this measurement addendum; no PR evidence is bundled.

SHA256 of measured executables:

- text: AAAEC68E80A62E89A98129A3BB342BBE2B75B6D2DEDD94B58A212809D2222145
- voice: 0F5403607ADF30286D37783FCAFF6D02BD2D65D4B634C1D7CB196D385735D8C6

One warmup and five measured direct executable runs per workload/revision:

| Metric | Baseline | Resident windows | Delta |
| --- | ---: | ---: | ---: |
| Immediate previews / 10,000 selections | 0 | 10,000 | +10,000 |
| Revalidation requests / 10,000 selections | 10,000 | 10,000 | 0 |
| Core select median, microseconds (five-run median) | 1.3 | 0.2 | -1.1 |
| Core select p95, microseconds (five-run median) | 1.7 | 0.2 | -1.5 |
| Ordinary 100,000-event replay median, ms | 37.7021 | 38.4952 | +0.7931 (+2.10%) |
| Ordinary replay retained live payload bytes / 500 rows | 220,992..221,477 | 220,992..221,477 | 0 |

Navigation: `cargo replay -- --navigation`; three synthetic 50-message DMs, three initial loads,
then 10,000 selections and handcrafted authoritative history responses. Only State::select is
timed; fixtures/responses and rendering/I/O are excluded. Baseline medians: 1.3, 1.2, 1.3, 1.3,
1.3 microseconds; p95: 1.7, 1.6, 1.8, 1.7, 1.6. Changed medians and p95 were 0.2 in all runs.
These tiny measurements are quantized and sensitive to timer overhead; this does not establish
native channel-switch p95. Every changed run retained two dormant windows / 150 total rows /
199,654 estimated allocation bytes. The accounting footer was added after the baseline capture;
the timed workload is unchanged. Baseline had no dormant windows.

Ordinary replay baseline samples: 38.6347, 36.0714, 35.9521, 37.7021, 38.2936 ms; changed:
38.4947, 39.0375, 39.4187, 38.2191, 38.4952 ms. The median is slower, with overlapping ranges;
no ordinary-message speed improvement is claimed. This is not RSS or live compatibility evidence.

Component bounds: at most two dormant windows, with active+dormant rows capped at 1,475 and
estimated allocations at 16 MiB minus 66 KiB; reserve 25 rows and 66 KiB for search/pins.
The estimate now includes row/container storage, pending patch capacity and reconciliation sets,
with a B-tree slack allowance. Individual timelines retain their 500-row / 4 MiB payload limit.
No additional worker, network request, schema or runtime dependency. Native screenshots,
CPU/RSS, GPU allocations and frame/startup/channel-switch measurements remain owner-paused.

## Deleted message state - September 10, 2026

Baseline e0f18d0d2c2d783ad85ec8793f62288aca259d31 (PR #28) packages and replay executable
were separately copied and hash-verified before edits. Same Windows 11 Home 10.0.26200 /
Ryzen 7 7800X3D (16 logical CPUs) / about 31 GiB RAM / Rust 1.98.1, release thin LTO,
one codegen unit, wgpu.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,810,944 | 49,824,256 | +13,312 (+0.027%) |
| text installed bytes | 50,304,421 | 50,326,094 | +21,673 (+0.043%) |
| text ZIP bytes | 31,173,706 | 31,186,721 | +13,015 (+0.042%) |
| voice executable bytes | 53,163,520 | 53,180,416 | +16,896 (+0.032%) |
| voice installed bytes | 53,879,770 | 53,905,027 | +25,257 (+0.047%) |
| voice ZIP bytes | 32,543,824 | 32,560,106 | +16,282 (+0.050%) |

Both unsigned Windows packages passed. One size measurement each; ZIP DEFLATE level 9;
text excludes the voice folder. File counts remain 50/96; no dependency or notices changes.
Installed totals describe staged docs before this measurement addendum; no PR evidence is bundled.

SHA256 of measured executables:

- text: 57AF34D6F35E3576E2D0C3E3F9560265ED3991863BB2D1356CE5FF7225F6804C
- voice: 6608248BC069ADD7CE9520411C9AF7C1DC832FC5139DC166F6641CECE7D38A56

The synthetic 100,000-event reducer replay used one warmup and five measured direct executable
runs per revision. Baseline: 36.4297, 37.0517, 38.0769, 36.5077, 36.9497 ms; changed:
36.0073, 37.0905, 39.5844, 40.1928, 36.6144 ms. Median 36.9497 -> 37.0905 ms,
+0.1408 ms (+0.38%); ranges overlap. Both retain 500 live records and
220,992..221,477 estimated live payload bytes. No speed or acceptance-latency claim is made.
This workload has ordinary message events, not deletion I/O, process RSS or UI frame timing.

Deleted reading rows retain an ID and an empty Option<Message> slot, without message payloads.
Eviction charges live heap plus the Option slots against the existing 500-row / 4 MiB estimate;
the live-only bytes API still returns zero for an all-deleted window. These component bounds
exclude tree allocator overhead and are not whole-process RAM measurements. Tests exercise
deleted rows at both eviction edges, payload release, late arrivals and the 1,024-deletion guard.
SQLite deletion uses one transaction for at most 100 IDs. The existing worker keeps its
16-command / 16-result queues, with at most 16 additional pending cleanup account IDs.
No new worker, dependency, schema or persistent journal is added.

Native screenshots, CPU/RSS and frame timing remain unmeasured while desktop automation is
owner-paused. No account, browser or microphone was used.

## Unsupported-content markers - September 10, 2026

Baseline 8c6976edc85cc0d5b5e86b636cfd331b4063c747 (PR #27) packages were copied and
hash-verified before edits. Same Windows 11 Home 10.0.26200 / Ryzen 7 7800X3D (16 logical CPUs) /
about 31 GiB RAM / Rust 1.98.1, release thin LTO, one codegen unit, wgpu.
Both unsigned Windows packages passed. One size measurement each; ZIP DEFLATE level 9;
text excludes the voice folder.

| Metric | Baseline | Markers | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,798,144 | 49,810,944 | +12,800 (+0.026%) |
| text installed bytes | 50,286,162 | 50,304,421 | +18,259 (+0.036%) |
| text ZIP bytes | 31,169,312 | 31,173,706 | +4,394 (+0.014%) |
| voice executable bytes | 53,150,720 | 53,163,520 | +12,800 (+0.024%) |
| voice installed bytes | 53,861,511 | 53,879,770 | +18,259 (+0.034%) |
| voice ZIP bytes | 32,541,063 | 32,543,824 | +2,761 (+0.008%) |
| replay median ms / 100,000 events | 37.0330 | 38.3804 | +1.3474 (+3.64%) |
| replay retained estimated bytes | 220,992..221,477 | 220,992..221,477 | 0 |

Replay: one warmup and five measured direct executable runs per revision, 500 retained records.
Baseline samples: 35.7539, 37.0330, 37.1835, 36.2793, 38.5828 ms. Changed samples:
37.4959, 38.3804, 36.7489, 39.1534, 38.6181 ms. The measured median is slower, with overlapping
ranges and a small sample; no speed improvement or acceptance-latency claim is made.
This existing workload measures ordinary synthetic reducer events, not poll payload decoding,
SQLite migration, process RSS or UI latency.

RAM retains five booleans; SQLite stores one integer (0..31). Presence-only parsing discards
payloads, capped at 100 array objects / 64 direct fields per object within the 4 MiB wire limit.
Pending patch accounting now includes the fixed MessagePatch struct as well as retained payloads.
No new worker, network request, runtime dependency or notices. Package file counts remain 50/96;
installed totals include staged docs before this measurement addendum, with no PR evidence files.
Native screenshots, CPU/RSS, frame timing and live behavior remain unmeasured while desktop
automation is owner-paused. No browser/account/microphone action was used.

SHA256 of measured executables:
- text: B6B6E6BCA86F29F6ECBE1B5C5AF0FA9EFD4F9ED2C480C23E02EDCB0E7C70D5DA
- voice: 06E843CBAA0ECCB099D405153CB293EC5B09CB75B4936BC2A0AC2FCB555E02DD

## Unsupported-content fallback - Windows packages, September 10, 2026

Baseline 36ab5e729571d7e42650b02ec3feefb9418652c3 (PR #26) packages were separately
copied and hash-verified before edits. The changed branch also integrates the later Gateway
fixture-only CI fix. Same Windows 11 Home 10.0.26200 / Ryzen 7 7800X3D (16 logical CPUs) /
about 31 GiB RAM / Rust 1.98.1, release thin LTO, one codegen unit, wgpu.
Both unsigned package commands passed. One size measurement per variant, ZIP DEFLATE level 9;
text excludes the voice subdirectory.

| Metric | Baseline | Fallback | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,776,640 | 49,798,144 | +21,504 (+0.043%) |
| text installed bytes | 50,255,386 | 50,286,162 | +30,776 (+0.061%) |
| text ZIP bytes | 31,159,877 | 31,169,312 | +9,435 (+0.030%) |
| voice executable bytes | 53,128,704 | 53,150,720 | +22,016 (+0.041%) |
| voice installed bytes | 53,833,119 | 53,861,511 | +28,392 (+0.053%) |
| voice ZIP bytes | 32,536,920 | 32,541,063 | +4,143 (+0.013%) |

File counts remain 50/96; no dependency or notice change. Installed totals describe actual
staged docs before this measurement addendum. Packages contain no PR evidence. Generated
routes are bounded by three u64 IDs; the existing single pending external-link slot retains
the 2,048-byte URL limit. Only visible rows create fallback controls. No new worker, queue,
network request or persistent cache is added. No reducer/cache algorithm changed, so replay
was not rerun. Native CPU/RSS, frame timing, browser-helper footprint and destination resolution
remain unmeasured because desktop automation is paused. No runtime improvement is claimed.

SHA256 of measured executables:
- text: 6D1B66CA08D75A3910633388B43B3B9B41EFCEF54DE5AA44BC7CC4C2B84E3A9C
- voice: 1A1FD426CED5D23386B6B8417A47940A8998D04364122542ECCF82D99B64A97E

## Session voice gain - Windows package comparison, September 10, 2026

Baseline 3f9aa0edcd35f82eb83d5576a63d453fc384725c (PR #24) packages were copied and
hash-verified before edits. The changed branch also integrates the subsequent Linux-only
GTK v4_10 feature/docs fix. Same Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical
CPUs), about 31 GiB RAM, Rust 1.98.1, release thin LTO, one codegen unit and wgpu.
Both unsigned package commands passed. One size measurement per variant; ZIP DEFLATE
level 9; text excludes the voice subdirectory.

| Metric | Baseline | Gain controls | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,657,344 | 49,776,640 | +119,296 (+0.240%) |
| text installed bytes | 50,127,753 | 50,255,386 | +127,633 (+0.255%) |
| text ZIP bytes | 31,117,348 | 31,159,877 | +42,529 (+0.137%) |
| voice executable bytes | 53,016,064 | 53,128,704 | +112,640 (+0.212%) |
| voice installed bytes | 53,714,247 | 53,833,119 | +118,872 (+0.221%) |
| voice ZIP bytes | 32,493,543 | 32,536,920 | +43,377 (+0.133%) |

Packages retain 50/96 files and unchanged dependency notices. Installed totals describe the
actual staged docs before this measurement addendum; neither package includes PR evidence.
Shared UI includes the new sliders in both variants; the default build still excludes the
voice transport. No runtime speed or memory improvement is claimed.

Gain adds two bounded u16 atomics and one snapshot per audio callback, with finite/clipped
PCM multiplication and no new callback allocation, lock, queue or device restart. Existing
capture buffering/resampler latency remains. Device-free tests exercise the actual callback
helpers; no reducer/cache change warrants a replay rerun. Native screenshots, CPU/RSS, frame
timing, callback latency and physical audio remain unmeasured because native desktop automation
is paused after owner Escape stops. No account or microphone was used.

SHA256 of measured executables:
- text: 77FFE0271B1EAFFC7C9431DEB85D8EBE162D89377638C2259779E6E49EF12476
- voice: 91E61133C2D7FCF0F0BDDFBC2A379102A28F790C3F4FDEA1D293079E8E5830B2

## Linux login migration - Windows package comparison, September 10, 2026

Baseline 512b5e73fa4abac4a8bbe2233ceb74f56b74cdf8 (PR #23) was copied and hash-verified before
edits. Same Windows 11 Home 10.0.26200 / Ryzen 7 7800X3D / about 31 GiB RAM / Rust 1.98.1,
release thin LTO, one codegen unit, wgpu. Both unsigned Windows package commands passed.
One size measurement per variant; ZIP DEFLATE level 9; text excludes the voice subdirectory.

| Metric | Baseline | Migration | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,658,368 | 49,657,344 | -1,024 (-0.002%) |
| text installed bytes | 50,105,951 | 50,127,753 | +21,802 (+0.044%) |
| text ZIP bytes | 31,106,640 | 31,117,348 | +10,708 (+0.034%) |
| voice executable bytes | 53,016,064 | 53,016,064 | 0 |
| voice installed bytes | 53,687,168 | 53,714,247 | +27,079 (+0.050%) |
| voice ZIP bytes | 32,480,689 | 32,493,543 | +12,854 (+0.040%) |

File counts rise from 42/88 to 50/96 because eight login notice files are now included.
Notice bytes were verified in both packages. Totals describe actual staged docs before this
measurement addendum; neither package contains PR screenshots. Small executable differences
are not a runtime speed or memory improvement claim. Windows/macOS Wry backend sources remain
unchanged. No reducer/cache/audio algorithm changed, so reducer replay was not rerun.

Linux uses one ephemeral WebKit6 session, one bounded token slot, one in-flight main-frame query
and coalesced boolean wake state. Queries are at least 100 ms apart; the event pump checks a
2-ms deadline between at most 16 callbacks. A single native callback may exceed that budget.
Linux executable/system-library footprint, web helper CPU/RSS, startup, input latency, storage
writes and teardown remain unmeasured. Native desktop automation is paused; no window was run.
The existing WSL lacks GTK4/WebKit6 development libraries; Linux CI compilation is separate.

SHA256 of measured executables:
- text: 214CB9A68E1C58DC8CAFFF7B21C07D01EF150E92434F95036516CA7865483A63
- voice: 99A34EC7A3F647B119106E22E35DE9E2FD1847631E6E8C16AFC4ADE4536ECA72

Historical measurements follow.

Measured 2026-09-09 on an Apple M1 Pro (8 CPU cores), 16 GiB RAM, macOS 27.0 beta build 26A5425a, arm64, Rust 1.98.1. These are initial samples, not a completed acceptance benchmark.

| Workload / metric | Actual result | Limits |
|---|---|---|
| Synthetic reducer replay | 100,000 events in 21.99 ms, release | Not transport, parsing, UI frame time or live messaging |
| Retained replay timeline | 500 messages; 156,992–157,477 estimated entity bytes over the last 100 cycles | Demonstrates the application timeline bound, not process RSS/GPU stability |
| Native synthetic window | Debug build launched and was interactive; keyboard-composed synthetic message appeared in timeline | No p95 startup measurement; no live account |
| Settled synthetic process | `ps` RSS 118,896 KiB (~116.1 MiB), sampled CPU 0.0% | Debug, 50-message synthetic page plus one send; wgpu/Metal; 1120×760 requested logical window, observed screenshot 1088×768. Display scale not instrumented |
| Release/package size | Recorded in progress.md after packaging | System webview/OS frameworks are external footprint; no audio engine |

The sampled debug RSS exceeds the 80 MiB settled text-idle target and is below the 150 MiB active-channel starting target, but these workloads are not the authenticated acceptance workload. No pass is claimed for either. Shader/GPU allocations, driver components, login webview helper processes, cold/warm login bandwidth, 60 Hz scroll frame p95, cached-navigation p95, and long-running process-memory soak are unmeasured. The fixture path does not open WebKit, keyring or SQLite; those costs need separate controlled-session measurement.

The release profile uses thin LTO, one codegen unit and stripped executable debug information with separate line-table symbols. The first optimized build took 2m39s in this environment while a developer audit tool also compiled; build time is not startup time.

Run `cargo replay` for the reproducible synthetic reducer workload. Use `cargo xtask package` for the actual staged package, then measure the native process with platform tools. Targets in SPEC.md remain targets. No voice or image-preview resource claims are made.


September 10 continuation: reducer replay measured 19.791708 ms for the same 100,000 synthetic events, retaining 500 records / 156,992–157,477 estimated bytes. The package now embeds 16.51 MiB of licensed font data; its local ad-hoc-signed macOS executable is 36,933,088 bytes (~35.2 MiB). Formatting has an 8192-byte/128-line per-message input ceiling and a 64-entry/1 MiB estimated source/parsed-span cache. Spoiler consent stores original text bounded by the active timeline. These additions have no new process RSS, GPU or p95 frame measurements yet; the September 9 memory sample must not be reused as their measured footprint.

September 10 avatar/member addition: active member data is capped at 100 list positions / 128 KiB, separate from navigation and messages. Avatar textures are capped at 64 × 128 × 128 × 4 = 4 MiB; eight queued decoded outputs add at most 512 KiB. One image worker holds a compressed body capped at 512 KiB and decodes with dimensions at most 256×256 and a 1 MiB allocation limit before resizing to 128×128. Disk metadata is streamed with 32 eviction candidates, rather than retaining a full RAM index. Avatar disk allowance is deliberately generous (1 GiB / 4096 files per account) to reduce repeat network work. These are tested component ceilings, not whole-process RSS, GPU-driver totals or long-running measurements. No new codec dependency or bitmap asset was added.

macOS arm64 release packaging passed with strict ad-hoc signature verification. Executable: **37,443,712 bytes** (35.71 MiB), up **359,296 bytes** (0.97%) from the preceding UI build. No Developer ID/notarization or Windows/Linux packaging validation is claimed.

## September 10: optional one-to-one DM voice

Same macOS arm64 host/toolchain, release thin-LTO build. Text executable: **37,634,384 bytes** (35.89 MiB). Voice executable: **40,491,296 bytes** (38.62 MiB), **+2,856,912 bytes / 7.59%** over the current text build. Both local packages passed strict ad-hoc signature verification; neither is notarized. `otool -L` selects system frameworks and libraries only, with no Homebrew/libopus dynamic-library dependency. The codec is statically bundled. Voice packages also include codec/crypto notices and the modified MPL HPKE component source.

The voice-enabled release launched natively with `--demo`; the DM header exposed Call/Audio, Call was disabled, and the Audio menu's accessibility text reported that microphone/speakers are unavailable in preview. This is an offline startup/UI check, not physical audio or Discord validation. The preview was closed after inspection.

Call media budgets include four eight-frame PCM rings/queues (20 ms, 960 mono f32 samples each: 120 KiB sample storage total), eight encoded jitter packets (at most 10,200 bytes), 64 KiB voice WebSocket messages and 4 KiB UDP input packets. Codec, MLS, device/driver, task and operating-system allocations are additional. Call audio threads/devices stop on leave; the next call waits for native teardown completion. These are component limits and ownership checks, not measured call RSS or a hardware teardown result. No active-call CPU, RSS, latency, audio-quality, device-change or long-duration measurement is claimed.

## September 10: categories, embeds and server icons

Same macOS arm64 host/toolchain. Text executable: **37,972,480 bytes** (36.21 MiB), +338,096 bytes over the preceding text build. Voice executable: **40,844,544 bytes** (38.95 MiB), +353,248 bytes over the preceding voice build. Both packages passed strict local ad-hoc signature verification. The lockfile has no new package entries, only direct edges to existing SHA-256/URL/serde dependencies. No new image codec, font or bitmap asset was bundled.

The shared avatar/icon/embed texture allowance is now 64 entries / 16 MiB RGBA, replacing the preceding avatar-only 4 MiB bound. Two decoded-result slots add at most 2 MiB. One worker holds at most a 2 MiB encoded response and uses an 8 MiB PNG decoder allocation limit for embeds (512 KiB / 1 MiB for avatars/icons); dimensions and resized outputs have separate limits. Disk remains 1 GiB / 4096 files / 90 days per account. These are component ceilings, not whole-process RSS or GPU measurements.

Embed metadata is limited to 10 cards, 25 fields/card and 64 KiB/message, counted within the existing 4 MiB timeline and 1 MiB pending-patch limits. Parsed message/embed sections share the existing 64-entry / 1 MiB formatting cache. Spoiler consent retains a bounded copy of active revealed message content and embed metadata. SQLite eviction includes serialized embed bytes and preserves its 64 MiB database ceiling. Navigation remains limited to 4000 items / 4 MiB metadata; collapsed category keys are pruned with navigation.

A native voice-build offline preview verified complete card/image layout, category collapse/selection visibility, generated server icons and external-link confirmation at approximately 1088×768. No production CDN request, authenticated workload, process-memory/frame-time benchmark, hardware call or Windows/Linux run was measured. Earlier RSS samples do not represent these additions.

September 10 inline-link correction: locally ad-hoc-signed text executable **37,988,976 bytes**; voice **40,844,624 bytes**. The existing parser and native egui link widgets are reused, with no new dependencies/assets. Message and embed text keep their shared bounded formatting cache. Native synthetic layout and pointer checks passed; no new RSS/frame-time or live-service measurement was taken.

September 10 chat-image attachments: text executable **38,030,064 bytes** (+41,088); voice **40,902,080 bytes** (+57,456) over the preceding inline-link builds. Both passed strict local ad-hoc signature verification. No new dependency/codec/bitmap asset was added. Attachment metadata is capped at 10 / 64 KiB per message within existing timeline/event/pending-patch budgets; typed cache JSON is capped at 256 KiB and counted in SQL eviction. The shared 16 MiB/64-texture media allowance, two decoded-result slots and disk cache limits are unchanged. Inline images fit 420×280 and the enlarged viewer uses the existing at-most-512-pixel output, reusing the same texture. Native synthetic layout/viewer checks passed; no actual image-download or whole-process RSS/frame-time benchmark was measured.

## Mentions, profile cards and native image saving

September 10 follow-up, same macOS arm64 host/toolchain. The lockfile adds rfd 0.17.2 and pollster 0.4.0 for native Save As (423 non-workspace packages in the all-feature macOS metadata inventory). Both package variants include their license texts. No new decoder, font or bundled bitmap was added.

One profile retains at most 64 KiB metadata after a 256 KiB response cap. Banners reuse static 512-pixel textures; enlarged attachment viewing reuses/upscales the same 512-pixel preview without allocating a full-resolution texture. Mention suggestions inspect at most 256 locally known users and show eight rows. Existing 16 MiB/64-texture, timeline and disk budgets remain in force. REST admits at most four concurrent requests. Explicit original image saves allow one transfer, stream transport chunks through at-most-32-KiB file writes with coalesced progress, and cap the original at 100 MiB; they do not decode or buffer an entire image.

Native macOS offline checks exercised mention insertion/profile activation, banner card layout and scrolling, and enlarged image controls. Native Save As interaction and live CDN requests were not exercised; localhost tests establish transfer/cancellation/file replacement behavior only. No whole-process RSS, idle CPU or frame-time benchmark was performed for these additions.

Final staged executable sizes: text **38,374,272 bytes** (36.60 MiB), +344,208 bytes over the preceding attachment build; voice **41,248,800 bytes** (39.34 MiB), +346,720 bytes. Both variants passed strict local ad-hoc signature verification; these are development packages, not notarized releases.

## September 10: reaction refresh and pinned-message browsing (Windows)

Baseline 74c0d79 versus this reaction/pins change; both text and voice packages rebuilt with pinned Rust 1.98.1, the existing release profile and lockfile on Windows 11 Home 10.0.26200, AMD Ryzen 7 7800X3D (16 logical processors), approximately 31 GiB visible RAM. No dependencies changed. These are unsigned local packages, excluding the separate OS WebView2 runtime and drivers. Full package and ZIP sizes below were measured immediately after packaging, before this measurement note was added to repository documentation. Text packaging excludes the sibling voice directory; both exclude docs/pr-evidence. ZIP uses Python zipfile DEFLATE level 9 over the entire respective package.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 41,813,504 | 41,835,008 | +21,504 (+0.051%) |
| Voice executable, bytes | 45,160,448 | 45,182,976 | +22,528 (+0.050%) |
| Text installed package, bytes | 42,033,731 | 42,060,801 | +27,070 (+0.064%) |
| Voice installed package, bytes | 45,604,233 | 45,632,327 | +28,094 (+0.062%) |
| Text ZIP, bytes | 24,370,088 | 24,380,566 | +10,478 (+0.043%) |
| Voice ZIP, bytes | 25,737,983 | 25,746,892 | +8,909 (+0.035%) |
| Reducer replay median, milliseconds | 26.2139 | 26.3168 | +0.1029 (+0.39%) |
| Retained timeline estimated bytes | 220,992-221,477 | 220,992-221,477 | Unchanged, 500 records |

Replay method: `cargo build --release --locked -p replay-bench`, then run the produced executable once for warmup and five measured times on each revision. Baseline runs: 26.4021, 26.2139, 25.7050, 26.6599, 25.9837 ms; after: 26.1046, 28.5302, 26.8548, 26.3168, 26.2875 ms. Each run reduces 100,000 synthetic events and checks bounded state/logout. The small median difference is noise, not an optimization claim. This workload does not measure the pins endpoint, actual network latency, UI frame time, process RSS or voice audio.

Native baseline only: rebuilt text --demo, default dark 1122x792 captured window, wgpu renderer; selected adapter and display scale were not independently measured. After more than 30 seconds settling and one synthetic reaction toggle, 16 Get-Process samples at one-second intervals covered 15.247 seconds. Working set stayed 198,946,816 bytes, private bytes 412,827,648; CPU delta was 0.046875 seconds (0.307% of one core). This baseline exceeds the initial 150 MiB active-text working-set target. Whole-system contention, GPU allocations, helper-process usage, startup/frame p95 and voice memory were not measured. The owner stopped Computer Use before after-build interaction; no comparable after native-memory/CPU sample or performance improvement is claimed.

## September 10: single-file uploads (Windows)

Baseline e99bc8817454335789b84140c2998394bca899cf versus this attachment change. Same Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical processors), approximately 31 GiB visible RAM, pinned Rust 1.98.1 and release profile. The baseline text/voice packages were copied separately after their executable hashes matched the reviewed baseline builds. Both changed variants were packaged successfully. New native tokio-util 0.7.19 comes from enabling reqwest streaming; its MIT license is included. No fonts/codecs changed. These unsigned development-package measurements exclude the OS WebView2 runtime and drivers. Installed totals and ZIPs reflect staging before this final progress/performance note was added; text excludes its sibling voice directory and both exclude PR evidence. ZIP: all respective package files, Python zipfile DEFLATE level 9.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 41,835,008 | 42,109,440 | +274,432 (+0.656%) |
| Voice executable, bytes | 45,182,976 | 45,423,616 | +240,640 (+0.533%) |
| Text installed package, bytes | 42,060,801 | 42,345,271 | +284,470 (+0.676%) |
| Voice installed package, bytes | 45,632,327 | 45,884,089 | +251,762 (+0.552%) |
| Text ZIP, bytes | 24,380,566 | 24,476,080 | +95,514 (+0.392%) |
| Voice ZIP, bytes | 25,746,892 | 25,829,810 | +82,918 (+0.322%) |
| Reducer replay median, milliseconds | 26.8048 | 26.5324 | -0.2724 (-1.02%, noise) |
| Retained timeline estimated bytes | 220,992-221,477 | 220,992-221,477 | Unchanged, 500 records |

Replay: build release replay-bench once per revision; one warmup plus five direct executable runs, 100,000 synthetic events each. Baseline: 26.5979, 28.1066, 26.8048, 26.5815, 27.4200 ms. After: 26.2354, 26.3576, 27.3465, 28.1847, 26.5324 ms. This reducer workload does not measure file transfers, network throughput, process memory, UI latency or voice.

Native baseline only: text --demo, dark 1122x792 window, wgpu; selected adapter/display scale unmeasured. After more than 30 seconds idle settling, 16 Get-Process samples at one-second intervals covered 15.273 seconds. Working set peaked at 158,564,352 bytes and settled at 158,547,968; private bytes peaked at 394,444,800 and settled at 394,412,032. CPU delta was 0.046875 seconds (0.307% of one core). The baseline was idle, with no picker/upload interaction. The owner stopped Computer Use before changed-build inspection, so no comparable after sample, upload-load memory, frame/startup p95, GPU/helper-process or voice-memory measurement is available. No native performance improvement is claimed.

Resource ceilings: one selected file/job, file <=20,000,000 bytes, 64 KiB application chunks, latest-value progress, staging/storage responses <=64 KiB, local path <=4096 encoded bytes and filename <=256 UTF-8 bytes. Library, TLS, OS and driver buffers are additional. This avoids a deliberate whole-file allocation but is not a measured process-memory ceiling. Pending filename metadata shares existing draft/pending admission budgets; no upload disk snapshot or new database table exists.

### Updated attachment packages including drag-and-drop

Same reference host, Rust profile and original e99bc88 baseline above; both Windows package variants rebuilt after adding drop selection. No dependency or reducer source changed in this continuation. Incremental executable growth over picker-only b83b41a is 28,160 bytes text (0.067%) and 26,624 bytes voice (0.059%). The complete PR comparison is:

| Metric | Original baseline | Picker and drop | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 41,835,008 | 42,137,600 | +302,592 (+0.723%) |
| Voice executable, bytes | 45,182,976 | 45,450,240 | +267,264 (+0.592%) |
| Text installed package, bytes | 42,060,801 | 42,382,555 | +321,754 (+0.765%) |
| Voice installed package, bytes | 45,632,327 | 45,918,753 | +286,426 (+0.628%) |
| Text ZIP, bytes | 24,380,566 | 24,486,146 | +105,580 (+0.433%) |
| Voice ZIP, bytes | 25,746,892 | 25,838,616 | +91,724 (+0.356%) |
| Reducer replay median, milliseconds | 26.8048 | 27.9813 | +1.1765 (+4.39%, noisy) |

Package totals use the same full-file/DEFLATE-9 method and exclusions, measured before this final progress/performance addendum was staged. After release builds finished, the existing release replay executable (reducer source unchanged) was run once for warmup plus five samples: 26.2423, 28.5555, 32.6936, 27.9813, 26.8632 ms. Retained state remains 500 messages and 220,992-221,477 estimated bytes. The spread and unchanged reducer make this a noisy observation, not an algorithmic regression or optimization claim; drop throughput and UI latency were not measured.

The native picker-only parent composer was inspected, then Computer Use stopped with physical Escape before chooser interaction. No native drop-build process sample, final screenshot or actual OS drag/drop measurement is available. Admission moves egui handles once, retains at most one bounded local path, and reuses asynchronous metadata validation; it never calls the handle's whole-file bytes method. All original upload resource ceilings remain unchanged.

## September 10: channel references (Windows)

Baseline 14dd066abf2c15b5dc07fca9fb2d14a2a651a99c versus this channel-reference change. Baseline packages were copied separately from the verified parent builds before edits. Both changed text and voice packages passed packaging on Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical processors), approximately 31 GiB visible RAM, pinned Rust 1.98.1 and the existing release profile. No dependency/font/codec change. Unsigned development packages exclude WebView2 and drivers. Installed/ZIP totals were measured after packaging, before this final progress/performance note was staged; text excludes the sibling voice directory, both exclude PR evidence. ZIP includes all respective package files using Python zipfile DEFLATE level 9.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 42,137,600 | 42,149,888 | +12,288 (+0.029%) |
| Voice executable, bytes | 45,450,240 | 45,462,528 | +12,288 (+0.027%) |
| Text installed package, bytes | 42,382,555 | 42,403,234 | +20,679 (+0.049%) |
| Voice installed package, bytes | 45,918,753 | 45,939,432 | +20,679 (+0.045%) |
| Text ZIP, bytes | 24,486,146 | 24,491,835 | +5,689 (+0.023%) |
| Voice ZIP, bytes | 25,838,616 | 25,845,493 | +6,877 (+0.027%) |
| Reducer replay median, milliseconds | 26.7122 | 27.6904 | +0.9782 (+3.66%, noisy) |
| Retained timeline estimated bytes | 220,992-221,477 | 220,992-221,477 | Unchanged, 500 records |

Replay: release replay-bench built per revision, one warmup plus five direct executable runs after compilation finished, 100,000 synthetic events per run. Baseline: 25.8563, 26.7122, 25.7093, 27.1198, 27.2082 ms; after: 26.3900, 27.3277, 27.6904, 27.8245, 35.7749 ms. The demo's generated message 500 gained a channel token; reducer logic is unchanged. The spread makes the timing difference inconclusive, with no optimization claim. This workload does not measure reference parsing/rendering, network, process memory or voice.

Native automation remains paused after the owner's preceding physical Escape stops. No comparable current-task native process samples, idle CPU, peak/settled memory, helper/GPU usage, actual adapter/display scale or startup/frame p95 are available. The application uses wgpu, but no renderer timing claim is made. Required native before/after screenshots and changed-build interaction remain draft blockers.

Resource limits: eight locally loaded same-server channel suggestions, 120-character suggestion labels, 64-character queries; user/channel links share 100 interactive references per formatted part. Existing 8 KiB formatter input, event/depth/link and cache limits remain unchanged. A bounded channel-label hash is calculated on state revision changes to invalidate row heights, never on unchanged idle frames. No new cache or directory lookup was introduced.

## September 10: general attachment downloads (Windows)

Baseline ae360f3f912ff15e99af5d1fe0d4f330b2545414 versus this general-file download change. The verified parent text/voice packages were copied separately before edits. Host: Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical processors), approximately 31 GiB visible RAM; pinned Rust 1.98.1, existing release profile and feature split. No dependencies/fonts/codecs changed. Both unsigned packages passed packaging. Installed/ZIP totals were taken before this final measurement/progress addendum was staged; text excludes its sibling voice directory and both exclude PR evidence. ZIP uses all respective package files with Python zipfile DEFLATE level 9. WebView2 and drivers are external.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 42,149,888 | 42,150,400 | +512 (+0.001%) |
| Voice executable, bytes | 45,462,528 | 45,463,552 | +1,024 (+0.002%) |
| Text installed package, bytes | 42,403,234 | 42,411,458 | +8,224 (+0.019%) |
| Voice installed package, bytes | 45,939,432 | 45,948,168 | +8,736 (+0.019%) |
| Text ZIP, bytes | 24,491,835 | 24,495,907 | +4,072 (+0.017%) |
| Voice ZIP, bytes | 25,845,493 | 25,848,999 | +3,506 (+0.014%) |
| Reducer replay median, milliseconds | 26.7206 | 27.7784 | +1.0578 (+3.96%, noisy) |
| Retained timeline estimated bytes | 220,992-221,477 | 220,992-221,477 | Unchanged, 500 records |

Release replay-bench: one warmup plus five direct executable samples per revision, 100,000 synthetic events each, after compilation finished. Baseline: 26.4996, 26.7206, 26.4924, 26.8281, 28.4031 ms. After: 31.0129, 27.7784, 27.5397, 27.0537, 28.2892 ms. Reducer logic is unchanged; only generated demo message 500 gained file-attachment metadata. The small timing difference is inconclusive; this workload measures neither downloads nor UI latency.

Native automation remains paused after the owner's earlier physical Escape stops. Comparable native process-memory/CPU samples, actual Save dialog/transfer-load measurements, GPU/helpers, display scale/adapter and startup/frame p95 remain unmeasured. No screenshot or runtime performance improvement is claimed; native evidence remains a draft blocker.

The existing one-dialog/transfer slot, 100 MiB original-file cap, at-most-32-KiB disk writes, expected-length checking, latest-value progress and 15/30/300-second connection/read/overall timeouts are unchanged. General files are streamed without decoding or deliberately retaining a complete file buffer. Transport/TLS/OS buffers are additional; this is a component policy, not a measured process-memory ceiling.

## September 10: loaded thread navigation (Windows)

Baseline 985bfc6ded2740a660abeeb496720b6fd0cb8489 versus this thread-navigation change. Baseline packages were copied separately before edits and verified against parent executable hashes. Both changed unsigned Windows packages passed. Host: Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical processors), approximately 31 GiB visible RAM; pinned Rust 1.98.1, existing release profile and text/voice feature split. No dependency/font/codec change. Full installed/ZIP totals reflect documentation staged by each package invocation before this final measurement/progress addendum; the voice package also includes the latest progress text. Text excludes its sibling voice directory, and both exclude PR evidence. ZIP: all respective package files, Python zipfile DEFLATE level 9. External WebView2 and drivers are excluded.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 42,150,400 | 42,330,624 | +180,224 (+0.428%) |
| Voice executable, bytes | 45,463,552 | 45,643,264 | +179,712 (+0.395%) |
| Text installed package, bytes | 42,411,458 | 42,598,237 | +186,779 (+0.440%) |
| Voice installed package, bytes | 45,948,168 | 46,138,097 | +189,929 (+0.413%) |
| Text ZIP, bytes | 24,495,907 | 24,571,118 | +75,211 (+0.307%) |
| Voice ZIP, bytes | 25,848,999 | 25,924,133 | +75,134 (+0.291%) |
| Reducer replay median, milliseconds | 26.5772 | 28.3909 | +1.8137 (+6.82%, noisy) |
| Retained timeline estimated bytes | 220,992-221,477 | 220,992-221,477 | Unchanged, 500 records |

Release replay-bench built per revision, one warmup plus five direct samples after compilation finished, 100,000 synthetic events each. Baseline: 28.8253, 26.1273, 29.2089, 26.4214, 26.5772 ms. After: 26.8821, 29.8107, 26.7173, 31.2080, 28.3909 ms. The overlapping spread makes the median increase inconclusive; it is reported rather than called an improvement. This existing message reducer workload does not measure thread-snapshot throughput, sidebar rendering, network, process RSS or voice. Dedicated thread reconciliation is covered behaviorally by bounded offline tests.

Native automation remains paused after the owner's preceding physical Escape stops. Before/after process samples, idle CPU, peak/settled memory, actual display scale/adapter, GPU/helpers and startup/frame p95 are unavailable. The app uses wgpu; no native performance claim or screenshot is manufactured. Native evidence remains a draft blocker.

Threads share the 4,000-entry navigation ceiling; sync metadata is capped at 2 MiB inside the 4 MiB wire cap. Reconciliation and sidebar grouping use temporary bounded ordered maps/sets without an additional persistent cache. Snapshot omission/removal clears existing account history conservatively; frequent thread churn may cause extra refetches. No background thread directory, polling loop, new database table or subscription expansion is included.

## September 10: older pinned-message pages (Windows)

Baseline 188c5557a024d8ce2e1ca2ca965289a6c7dd5559 versus this pin-pagination change. The verified parent release packages and reducer executable were copied separately before edits. Both changed unsigned packages passed on Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical processors), approximately 31 GiB visible RAM, Rust 1.98.1 and the existing release profile/text-voice split. The existing time 0.3.55 formatting feature serializes pin cursors; Cargo.lock and versions are unchanged. Installed/ZIP totals reflect package-staged documentation before this final measurement/progress addendum; text excludes the sibling voice directory and both exclude PR evidence and external WebView2/drivers. ZIP includes all respective package files with Python zipfile DEFLATE level 9.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 42,330,624 | 42,341,888 | +11,264 (+0.027%) |
| Voice executable, bytes | 45,643,264 | 45,655,040 | +11,776 (+0.026%) |
| Text installed package, bytes | 42,598,237 | 42,620,650 | +22,413 (+0.053%) |
| Voice installed package, bytes | 46,138,097 | 46,157,505 | +19,408 (+0.042%) |
| Text ZIP, bytes | 24,571,118 | 24,580,394 | +9,276 (+0.038%) |
| Voice ZIP, bytes | 25,924,133 | 25,933,749 | +9,616 (+0.037%) |
| Reducer replay median, milliseconds | 27.5110 | 28.2131 | +0.7021 (+2.55%, noisy) |
| Retained timeline estimated bytes | 220,992-221,477 | 220,992-221,477 | Unchanged, 500 records |

Replay: one release build per revision, one warmup plus five direct samples after compilation finished, 100,000 synthetic events each. Baseline: 30.9570, 29.3369, 27.0244, 27.5110, 27.2474 ms. After: 26.7628, 29.5610, 28.2131, 26.8470, 28.5889 ms. Samples overlap; the small median increase is inconclusive. This existing reducer workload does not measure pin HTTP requests, parser throughput, UI latency, process RSS or voice.

Native automation remains paused after the owner's prior physical Escape stops. Comparable process-memory/CPU samples, actual display scale/adapter, GPU/helpers, and startup/frame p95 are unavailable. No native performance or visual-inspection claim is made. Runtime ceilings remain one replaceable 25-item / 64 KiB result page, 512 KiB response body and existing shared request task. Fixed-size timestamp cursors add no growing history or query cache. Pagination only occurs after a deliberate action.

## September 10: archived-thread browsing (Windows)

Baseline 293c72b3ab3570e904cf23b52b479509d8ee75bc versus this archive-browser change. Baseline text/voice packages and reducer executable were copied before edits; parent package hashes were reverified. Both changed unsigned packages passed. Host: Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical processors), approximately 31 GiB visible RAM; Rust 1.98.1, existing release profile and text/voice split. No dependency/font/codec change. Disk pressure moved changed build and test-temporary output to a separate E: directory; packaging now honors CARGO_TARGET_DIR, and the final voice package hash matches that target's executable. Installed/ZIP totals reflect documentation staged by packaging before this final evidence addendum. Text excludes its sibling voice directory; both exclude PR evidence and external WebView2/drivers. ZIP: all respective package files, Python zipfile DEFLATE level 9.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 42,341,888 | 42,437,632 | +95,744 (+0.226%) |
| Text installed package, bytes | 42,620,650 | 42,726,195 | +105,545 (+0.248%) |
| Text ZIP, bytes | 24,580,394 | 24,615,435 | +35,041 (+0.143%) |
| Voice executable, bytes | 45,655,040 | 45,750,784 | +95,744 (+0.210%) |
| Voice installed package, bytes | 46,157,505 | 46,262,905 | +105,400 (+0.228%) |
| Voice ZIP, bytes | 25,933,749 | 25,969,231 | +35,482 (+0.137%) |
| Initial replay median, ms | 27.2874 | 30.2790 | +2.9916 (+10.96%) |
| Interleaved replay median, ms | 27.9277 | 27.2380 | -0.6897 (-2.47%) |
| Retained timeline estimated bytes | 220,992-221,477 | 220,992-221,477 | Unchanged, 500 records |

Replay: one release build per revision and one warmup plus five direct samples after builds finished, 100,000 synthetic events each. The initial median increase warranted a second comparison: both executables were warmed again, then sampled five times each with alternating execution order. The interleaved result did not reproduce the initial increase, and its samples overlap. No speed improvement or established regression is claimed from these short runs. This reducer workload does not measure archive transport/parser throughput, native rendering, process RSS or voice.

Initial replay median: baseline [26.1177, 29.5278, 26.0407, 28.8295, 27.2874]; after [30.279, 29.9318, 33.3689, 30.5462, 29.9439] ms.

Interleaved replay median: baseline [27.9277, 26.1154, 27.0548, 28.2489, 29.8243]; after [29.7797, 27.238, 26.9207, 30.8193, 26.5756] ms.

Native automation remains paused after the owner's prior physical Escape stops. Comparable native CPU/memory samples, actual adapter/display scale, GPU/helpers and startup/frame p95 are unavailable. No native screenshot or performance claim is made. Runtime limits are one replaceable 25-entry / 64 KiB archive page, a 512 KiB response, existing shared read worker and at most one transient channel within the account navigation budget. No growing archive cache or automatic pagination was added.

## September 10 — chat timeline comparison

Baseline is origin/main **74c0d79**, rebuilt in a detached worktree with a separate target
and package directory. After is the final chat-timeline source including the edited-label
correction. The earlier e9fb3e4 binaries and measurements were superseded after integration.
macOS 27.0 (26A428), Apple M1 Pro, 16 GiB RAM, Rust 1.98.1, arm64 release/thin LTO.
Both text and optional voice variants were built with the lockfile, packaged and verified
with strict local ad-hoc codesign. Archives contain the complete .app, including notices/docs;
logical file-byte sums exclude filesystem allocation overhead. Package snapshots precede
this final measurement report; the voice snapshot includes the new scope/progress docs.

| Metric | Main 74c0d79 | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 38,604,512 | 38,622,688 | +18,176 (+0.047%) |
| Text full .app bytes | 38,832,679 | 38,850,855 | +18,176 (+0.047%) |
| Text tar.gz bytes | 23,271,482 | 23,277,295 | +5,813 (+0.025%) |
| Voice executable bytes | 41,445,968 | 41,464,160 | +18,192 (+0.044%) |
| Voice full .app bytes | 41,905,096 | 41,929,046 | +23,950 (+0.057%) |
| Voice tar.gz bytes | 24,587,808 | 24,600,845 | +13,037 (+0.053%) |
| Text settled RSS | 113,568 KiB | 113,712 KiB | +144 KiB (+0.127%) |
| Text peak sampled RSS | 113,760 KiB | 113,808 KiB | +48 KiB (+0.042%) |
| Reducer median, 100,000 events | 27.005 ms | 27.224 ms | +0.219 ms (+0.81%) |
| Text median idle CPU | 0.0% | 0.0% | 0.0 percentage points |

Reducer: one discarded warmup plus five direct executable runs per revision, after builds
finished. Both retained **500 records / 220,992–221,477 estimated timeline bytes**. The 0.81%
elapsed difference is a small noisy change, not evidence of a throughput improvement or
meaningful regression. This workload does not exercise GUI layout.

Process: each text .app launched explicitly with `--demo`, same unchanged mixed-content
fixture and default launch options; 10-second warmup, ten `ps -p PID -o rss=,%cpu=` samples
at one-second intervals, then terminated. RSS is process resident memory; peak is only the
sampled interval, not startup peak. Both samples exceed the 80 MiB aspirational logged-in-idle
target, but these are offline mixed-media previews, not logged-in idle. No interaction was
scripted: native automation failed with `Sky Computer Use native pipe startup failed`.
wgpu is configured; actual backend, display scale, viewport, GPU/OS-helper memory, interactive
scroll peaks and p95 frame/startup times could not be verified. No native visual or performance
acceptance is claimed from the launch-only memory samples. Raw local measurements and the
measurement script are in ignored `target/chat-measurements.json` and `target/measure-chat.py`.

### September 10, 2026 — Twemoji artwork

Baseline `f708cb21fcefa741ce294afb49f1d34211a8160e`; changed branch `feat/twemoji`.
Apple M1 Pro / MacBookPro18,3, 16 GiB RAM, macOS 27.0 build 26A428, arm64,
Rust 1.98.1, locked release builds. Both text and voice packages passed local
ad-hoc signature verification; neither is notarized.

| Metric (bytes) | Baseline | Twemoji | Delta |
| --- | ---: | ---: | ---: |
| Text executable | 38,622,688 | 44,792,944 | +6,170,256 (+15.98%) |
| Text installed | 38,860,167 | 45,053,460 | +6,193,293 (+15.94%) |
| Text compressed | 23,250,661 | 29,326,446 | +6,075,785 (+26.13%) |
| Voice executable | 41,464,160 | 47,634,496 | +6,170,336 (+14.88%) |
| Voice installed | 41,932,600 | 48,125,973 | +6,193,373 (+14.77%) |
| Voice compressed | 24,564,378 | 30,641,440 | +6,077,062 (+24.74%) |

Installed = sum of every file in the complete .app, including docs, licenses
and required voice source; excludes filesystem allocation overhead. Compressed =
Python tarfile gzip level 9 of that complete app. These package snapshots precede
this final measurement report. The PNG alone adds 6,002,931 executable bytes; the
fixed 2,048×2,016 RGBA atlas has 15.75 MiB pixel payload. CPU conversion, upload
copies and GPU driver overhead are additional; no new resolved Cargo package.

| Process sample | Baseline | Twemoji | Delta |
| --- | ---: | ---: | ---: |
| Median RSS (KiB) | 108,784 | 125,040 | +16,256 (+14.94%) |
| Peak sampled RSS (KiB) | 108,784 | 125,168 | +16,384 (+15.06%) |

Process samples: text-only native `--demo`, unchanged fixture plus the same local
Unicode message shown in the screenshots, default 1120×760 requested logical size
(1088×768 exported capture), dark theme; wgpu configured (Metal expected on macOS,
backend/display scale not instrumented). At least ten seconds after interaction,
ten `ps -p PID -o rss=,%cpu=` samples one second apart. RSS is resident process
memory, not GPU memory or total system cost; sampled peak is not startup peak.
The baseline's smoothed `ps %cpu` was 0.0%, after 0.8–2.3% (median 1.55%); different
process ages and recent startup/input make those CPU values unsuitable for an idle
regression conclusion. No precise interval CPU, startup/frame p95, GPU/driver/helper
memory or long-duration soak claim. Another offline preview remained open during
the final sample; only the measured PID is included. No login webview/audio helpers
were started. Synthetic results do not establish logged-in performance targets.
Raw measurements and the sampling/package-size script are in ignored
`target/twemoji-*-samples.json`, `target/twemoji-sizes.json`, and
`target/measure-twemoji.py`.


### September 10, 2026 — server emoji, picker and text selection

Baseline `a270437cb6c1e69030420cda4e0fb6efa957bcb0` (`feat/twemoji`); follow-up
`feat/server-emoji-picker`. Same Apple M1 Pro, 16 GiB RAM, macOS 27.0 build 26A428,
arm64 Rust 1.98.1 locked release builds. Both packages passed strict local ad-hoc
signature verification. Baseline runtime binaries were compared byte-for-byte with
the previously verified packages; their docs precede the previous final report.
Current package snapshots likewise precede this final measurement report.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text exe bytes | 44,792,944 | 45,017,184 | +224,240 (+0.50%) |
| Text installed bytes | 45,053,460 | 45,283,476 | +230,016 (+0.51%) |
| Text compressed bytes | 29,326,284 | 29,391,804 | +65,520 (+0.22%) |
| Voice exe bytes | 47,634,496 | 47,875,680 | +241,184 (+0.51%) |
| Voice installed bytes | 48,125,973 | 48,378,556 | +252,583 (+0.52%) |
| Voice compressed bytes | 30,641,220 | 30,708,940 | +67,720 (+0.22%) |
| Text median/peak sampled RSS, KiB | 114,768 | 136,368 | +21,600 (+18.82%) |
| Text interval idle CPU | 0.0% | 0.0% | 0.0 percentage points |
| Reducer median, ms | 27.093 | 26.771 | -0.323 (-1.19%) |

Installed is the sum of complete .app file bytes; compressed is Python tarfile gzip
level 9 of the complete package, including notices/docs. No new Cargo dependency.
Unicode names add 158,704 source bytes; catalogs have per-guild 1,000-entry/256-KiB
limits within shared 4-MiB navigation storage. Custom images reuse the existing
64-entry/16-MiB decoded-texture cache and bounded credential-free media worker.

Replay: isolated baseline build and current release executable, one discarded warmup
plus five direct runs each. Both retain 500 messages / 220,992–221,477 estimated bytes.
The small elapsed difference is noise, not a throughput improvement claim; this
workload does not exercise picker layout or catalog parsing.

Process: native text `--demo`, same mixed fixture plus the same message containing
Unicode and custom markup, dark theme, 1088×768 captured viewport, default requested
1120×760 logical size. Current fixture additionally supplies two original synthetic
custom images. After interaction, ten-second warmup then eleven RSS/cumulative CPU
samples at one-second intervals; CPU is cumulative-time delta over about 10.16 seconds.
Both medians equal sampled peaks; peaks exclude startup. After had additional picker
activation attempts, so interaction histories are not identical. RSS increased 21.1 MiB;
this is a material observed increase, with allocator/font/media contributions not
isolated. It is not a claim about logged-in workloads or the exact cost of the catalog.
wgpu configured; actual backend/display scale not instrumented. No helper processes
were launched. GPU memory, frame/startup p95, open-picker scroll performance and long
soak remain unmeasured. Native automation intermittently failed to activate controls,
preventing reliable completion of picker/light/narrow interaction evidence.
Raw local samples: `target/custom-{before,after,replay,sizes}.json`;
script: `target/measure-custom-emoji.py`.


## September 10, 2026 — message hover controls

Baseline: clean `d802b2a`, rebuilt before UI edits; after: `feat/message-hover`.
Rust 1.98.1, macOS 27.0 (26A428), Apple M1 Pro, 16 GiB RAM, release text-only
(`--no-default-features`) and separate `--features voice` packages. Native runtime samples
use text-only `--demo --demo-chat`, unchanged nine-message synthetic fixture, default window
size, dark/system appearance, scroll two pages upward to the first message, pointer over
the conversation. wgpu configured; actual backend, GPU allocations and display scale
were not instrumented. Captures share the same 1087×768 window-image size.

| Metric | Baseline | After | Delta |
| --- | --- | --- | --- |
| Text executable, bytes | 45,017,184 | 45,033,392 | +16,208 (+0.036%) |
| Text package, bytes | 45,316,618 | 45,332,937 | +16,319 (+0.036%) |
| Text gzip, bytes | 29,438,513 | 29,444,125 | +5,612 (+0.019%) |
| Voice executable, bytes | 47,875,680 | 47,875,472 | -208 (-0.000%) |
| Voice package, bytes | 48,406,075 | 48,405,978 | -97 (-0.000%) |
| Voice gzip, bytes | 30,757,714 | 30,759,656 | +1,942 (+0.006%) |
| Median sampled RSS, KiB | 124,088.00 | 116,720.00 | -7,368.00 |
| Peak sampled RSS, KiB | 124,112.00 | 116,848.00 | -7,264.00 |
| Mean sampled CPU, % | 0.36 | 4.37 | +4.01 |

Process samples: ten `ps -p PID -o %cpu=,rss=` samples at one-second intervals,
at least ten seconds after interaction. RSS is resident memory, not physical footprint;
peaks cover only this sample window, not startup or a soak. Baseline PID 49815, after
PID 52432; no authentication webview/audio helper was launched by either demo. Other
app instances and system processes are excluded. Sampling occurred on a shared desktop
while voice packaging could still run. The after CPU samples range 0–14%, versus 0–3.6%
baseline: this noisy sample does not establish idle-CPU improvement or steady-state
regression. Lower sampled RSS is not a whole-process memory budget guarantee. Startup,
p95 frames, GPU memory, live account and voice-call performance remain unmeasured.

Package sizes sum all regular files in Serein.app plus the four accompanying root
README/license/notice files. Compressed sizes use `tar -czf` on those same paths, excluding
other distribution variants and pre-existing archives. Snapshots precede this measurement
report; packaging source and runtime assets are unchanged. Raw samples, sizes and archives
remain under ignored `target/message-hover-*`. Text grows by 16,208 executable bytes;
voice changes by −208 bytes. No new dependencies or cache/queue budgets.


## September 10, 2026 — guild voice

Baseline: clean `main` / `619071c`, fetched origin/main, Rust 1.98.1. Changed branch:
`feat/guild-voice`. Both revisions use macOS 27.0 (26A428), Apple M1 Pro (8 cores),
16 GiB RAM, arm64 locked release thin-LTO builds. Both text (`--no-default-features`)
and optional `voice` packages were built and locally ad-hoc signature verified.

| Metric / method | Baseline | After | Delta |
| --- | --- | --- | --- |
| Text executable, bytes | 45,033,392 | 45,176,320 | +142,928 (+0.32%) |
| Text package, bytes | 45,314,977 | 45,462,380 | +147,403 (+0.33%) |
| Text zip, bytes | 29,471,417 | 29,529,234 | +57,817 (+0.20%) |
| Voice executable, bytes | 47,875,472 | 48,060,368 | +184,896 (+0.39%) |
| Voice package, bytes | 48,388,018 | 48,577,389 | +189,371 (+0.39%) |
| Voice zip, bytes | 30,848,320 | 30,920,827 | +72,507 (+0.24%) |
| Reducer median, ms | 26.700 | 27.443 | +0.743 (+2.78%) |
| Idle CPU, % | 0 | 0 | +0 percentage points |
| Settled RSS, KiB | 100,448 | 105,120 | +4,672 (+4.65%) |
| Sampled peak RSS, KiB | 100,448 | 105,280 | +4,832 (+4.81%) |

Package = logical sum of every file in the installed .app, including bundled notices,
docs and modified HPKE source for voice; excludes filesystem allocation overhead.
ZIP = `ditto -c -k --sequesterRsrc --keepParent` of that same .app. Package snapshots
precede this measurement report and the final progress/PR notes. No dependency or
lockfile change. These are development artifacts, not notarized releases.

Reducer: build once per revision (`cargo replay`), invoke each executable directly,
one warmup plus five runs of 100,000 synthetic events. Retained timeline range stays
220,992–221,477 estimated bytes / 500 records. Baseline median 26.700 ms vs 27.443 ms
is a small noisy slowdown, not an improvement claim or a voice-workload measurement.

Native process: voice-enabled `--demo`, WGPU, 2× scale, approximately 1088×768 captured
viewport, dark theme, standard fixture. Navigate from getting-started to long-form,
leave People pane visible, wait 60 seconds after interaction, then 30 `ps -p PID -o
%cpu=,rss=` samples at one-second intervals. Both native processes sampled on the same
host; baseline foreground and changed window background during the quiet interval.
The changed process also visited the empty voice page before returning to text, so
its additional retained UI work is included. RSS is resident process memory, not
physical footprint or GPU allocation. Earlier transient samples were discarded before
this controlled interval. No helper child processes were launched by the demo; shared
OS/WindowServer allocations are not attributed. Foreground placement and shared-machine
noise limit interpretation; the 4,672 KiB settled RSS increase is not a precise live-call
cost. Startup, p95 frames, GPU memory, hardware audio and live voice RSS remain unmeasured.

New component workload: `cargo test --locked -p discord-voice --release
synthetic_mix_workload -- --ignored --nocapture` passed. Five measured 1,000-tick runs
after one warmup, with real Opus decoding and independent jitter state, gave medians:

| Remote speakers | Total for 1,000 ticks | Mean time per 20 ms tick within median run |
| --- | --- | --- |
| 1 | 16.027 ms | 16.03 µs |
| 8 | 125.254 ms | 125.25 µs |
| 63 | 1,002.216 ms | 1,002.22 µs |

The baseline rejects multi-party voice, so there is no working baseline group mixer
comparison. This synthetic computation excludes encryption, sockets, devices, callback
scheduling and real-time latency. The 64-total-person limit retains at most 63 decoder
states, 63 × 10,200 encoded jitter bytes and 63 × 23,040 decoded PCM bytes, plus codec,
MLS and native allocations. Eight-frame device queues remain bounded. See the adapter
README for packet, transition and timeout limits. No acoustic echo cancellation was added.


## Server people subscription repair — September 10, 2026

Clean baseline `dc48391` vs `fix/server-people-list`; Rust 1.98.1, macOS 27.0
(26A428), Apple M1 Pro, 16 GiB RAM. Both release configurations built using
`cargo xtask package` and `cargo xtask package-voice`, including strict ad-hoc
signature verification. Baseline artifacts were preserved in
`target/people-baseline`; changed distribution measurements in `target/people-after`.
Installed bytes sum regular bundle files, archives use `tar -czf` per bundle;
package totals include the documentation snapshot present at packaging time.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 45,176,320 | 45,176,288 | -32 (-0.000%) |
| Text installed bytes | 45,470,755 | 45,473,718 | +2,963 (+0.007%) |
| Text tar.gz bytes | 29,505,591 | 29,506,659 | +1,068 (+0.004%) |
| Voice executable bytes | 48,060,368 | 48,060,336 | -32 (-0.000%) |
| Voice installed bytes | 48,585,764 | 48,588,727 | +2,963 (+0.006%) |
| Voice tar.gz bytes | 30,834,259 | 30,835,849 | +1,590 (+0.005%) |
| Reducer median ms | 26.474292 | 27.127792 | +0.653500 (+2.468%) |

`cargo replay` built the reducer workload, followed by one direct warmup and five
measured direct executions per comparison. Both retained 220,992–221,477 estimated
timeline bytes and 500 records. The reducer does not exercise Gateway subscriptions;
its unchanged executable measures host timing noise here, not a fix-related speedup.
The +0.654-ms median difference is not evidence of a Gateway regression. Full installed
and archive deltas also include documentation, signatures and archive metadata.
No dependency changes; both stripped executable sizes decreased by only 32 bytes.
UI latency, process RSS/CPU and live event rates were not measured: this patch changes
wire subscription control only. Receiving the required guild typing subscription can
increase incoming event traffic; Serein discards unhandled typing events and clears the
subscription when the member pane closes. No live performance claim is made.

## Composer, inline editing and notifications — September 10, 2026

Baseline `efa724b` versus `feat/composer-notifications`, locked release builds on
macOS 27.0 arm64, Apple M1 Pro (8 CPU cores), 16 GB RAM, WGPU/Metal. Text and
optional voice packages were built separately with `cargo xtask package` and
`cargo xtask package-voice`. Installed bytes sum regular files inside each `.app`;
compressed bytes use `ditto -c -k --sequesterRsrc --keepParent`. These are local
ad-hoc-signed packages; archive metadata can introduce small variation.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 45,176,288 | 45,521,920 | +345,632 (+0.77%) |
| Text installed bytes | 45,476,218 | 45,842,027 | +365,809 (+0.80%) |
| Text compressed bytes | 29,533,549 | 29,690,224 | +156,675 (+0.53%) |
| Voice executable bytes | 48,060,336 | 48,384,320 | +323,984 (+0.67%) |
| Voice installed bytes | 48,591,227 | 48,937,514 | +346,287 (+0.71%) |
| Voice compressed bytes | 30,925,091 | 31,074,546 | +149,455 (+0.48%) |
| 100,000-event reducer median | 27.854 ms | 30.185 ms | +2.332 ms (+8.37%) |
| Retained timeline estimate | 220,992–221,477 B / 500 records | same | unchanged |

Replay: one warmup and five measured direct binary runs per revision, builds
completed beforehand. Baseline measured runs: 28.682, 27.871, 27.854, 27.097,
27.139 ms; after: 35.392, 33.829, 30.185, 30.032, 30.069 ms. The observed
regression includes the new bounded activity tracking (~23 ns/event at these
medians). It is a synthetic reducer measurement, not UI latency, live throughput
or RSS; concurrent desktop work and short durations add noise.

Native sampling uses the same offline chat, nominal 1120×760 viewport, 2× display
scale, external 4096×2304 display and draft `Hi <@2> 👋 🎉 <:serein_leaf:9001>`.
After at least 60 seconds of warmup, `ps` samples RSS and CPU every second for
10 samples. The final validation copy changes only the default demo argument
selection and bundle identity to prevent accidental authenticated auto-launches;
shipping package size/replay numbers above use the normal builds. System alerts
are disabled for this memory scenario. RSS excludes GPU allocations and other
OS processes; no app helper process is spawned for this scenario. Startup peak,
physical footprint, p95 frame time and OS notification service memory are unmeasured.

Native RSS samples were 113,088 KiB throughout baseline and 135,648 KiB throughout
after: +22,560 KiB (+19.95%). This is the maximum and settled RSS within the
10-second sampling window, not a startup peak. CPU median was 0.0% for both; after
had two samples at 1.7% and 2.1% (mean 0.38%), baseline all 0.0%. This shows higher
retained process memory for the rich composer scenario; no memory improvement is
claimed. The package measurements precede these final evidence paragraphs; bundled
documentation/archive metadata can change the distribution total slightly.
## September 10: integration of all open PRs (Windows)

The owner explicitly requested merging every open PR into main. Baseline is the verified feature-stack tree 0f67e69757983c52e1a45c03030a09e7c16a3b30 (same tree as consolidated 9e069ca), copied before integration. After combines that stack with main efa724b, including the already-merged Twemoji artwork/picker, grouped/hover timeline, guild voice and People subscription repair. These deltas measure the combined feature set against the incoming stack, not an individual feature or a comparison against main alone.

Both unsigned Windows packages passed. Same Windows 11 Home 10.0.26200, Ryzen 7 7800X3D / 16 logical processors, about 31 GiB RAM, Rust 1.98.1 and existing release profile; artifacts and test temporary files used the E: build directory. Text excludes the sibling voice directory; both exclude PR evidence and external WebView2/drivers. Installed/ZIP sizes reflect packaging before this final evidence addendum; ZIP uses all respective files with Python DEFLATE level 9. License files from both branches are preserved and packaged.

| Metric | Feature stack before integration | Integrated | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 42,437,632 | 49,067,008 | +6,629,376 (+15.621%) |
| Text installed package, bytes | 42,726,195 | 49,431,699 | +6,705,504 (+15.694%) |
| Text ZIP, bytes | 24,615,435 | 30,843,886 | +6,228,451 (+25.303%) |
| Voice executable, bytes | 45,750,784 | 52,400,128 | +6,649,344 (+14.534%) |
| Voice installed package, bytes | 46,262,905 | 52,988,377 | +6,725,472 (+14.538%) |
| Voice ZIP, bytes | 25,969,231 | 32,209,427 | +6,240,196 (+24.029%) |
| Reducer median, ms | 27.5046 | 26.5682 | -0.9364 (-3.40%) |
| Retained timeline estimate | 220,992-221,477 bytes | Same | 500 records |

One release reducer executable per revision, one warmup plus five direct measured runs, 100,000 synthetic events each. Baseline samples: [27.4948, 27.5046, 26.2998, 28.5269, 29.6689] ms. Integrated: [26.4394, 27.3291, 27.4944, 26.3761, 26.5682] ms. This short reducer benchmark is not native frame time, process RSS, network or audio performance; no user-visible speed claim is made. Existing per-feature native evidence is retained from its original PRs. New combined native inspection/process samples remain unverified while this session's desktop automation is paused.


## Channel visibility - September 10, 2026 (Windows)

Baseline main 92e82e7b72c716340a20c2643069c482933acd4e has the identical source tree to
tested integration a2af8d8; its verified text/voice package executables were copied into a
separate baseline before porting the visibility patch. Original dirty checkout work was
preserved. Baseline hashes: text BA52F1BD1E5E3DC6042FFBA7A670E73201095AFED6A33320013109DA267CDC1A;
voice A3668B41A112FEB9DFDF505A7D6A0C36F9C6BE9BAA4070CFD9B0BAC826626EAE.
Same Windows 11 Home 10.0.26200, Ryzen 7 7800X3D / 16 logical CPUs, approximately 31 GiB
RAM, pinned Rust 1.98.1, release profile and E: target/temp paths. Both variants are unsigned.

| Metric | Main baseline | Channel visibility fix | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 49,067,008 | 49,092,608 | +25,600 (+0.052%) |
| Text installed package, bytes | 49,431,699 | 49,466,993 | +35,294 (+0.071%) |
| Text ZIP, bytes | 30,843,886 | 30,860,898 | +17,012 (+0.055%) |
| Voice executable, bytes | 52,400,128 | 52,425,728 | +25,600 (+0.049%) |
| Voice installed package, bytes | 52,988,377 | 53,023,671 | +35,294 (+0.067%) |
| Voice ZIP, bytes | 32,209,427 | 32,225,829 | +16,402 (+0.051%) |
| Reducer median, ms | 26.2823 | 26.5260 | +0.2437 (+0.93%) |
| Retained timeline estimate | 220,992-221,477 bytes | Same | 500 records |

One release replay executable per revision, one direct warmup and five direct measured
runs of 100,000 synthetic events. Baseline samples: [26.2234, 26.2756, 26.8052, 26.2823, 28.1089] ms; after: [26.526, 26.5737, 26.175, 29.9168, 26.3162] ms.
This small median change is noisy and not evidence of a user-visible improvement or regression.
The replay exercises the reducer, not the complete Gateway obfuscation or native UI flow;
focused offline tests cover those state transitions. No RSS, idle CPU, frame/startup latency,
DPI/adapter capture, network, microphone or live audio measurements were taken while native
automation remains paused under the owner's stop.

Sizes include each complete package, excluding the sibling voice directory for text, PR
evidence, external WebView2 and audio drivers. Python ZIP DEFLATE level 9 was used consistently.
Baseline packages precede the integration evidence addendum; after packages include this task's
behavior docs but precede this final performance/progress addendum. Installed/ZIP deltas include
those documentation differences. No dependencies or license contents changed.
Final executable hashes: text FE540FEEC0781FC54D1CB3C950D53A512A96B770AF26F72D27BC49092F02BBB5;
voice 264B6EBEAF5FECBC08A33385B4F9A2073F318E3EE03D8E845D3E0A4E7191A80B.


## Permission-aware actions - September 10, 2026

Baseline is verified channel-visibility PR #17 revision
6fb81da12942b03e0a51599803d74ea37cbd3503. Its package outputs and replay executable were
copied into this task's separate baseline directory before permission implementation;
no existing unrelated debug build was used. After is feat/permission-aware-actions.
Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), about 31 GiB RAM,
Rust 1.98.1 release, wgpu. Text is --no-default-features; voice adds --features voice.
Both unsigned Windows packages were built by cargo xtask package/package-voice.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,092,608 | 49,304,064 | +211,456 (+0.431%) |
| text installed bytes | 49,466,993 | 49,690,144 | +223,151 (+0.451%) |
| text zip bytes | 30,860,898 | 30,948,093 | +87,195 (+0.283%) |
| voice executable bytes | 52,425,728 | 52,637,184 | +211,456 (+0.403%) |
| voice installed bytes | 53,023,671 | 53,247,734 | +224,063 (+0.423%) |
| voice zip bytes | 32,225,829 | 32,314,397 | +88,568 (+0.275%) |
| 100,000-event replay median ms | 27.9407 | 29.2795 | +1.3388 (+4.79%) |

Installed totals enumerate package files (32 text, 78 voice), excluding the text package's
voice sibling. ZIP is Python zipfile DEFLATE level 9. Both packages retain dependency notices
and exclude PR screenshots. Baseline packaged docs predate #17's final measurement addendum;
changed packaged docs predate this final measurement addendum. Installed/ZIP deltas therefore
include their actual documentation snapshots and are not executable-only code growth.

cargo replay builds once; each revision receives one direct warmup and five measured direct
replay-bench runs. Baseline samples (ms): 29.9405, 26.4015, 27.9407, 29.2019, 27.8303.
After samples (ms): 29.0536, 29.2975, 29.189, 29.2795, 30.2557.
Both retain exactly 500 records and 220,992-221,477 estimated timeline bytes. This measures
synthetic reducer/message admission, not role-update storms, native latency, RSS or Discord
compatibility. The new fixture explicitly supplies permission metadata. The small increase
is reported as overhead, not an improvement; run-to-run noise is visible in the samples.

Native process memory, idle CPU, startup/frame timing, display scale and GPU adapter were
not measured: owner Escape stops paused native automation. No physical UI/audio/account test
was attempted. Permission metadata plus reserved bounded decision storage has a 2 MiB
estimated budget; an update's temporary cloned candidate is additional peak allocation.
No external runtime dependency or codec was added. UI tests now depend on existing test-support.

Executable SHA256: text 991592D31AB2C330F55F97B81755ACF8DA761ACE31B7D3E32B5D4205ADA49109;
voice BCFA2FB8440261A0DA12B2995C18CAA4A44AA48EFB01C5AABFFD62844801D9A5.


## Rich editor / notification integration - September 10, 2026

Actual starting revision is 67bff236e6186a306023c156f8027712049681ba (permission PR #19).
Its text/voice executable SHA256 values were checked against the preceding recorded builds,
then only the distribution files were copied into a separate integration baseline directory.
After integrates main 2879fbc7de8fb7de30664fb64666f6d93482b827 (PR #18) plus permission-aware
notifications/inline edits and the Windows notification compile fix. This comparison includes
the newly integrated rich editor and notification dependencies; it does not isolate the cost
of those fixes or compare against a separately measured #18 build.

Same Windows 11 Home 10.0.26200 / Ryzen 7 7800X3D / 16 logical CPUs / about 31 GiB RAM /
Rust 1.98.1 release/wgpu host. Text uses no-default-features; voice adds voice. Both packaging
commands passed and produced unsigned Windows binaries. ZIP uses Python DEFLATE level 9.

| Metric / method | Before integration (67bff23) | Integrated | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,304,064 | 49,610,752 | +306,688 (+0.622%) |
| text installed bytes | 49,690,144 | 50,033,083 | +342,939 (+0.690%) |
| text zip bytes | 30,948,093 | 31,080,883 | +132,790 (+0.429%) |
| voice executable bytes | 52,637,184 | 52,943,872 | +306,688 (+0.583%) |
| voice installed bytes | 53,247,734 | 53,589,761 | +342,027 (+0.642%) |
| voice zip bytes | 32,314,397 | 32,444,614 | +130,217 (+0.403%) |
| Reducer median ms / 100,000 events | 29.2795 | 37.5618 | +8.2823 (+28.29%) |

Installed file counts are now 42 text and 88 voice (32/78 before): notification license texts,
a documentation page and the explicitly operated shortcut-registration script are included.
The script is packaged but was not run. Text excludes the voice sibling; both exclude PR
screenshots. Totals describe actual packaged documentation snapshots before this final
measurement addendum. No package was launched or claimed signed.

Reducer workload: one direct warmup plus five measured runs per revision; prior baseline
samples were recorded at the start of this continuation. Before ms: 29.0536, 29.2975, 29.189, 29.2795, 30.2557.
After ms: 37.5618, 36.5306, 37.2953, 38.116, 37.8277.
Both retain 500 records / 220,992-221,477 estimated timeline bytes. The measured increase
includes new notification observation/count work; no speed improvement is claimed. This is
synthetic reducer timing, not process RSS, role-update storms or UI frame latency. Native
RSS/idle CPU, display/GPU parameters and physical notifications remain unmeasured because
owner Escape stops paused native automation. Imported #18 macOS evidence is not evidence for
this integrated Windows executable.

Integrated executable SHA256:
- text: 0020AF6225691A06B5C45E47B4F85F7061FFBD046C92B779EFB5F8D1EE15B4E4
- voice: EAD41DAC6D98F404881987517E83FCC8E3CADBD9BCFC9514985EAD3D853DF145


## Loaded People presence - September 10, 2026

Baseline f20042a73ca101c377a4c96734aa9f0d833d6fa4 is the integrated permission/editor/notification
PR #19. Its preceding measured executable SHA256 values were verified before copying separate
text and voice distribution baselines. This branch is stacked on #19; main's later image
aspect-ratio PR #20 is outside this comparison. Same Windows 11 Home 10.0.26200 / Ryzen 7
7800X3D / 16 logical CPUs / about 31 GiB RAM / Rust 1.98.1 release/wgpu host. Both unsigned
Windows packaging commands passed; text uses no-default-features, voice adds voice.
ZIP uses Python DEFLATE level 9; text excludes the voice sibling and both exclude PR evidence.

| Metric / method | Baseline f20042a | Presence | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,610,752 | 49,624,576 | +13,824 (+0.028%) |
| text installed bytes | 50,033,083 | 50,055,923 | +22,840 (+0.046%) |
| text zip bytes | 31,080,883 | 31,088,897 | +8,014 (+0.026%) |
| voice executable bytes | 52,943,872 | 52,957,184 | +13,312 (+0.025%) |
| voice installed bytes | 53,589,761 | 53,612,089 | +22,328 (+0.042%) |
| voice zip bytes | 32,444,614 | 32,455,653 | +11,039 (+0.034%) |
| Reducer median ms / 100,000 events | 37.5618 | 37.4956 | -0.0662 (-0.176%) |

Installed file counts remain 42 text / 88 voice. Totals include the actual packaged documentation
snapshots before this measurement addendum. Executable hashes below identify the measured builds;
no package was launched, signed or published. No new external dependency or media codec was added.

Reducer workload: one direct release warmup plus five measured runs per revision, using the
existing cargo replay executable. Baseline samples: 37.5618, 36.5306, 37.2953, 38.116, 37.8277 ms.
Presence samples: 37.4956, 38.6994, 36.2119, 36.4057, 37.8734 ms. Both retain 500 records and
220,992-221,477 estimated timeline bytes. The 0.0662-ms median difference is noise, not a speed
improvement. This message-reducer workload checks for unrelated timeline regression; it does
not measure presence-event throughput, process RSS or UI frame latency. Synthetic Gateway tests
separately verify 1,000 incoming users retain at most 100 pending loaded users and an 8-KiB event,
with no deadline extension. Those assertions are component bounds, not measured load performance.

Native screenshots, display/GPU parameters, idle CPU and process RSS remain unmeasured because
native automation is paused after owner Escape stops. Presence-only events leave timeline revision
unchanged, but no native repaint-rate or physical account delivery claim is made.

Measured executable SHA256:
- text: BB8FA32318B2FC16CBBA41FD46124C499363DDF8EEFA70BF7DB24D0C11DF8AE0
- voice: E403D2AFA448FDC8B78327FACFC3B87F4AEE99D26AC53A25FC10951B658E6AC9


## Keyboard conversation switcher - September 10, 2026

Baseline f8baa2df1f705b771b5a2cd99a10bb13b4d8a4c3 is draft presence PR #21. Its recorded
text/voice executable hashes were verified before separate package baselines were copied.
This comparison excludes main's independent image-aspect-ratio PR #20. Same Windows 11 Home
10.0.26200 / Ryzen 7 7800X3D / 16 logical CPUs / about 31 GiB RAM / Rust 1.98.1 release/wgpu
host. Both unsigned Windows packaging commands passed; text uses no-default-features and
voice adds voice. ZIP uses Python DEFLATE level 9, excluding the voice sibling from text.

| Metric / method | Baseline f8baa2d | Switcher | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,624,576 | 49,658,368 | +33,792 (+0.068%) |
| text installed bytes | 50,055,923 | 50,098,393 | +42,470 (+0.085%) |
| text zip bytes | 31,088,897 | 31,104,011 | +15,114 (+0.049%) |
| voice executable bytes | 52,957,184 | 52,990,464 | +33,280 (+0.063%) |
| voice installed bytes | 53,612,089 | 53,654,047 | +41,958 (+0.078%) |
| voice zip bytes | 32,455,653 | 32,466,873 | +11,220 (+0.035%) |

Installed file counts remain 42 text / 88 voice. Neither package includes PR screenshots.
Totals describe actual packaged documentation before this measurement addendum, not a
subsequently reconstructed archive. No package was launched, signed or published. No runtime
crate, core reducer, protocol, worker, storage schema or codec was added.

The picker limits its query to 128 characters / 512 bytes and results to 20 bounded labels.
It normalizes one eligible channel's bounded metadata at a time and retains no background
index. Matching and candidate allocations occur only while open. Headless checks assert query
length/capacity after an oversized Unicode paste and verify bounded results. These are component
bounds, not measured picker latency or RSS. The existing message-reducer replay does not exercise
this UI feature, so it was not rerun as a substitute for picker measurements.

Native screenshots, input/display/GPU parameters, process RSS/idle CPU and interactive latency
remain unmeasured because desktop automation is paused after owner Escape stops. Focus cleanup
requests repaint only while this modal's closing layer remains; no continuously active timer
or search worker was added. No improvement in native latency or memory is claimed.

Measured executable SHA256:
- text: F127A6ACB7F1857A39C174BBF348603DAF623A3BE4D3BEB74FC4424103D6EF39
- voice: 8BDE5C39E0E383BA323BEB7E9021D51D640684861D2F4104F8A84B567732B8E3


## Native dependency hardening - September 10, 2026

Baseline e4ef4a852415e5061735f9183ca65dd202406ecd (PR #22), separately copied and hash-verified
before edits. Same Windows 11 Home 10.0.26200 host, Ryzen 7 7800X3D (16 logical CPUs), about
31 GiB RAM, Rust 1.98.1, release thin LTO / one codegen unit / wgpu; text default and optional
voice packages built separately. No native process was launched. One package measurement per
variant; ZIP uses Python zipfile DEFLATE level 9, excluding the voice subdirectory from text.
Both unsigned cargo xtask package and package-voice commands passed.

| Metric / method | Baseline e4ef4a8 | Dependency patch | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 49,658,368 | 49,658,368 | +0 (+0.000%) |
| text installed bytes | 50,098,393 | 50,105,951 | +7,558 (+0.015%) |
| text zip bytes | 31,104,011 | 31,106,640 | +2,629 (+0.008%) |
| voice executable bytes | 52,990,464 | 53,016,064 | +25,600 (+0.048%) |
| voice installed bytes | 53,654,047 | 53,687,168 | +33,121 (+0.062%) |
| voice zip bytes | 32,466,873 | 32,480,689 | +13,816 (+0.043%) |

Text/voice installed file counts remain 42/88. Text executable size is unchanged; the voice
executable grows 25,600 bytes (0.048%). Removing unused lockfile paths is not an executable-size
or runtime-speed optimization claim. Installed/ZIP totals include the actual staged docs and
modified HPKE source before this measurement addendum. Packages contain no PR screenshots.
Packaged HPKE source and the existing Davey MIT notice were byte-checked against the workspace.
Davey Rust source is unchanged; its manifest loses browser timer features, and the HPKE fork
loses its unused libcrux provider declaration. DAVE/MLS, SHAKE vectors and encrypted local Opus
transport pass in the 190-test workspace suite. No reducer, cache, UI or audio algorithm changes
were made, so the unrelated reducer workload was not rerun. Native CPU/RSS and real voice
latency remain unmeasured under the owner's paused desktop/live gate.

Cargo.lock package count falls from 775 to 738; strict cargo-audit 0.22.2 falls from six
vulnerability-class findings/five denied warnings to zero/two. It still fails on Linux GLib
unsoundness and proc-macro-error maintenance; no suppression or release approval is implied.

Measured executable SHA256:
- text: 16F944D856C896C9D5EC2A388E6434E20BEC5F972302B5884B56922FE770826F
- voice: 161A9A1ADD3F4A90A5B7448E4B46633ED4A7515CF993C0EA0BDC5F95F55ED8F2

## Image aspect ratios — September 10, 2026

| Metric / method | Baseline | After | Delta |
| --- | --- | --- | --- |
| text executable, bytes | 46,098,496 | 46,098,512 | +16 (+0.0000%) |
| text package, bytes | 69,734,675 | 69,736,433 | +1,758 (+0.0025%) |
| text compressed, bytes | 53,074,332 | 53,075,144 | +812 (+0.0015%) |
| voice executable, bytes | 48,917,808 | 48,917,808 | +0 (+0.0000%) |
| voice package, bytes | 74,162,600 | 74,164,342 | +1,742 (+0.0023%) |
| voice compressed, bytes | 55,695,929 | 55,697,942 | +2,013 (+0.0036%) |
| Settled RSS, KiB (`ps`, 10 × 1 s) | 152,592 | 99,328 | −53,264; noisy, no improvement claim |
| Settled CPU, median / maximum | 0 / 0% | 0 / 2.4% | median unchanged |

macOS 27.0 arm64, Apple M1 Pro (8 CPU / 14 GPU cores), 16 GB RAM, wgpu/Metal, built-in 3024×1964 Retina display at unchanged system scale; default ~1087×768 captured window. Rust 1.98.1 locked release text-only/voice packages, baseline clean `2879fbc`; native runs explicitly `--demo`, default synthetic timeline followed by opening its image viewer. Separate baseline/final app copies. Full package totals count files, excluding the nested voice distribution from text; gzip tarballs include package files. Package measurements precede appending these measurements to docs.

Ten OS RSS/CPU samples at one-second intervals after interaction settled (baseline at least 10 s; final resampled after at least 30 s). Initial final capture/packaging-period samples were 108,400–108,928 KiB and 1.7–41.1% CPU; the settled retry above is reported separately, not hidden. Different warmup/background capture/packaging activity and OS memory accounting prevent interpreting the RSS drop as an improvement. These are sampled process RSS, not allocation limits or physical footprint; startup/interaction peak, GPU allocations and p95 frame/startup latency are unmeasured. Baseline had no child processes; no voice call or audio measurement. No performance improvement claim.

## Profile popout — September 10, 2026

Same macOS arm64 host, Rust 1.98.1, release profile (thin LTO), lockfile unchanged. Baseline is `main` at `74709e2` built with `cargo build --release --locked -p serein` before editing; the after build is the same command on the task branch. Package outputs come from `cargo xtask package` / `package-voice` on the task branch only; no voice baseline was built for this task.

| Metric / method | Baseline | After | Delta |
| --- | --- | --- | --- |
| Text executable, unsigned `cargo build --release` | 46,367,952 B | 46,408,896 B | +40,944 B (+0.09%) |
| Text package executable, ad-hoc signed (`dist/Serein.app`) | not built | 46,139,168 B | — |
| Voice package executable, ad-hoc signed (`dist/voice/Serein.app`) | not built | 48,958,768 B | — |
| Full package directory (`du -sk`) text / voice | not built | 45,556 KiB / 48,652 KiB | — |
| Idle `ps` RSS, release `--demo`, 10 s warmup, 5 samples at 2 s | 153,280–153,328 KiB | 155,072–155,216 KiB | ≈ +1.2% (noise-level) |
| Idle `ps` RSS, release `--demo --demo-profile` (popout + People open) | n/a | 153,424 KiB (stable) | — |
| Sampled CPU after warmup | 0.0% | 0.0–1.0% (one 7.1% first sample) | no sustained change |

Method: wgpu/Metal renderer, 1120×792 logical window at 2× scale, no interaction after launch beyond activating the window, no network, no helper processes. RSS is whole-process resident memory of the single executable; GPU and driver allocations are not included. The two-sample difference is inside run-to-run noise and is not called a regression or improvement. Frame time and p95 latency were not instrumented. A new RAM-only profile cache holds at most 32 entries / 1 MiB estimated model bytes for 15 minutes (cleared on session changes), so reopening a card costs one clone instead of a REST round-trip. Other new retained data is bounded inside existing caps: profile ≤64 KiB (theme colors 8 B, tag ≤8 chars + 32-hex badge hash, ≤16 badge icon hashes), member custom status ≤128 chars inside the 128 KiB member budget; badge/tag artwork uses the existing 16 MiB / 64-texture allowance with 64-pixel requests.

## September 10: welcome and system messages

Baseline `6053299` (clean main, origin/main fetched and unchanged), task branch `feat/system-messages`.
macOS 27.0 (26A428), MacBookPro18,3 / Apple M1 Pro, 16 GiB, arm64, pinned Rust 1.98.1, release thin LTO / one codegen unit, wgpu. No dependency or lockfile changes.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text executable bytes | 46,141,408 | 46,159,872 | +18,464 (+0.040%) |
| text installed bytes | 46,581,502 | 46,599,966 | +18,464 (+0.040%) |
| text zip bytes | 29,911,653 | 29,918,046 | +6,393 (+0.021%) |
| voice executable bytes | 48,977,264 | 48,979,328 | +2,064 (+0.004%) |
| voice installed bytes | 49,648,319 | 49,652,424 | +4,105 (+0.008%) |
| voice zip bytes | 31,253,839 | 31,258,631 | +4,792 (+0.015%) |
| Native RSS median KiB | 153888 | 155200 | +1312 |
| Native RSS sample peak KiB | 153952 | 155584 | +1632 |
| Native CPU median % | 0 | 0 | +0 |
| Reducer replay median ms | 29.331958 | 28.687209 | -0.644749 (-2.20%) |

Both `cargo xtask package` and `cargo xtask package-voice` passed with strict ad-hoc signature verification. Package file counts are 47 text / 93 voice. Sizes are one measurement per variant: executable, sum of staged installed files, Python ZIP DEFLATE level 9. Excludes prior distribution ZIPs left in dist and the separate voice subdirectory from text. Outputs were copied to separate before/after directories; baseline voice was built from an isolated baseline checkout. Staged docs precede this measurement addendum; changed voice includes the compatibility/storage notes written between package builds. No PR evidence or debug symbols are bundled.

Native samples use the same existing `--demo --demo-chat` fixture, default viewport (observed 1087×768 screenshot), system dark appearance, no extra navigation, 10-second warmup and ten `ps -p PID -o %cpu=,rss=` samples at one-second intervals for each fresh process. Median and sample peak RSS are not launch peak, GPU memory or macOS physical footprint. CPU ranges were 0..0.2% before and 0..3.0% after; medians 0%. No login/voice helpers were started by these demo processes. Display scale was not instrumented; both runs used the same display without changing its settings. RSS rose about 1.3 MiB; this short synthetic sample does not establish long-run memory or live-channel acceptance.

Replay: `cargo replay`, then one warmup and five direct `replay-bench` runs per revision. Baseline measured runs: 29.622583, 29.331958, 29.198583, 29.183917, 29.527458 ms. After: 29.256916, 29.038542, 28.407292, 28.603875, 28.687209 ms. Both retain 500 records / 220,992–221,477 estimated bytes. These timings overlap; no speed improvement is claimed. Replay measures the existing 100,000 ordinary synthetic reducer events, not system-message rendering or SQLite migration. A stale cross-worktree Cargo artifact initially failed the changed replay compile; rebuilding the model resolved it before these measured runs.

The screenshot pair uses an identical new eight-event synthetic fixture (`--demo --demo-system-messages`) and scroll-to-top at the same viewport/appearance. Baseline screenshot was rebuilt at `6053299` with only the fixture and demo selector backported; it still discards kinds and renders the old placeholder. Package size and CPU/RSS baseline use the unmodified baseline. Dark native rendering/scrolling inspected; light/narrow rendering checked with headless egui at 280px, wide/dark at 900px. Native light-theme selection did not visibly change via automation, so native light/keyboard, display scale, p95 startup/frame latency, GPU allocation and other platforms remain unverified. No live Discord compatibility claim.

## Integration of PRs #17 through #32 with main (2026-09-10)

Baseline is published PR #32 at 57927c7, not a separately built main. Its text/voice
executables and package reports were copied and hash-verified before integration.
After includes main 85fde15 (image aspect ratio, profile popout/cache, system messages)
plus schema-union and permission/profile/presence integration repairs. Both release
package commands passed; text uses no default features, voice explicitly enables voice.

| Metric | Published PR #32 baseline | Integrated main + stack | Delta |
| --- | ---: | ---: | ---: |
| text executable, bytes | 49,855,488 | 49,938,944 | +83,456 (+0.17%) |
| text installed, bytes | 50,370,977 | 50,478,583 | +107,606 (+0.21%) |
| text zip, bytes | 31,205,377 | 31,244,819 | +39,442 (+0.13%) |
| voice executable, bytes | 53,213,184 | 53,297,152 | +83,968 (+0.16%) |
| voice installed, bytes | 53,951,446 | 54,059,969 | +108,523 (+0.20%) |
| voice zip, bytes | 32,576,903 | 32,615,326 | +38,423 (+0.12%) |
| Replay median, ms | 38.5417 | 38.4845 | -0.0572 (-0.15%; noise) |
| Retained 500-message timeline, estimated bytes | 220,992..221,477 | 228,992..229,477 | +8,000 |

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), approximately 31 GiB RAM,
Rust 1.98.1, release thin LTO/one codegen unit and wgpu. Reducer: one warmup plus five
direct executable runs, 100,000 synthetic events each. Before: 36.6465, 39.2736, 37.5642,
38.5417, 38.7553 ms; after: 42.3323, 36.6533, 36.7709, 38.4845, 38.8944 ms. The small
timing difference is noise; the new message-kind field accounts for the retained-size
increase. This workload measures neither process RSS nor native frame or startup latency.
Native automation remains owner-paused, so those metrics and new screenshots are unmeasured.
ZIPs use Python DEFLATE level 9; text excludes nested voice, both exclude PR screenshots.
Installed packages contain the documentation snapshot staged during packaging, before
this final measurement addendum. No live Discord, microphone or account test was run.


## Reply-target navigation (2026-09-10)

| Metric / method | Main 33181a0 baseline | Reply targets | Delta |
| --- | ---: | ---: | ---: |
| text executable, bytes | 49,938,944 | 49,956,352 | +17,408 (+0.03%) |
| text installed, bytes | 50,481,170 | 50,502,356 | +21,186 (+0.04%) |
| text zip, bytes | 31,245,568 | 31,255,214 | +9,646 (+0.03%) |
| voice executable, bytes | 53,297,152 | 53,314,560 | +17,408 (+0.03%) |
| voice installed, bytes | 54,059,969 | 54,083,337 | +23,368 (+0.04%) |
| voice zip, bytes | 32,615,326 | 32,627,919 | +12,593 (+0.04%) |
| 100,000-event replay median, ms | 36.6917 | 36.9486 | +0.2569 (+0.70%; noise) |
| Retained 500-message timeline, estimated bytes | 228,992..229,477 | 228,992..229,477 | 0 |

Baseline executables copied and hash-verified from the merged integration packages;
source tree equals main 33181a0. Text uses no default features; voice enables voice explicitly.
Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), approximately 31 GiB RAM,
Rust 1.98.1, release thin LTO/one codegen unit, wgpu. Replay: one warmup and five direct
executable runs on each build, 100,000 synthetic events per run. Before samples:
36.4011, 36.769, 36.6917, 37.293, 36.3099 ms; after: 36.2521, 36.2958, 36.9971, 36.9486, 36.9554 ms.
This is reducer time, not native frame/startup latency or process RSS; the small timing
difference is noise. Native desktop automation remains paused, so screenshots, idle CPU,
process/GPU memory and screen-reader/IME checks were not measured for this change.
Package ZIPs use Python DEFLATE 9. Text excludes nested voice; all packages exclude PR
screenshots. Packaged docs are the snapshot copied during packaging, before this final
measurement addendum. No live Discord, microphone or account action was performed.

## Discord-style theme shell — September 10, 2026

Baseline `85fde15` (`main`) versus branch `feat/discord-theme-shell`, same macOS 27.0 arm64 host (Apple M1 Pro, 16 GiB), pinned Rust 1.98.1, release thin-LTO profile, `cargo build --locked --release -p serein` in separate target directories.

| Metric / method | Baseline | After | Delta |
| --- | --- | --- | --- |
| Text executable (unsigned `cargo build --release`), bytes | 46,429,696 | 47,260,928 | +831,232 (+1.79%) |
| Text package executable (`cargo xtask package`, ad-hoc signed), bytes | 46,159,872 | 46,986,288 | +826,416 (+1.79%) |
| Voice package executable (`cargo xtask package-voice`, ad-hoc signed), bytes | 48,979,328 | 49,805,744 | +826,416 (+1.69%) |
| Native RSS median KiB (`--demo --demo-chat`, 10 samples) | 153,488 | 151,280 | -2,208 |
| Native RSS sample peak KiB | 153,488 | 151,280 | -2,208 |
| Native CPU median % | 0.0 | 0.0 | +0 |

The size increase is the three embedded Inter faces (799,444 bytes of OFL font data) plus the new vector icon and theme code; no dependency changed (lockfile untouched). Both text and voice packages passed strict local ad-hoc signature verification; the baseline packages were built from an isolated `main` worktree with its own target directory. Native samples: fresh release process per revision, default 1120×760 viewport, dark system appearance, Default preset, 10-second warmup, ten `ps -p PID -o %cpu=,rss=` samples at one-second intervals. Every sample was identical within each run, so the RSS difference is a single-process comparison, not a distribution; it does not establish memory behaviour for gradient presets (which add one full-window mesh per frame), long sessions or live channels. Reducer replay was not rerun: no reducer, cache or protocol code changed. p95 frame/startup latency, GPU allocation and Windows/Linux were not measured.


## Theme integration refresh — September 10, 2026

| Metric | Baseline | After | Delta |
| --- | --- | --- | --- |
| Text executable, bytes | 46,570,512 | 47,415,168 | +844,656 (+1.81%) |
| Text installed, bytes | 47,125,105 | 47,990,950 | +865,845 (+1.84%) |
| Text zip, bytes | 30,139,009 | 30,616,531 | +477,522 (+1.58%) |
| Voice executable, bytes | 49,390,112 | 50,234,752 | +844,640 (+1.71%) |
| Voice installed, bytes | 50,174,916 | 51,040,745 | +865,829 (+1.73%) |
| Voice zip, bytes | 31,484,290 | 31,959,704 | +475,414 (+1.51%) |
| CPU median, % | 0 | 0 | +0 |
| RSS median, KiB | 155152 | 154144 | -1008 |
| RSS sample peak, KiB | 155216 | 154240 | -976 |

Clean baseline `dff0975` (existing isolated worktree) versus final theme code `70e4f54`, integrated on `b92a082`. Baseline predates the incoming-typing commit; these chat fixtures contain no typing events. Same macOS 27.0 arm64, Apple M1 Pro / 16 GiB, Rust 1.98.1, release thin-LTO, wgpu/Metal, 2× capture scale, requested 1120×760 viewport. Both text/voice packages pass strict local ad-hoc signature verification; they are not notarized. Installed sizes sum files within each app bundle, ZIPs use Python DEFLATE 9, separately for text/voice. Packaged documentation reflects its build-time snapshot before this final report.

Fresh text processes use `--demo --demo-chat`, Default dark, ten-second warmup and ten one-second `ps -p PID -o %cpu=,rss=` samples, no scripted interaction. These short samples ran on a shared development host during builds; CPU varied and RSS differences are noise, not a performance improvement. No child processes were launched by these offline text fixtures. Long sessions, interactive p95 frame/startup timing, GPU allocations and live memory remain unmeasured. Gradient screenshots are visual checks, not equivalent performance comparisons. This addendum supersedes the original theme measurements for the rebased code.


## Server member identity repair — September 10, 2026

Baseline `9fcce51` versus `fix/server-member-sync`; macOS 27.0 (26A428), Apple M1 Pro,
16 GiB RAM, Rust 1.98.1, locked release profile. Both text-only and optional voice packages
were rebuilt and ad-hoc signature verification passed. Installed bytes sum all files in
the app bundle, including required resources/notices; compressed bytes use Python tarfile
`w:gz` with the bundle named `Serein.app`. Snapshot outputs were kept separately under
`target/member-sync-baseline` and `target/member-sync-after`. Bundles include documentation
at packaging time, before these final measurement notes were appended.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 47,431,712 | 47,432,928 | +1,216 (+0.003%) |
| Text installed bytes | 48,014,712 | 48,019,348 | +4,636 (+0.010%) |
| Text compressed bytes | 30,590,560 | 30,593,122 | +2,562 (+0.008%) |
| Voice executable bytes | 50,251,296 | 50,252,496 | +1,200 (+0.002%) |
| Voice installed bytes | 51,064,507 | 51,069,127 | +4,620 (+0.009%) |
| Voice compressed bytes | 31,901,498 | 31,904,374 | +2,876 (+0.009%) |
| Reducer median ms | 37.244 | 37.569 | +0.325 (+0.87%) |

`cargo replay` was built for each revision; the resulting binary ran one warmup followed
by five measured executions. Both retained 228,992–229,477 estimated timeline bytes and
500 records for 100,000 synthetic events. Baseline runs: 40.181, 37.590, 37.244, 36.541,
36.311 ms; after: 37.830, 37.724, 36.808, 37.278, 37.569 ms. The 0.325-ms median difference
is within observed run variation; no speed improvement is claimed. This message-reducer
workload does not measure member-list synchronization latency, process RSS, UI frame time
or live service behavior. No protocol range, persistent cache or dependency was added;
member snapshots remain bounded to 100 positions and 128 KiB.

### Final member decoding and queue repair — September 10, 2026

Same macOS 27.0 / M1 Pro / 16 GiB host and locked release text/voice packaging; baseline
`9fcce51`, final working tree based on `702b2ed`. Byte measurements reuse verified packages.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 47,431,712 | 47,435,472 | +3,760 |
| Text installed bytes | 48,014,712 | 48,029,580 | +14,868 |
| Text gzip distribution bytes | 30,590,560 | 30,599,671 | +9,111 |
| Voice executable bytes | 50,251,296 | 50,254,960 | +3,664 |
| Voice installed bytes | 51,064,507 | 51,079,279 | +14,772 |
| Voice gzip distribution bytes | 31,901,498 | 31,909,408 | +7,910 |
| Reducer median ms / 100,000 events | 37.244 | 51.645 | +14.401 (+38.7%) |

One warmup, five measured reducer runs: 45.286, 48.421, 59.164, 53.595, 51.645 ms.
Retained timeline bounds remain 228,992–229,477 estimated bytes / 500 records. This shared-host
measurement is materially slower than the earlier baseline and has wide variation; no speed
improvement is claimed. The changed queue and Gateway decoder are outside this reducer
workload. This is not member latency, RSS or frame timing. Reliable queue admission is tested
at 4,008 items and its unchanged 32 MiB estimated-byte ceiling; the UI still drains eight/frame.

### Member role display — September 10, 2026

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 47,435,472 | 47,476,192 | +40,720 (+0.09%) |
| Text installed bytes | 48,029,580 | 48,074,988 | +45,408 (+0.09%) |
| Text gzip distribution bytes | 30,599,671 | 30,615,929 | +16,258 (+0.05%) |
| Voice executable bytes | 50,254,960 | 50,295,552 | +40,592 (+0.08%) |
| Voice installed bytes | 51,079,279 | 51,124,559 | +45,280 (+0.09%) |
| Voice gzip distribution bytes | 31,909,408 | 31,926,452 | +17,044 (+0.05%) |
| Reducer median ms / 100,000 events | 37.213 | 38.166 | +0.953 (+2.56%) |
| Peak sampled RSS KiB | 141,072 | 154,672 | +13,600 |
| Final sampled RSS KiB | 92,752 | 152,080 | +59,328 |
| Median idle CPU % | 0.0 | 0.0 | +0.0 |

Baseline `7221390`; final role-display working tree. macOS 27.0 (26A428), Apple M1 Pro,
16 GiB RAM, Rust 1.98.1, locked text/voice release builds. Wgpu native demo at 1120×760
logical pixels, 2× display scale. Packages are verified ad-hoc builds; gzip archives include
the complete app. One warmup and five alternating baseline/after reducer runs; retained
timeline stays 228,992–229,477 estimated bytes / 500 records. No member latency/frame-time claim.

Native memory: fresh `--demo` process, automatically open member pane, 10-second warmup then
10 one-second `ps rss,%cpu` samples, no helper processes. After fixture adds two roles and two
member rows. A matched repeat followed an initial noisy comparison (peak 140,592→150,464 KiB;
final 79,936→133,600 KiB). The table records the repeat, not a selected minimum. Baseline RSS
fell sharply during sampling while the changed build stayed higher. Other worktrees were
compiling on this shared host, so memory pressure and fixture differences prevent attributing
this RSS increase solely to role code. The measured increase is material; no memory improvement
or isolated regression estimate is claimed. Startup and p95 frame latency are unmeasured.

## Voice recovery and playback completion - September 10, 2026

| Metric / method | Main 9fcce51 | Voice fixes | Delta |
| --- | ---: | ---: | ---: |
| text executable, bytes | 50,789,376 | 50,791,936 | +2,560 (+0.005%) |
| text installed, bytes | 51,365,836 | 51,371,937 | +6,101 (+0.012%) |
| text ZIP, bytes | 31,740,225 | 31,742,889 | +2,664 (+0.008%) |
| voice executable, bytes | 54,142,464 | 54,142,464 | +0 (+0.000%) |
| voice installed, bytes | 54,941,697 | 54,945,238 | +3,541 (+0.006%) |
| voice ZIP, bytes | 33,116,953 | 33,122,262 | +5,309 (+0.016%) |
| 100,000-event replay median, ms | 36.7943 | 36.6734 | -0.1209 (-0.33%; noise) |
| Retained 500-message timeline, estimated bytes | 228,992..229,477 | 228,992..229,477 | 0 |
| 1-speaker mixer, median us / 1,000 ticks | 42,850 | 41,672 | -1,178 (-2.75%; noise) |
| 8-speaker mixer, median us / 1,000 ticks | 336,640 | 333,531 | -3,109 (-0.92%; noise) |
| 63-speaker mixer, median us / 1,000 ticks | 2,758,741 | 2,661,914 | -96,827 (-3.51%; noise) |

Windows 11 Home 10.0.26200, AMD Ryzen 7 7800X3D (16 logical CPUs), approximately 31 GiB RAM,
Rust 1.98.1, release thin LTO/one codegen unit. Both unsigned Windows packages built successfully;
text uses no default features and voice explicitly enables voice. No dependency/lockfile changed;
package policy/license collection passed. Separate clean baseline and changed package directories
prevent artifact overwrite. Installed sizes sum package files, ZIPs use Python DEFLATE 9; text
excludes nested voice and both exclude PR screenshots. Packaged docs are the snapshot copied
during packaging, before this final measurement addendum.

Replay: one warmup and five direct executable runs per build. Baseline samples 36.7943,36.1554,
36.4590,39.0662,39.1083ms; after 36.6734,36.2471,37.0721,36.2804,36.8797ms. These are synthetic
reducer timings, not process RSS or UI latency. Mixer uses the existing ignored release workload:
one warmup and five 1,000-tick runs at each participant count, 20 ms Opus packets and device-free
mono 48 kHz decoding/mixing. The small decreases are not a performance improvement claim; this
shared-host run does not measure microphone/speaker callback latency, network jitter or live audio.

Encoded reorder storage remains at most 10,200 bytes per remote speaker; PCM remains 23,040 bytes
per speaker, at most 63 remote speakers. Each 20 ms mixer tick decodes at most 8 short packets per
speaker; loss concealment follows the last packet duration (up to 120 ms), at most 3 consecutive
missing packets, streamed through the existing buffer. Physical call RSS/CPU, p95 latency,
device teardown and native screenshots remain unmeasured because the owner's native/live gate
is still closed. No audio devices or live accounts were accessed.


## Voice key-package framing and startup diagnostics - September 10, 2026

| Metric / method | Main b72b3b1 | Negotiation fix | Delta |
| --- | ---: | ---: | ---: |
| text executable, bytes | 50,791,936 | 50,791,936 | +0 (+0.000%) |
| text installed, bytes | 51,371,937 | 51,377,982 | +6,045 (+0.012%) |
| text zip, bytes | 31,742,889 | 31,744,858 | +1,969 (+0.006%) |
| voice executable, bytes | 54,142,464 | 54,146,560 | +4,096 (+0.008%) |
| voice installed, bytes | 54,945,238 | 54,955,379 | +10,141 (+0.018%) |
| voice zip, bytes | 33,122,262 | 33,124,934 | +2,672 (+0.008%) |
| 100,000-event replay median, ms | 36.6734 | 36.1368 | -0.5366 (-1.46%; noise) |
| Retained 500-message timeline, estimated bytes | 228,992..229,477 | 228,992..229,477 | 0 |

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), approximately 31 GiB RAM,
Rust 1.98.1, release thin LTO/one codegen unit. Both unsigned Windows packages passed; no
dependency or lockfile changed, and existing policy/notices collection passed. Baseline
executables were reused and SHA256-verified from the previous voice-completion worktree,
whose implementation is the merged b72b3b1 source. Text uses no default features; voice
explicitly enables voice. Installed sums and DEFLATE 9 ZIPs exclude PR screenshots; text
excludes nested voice. Each package contains its build-time documentation snapshot, before
its final measurement addendum; installed deltas include documentation updates.

Replay uses one warmup and five direct executable runs per revision. Baseline measured samples
36.6734,36.2471,37.0721,36.2804,36.8797ms; new 37.9821,35.9626,36.8517,36.1368,35.8853ms.
These shared-host synthetic reducer timings are not a speedup claim or measurements of native
startup/frame latency, call CPU/RSS, network negotiation latency or physical audio quality.
Mixer/codec/queue implementations are unchanged, so their workload was not rerun. Startup
adds only bounded stage notices and a 20-second device-opening deadline. Native automation
remains owner-paused; no screenshots, microphone/speaker access or live call test was run.


## Incoming custom-status updates - September 10, 2026

| Metric / method | Main 5b2cc9e | Custom-status updates | Delta |
| --- | ---: | ---: | ---: |
| text executable, bytes | 50,791,936 | 50,805,760 | +13,824 (+0.027%) |
| text installed, bytes | 51,377,982 | 51,397,479 | +19,497 (+0.038%) |
| text zip, bytes | 31,744,858 | 31,751,925 | +7,067 (+0.022%) |
| voice executable, bytes | 54,146,560 | 54,159,872 | +13,312 (+0.025%) |
| voice installed, bytes | 54,955,379 | 54,974,364 | +18,985 (+0.035%) |
| voice zip, bytes | 33,124,934 | 33,129,218 | +4,284 (+0.013%) |
| 100,000-event replay median, ms | 36.1368 | 37.6836 | +1.5468 (+4.28%; noise) |
| Retained 500-message timeline, estimated bytes | 228,992..229,477 | 228,992..229,477 | 0 |

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), approximately 31 GiB RAM,
Rust 1.98.1, release thin LTO/one codegen unit. Both unsigned Windows packages passed.
Baseline text/voice SHA256 hashes were rechecked against the PR #39 build whose source tree
matches main 5b2cc9e. Text uses no default features; voice explicitly enables voice. Installed
sums and DEFLATE 9 ZIPs exclude PR screenshots; text excludes nested voice. Package documentation
is its build-time snapshot before this final measurement addendum; installed deltas include docs.

Replay uses one warmup and five direct executable runs per revision. Baseline samples:
37.9821,35.9626,36.8517,36.1368,35.8853ms. New samples: 37.0051,37.6836,38.9883,37.3383,38.0065ms.
These noisy shared-host reducer timings are not a speedup claim or a direct custom-status
throughput benchmark. Native RSS, idle CPU, frame/startup latency and screenshots remain
unmeasured because desktop automation is owner-paused. No live account/audio actions occurred.

Member storage remains 100 rows/128 KiB. Pending presence is at most 100 complete values with
7 status bytes and 128 characters/512 custom-text bytes each plus bounded map overhead; emitted
batches admit 64 KiB including allocated vector/string capacity. The existing global event queue
budget is unchanged. There is no new dependency, cache, persistence or subscription flag.

## Message images and continuation spacing — September 10, 2026

Baseline `c4ae54d` versus this task's changes, same Rust 1.98.1 lockfile, macOS 27.0
(26A428), Apple M1 Pro / 16 GiB, wgpu/Metal on the built-in 3024×1964 Retina display.
Release packages built with `cargo xtask package` and `cargo xtask package-voice` on both
revisions. Sizes include the locally ad-hoc signed executable, the complete package's file
bytes (text package excludes its sibling voice package), and `tar -czf` distribution.
Package snapshots precede the final evidence documentation; no dependencies were changed.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 47,434,160 | 47,451,120 | +16,960 (+0.036%) |
| Text installed files, bytes | 48,060,349 | 48,079,527 | +19,178 (+0.040%) |
| Text tar.gz, bytes | 30,648,320 | 30,658,560 | +10,240 (+0.033%) |
| Voice executable, bytes | 50,270,384 | 50,270,928 | +544 (+0.001%) |
| Voice installed files, bytes | 51,126,784 | 51,129,546 | +2,762 (+0.005%) |
| Voice tar.gz, bytes | 31,969,280 | 31,969,280 | +0 (+0.000%) |
| Peak sampled RSS, MiB | 150.52 | 131.53 | -18.98 (-12.6%) |
| Final sampled RSS, MiB | 150.52 | 124.33 | -26.19 (-17.4%) |
| Median sampled idle CPU | 0.0% | 0.0% | 0.0 percentage points |

Idle method: fresh native text-only `--demo --demo-chat` processes, default 1120×760 window,
10-second launch warmup, then ten `ps -p PID -o %cpu=,rss=` samples one second apart.
No interaction during the matched sampling window; separate native screenshot runs exercised
scrolling and the image viewer. No helper process was observed in the text demo. Exact egui
display scale, GPU memory, physical footprint, voice-call memory and p95 latency were not
instrumented. RSS is resident process memory, not a heap-retention or GPU measure.

These are single-run samples on a shared desktop, not proof of a memory improvement. An
earlier scroll/screenshot run under concurrent compilation ranged from 73.58 MiB final RSS
on baseline to 118.66 MiB after; the matched idle rerun above reversed that ordering. OS
compression, process age and desktop load make the observed RSS delta inconclusive. Both
matched samples exceed the 80 MiB idle target; no claim is made that this change meets it.
The two variants' executable deltas are below 0.04%. No frame-latency or live-media claim.
Raw samples and package counts: `docs/pr-evidence/message-images-spacing/measurements.json`.

## Unread navigation and forward history - September 10, 2026

| Metric / method | Main c4ae54d | Unread navigation | Delta |
| --- | ---: | ---: | ---: |
| text executable, bytes | 50,805,760 | 50,814,464 | +8,704 (+0.017%) |
| text installed, bytes | 51,397,479 | 51,413,035 | +15,556 (+0.030%) |
| text zip, bytes | 31,751,925 | 31,758,083 | +6,158 (+0.019%) |
| voice executable, bytes | 54,159,872 | 54,170,624 | +10,752 (+0.020%) |
| voice installed, bytes | 54,974,364 | 54,991,968 | +17,604 (+0.032%) |
| voice zip, bytes | 33,129,218 | 33,134,023 | +4,805 (+0.015%) |
| 100,000-event replay median, ms | 37.6836 | 36.6610 | -1.0226 (-2.71%; noise) |
| Retained 500-message timeline, estimated bytes | 228,992..229,477 | 228,992..229,477 | 0 |

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), approximately 31 GiB RAM,
Rust 1.98.1, release thin LTO/one codegen unit. Both unsigned Windows packages passed.
Baseline executables from PR #40 were SHA256-verified; its source tree matches main c4ae54d.
Text uses no default features; voice explicitly enables voice. Installed sums and DEFLATE 9 ZIPs
exclude PR screenshots; text excludes nested voice. Package documentation is its build-time
snapshot before final measurement addenda; installed deltas include documentation updates.

One warmup and five direct replay runs per revision. Baseline samples: 37.0051,37.6836,38.9883,37.3383,38.0065ms.
New samples: 36.0087,36.8362,36.0801,37.7125,36.661ms. This shared-host reducer workload checks the common
message path, not forward-page network latency or native rendering. Timing differences are noisy;
no speedup claim. Native RSS/CPU, startup/frame timing and screenshots remain unmeasured because
desktop automation is owner-paused. No live account or audio-device interaction occurred.

Forward history adds fixed-size cursor/flag state and reuses the 50-message response limit,
500-row/4-MiB active window, global resident ceiling and existing cancellable request worker.
Pages replace the active window. No added dependency, migration, cache or queue.


## Group mentions and silent notifications - September 10, 2026

| Metric / method | Main 9ce22a9 | Notification mentions | Delta |
| --- | ---: | ---: | ---: |
| text executable, bytes | 50,814,464 | 50,827,776 | +13,312 (+0.026%) |
| text installed, bytes | 51,413,035 | 51,433,272 | +20,237 (+0.039%) |
| text zip, bytes | 31,758,083 | 31,768,372 | +10,289 (+0.032%) |
| voice executable, bytes | 54,170,624 | 54,181,376 | +10,752 (+0.020%) |
| voice installed, bytes | 54,991,968 | 55,009,645 | +17,677 (+0.032%) |
| voice zip, bytes | 33,134,023 | 33,140,030 | +6,007 (+0.018%) |
| 100,000-event replay median, ms | 36.6610 | 39.9284 | +3.2674 (+8.91%) |
| Retained 500-message timeline, estimated bytes | 228,992..229,477 | 236,992..237,477 | +8,000 |

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), approximately 31 GiB RAM,
Rust 1.98.1, release thin LTO/one codegen unit. Both unsigned Windows packages passed.
Baseline PR #41 executables were SHA256-verified against their original recorded artifacts.
Text has no default features; voice explicitly enables voice. Installed sums and DEFLATE 9 ZIPs
exclude PR screenshots; text excludes nested voice. Bundled documentation is its build-time
snapshot before final measurement addenda; installed deltas include documentation updates.

One warmup and five direct replay runs per revision. Baseline samples:
36.0087,36.8362,36.0801,37.7125,36.661ms. New samples:
39.9284,40.3984,40.9795,39.8744,39.1995ms. The measured median increased 8.91%; this shared-host sample
cannot separate timing noise from regression and is not a native latency measurement.
Retained timeline estimates grow by 8,000 bytes for 500 records because of the new message
metadata. No process RSS, native CPU/frame/startup or OS alert measurements were made;
desktop automation remains owner-paused. No live account or audio actions occurred.

Mention roles are capped at 100 positive unique IDs and 800 retained allocation bytes per message.
Notification delivery retains at most 32 items/16 KiB including reserved queue slots and role
allocations; observed badge records remain 4,096/128 KiB. No dependency or migration was added.


## Automated dependency license policy - September 10, 2026

No application runtime or build-dependency change; Cargo.lock is unchanged. The pinned external
cargo-deny tool runs only during development/CI. Release packages and replay were not rebuilt
for this tooling-only slice. CI installation/check cost is separate from client runtime cost;
no startup, memory or package-size improvement is claimed.


## Authorized single-message deletion - September 10, 2026

| Metric / method | Main 899fca7 | Authorized deletion | Delta |
| --- | ---: | ---: | ---: |
| text executable, bytes | 50,898,944 | 50,898,432 | -512 (-0.001%) |
| text installed, bytes | 51,535,910 | 51,538,690 | +2,780 (+0.005%) |
| text zip, bytes | 31,805,467 | 31,807,111 | +1,644 (+0.005%) |
| voice executable, bytes | 54,253,568 | 54,253,056 | -512 (-0.001%) |
| voice installed, bytes | 55,113,307 | 55,116,087 | +2,780 (+0.005%) |
| voice zip, bytes | 33,177,815 | 33,178,418 | +603 (+0.002%) |
| 100,000-event replay median, ms | 39.6040 | 39.5085 | -0.0955 (-0.24%) |
| Retained 500-message timeline, estimated bytes | 236,992..237,477 | 236,992..237,477 | 0 |

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), approximately 31 GiB RAM,
Rust 1.98.1, release thin LTO/one codegen unit. Both unsigned Windows variants passed.
Baseline packages were freshly built from clean main 899fca7 in a separate worktree before
source edits. Text has no default features; voice explicitly enables voice. Installed sums and
DEFLATE 9 ZIPs exclude PR screenshots and nested voice for text. Package documentation is its
build-time snapshot before this measurement addendum; installed deltas include docs.

Initial non-interleaved samples showed a large timing difference (baseline median 77.0567 ms),
so both retained executables were remeasured under the same current host load: one warmup each,
then five alternating before/after runs. Baseline samples:
39.1752,41.5123,39.2129,39.604,39.9511ms. Changed samples:
40.9527,40.2612,38.4499,39.5085,39.2426ms. Reported medians use this paired rerun. This common-path reducer
workload does not measure deletion network latency or native confirmation rendering; small
shared-host timing differences are noisy, with no speedup claim. Retained estimates are unchanged.
No new cache, queue or dependency was added. Native RSS/CPU/frame/startup measurements and
screenshots remain unavailable while desktop automation is owner-paused. No live account or
audio action occurred.


## Offline fuzzing tools - September 10, 2026

No application runtime or dependency change; release packages and replay were not rebuilt.
The new isolated developer workspace uses libFuzzer 0.4.13 through cargo-fuzz 0.13.2 and
nightly-2026-09-09, with ASAN, optimizations, debug assertions and overflow checks. Local
Windows 11/MSVC 14.44 runs on the Ryzen 7 7800X3D host reported:

| Target | Seed cases | Executions | Initial/final coverage counters | Reported RSS | Duration |
| --- | ---: | ---: | ---: | ---: | ---: |
| decode | 19 | 158,424 | 2,469 / 5,486 | 297 MiB | 31 s |
| state_transitions | 7 | 8,842 | 2,436 / 3,339 | 382 MiB | 31 s |

These are libFuzzer counters and its reported process RSS, not repository line coverage,
client process memory or native latency. One smoke run per target, one worker, 30-second
budget (checked between executions), five-second case timeout, one-million execution and
512 MiB RSS limits. Baseline had no fuzz target, so there is no before/after speed comparison.
No crash was found in these runs. Coverage growth proves feedback-guided mutation occurred;
it does not prove all inputs are safe. Generated corpora were removed after completion.

After integrating main `4325dd1`, a second smoke run passed: decoder 112,015 executions,
coverage counters 2,489 to 5,284, reported RSS 281 MiB; state 2,892 executions, counters
2,466 to 3,227, reported RSS 350 MiB. Each ran 31 seconds. Mutation is stochastic and the
shared host was running concurrent builds; these runs are not a throughput comparison.

## Rich presence - September 10, 2026

Baseline `5f11cb92644d06e2302678211ac21d9445fad692`, clean disposable worktree;
after `feat/rich-presence`, same pinned Rust1.98.1 / x86_64-pc-windows-msvc,
release thin LTO / one codegen unit. Windows11 Home10.0.26200, Ryzen7 7800X3D,
16 logical CPUs, 31.1GiB visible RAM. Text uses no default features; voice enables `voice`.
Both `cargo xtask package` variants passed. Baseline and after dist outputs are separate.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 50,833,408 | 50,919,424 | +86,016 (0.169%) |
| Text installed bytes | 51,440,341 | 51,532,519 | +92,178 (0.179%) |
| Text ZIP DEFLATE9 bytes | 31,765,742 | 31,801,677 | +35,935 (0.113%) |
| Voice executable bytes | 54,187,520 | 54,272,512 | +84,992 (0.157%) |
| Voice installed bytes | 55,017,226 | 55,108,428 | +91,202 (0.166%) |
| Voice ZIP DEFLATE9 bytes | 33,138,688 | 33,179,757 | +41,069 (0.124%) |
| Replay median ms (paired) | 37.6737 | 37.5833 | -0.0904 (-0.240%) |
| Retained timeline estimated bytes | 228,992..229,477 | 228,992..229,477 | 0 |
| Native idle CPU / working set | 0% median / 163,385,344 bytes settled | Blocked by owner UI stop | Unmeasured |

Installed files include build-time docs (before final performance addenda); text excludes nested
voice, both exclude PR evidence. ZIP uses Python zipfile DEFLATE9. Package size deltas include
documentation changes. No new dependency or native library.

Replay uses the existing100,000-message reducer and500-row retained timeline. Each revision
was built once, then one direct warmup and five measured runs. Initial sequential medians were
36.5291ms baseline /45.9004ms after, with after samples40.7552..58.1348ms. Because of this
variance, one justified paired rerun alternated baseline and after in the same period (one warmup
each, five pairs). The table reports those paired medians; it does not establish a speedup or a
rich-activity throughput result. Background desktop work and build/cache relocation are noise
sources. Raw initial and paired results and package hashes are in the evidence measurements file.

Baseline native sample: Wgpu, default1120x760 client, dark `--demo --demo-profile`,10-second
warmup plus ten approximately1-second Process.WorkingSet64/PrivateMemorySize64/CPU samples.
CPU is delta TotalProcessorTime / wall interval, one core=100%. Peak/settled working set
163,385,344 bytes; peak/settled private394,592,256 bytes; no child processes. GPU adapter/API,
exact pixels-per-point, GPU allocations and startup/frame p95 were not instrumented. Screenshot
size1122x791 is consistent with100% scale. Owner Escape stopped Computer Use before after UI
capture/sampling; no comparable after native CPU/RAM, keyboard/scroll, light/narrow or live claim.

Activity fields/caches are explicitly bounded; see storage-policy.md. Full check passed327 tests,
strict all-feature Clippy, text-only compilation and policy. C: exhaustion was recovered by
moving this task's temporary cache and using an isolated E: build cache. A stale copied no-feature
model artifact affected the first replay build; cleaning only that task cache's release model
artifacts and rebuilding replay resolved it. Packaged variants compiled the new activity model.


Rich-presence integration with main `dc49d64`: rebuilt text/voice executables are respectively
50,987,520 / 54,337,024 bytes. One replay smoke run passed at 39.4606 ms and retained
236,992..237,477 estimated bytes / 500 records. These include main's new message/role metadata;
they are not a controlled feature delta against the earlier 5f11cb9 baseline. The original
paired measurements above remain historical pre-integration evidence. Native after sampling
remains owner-stopped; 351 integrated tests and both packages passed before the authorized merge.

### Legacy READY game presence correction

Baseline `ebad184` verified text/voice packages and replay executable were copied to a separate
E: directory before edits. Same Windows/Rust/release environment as above; no dependency change.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 50,987,520 | 50,988,032 | +512 (0.001%) |
| Text installed bytes | 51,639,942 | 51,643,051 | +3,109 (0.006%) |
| Text ZIP bytes | 31,844,323 | 31,845,218 | +895 (0.003%) |
| Voice executable bytes | 54,337,024 | 54,337,536 | +512 (0.001%) |
| Voice installed bytes | 55,212,219 | 55,216,565 | +4,346 (0.008%) |
| Voice ZIP bytes | 33,223,263 | 33,224,694 | +1,431 (0.004%) |
| Replay median ms | 40.5364 | 39.6790 | -0.8574 (-2.1%; noise) |
| Retained timeline estimated bytes / records | 236,992..237,477 / 500 | Same | 0 |

Packages include docs as of each build, before this final evidence addendum; exclude PR evidence
and exclude nested voice files from text. ZIP uses Python zipfile DEFLATE level 9. Replay is one
direct warmup per revision followed by five alternating baseline/after runs of 100,000 events.
Baseline ms: 40.5364, 40.0639, 43.1171, 39.6161, 41.6839. After ms: 43.9627, 39.6790,
38.8943, 40.7952, 39.6128. This unchanged message workload is a smoke comparison, not an activity
throughput benchmark, RSS measurement or UI latency measurement; no speedup claim. Native capture
and process sampling remain owner-paused. Both release variants and the focused Gateway Clippy
check passed, including the subsequently added loopback test source.

## Discord-style voice UI and Phosphor icon atlas - September 10, 2026

| Metric / method | Main c4ae54d | Voice UI + icons | Delta |
| --- | ---: | ---: | ---: |
| text executable, bytes | 47,434,160 | 47,544,112 | +109,952 (+0.232%) |
| text installed, bytes | 48,034,667 | 48,147,235 | +112,568 (+0.234%) |
| text zip, bytes | 30,645,493 | 30,700,479 | +54,986 (+0.179%) |
| voice executable, bytes | 50,270,384 | 50,363,856 | +93,472 (+0.186%) |
| voice installed, bytes | 51,101,102 | 51,197,190 | +96,088 (+0.188%) |
| voice zip, bytes | 31,998,719 | 32,048,840 | +50,121 (+0.157%) |
| `--demo --demo-voice` settled resident memory, MiB | 117 | 119 | +2 (+1.7%; single run) |
| `--demo --demo-voice` peak resident memory, MiB | 118 | 120 | +2 |
| `--demo --demo-voice` idle CPU, % | 0.0 | 0.0 | 0 |

macOS 27.0, Apple M1 Pro, 16 GiB RAM, Rust 1.98.1, release thin LTO/one codegen unit, wgpu Metal
renderer at 2× display scale, 1120×760 window. Both ad-hoc-signed macOS packages were built
from a disposable baseline worktree at `c4ae54d` and this branch with separate target
directories; installed sums cover `Serein.app`, ZIPs are `zip -9` of the bundle; text excludes
the nested voice package. The executable growth is the 38,534-byte icon atlas plus its index
and the new voice views. Process figures are `top -l 1` resident memory and CPU sampled once per
second for 20 s after an 8 s launch and 4 s activation warmup, one run per build, with the
window frontmost and the fixture's one-second elapsed-time repaint active. They are not p95
frame or startup latency, which remain unmeasured. The atlas texture is 655,360 decoded bytes.
Replay, mixer and codec workloads are unchanged and were not rerun; no dependency changed.

### Activity artwork in profile cards - September 10, 2026

Baseline3307396 runtime packages and reducer executable were verified in the preceding merge,
then copied into a separate E: directory before edits. After packages are from the isolated
feat/activity-artwork worktree. Same Windows/Rust1.98.1 release thin-LTO configuration; no new
dependency. The running baseline client was left open during the task.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 51,126,272 | 51,143,680 | +17,408 (+0.034%) |
| Text installed bytes | 51,791,116 | 51,818,514 | +27,398 (+0.053%) |
| Text ZIP bytes | 31,902,903 | 31,915,284 | +12,381 (+0.039%) |
| Voice executable bytes | 54,479,360 | 54,496,768 | +17,408 (+0.032%) |
| Voice installed bytes | 55,366,977 | 55,394,375 | +27,398 (+0.049%) |
| Voice ZIP bytes | 33,284,503 | 33,292,893 | +8,390 (+0.025%) |
| Reducer replay median ms | 40.3052 | 40.4948 | +0.1896 (+0.47%; noise) |
| Retained estimated timeline bytes / records | 236,992..237,477 / 500 | Same | 0 |
| Native idle CPU / process memory | Owner-paused | Owner-paused | Unmeasured |

Installed/ZIP sums exclude PR evidence and nested voice files from text; Python ZIP DEFLATE9.
Packages include docs at each build, before this addendum. Baseline package docs/notices predate
main's fuzz tooling integration, whose application runtime was unchanged. One direct warmup each
then five alternating100,000-event runs. Baseline ms:40.3052,41.6879,39.7121,40.2579,40.8136;
after ms:40.8011,40.4193,40.3958,40.4948,41.0937. The message reducer does not exercise artwork
download/decode, and this variation does not establish a regression or speedup. Image rendering,
RSS, startup and frame latency remain unmeasured because native automation remains owner-paused.
Image storage/decoding/texture caps are unchanged; activity metadata has explicit byte accounting.


## Debian/Ubuntu packages - September 10, 2026

Package implementation `2ec2cfd`, integrated with main `3307396` at `1dd12b3`.
Ubuntu 26.04 x86_64 under WSL2 (kernel 6.6.87.2-microsoft-standard-WSL2), Rust 1.98.1,
release thin LTO / one codegen unit / stripped debug info. Text has no default features;
voice enables `voice`. These measurements precede this evidence append. No runtime source or
application dependency changes were made; there was no Debian archive before this slice.

| Metric | Text | Voice | Voice minus text |
| --- | ---: | ---: | ---: |
| Executable bytes | 61,102,936 | 64,821,592 | +3,718,656 |
| Installed regular-file bytes | 61,772,329 | 65,713,975 | +3,941,646 |
| Regular payload files | 52 | 98 | +46 |
| `.deb` bytes (dpkg xz compression) | 28,056,260 | 29,256,868 | +1,200,608 |
| Declared Installed-Size (KiB including per-file rounding) | 60,369 | 64,251 | +3,882 |

The measured text package is 26.76 MiB compressed and 58.95 MiB declared installed, within
SPEC13's initial 30/75 MiB distribution/installation targets for the application payload.
System GTK/WebKit, graphics, portal, credential service and audio dependencies are declared
separately; their installed size is not included or measured here. This is not an application
RSS, CPU, frame-time, startup or live-call measurement. Existing cache/renderer budgets are unchanged.

Each archive was extracted and compared byte-for-byte with its staged payload, including the
executable; file modes, owner IDs, metadata, desktop syntax and host ELF library closure passed.
Text SHA256: `97e1541e4c868f6b3328b5a0610fbf674a65f71254a8a8c00ac3d5797d25f066`.
Voice SHA256: `95c44d46b100f86f776bda199b26f8a5d22753f66363c7e5ff4888e3f26c6473`.
Fresh source/dependency download plus the first text release took 8m41s; after integration,
release commands reported 3m14s text and 3m19s voice. These one-off shared-host build observations
exclude package-tool timing and are not a before/after performance comparison. No desktop or
audio device was opened. Minimum distro compatibility is limited by generated dependencies
(in this host's artifacts, libc6 >=2.43); older systems and actual installation remain unverified.


## Bounded Gateway diagnostics - September 10, 2026

Baseline main `34c4a8f`, compared with diagnostics implementation `238c91d` integrated at
`8b8b64c`. Windows x86_64, pinned Rust 1.98.1, existing release profile, no default features
for text and `voice` for voice. Baseline and changed packages were built in separate worktrees;
the changed build used a private target after shared-cache validation results were discarded.
Both release package commands passed. Files include documentation at build time, before this
measurement append; text excludes nested voice output and both exclude PR evidence.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 51,143,680 | 51,143,680 | 0 |
| Text installed bytes | 51,827,424 | 51,831,323 | +3,899 (+0.008%) |
| Text ZIP bytes | 31,918,267 | 31,919,983 | +1,716 (+0.005%) |
| Voice executable bytes | 54,496,768 | 54,498,304 | +1,536 (+0.003%) |
| Voice installed bytes | 55,403,285 | 55,408,720 | +5,435 (+0.010%) |
| Voice ZIP bytes | 33,295,880 | 33,297,513 | +1,633 (+0.005%) |
| Reducer replay median ms | 41.1287 | 40.1348 | -0.9939 (-2.42%; noise) |
| Retained estimated timeline bytes / records | 236,992..237,477 / 500 | Same | 0 |

Python ZIP DEFLATE9; 52 text files and 98 voice files. Direct executable replay after builds
finished: one warmup each and five alternating baseline/after runs of 100,000 events.
Baseline ms: 42.4092, 39.7967, 40.5609, 41.3075, 41.1287; after ms: 40.1348, 39.6071,
40.8045, 39.1472, 41.7168. The workload does not exercise stderr output or establish a
speedup, RSS, frame latency, startup or live compatibility. Output-sink latency is unmeasured.
Diagnostics remain default-off; each enabled scope retains only static metadata and counters,
with 64 attempted records/8 KiB across reconnects plus one desktop terminal line under 256 bytes.
Native automation remains owner-paused; no account, microphone or speaker was accessed.

## September 10, 2026 — egui main with native font fallback

Baseline: clean `3307396005f72f2e2b26946204b27991881d4ad1` (registry egui 0.36.2).
After: egui main `65e7db3c06d779c60ac56647bdd3011ed8ba1cbd`, with eframe
`system_fonts` and its color-font support. macOS 27.0, Apple M1 Pro, 16 GiB,
Rust 1.98.1, unchanged release profile, wgpu Metal. Both variants were built
and ad-hoc signature-verified with `cargo xtask package` / `package-voice`.
Baseline bundles were copied before edits to a separate ignored directory.

| Metric (bytes) | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text executable | 47,704,288 | 47,910,256 | +205,968 (+0.43%) |
| text installed | 48,380,318 | 48,588,993 | +208,675 (+0.43%) |
| text zip | 30,780,448 | 30,695,297 | -85,151 (-0.28%) |
| voice executable | 50,540,576 | 50,730,160 | +189,584 (+0.38%) |
| voice installed | 51,446,817 | 51,640,560 | +193,743 (+0.38%) |
| voice zip | 32,124,914 | 32,039,962 | -84,952 (-0.26%) |

Installed is the sum of all regular files in the complete `Serein.app` bundle,
including its staged notices/docs; ZIP is that bundle with Python zipfile
DEFLATE level 9, sorted paths and fixed timestamps. One package sample each,
taken before this final evidence addendum. System fonts/frameworks remain OS
resources outside the distribution. Smaller compressed output is not evidence
of faster rendering. Font enumeration runs upstream on a worker thread; initial
fallback may wait for it. Long-session font-cache growth, GPU allocations, p95
frame/startup latency and Windows/Linux performance remain unmeasured.

Native-font-only comparison: the intermediate egui main text build without
`system_fonts` versus the final text build, both `--demo --demo-profile`, same
1120×760-point viewport at 2× scale, no scripted input after launch. Fresh process
per build, 10-second warmup then ten `ps -p PID -o %cpu=,rss=` samples at one-second
intervals; no auth or voice helper process. This single synthetic run is noisy
and does not represent live account memory or a long-session cache bound.

| Metric | No system fonts | With system fonts | Delta |
| --- | ---: | ---: | ---: |
| Median RSS (MiB) | 160.88 | 161.97 | +1.09 |
| Peak sampled RSS (MiB) | 160.89 | 161.98 | +1.09 |
| Median idle CPU (%) | 0.00 | 0.00 | +0.00 |


### Inline audio delivery baseline (September 11, 2026)

Baseline `fd20dc9`, Rust 1.98.1, Windows 11, Ryzen 7 7800X3D, 32 GiB RAM, release
text and voice packages were built with `cargo xtask package` / `package-voice`.
Sizes in bytes (ZIP Deflate level 9; text excludes nested voice):

| Variant | Executable | Installed files | ZIP |
| --- | ---: | ---: | ---: |
| Baseline text | 51,146,752 | 51,843,054 | 31,772,865 |
| Baseline voice | 54,500,864 | 55,405,357 | 33,149,124 |

The user stopped native automation and then requested pulling main and pushing.
Final release sizes, comparable native CPU/memory samples and playback latency were
not measured; no performance improvement is claimed. The encoded/decoded ceilings
are component limits, not whole-process RAM measurements.

## September 11, 2026 - message-history scroll stability

Baseline `fd20dc90c5bf25bce1cfc313944661f973f3c9e1`; after: `d6909bd`
(scroll fix before integrating the newer DM-ordering change from main).
Windows 11 Home 10.0.26200, AMD Ryzen 7 7800X3D, 31.12 GiB reported RAM,
Rust 1.98.1, unchanged pinned egui and release profile. Both text and optional voice
packages were built with `cargo xtask package` / `cargo xtask package-voice` in
separate baseline and task worktrees. No native renderer or live account was opened.

| Metric (bytes) | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable | 51,146,752 | 51,147,776 | +1,024 (+0.002%) |
| Text installed files | 51,843,054 | 51,846,013 | +2,959 (+0.006%) |
| Text ZIP | 31,772,863 | 31,774,478 | +1,615 (+0.005%) |
| Voice executable | 54,500,864 | 54,501,888 | +1,024 (+0.002%) |
| Voice installed files | 55,405,357 | 55,408,316 | +2,959 (+0.005%) |
| Voice ZIP | 33,149,120 | 33,150,379 | +1,259 (+0.004%) |

One package sample per variant/revision. Installed size sums every regular payload
file (53 text, 99 voice), including staged docs/notices and excluding PR evidence;
text excludes the separately packaged voice directory. ZIP uses Python `zipfile`,
DEFLATE level 9, sorted paths and fixed 2026-09-11 timestamps. Measurements precede
this final evidence addendum. These tiny size deltas are not a speed improvement.
Text SHA256: `3c742d7b307b80ed66640d4bb8df2babcf0a9475666778f9f4ca86012d704915`.
Voice SHA256: `fe290e2804f24a21d922574380142dd2e34822087b8bb1fd637b98b2eba6a27c`.

The shared synthetic egui regression renders 500 messages, including compact rows
and a long wrapped message every seventh row. After eight warmup frames and eight
frames settling an anchor at message 200, it supplies 4-point wheel input for 120
upward frames, 240 downward frames and four idle frames. Timestamps advance by
1/60 second; small point deltas avoid additional wheel smoothing. One transition
frame per direction is excluded for egui's normal input-to-layout delay. It compares
the same visible message's painted text position between consecutive presentations.
Run `cargo test --locked -p ui wheel_scrolling_keeps_visible_messages_stable_during_measurement -- --nocapture`;
for the baseline, add only this test to the baseline source (the production code is unchanged).

| Maximum displacement error (points) | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| 900x600 viewport | 80 | 0 | -80 |
| 360x600 viewport | 76 | 0 | -76 |

This deterministic CPU layout check measures scroll-position stability, not UI latency,
GPU frame timing, startup time or live Discord behavior. Existing virtualized-layout
checks still prove fewer than 60 initial measured rows from a 500-row timeline.
The implementation adds no extra layout pass, cache or dependency. Native before/after
screenshots, renderer/display scale, idle CPU, peak/settled process memory and native
p95 frame time are unmeasured because native Computer Use remains owner-paused after
the earlier Escape interruption. No native performance improvement is claimed.
### Audio player styling — September 11, 2026

Same Windows 11 host, Rust 1.98.1, locked release profile; baseline `d8e3bd6`
and the final audio styling before rebasing onto `7ce5c55`. One package per variant.
These isolate the UI change; they do not measure the subsequently integrated voice changes.

| Metric (bytes) | Baseline | Styled | Delta |
| --- | ---: | ---: | ---: |
| Text executable | 51,709,440 | 51,721,216 | +11,776 |
| Text installed files | 52,809,724 | 52,821,500 | +11,776 |
| Text ZIP | 32,348,612 | 32,353,436 | +4,824 |
| Voice executable | 54,986,752 | 54,998,528 | +11,776 |
| Voice installed files | 56,192,827 | 56,204,603 | +11,776 |
| Voice ZIP | 33,647,926 | 33,650,880 | +2,954 |

Installed sums include all package files (66 text, 93 voice), excluding the separate
voice subdirectory from text. ZIP: Python zipfile, sorted paths, DEFLATE level 9,
fixed 2026-09-11 timestamps. Sizes precede this documentation addendum. No dependency
was added. CPU, memory and frame latency remain unmeasured: the baseline process
ended before sampling, and the owner subsequently stopped Computer Use with Escape.
No runtime speed or memory improvement is claimed.

### Audio attachment metadata correction — September 11, 2026

Same Windows host, pinned Rust 1.98.1, locked release profile; baseline `2b75ba3`
versus the metadata correction. One package per variant, before this documentation
addendum. Commands: `cargo xtask package` and `cargo xtask package-voice`.

| Metric (bytes) | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable | 51,844,608 | 51,844,608 | 0 |
| Text installed files | 52,957,420 | 52,957,420 | 0 |
| Text ZIP | 32,396,566 | 32,397,025 | +459 |
| Voice executable | 56,952,832 | 56,952,832 | 0 |
| Voice installed files | 58,171,435 | 58,171,435 | 0 |
| Voice ZIP | 34,498,982 | 34,498,754 | -228 |

Installed sums include all 66 text / 93 voice package files, excluding the separate
voice directory from text. ZIP uses Python zipfile, sorted paths, DEFLATE level 9
and fixed 2026-09-11 timestamps. Tiny compressed-size differences are not a runtime
improvement. No dependency or cache was added. Native CPU, RSS and frame latency
remain unmeasured because Computer Use is owner-paused; no runtime improvement is claimed.


## Repeated core lifecycle soak - September 11, 2026

Baseline `c83f973`, branch `test/repeated-session-soak`. Windows 11 Home 10.0.26200,
Ryzen 7 7800X3D, 31.1 GiB visible RAM, Rust 1.98.1, existing release profile. No UI
renderer/scale, credentials, network, disk cache, audio device or authentication webview.
`cargo build --locked --release -p replay-bench`, then direct `replay-bench.exe --soak 120`.
The workload generated 50-row pages and individual messages across 32 channels, with
128-byte/16-KiB content, under unchanged production cache and reconciliation budgets.

The completed assertion run reported 120.0218 seconds, 5,546 passes, 177,472 channel visits,
106,483,200 live inserts and 22 logout cycles. There were 88,736 row-pressure and 88,736
byte-pressure visits. After the first two passes, aggregate active/dormant history estimates
ranged from 612,632..9,366,264 bytes during small-message visits and
5,602,264..13,130,264 bytes during large-message visits, within the existing 16 MiB minus
66 KiB aggregate budget. Mixed-size dormant windows explain overlap between phases.

An external PowerShell `System.Diagnostics.Process` observer sampled this replay process's
`PrivateMemorySize64` and `WorkingSet64` every 250 ms after a five-second warmup. Each table
row covers the next 30 seconds (last row stops at process exit); min/max are sampled values,
not allocation peaks. Concurrent independent Cargo builds affected host scheduling, so this
run makes no throughput or timing improvement claim. All assertions reached the final stdout
summary and stderr was empty; the observer's Process.ExitCode property was unavailable.

| Sample window | Samples | Private bytes min..max | Working-set bytes min..max |
| --- | ---: | ---: | ---: |
| 5..35 s | 115 | 16,777,216..18,784,256 | 19,169,280..22,011,904 |
| 35..65 s | 114 | 15,364,096..18,960,384 | 18,579,456..21,966,848 |
| 65..95 s | 113 | 16,769,024..18,804,736 | 19,853,312..22,130,688 |
| 95..120 s | 96 | 17,883,136..18,804,736 | 20,275,200..22,122,496 |

The sampled upper range stabilizes across these windows. This is evidence about this bounded
synthetic core workload only, not the complete client, permanent leak freedom, UI frame times,
image/voice teardown, storage policy, other platforms or Discord interoperability. CI's
five-second smoke checks assertions; it is not a substitute for sustained process measurement.


Pre-integration release comparison against clean `c83f973`, each built in its own private target directory.
These results predate the subsequent profile, attachment, settings and timeline updates from main.
Existing release profile, text default feature set and optional `voice`; package commands
passed. ZIP is Python DEFLATE9; text excludes nested voice, and evidence/debug artifacts are
excluded. There are 65 text and 128 voice files. Sizes include documentation at package time,
before this final evidence append. No dependency versions or application features changed.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 52,111,360 | 52,112,896 | +1,536 (+0.003%) |
| Text installed bytes | 53,218,876 | 53,224,674 | +5,798 (+0.011%) |
| Text ZIP bytes | 32,495,285 | 32,496,815 | +1,530 (+0.005%) |
| Voice executable bytes | 57,218,560 | 57,221,120 | +2,560 (+0.004%) |
| Voice installed bytes | 58,629,383 | 58,636,205 | +6,822 (+0.012%) |
| Voice ZIP bytes | 34,679,077 | 34,681,298 | +2,221 (+0.006%) |
| Existing reducer replay median ms | 40.0000 | 39.6707 | -0.3293 (-0.82%; noise) |
| Retained timeline estimated bytes / rows | 236,992..237,477 / 500 | Same | 0 |

The existing 100,000-event replay used one warmup and five alternating direct executable runs:
baseline ms 42.0013, 39.8364, 39.8950, 40.0632, 40.0000; after ms 39.6707, 39.4402,
39.7760, 39.6746, 39.0214. These small differences do not demonstrate a speedup. Package/CI
repairs preserve visible behavior; the new soak is development tooling, not bundled client code.


Final integrated packages on `4b45c7e` include main `c3f1ba0` and the task repairs. Both
`cargo xtask package` and `cargo xtask package-voice` passed on the same Windows host and
private target (1m35s / 1m43s). Absolute sizes below use the same sorted DEFLATE9 method;
text excludes nested voice. Documentation is measured at packaging time, before this append.
These totals include later main UI/dependency changes, so their growth against c83 is not
attributed to the soak workload. No additional replay run is needed for the later UI-only edits.

| Final integrated metric | Text | Voice |
| --- | ---: | ---: |
| Executable bytes | 53,339,648 | 58,447,872 |
| Installed bytes | 54,456,570 | 59,868,101 |
| DEFLATE9 ZIP bytes | 33,006,303 | 35,190,914 |
| Files | 65 | 128 |

Native automation remains paused after the owner stopped it with Escape. The edit-focus
repair has headless keyboard coverage; native focus and full-client UI performance remain
unmeasured. Test-only rendering assertions now identify the actual activity texture and
keyboard navigation reaches the unread button by accessible label. Neither changes shipped
layout or adds runtime instrumentation.


### Optional outgoing game activity (September 11, 2026)

Baseline `11d0416` versus final `feat/own-activity` implementation.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text exe bytes | 53,877,248 | 53,962,240 | +84,992 (+0.16%) |
| text installed bytes | 54,997,004 | 55,085,883 | +88,879 (+0.16%) |
| text zip bytes | 33,203,594 | 33,233,978 | +30,384 (+0.09%) |
| voice exe bytes | 58,981,888 | 59,069,952 | +88,064 (+0.15%) |
| voice installed bytes | 60,404,951 | 60,500,525 | +95,574 (+0.16%) |
| voice zip bytes | 35,389,001 | 35,415,860 | +26,859 (+0.08%) |
| Default-off idle private_bytes, peak bytes | 394,285,056 | 394,502,144 | +217,088 (+0.06%) |
| Default-off idle working_set, peak bytes | 165,109,760 | 166,281,216 | +1,171,456 (+0.71%) |
| Idle CPU seconds / 10 s | 0.015625 | 0.015625 | +0.000000 |

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs), 31.1 GiB visible RAM;
RTX 5070 Ti and Radeon integrated adapters present. Rust 1.98.1, release, pinned wgpu renderer,
text-only `--demo` with sharing off, identical 1120x760 requested viewport. Native capture is
unavailable, so actual display scale/selected adapter/window visibility are unverified; both
processes were launched with the same Hidden option, not an interactive workload.
Each process warmed for 30 seconds, then ten samples at one-second intervals recorded Windows
PrivateMemorySize64, WorkingSet64 and total process CPU seconds. Settled equals peak across
these ten samples. There were no after-run child processes. Background release builds and
allocator/driver noise limit the comparison; these small differences are not an improvement
or a meaningful regression. Detection-on CPU, native frame/startup latency, real-account RSS,
GPU allocations and screenshots remain unmeasured. No live session was started.

Packages use `cargo xtask package` / `package-voice`; installed totals sum all packaged files,
text excludes nested voice, ZIP uses sorted Python zipfile DEFLATE level 9. Text/voice each
contain 65/128 files. Documentation totals are snapshots at packaging time, before this report
append. No reducer or wire parser changed, so replay-bench would not exercise this feature.


## Dependency notice assembly - September 11, 2026

Windows x86_64 MSVC / pinned Rust 1.98.1. Clean baseline `11d0416` and task tree
`chore/dependency-notice-assembly` built in the same private E: target directory, with
CARGO_INCREMENTAL=0. Both release variants use the locked no-default-features package
commands, adding `voice` only for voice. One package-size sample per variant/revision:
sorted Python zipfile DEFLATE9, text excludes nested voice, no PR evidence is bundled.
Installed/ZIP sizes include documentation at packaging time, before this evidence append.

| Metric (bytes unless files) | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text executable | 53,877,248 | 53,877,248 | +0 (+0.00%) |
| text installed | 54,997,004 | 58,677,556 | +3,680,552 (+6.69%) |
| text zip | 33,203,585 | 34,897,031 | +1,693,446 (+5.10%) |
| text files | 65 | 668 | +603 (+927.69%) |
| voice executable | 58,981,888 | 58,981,888 | +0 (+0.00%) |
| voice installed | 60,404,951 | 65,105,373 | +4,700,422 (+7.78%) |
| voice zip | 35,389,027 | 37,521,548 | +2,132,521 (+6.03%) |
| voice files | 128 | 910 | +782 (+610.94%) |

Executable sizes are unchanged. Growth is the per-build notice inventory, texts and
corresponding component sources; build dependencies are conservatively included.
Baseline Cargo release builds reported 1m37s text / 1m44s voice; after 1m42s / 1m45s.
These are single warm-cache build observations, not a statistically controlled speed claim
or isolated collector timing. No app/runtime dependencies changed. The mechanical permission
lint repair preserves outcomes; avatar changes only repair tests for the existing map cache.
Reducer, native UI, process memory and live audio were not remeasured for packaging tooling.
Linux synthetic Debian assembly passed in WSL; actual Linux/macOS packages remain CI evidence.


The notice-only after column above is revision `584163e`, before integrating later main.
Final integration includes `ea68e0e` (GIF/media/autocomplete UI work). Both Windows packages
passed again with the same toolchain/flags/size method; the following are absolute combined
measurements before this evidence append, not a notice-only growth comparison.

| Combined metric | Text | Voice |
| --- | ---: | ---: |
| Executable bytes | 53,986,304 | 59,091,968 |
| Installed bytes | 58,793,234 | 65,218,622 |
| DEFLATE9 ZIP bytes | 34,939,566 | 37,558,925 |
| Files | 668 | 910 |

Cargo reported 1m37s text / 1m42s voice. The final conflict resolution only repairs test
assumptions relative to incoming animation code; it does not change that production renderer.
No native visual or full-client performance claim is added for the separately authored main UI.


The next combined measurement includes main `d8cb031` pending-message UI plus the integration
Clippy and scroll-reflow repairs. Same Windows toolchain, flags and DEFLATE9 method as above;
measured before this evidence append. These are absolute combined sizes, not notice-only deltas.

| Combined pending-UI metric | Text | Voice |
| --- | ---: | ---: |
| Executable bytes | 54,037,504 | 59,143,168 |
| Installed bytes | 58,848,537 | 65,273,925 |
| DEFLATE9 ZIP bytes | 34,958,789 | 37,578,520 |
| Files | 668 | 910 |

Cargo release build observations: 1m40s text / 1m41s voice. The integration repair removes one
immediate layout retry; all 106 synthetic UI tests pass, including wheel displacement and
anchor preservation. Native screenshot/process/frame measurements remain owner-paused; no
native UI performance improvement is claimed.


## Optimistic message rows ? September 11, 2026

Baseline `ea68e0e9afaa822e64e6bea1e144d48816aab339`; task branch
`feat/optimistic-message-rows`. Windows 11 Home 10.0.26200, Rust 1.98.1,
x86_64-pc-windows-msvc, pinned lockfile and existing release profile. Baseline packages
were rebuilt before production edits; baseline and changed packages were kept separately.
Both `cargo xtask package` (text, no default features) and `cargo xtask package-voice`
passed before and after. One package per variant/revision; these are size measurements,
not latency or throughput measurements.

| Metric, bytes | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable | 53,986,304 | 54,011,904 | +25,600 (+0.047%) |
| Text installed, 65 files | 55,106,060 | 55,131,660 | +25,600 (+0.046%) |
| Text ZIP | 33,243,793 | 33,254,716 | +10,923 (+0.033%) |
| Voice executable | 59,091,968 | 59,117,568 | +25,600 (+0.043%) |
| Voice installed, 128 files | 60,515,031 | 60,540,631 | +25,600 (+0.042%) |
| Voice ZIP | 35,425,347 | 35,434,915 | +9,568 (+0.027%) |

Installed sums include all package files; text excludes nested `voice/`. ZIPs use Python
`zipfile`, sorted relative paths and DEFLATE level 9. Bundled documentation is the snapshot
copied during packaging, before this evidence append. No dependencies or network workers
were added. Pending bodies stay in the existing 64-item / shared 2 MiB input budget;
the UI retains only up to 64 nonce/height entries, prunes them on confirmation/channel
changes, and lays out nearby pending rows with 100-point overscan.

Native before/after CPU, memory, renderer/display-scale and frame/startup latency are
unmeasured: `orca` is not installed and the bundled Windows Computer Use API returned
`Computer Use native pipe is unavailable: failed to connect native pipe: The system
cannot find the file specified. (os error 2)`. Native screenshots could not be captured.
Headless egui tests cover dark/light wrapping and scroll behavior; they are not native
screenshots or proof of Discord compatibility. No runtime speed or memory improvement
is claimed. No account, message, microphone or call actions were performed.


The later owner-requested integration with main `d8cb031` preserves #68's shared
pending/upload renderer and removes this branch's duplicate implementation. The table
above remains historical evidence for the pre-integration revision; no size or native
performance delta is attributed to the combined implementation.


Notice-assembly integration with main `36b5c33` also preserves user context menus and DM
actions. Both combined Windows release packages passed with the same size method above.
These absolute measurements precede this evidence append and are not notice-only deltas.

| Combined user-menu metric | Text | Voice |
| --- | ---: | ---: |
| Executable bytes | 54,099,456 | 59,205,632 |
| Installed bytes | 58,929,204 | 65,355,104 |
| DEFLATE9 ZIP bytes | 34,990,295 | 37,608,788 |
| Files | 668 | 910 |

Cargo reported 1m38s text / 1m47s voice. No native UI performance claim is added.


## Chat alignment - September 11, 2026

Baseline `36b5c33180a90353ece9fde18b9c86695c1c9ef4`; branch `fix/chat-control-alignment`.
Windows 11 Home 10.0.26200, Rust 1.98.1, x86_64-pc-windows-msvc, pinned lockfile.
Both variants were rebuilt before production edits and kept separate from changed packages.
One package per variant/revision, same release profile. Main advanced during this task;
these deltas isolate alignment work against the recorded baseline, not the later main.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text exe, bytes | 54,099,456 | 54,100,480 | +1,024 (+0.002%) |
| Text installed, bytes | 55,236,270 | 55,237,294 | +1,024 (+0.002%) |
| Text zip, bytes | 33,292,420 | 33,292,471 | +51 (+0.000%) |
| Voice exe, bytes | 59,205,632 | 59,207,168 | +1,536 (+0.003%) |
| Voice installed, bytes | 60,645,753 | 60,647,289 | +1,536 (+0.003%) |
| Voice zip, bytes | 35,473,550 | 35,473,705 | +155 (+0.000%) |

`cargo xtask package` and `cargo xtask package-voice` pass before and after. Installed
sums include all 65 text / 128 voice files; text excludes the nested voice directory.
ZIPs use Python zipfile, sorted relative paths, DEFLATE level 9. Package documentation
was copied before this evidence append. Tiny size differences are not speed improvements.
No dependency changes, new caches or background work.

Synthetic egui geometry with bundled Inter fonts reproduces a -2 logical-pixel composer
text center offset at 100% scale before the fix, and centered text afterward. The new
check covers light/dark, 100/125/150/200% scale, 320/900-point widths and multiline text.
The scroll check detects an 8-point jump with original spacing and independently a
2-point jump with the original voice row height; both corrections pass.

Native screenshots and process CPU/RSS/frame/startup measurements remain unavailable:
`orca` is not installed; Windows Computer Use reports its native pipe unavailable with
OS error 2 (file not found). No runtime performance or live Discord compatibility claim.


## Game reply framing - September 11, 2026

Baseline `037a44cf8af5c8ccbbb98afd6c9042c7d8d9ca89`; branch `fix/game-presence-frames`.
Windows 11 Home 10.0.26200, Ryzen 7 7800X3D / 16 logical processors,
33,410,678,784 bytes visible RAM; Rust 1.98.1, x86_64-pc-windows-msvc.
Both baseline release variants were built before changing the reply writer and retained
separately. One locked package per revision/variant; same release profile.

| Metric (bytes) | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text exe | 54,850,048 | 54,848,000 | -2,048 (-0.004%) |
| text installed | 59,725,983 | 59,723,935 | -2,048 (-0.003%) |
| text zip | 35,281,780 | 35,281,371 | -409 (-0.001%) |
| voice exe | 59,945,984 | 59,944,448 | -1,536 (-0.003%) |
| voice installed | 66,141,643 | 66,141,182 | -461 (-0.001%) |
| voice zip | 37,897,377 | 37,896,986 | -391 (-0.001%) |

Installed sums cover 670 text / 912 voice files; text excludes the nested voice directory.
Archives use sorted paths and Python zipfile DEFLATE level 9. Voice includes the updated
compatibility/storage documentation; later performance/progress appends are not packaged.
Tiny size differences are not a performance improvement. No dependency changes.

The existing 100,000-event reducer replay was built for each revision, warmed once, then
run five times: median 43.9801 ms before / 44.9107 ms after
(+0.9306 ms, +2.12%). Both retain 500 records,
236,992..237,477 estimated timeline bytes. Warmups were 44.2414 /
45.6009 ms. This unchanged workload does not exercise local game IPC;
these noisy timings do not establish an IPC latency, CPU/RSS or frame-time change.

The focused native test observed a pending game read completing with four bytes before
the fix and the complete 240-byte reply afterward. An isolated probe of osu!'s installed
DiscordRPC.dll 1.5.0.51 decoded 0/10 split replies and 10/10 combined replies; no live
endpoint or account was used. The shared writer now assembles one temporary buffer,
bounded at 16,392 bytes per writing client (131,136 bytes across the eight-client limit).
Normal READY/ACK frames are smaller. Input limits and five-second write timeout remain.
No native UI changed; screenshots and UI-process sampling are not applicable.


## Own profile/member game activity - September 11, 2026

Baseline `5c42350`, branch `fix/own-game-presence`. Reused the verified prior task's
text/voice binaries built at `1a4a063`: both commits have identical source tree
`4bb824e85b18f1ca6db1125db7e7b0c4260b72d1`. Binary SHA-256 values and raw samples
are in `docs/pr-evidence/own-game-presence/measurements.json`. Baseline package docs
reflect the prior build's evidence snapshot; executable source equivalence was checked.
Both changed variants were rebuilt with locked Rust 1.98.1 release settings.

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D / 16 logical processors, 33,410,678,784
bytes visible RAM. Native text-only demo, wgpu (adapter unmeasured), 1400x950 pixels,
125% scale, dark theme; own profile and People open with synthetic activity.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text exe (bytes) | 54,848,000 | 54,856,192 | +8,192 (+0.01%) |
| text installed (bytes) | 59,723,935 | 59,737,666 | +13,731 (+0.02%) |
| text zip (bytes) | 35,281,371 | 35,284,568 | +3,197 (+0.01%) |
| voice exe (bytes) | 59,944,448 | 59,952,128 | +7,680 (+0.01%) |
| voice installed (bytes) | 66,141,182 | 66,153,326 | +12,144 (+0.02%) |
| voice zip (bytes) | 37,896,986 | 37,904,623 | +7,637 (+0.02%) |
| working_set_median (bytes) | 168,202,240 | 167,755,776 | -446,464 (-0.27%) |
| private_bytes_median (bytes) | 398,155,776 | 395,669,504 | -2,486,272 (-0.62%) |
| Idle CPU, one logical core (percent) | 0.1723 | 0 | -0.1723 (-100.00%) |
| Reducer replay median (ms) | 45.6102 | 40.1286 | -5.4816 (-12.02%) |

Installed sums include 670 text / 912 voice files; text excludes nested voice files.
Archives use sorted paths and Python zipfile DEFLATE level 9. Package docs were copied
before this task's final evidence appends. One package per variant/revision.

Native process samples: at least ten seconds settling, then ten samples about one second
apart. Baseline settled longer while the own card was selected; after opens it directly
with `--demo --demo-profile --demo-game-activity`. This timing/interaction difference,
normal allocator noise, and concurrent local builds limit comparison; no CPU/memory gain
is claimed. Only the demo process was sampled; child/helper memory, frame/startup p95,
GPU allocations and live-account load remain unmeasured.

Reducer replay: one warmup plus five measured runs per revision, 100,000 events each.
Both retain 500 records and 236,992..237,477 estimated timeline bytes. The baseline uses
the verified identical-tree replay executable from the prior build; after was rebuilt.
This workload does not exercise local game publication or UI; its noisy delta is not a
performance improvement claim. The local view adds one activity capped at 4 KiB retained
heap; selectors borrow it and equal activity reports do not invalidate the timeline.


## Unread/history banner fix — September 11, 2026

Baseline `690ce91`, clean task branch `t3code/fix-unread-message-banner`, Rust 1.98.1
locked release builds. macOS 27.0 (26A428), Apple M1 Pro, 16 GiB RAM. Text-only native
`--demo`, default synthetic getting-started channel, wgpu; adapter/display scale
unmeasured. Raw samples and executable/source hashes are in
`docs/pr-evidence/unread-message-banner/measurements.json`.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable (bytes) | 51,620,640 | 51,620,704 | +64 (+0.0001%) |
| Text installed package (bytes) | 58,850,069 | 58,850,133 | +64 (+0.0001%) |
| Text ZIP (bytes) | 36,630,393 | 36,631,068 | +675 (+0.0018%) |
| Median sampled RSS (KiB) | 161,328 | 157,616 | -3,712 (-2.30%) |
| Median sampled `ps %CPU` | 0.0% | 0.0% | 0 percentage points |

Packages contain 686 files each, excluding `dist/voice`; ZIP uses sorted relative paths
and Python DEFLATE level 9. Package docs were copied before these final evidence appends.
Binary/installed growth is 64 bytes; compressed variation also includes ZIP/signature
metadata. No dependencies or persistent runtime state were added.

Process sampling: separate fresh launches with 10 seconds warmup, ten samples about
one second apart via `ps -o rss=,%cpu=`; no task Cargo build during the reported samples.
Baseline sampled maximum/final RSS: 164,816 / 161,328 KiB; after: 161,104 / 157,632 KiB.
These are interval observations, not lifetime peaks or OS physical footprint. Earlier
exploratory samples during compilation and on intermediate code varied substantially;
the reported after samples use the final implementation. Foreground input and other
machine work were uncontrolled, and native viewport equality could not be verified.
The small RSS difference is not an improvement claim. Child/helper memory, compressed
memory, GPU allocation, startup/frame p95, live load and scroll latency remain unmeasured.

Both release executables build. Final voice executable is 57,249,216 bytes. Baseline
voice build terminated with signal 15 before package evidence; no voice delta is claimed.
Final voice packaging fails on existing missing exact-version license texts for
`objc2-core-media 0.3.2`, `objc2-core-video 0.3.2`, `openh264-sys2 0.9.8`, and
`openh264 0.9.8`; its complete installed/archive sizes are therefore unavailable.
Native before/after interaction evidence is blocked by ineffective input and Computer
Use `-10005: noWindowsAvailable`. The focused synthetic UI tests are separate evidence;
no live Discord compatibility or native scroll-performance claim is made.
## Voice settings design — macOS, September 11, 2026

Baseline `283686ae4dc4f9ba1b20a5e4b14ebe9ca570e5d5` versus the voice settings
redesign on `t3code/polish-voice-settings-calls`. Apple M1 Pro, MacBookPro18,3,
16 GiB RAM, macOS 27.0 (26A428), Rust 1.98.1, release profile, wgpu renderer.
No dependency, transport, codec, cache, or audio callback changes.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable, packaged bytes | 51,621,632 | 51,662,400 | +40,768 (+0.079%) |
| Text complete .app, sum of regular-file bytes | 58,811,147 | 58,854,652 | +43,505 (+0.074%) |
| Text .app, `tar -czf` bytes | 35,864,221 | 35,888,336 | +24,115 (+0.067%) |
| Voice release executable, bytes | 57,250,304 | 57,274,544 | +24,240 (+0.042%) |
| Voice complete package / archive | Blocked | Blocked | Missing existing dependency license texts |
| Early sample CPU, one core | 2.38% | 10.20% | +7.82 percentage points |
| Peak sampled RSS, KiB | 172,032 | 140,416 | -31,616 (-18.38%) |
| Last-five-sample median RSS, KiB | 122,736 | 140,416 | +17,680 (+14.41%) |

Packages were built from the recorded sources into separate baseline/changed directories;
text sums include bundled docs/licenses, before these final evidence appends. Voice
compilation succeeds on both revisions, but `cargo xtask package-voice` stops on missing
exact-version license texts for objc2-core-media/core-video 0.3.2 and
openh264/openh264-sys2 0.9.8. Partial package directories are not counted as complete
installed artifacts. No new dependency or redistribution-clearance claim is made.

Process method: launch the text package with `--demo --demo-call --demo-settings=voice`,
1120×760 window, default dark palette, 100% UI zoom (native captures are 1120×760).
Inspect the settings accessibility tree, wait ten seconds, then take 21 `ps -p PID
-o time=,rss=` readings one second apart. CPU is cumulative CPU-time delta / monotonic
wall-time delta; sample durations were 20.60 s baseline and 20.39 s after. RSS includes
this process only; `pgrep -P PID` found no child processes. Temporary copied .app bundle
names/identifiers were changed solely to isolate native automation from other Serein
instances; executable code was unchanged.

These short early samples are noisy, not a performance improvement or a sustained idle
regression claim. Concurrent local builds, delayed synthetic history/render initialization,
and OS reclamation were not controlled. The first baseline attempt measured 0% CPU and
115,456 KiB final median RSS; the repeated baseline above changed substantially. About two
minutes after launch, the final changed process reported 0.0% CPU / 116,224 KiB RSS;
a one-second `/usr/bin/sample` showed its main thread waiting in the native event loop
(physical footprint 120.5 MiB, peak 147.8 MiB). That later spot check is not a paired
benchmark. Longer controlled measurements would be needed to attribute the differences.
Startup/frame p95, GPU allocations, live calls, Windows and Linux remain unmeasured.

Native evidence is in `docs/pr-evidence/voice-settings-polish`: matching settings and
call-popup pairs, plus light-theme settings. Baseline demo hid all audio controls; after
shows disabled controls. Light/dark, 150% zoom (stacked form / bounded popup), scrolling,
Escape and navigation to the full page were inspected. These are synthetic UI checks,
not evidence of Discord voice interoperability.

## Activity privacy and opt-in tray - September 11, 2026

Baseline `381e178295d9b0f4b76d11a307d2586377364a06`; isolated branch
`fix/presence-and-tray`. Both variants rebuilt at baseline and after with locked
Rust 1.98.1 release settings. Text packaging passed. Voice executables compiled,
but both voice packaging runs failed on pre-existing missing license texts for
openh264/openh264-sys2 0.9.8; voice staging/zip figures below are incomplete, not
redistributable packages. Existing realfft 3.5.0 license-evidence warning remains.

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D / 16 logical processors,
33,410,678,784 bytes visible RAM. Text demo with Appearance open, 1400x950 pixels,
125% scale, dark theme, wgpu (adapter unmeasured). Tray off in both process samples.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| text exe (bytes) | 54,935,552 | 55,026,176 | +90,624 (+0.16%) |
| text installed (bytes) | 59,841,905 | 59,936,641 | +94,736 (+0.16%) |
| text zip (bytes) | 35,322,352 | 35,351,550 | +29,198 (+0.08%) |
| voice exe (bytes) | 60,951,040 | 61,040,128 | +89,088 (+0.15%) |
| voice installed (incomplete staging) (bytes) | 61,863,480 | 61,952,568 | +89,088 (+0.14%) |
| voice zip (incomplete staging) (bytes) | 35,967,242 | 35,986,991 | +19,749 (+0.05%) |
| working_set_median (bytes) | 167,272,448 | 168,824,832 | +1,552,384 (+0.93%) |
| private_bytes_median (bytes) | 395,055,104 | 396,783,616 | +1,728,512 (+0.44%) |
| Idle CPU, one logical core (percent) | 0.1717 | 0 | -0.1717 (-100.00%) |
| Reducer replay median (ms) | 41.6340 | 41.0937 | -0.5403 (-1.30%) |

Native sampling: at least ten seconds settling, ten samples about one second apart.
After sampling followed ordinary minimize/restore and failed attempts to focus the
owned demo window for a checkbox click; baseline did not include those interactions.
Concurrent builds and allocator noise limit comparison; no CPU/memory improvement
is claimed. Hidden/resident tray process CPU, GPU allocation, startup/frame p95,
child/helper memory, network privacy requests and live-account loads are unmeasured.

Reducer: one warmup plus five measured 100,000-event runs per revision. Both retain
500 records and 236,992..237,477 estimated timeline bytes. The workload does not
exercise the new HTTP preference or tray paths; its small timing change is noise.
Packages use sorted paths with Python zipfile DEFLATE level 9; text excludes nested
voice staging. Package docs precede final progress/performance appends. Executable
hashes, package file counts and raw samples are in
`docs/pr-evidence/presence-and-tray/measurements.json`.

Runtime bounds: one native icon/menu, three coalesced event bits, no tray timer/thread;
one cancellable preference operation and one queued boolean action; 1 MiB HTTP
response / 16 KiB retained status subtree; 64 KiB borrowed session projection and
16 entries per session/activity list. These limits are not whole-process memory.

## Mention highlighting - September 11, 2026

Compared clean baseline `0c1a43d` with `fix/mention-highlights` on Windows 11 Home,
AMD Ryzen 7 7800X3D (16 logical CPUs), 32,627,616 KiB visible RAM, Rust 1.98.1
x86_64-pc-windows-msvc. Default release text build, no default features, configured
wgpu renderer; GPU adapter and display scale unverified because native capture failed.
Package snapshots precede these documentation appends and exclude nested voice staging.
ZIP uses PowerShell `Compress-Archive -CompressionLevel Optimal` on each package.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 55,043,584 | 55,044,608 | +1,024 (+0.0019%) |
| Text installed package bytes | 59,961,138 | 59,962,162 | +1,024 (+0.0017%) |
| Text ZIP bytes | 35,539,850 | 35,540,776 | +926 (+0.0026%) |
| Settled working set bytes | 167,333,888 | 167,071,744 | -262,144 (-0.16%) |
| Peak working set bytes | 194,613,248 | 194,650,112 | +36,864 (+0.019%) |
| Settled private bytes | 395,259,904 | 395,378,688 | +118,784 (+0.030%) |
| CPU seconds over nominal 10-second sample | 0 | 0.0625 | +0.0625 |

Idle sampling: one process/run, explicit `--demo`, `Start-Process -WindowStyle Hidden`,
10-second warmup, 20 Get-Process samples spaced 500 ms apart. No scripted interaction:
the native Computer Use pipe is unavailable. These are limited idle process samples,
not proof of foreground rendering, typing latency, p95 frames, startup latency, or
GPU memory. Both runs overlapped build/test work; tiny differences are noise, not an
improvement or established regression. No helper process accounting was performed.
Raw task samples remain in the isolated worktree's ignored `target/mention-baseline`
and `target/mention-after` directories. Existing UI caches and input limits unchanged;
one background color is stored per existing composer inline slot.

Voice release compilation succeeded before and after: executable 61,057,024 ->
61,058,560 bytes (+1,536; +0.0025%). `cargo xtask package-voice` fails after compilation
on both revisions because exact license texts for openh264-sys2 0.9.8 and openh264
0.9.8 are missing. The realfft 3.5.0 incomplete upstream license-evidence warning also
remains. Full voice installed/ZIP comparison is unavailable; partial staging is not
reported as a complete package. Voice was never activated during measurement.

## Message right-click menu — September 11, 2026

Baseline `22e2283` versus `fix/message-context-menu`, Windows 11 Home,
AMD Ryzen 7 7800X3D, 16 logical CPUs, 32,627,616 KiB visible RAM,
Rust 1.98.1 x86_64-pc-windows-msvc. Release text build with no default features,
configured wgpu renderer. GPU adapter and display scale remain unverified.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable bytes | 55,044,608 | 55,047,168 | +2,560 (+0.0047%) |
| Text installed package bytes | 59,973,205 | 59,975,765 | +2,560 (+0.0043%) |
| Text ZIP bytes | 35,545,309 | 35,545,797 | +488 (+0.0014%) |
| Settled working set bytes | 172,957,696 | 167,301,120 | -5,656,576 (-3.27%) |
| Peak working set bytes | 194,273,280 | 196,333,568 | +2,060,288 (+1.06%) |
| Settled private bytes | 402,104,320 | 395,128,832 | -6,975,488 (-1.73%) |
| CPU seconds across sample interval | 0.71875 | 0.1875 | -0.53125 |

Package snapshots exclude nested voice staging and precede this documentation
update; 671 files each. PowerShell `Compress-Archive -CompressionLevel Optimal`
compressed each complete text package. Snapshots and raw idle samples are in
ignored `target/message-context-baseline` and `target/message-context-after`.

Idle method: one synthetic `--demo` process per revision, launched with
`Start-Process -WindowStyle Hidden`, 10-second warmup, then 20 `Get-Process`
samples spaced 500ms apart (about 10 seconds). Settled means the last sample;
peak is the OS process peak working set. Builds overlapped these samples.
No scripted UI interaction was possible because both native control providers
were unavailable. Child processes were not separately accounted for. These noisy
single-run idle samples establish neither an improvement nor a regression and
do not measure foreground responsiveness, startup/frame p95, or GPU memory.
The menu uses egui's existing popup state and adds no cache or dependency.

Both voice release builds compiled: executable 61,058,560 -> 61,060,608 bytes
(+2,048; +0.0034%). `cargo xtask package-voice` failed on both revisions because
exact license texts for `openh264-sys2 0.9.8` and `openh264 0.9.8` are missing.
The existing realfft 3.5.0 license-evidence warning also remains. Complete voice
installed/ZIP sizes are unavailable; partial staging is not a complete package.
No voice session or microphone was activated.

## September 11: existing DM call discovery and Join banner

Baseline 1ff190b1eafb6ff701231f56b4eff296c7d6c578 versus feature commit fe74390,
Windows 11 Home 10.0.26200 x64, Ryzen 7 7800X3D (16 logical CPUs), 31.1 GiB RAM,
Rust 1.98.1, pinned lockfile and existing optimized release profile. Baseline and
changed artifacts were built from separate worktrees and retained separately.
These comparisons precede integration of origin/main feedd94 (profile editing and CI
repairs); they isolate the original feature snapshot, not the later combined build.
No dependency or audio/codec change. Package sizes were measured before these final
performance/progress addenda; docs/pr-evidence and the sibling voice package are excluded.
ZIP uses Python zipfile DEFLATE level 9 over all 670 text-package files.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Text executable, bytes | 55,097,344 | 55,101,952 | +4,608 (+0.0084%) |
| Text installed package, bytes | 60,039,047 | 60,049,160 | +10,113 (+0.0168%) |
| Text ZIP, bytes | 35,394,655 | 35,397,727 | +3,072 (+0.0087%) |
| Voice executable, bytes | 61,114,368 | 61,123,584 | +9,216 (+0.0151%) |
| Voice full package / ZIP | Blocked | Blocked | Missing OpenH264 license texts |
| Reducer replay median, ms | 42.7112 | 43.4964 | +0.7852 (+1.84%) |
| Sampled process peak / final working set, bytes | 172,339,200 | 169,140,224 | -3,198,976 |
| Sampled peak private bytes | 400,502,784 | 396,980,224 | -3,522,560 |
| Sampled CPU time, ms | 15.625 / 10,112.516 ms | 234.375 / 10,150.817 ms | +218.750 ms |

Reducer: one warmup then five direct replay-bench runs per build, 100,000 synthetic
message events; retained timeline 236,992..237,477 estimated bytes / 500 records on both.
Baseline samples (ms): 43.9417, 41.6154, 42.7112, 47.5427, 40.7737.
After: 42.0201, 44.4412, 39.3236, 43.4964, 45.7269. This does not exercise live calls
or prove a speed improvement/regression; the change is within these short-run ranges.

Process samples: each text release launched with --demo, 8-second warmup, ten 1-second
PowerShell Process samples, no scripted input. CPU was 0.155% versus 2.309% of one logical
CPU (0.0097% versus 0.1443% of the 16-CPU machine). After working set ranged from
167,018,496 to 169,140,224 bytes, so that sample had not fully settled. These are limited
process observations, not controlled UI-idle evidence: builds were also running, native
inspection was unavailable, and viewport/display scale/occlusion were not verified.
wgpu is the configured renderer; backend, GPU memory, helpers, p95 frame/startup latency,
and scripted call-banner interaction cost are unmeasured. Do not attribute the CPU or
memory differences to this small feature without a controlled native comparison.

Both text packages and both voice release compilations passed. Both voice packaging
attempts stopped on missing exact license texts for openh264-sys2 0.9.8 and openh264 0.9.8;
the pre-existing realfft license-evidence warning also remains. No notices were bypassed.
The changed voice executable started --demo --demo-existing-call and remained responsive
with a native window; its rendered appearance could not be inspected because the Computer
Use native pipe was unavailable (Windows error 2). It was then stopped. No live account,
microphone, call, recording or non-Windows test was used. Passive call storage is bounded
to 64 channel IDs / 512 bytes plus the Vec header, with no polling or new disk storage.

## September 11: group chat menu and editor

Baseline `c6a7678` versus application commit `90215ef`, built from separate clean
worktrees with Rust 1.98.1 and the pinned lockfile on Windows 11 Home, Ryzen 7
7800X3D (16 logical CPUs), 32,627,616 KiB visible RAM. Standard release packages
include voice. Package snapshots precede this report and the fuzz-fixture-only
follow-up. PowerShell Compress-Archive compressed each fresh complete package;
stale staging from other tasks was excluded by building in clean worktrees.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable bytes | 61,676,032 | 61,890,048 | +214,016 |
| Installed package bytes | 66,152,280 | 66,368,675 | +216,395 |
| Package ZIP bytes | 39,526,857 | 39,595,728 | +68,871 |
| Demo executable bytes | 61,934,080 | 62,155,776 | +221,696 |
| Reducer replay median ms | 39.0069 | 43.3565 | +4.3496 (+11.15%) |
| Sampled peak/final private bytes | 384,749,568 | 399,253,504 | +14,503,936 |
| Process CPU seconds | 0.03125 / 10.1212 s | 1.25000 / 10.1006 s | +1.21875 s |

Reducer: one warmup then five direct release replay-bench runs per revision,
100,000 synthetic events, retaining 500 records / 236,992..237,477 estimated bytes
on both. Baseline ms: 39.8373, 39.5234, 38.7091, 38.8876, 39.0069. After ms:
43.3565, 41.6803, 43.8050, 44.4834, 40.4160. This limited sequential comparison
shows slower after samples; it does not isolate a cause or exercise menu actions.

Process method: locked release builds with `--features demo`, launched with
`--demo` and Start-Process -WindowStyle Hidden, eight-second warmup then ten
one-second Process samples; no compilation during sampling. After private bytes
rose from 397,840,384 to 399,253,504. Baseline was flat. The after demo additionally
contains the synthetic group fixture. CPU corresponds to 0.31% versus 12.38% of
one logical CPU; these uncontrolled single runs do not establish steady idle
cost or attribute the difference to the menu. Raw artifacts and scripts are in
`E:/codex-builds/group-menu-evidence` on the measurement machine.

wgpu and an initial 1120x760 viewport are configured; actual adapter, display
scale, visibility/occlusion and rendered appearance were not inspected. Native
capture failed because the Computer Use native pipe was unavailable (Windows
error 2). Menu latency, frame/startup p95, GPU memory and live interoperability
remain unmeasured. Focused headless egui tests are not native visual evidence.

Both release packages and the changed fuzz targets compile. No new dependency
or persistent cache was added. One native picker reads at most 8 MiB and decodes
off the rendering thread under 4096x4096 / 64 MiB allocation limits. The upload
is normalized to at most 256x256 / 256 KiB PNG. Only one icon picker and one group
write can be pending; core pending state retains no image payload.


### Inline Windows attachment video (September 12, 2026)

Baseline `96d9428` in a clean detached worktree; after `fix/video-attachment-playback`.
Windows x86_64, pinned Rust 1.98.1; standard `cargo xtask package` release builds,
including voice, without demo/developer features. Separate output directories;
213 installed files in each snapshot. ZIP via PowerShell `Compress-Archive` Optimal.
These package snapshots precede appending this measurement note to the bundled docs.

| Metric | Base 96d9428 | Inline video | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 62,018,048 | 62,219,264 | +201,216 (+0.324%) |
| Installed package bytes (213 files) | 66,503,839 | 66,707,110 | +203,271 (+0.306%) |
| ZIP bytes (Optimal) | 39,642,870 | 39,701,109 | +58,239 (+0.147%) |

Native screenshot/interaction service was unavailable (native pipe Windows error 2).
No CPU/RSS, GPU memory, startup/frame percentiles or visual timing comparison is claimed.
The baseline has no inline video player. Synthetic native MOV decoder and muted local
output-device checks pass, including portrait, pause/seek/cancel and video tails after
short/absent audio. This proves the local fixture path, not live Discord interoperability.
Application retention: two 1080p queued frames, one replaceable display frame and texture,
one second of PCM and a bounded pending packet, with a 16 KiB encoded range cache.
OS decoder/GPU allocations are additional. No runtime codec bundle was added.

Release native decode: one warmup and five direct test-executable runs, 40.275,
40.616, 41.409, 40.266, 40.774 ms; median **40.616 ms** for all 72 frames and AAC
samples of the three-second 320x180/24fps synthetic MOV. AMD Ryzen 7 7800X3D,
16 logical CPUs. Includes in-memory decoding and RGBA conversion; excludes
Decoder::open, seek, portrait check, network, GUI, audio output and playback timing.
The baseline lacks this player, so no comparable decode delta is asserted.


### Friends overview (September 12, 2026)

Baseline `ecf6eca` in a clean detached worktree, compared with `feat/friends-list`.
Windows x86_64, pinned Rust 1.98.1, standard `cargo xtask package` including voice,
without demo/developer features. Separate package outputs; ZIP uses PowerShell
Compress-Archive Optimal. Snapshots precede adding this note to bundled docs.

| Metric | Base ecf6eca | Friends overview | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 62,275,072 | 62,333,440 | +58,368 (+0.094%) |
| Installed bytes (214 files) | 68,385,759 | 68,444,701 | +58,942 (+0.086%) |
| ZIP bytes (Optimal) | 41,300,580 | 41,323,504 | +22,924 (+0.056%) |

The page virtualizes 64-point rows and reuses existing avatar/relationship/presence
bounds. Native capture/interaction failed with native pipe Windows error 2;
CPU/RSS, frame/startup percentiles and native rendered appearance are unmeasured.
No responsiveness improvement or live account interoperability is claimed.

Reducer control workload: 100,000 synthetic events, one warmup then five direct
release runs per revision on the same Ryzen 7 7800X3D host. Base milliseconds:
43.2221, 40.7410, 40.0848, 39.0685, 39.6544 (median 40.0848). After:
39.9235, 39.6237, 40.2908, 39.6087, 39.6787 (median 39.6787; -0.4061 ms / -1.01%).
Both retain 500 timeline records / 236,992..237,477 estimated bytes. This small
sequential difference is noise, not a demonstrated improvement. The workload
checks reducer cost; it does not exercise friend rendering or measure RSS.


## Webhook profile identification - September 12, 2026

| Metric / method | Baseline | After | Delta |
| --- | --- | --- | --- |
| serein.exe, bytes | 62,353,920 | 62,359,040 | +5,120 (+0.008%) |
| Installed package (217 files), bytes | 68,551,968 | 68,556,749 | +4,781 (+0.007%) |
| Optimal ZIP, bytes | 41,416,919 | 41,416,036 | -883 (-0.002%) |
| 100,000-event replay median, ms | 41.2358 | 39.9020 | -1.3338 (-3.23%) |

Baseline `28a73c7`; Windows x64, Ryzen 7 7800X3D, Rust 1.98.1, locked dependencies.
Both packages use `cargo xtask package`, including voice. Package snapshots precede
this measurement note. ZIPs use .NET ZipFile with Optimal compression, no enclosing folder.
Replay uses the release binary directly: one warmup and five measured runs per revision.
Before milliseconds: 41.2358, 40.9357, 41.1552, 42.4197, 41.4996. After: 44.1445, 41.145, 39.5414, 39.8563, 39.902.
Both retain 500 records / 236,992..237,477 estimated timeline bytes. Sequential timings
are subject to noise; no responsiveness improvement is claimed. This reducer workload
is not a UI frame-time, RSS, or live compatibility measurement.
Native CPU/RSS, frame/startup timing and screenshots are unmeasured: native computer-use
failed with "native pipe is unavailable ... The system cannot find the file specified.
(os error 2)". No live Discord account was used.

## Windows and Linux camera capture - September 12, 2026

Baseline `e9ce858406e5cb016b86bd4399edec8afdfd5224` versus camera implementation
`2622717` (subsequent commit changes only this evidence). Both Windows packages
use the locked standard `cargo xtask package`, including voice and existing notices.
Package snapshots precede this measurement note; 217 installed files, no PR images
or build/debug artifacts. ZIPs use .NET ZipFile Optimal compression without an
enclosing directory. No dependency versions or codec binaries were added.

| Metric / method | Baseline | After | Delta |
| --- | --- | --- | --- |
| Standard executable, bytes | 62,360,576 | 62,393,856 | +33,280 (+0.053%) |
| Full installed package, bytes | 68,560,885 | 68,598,162 | +37,277 (+0.054%) |
| Optimal ZIP, bytes | 41,418,852 | 41,434,153 | +15,301 (+0.037%) |
| Settled working set, MiB (first pair) | 196.254 | 200.695 | +4.441 (+2.26%) |
| Peak sampled working set, MiB (first pair) | 196.258 | 200.703 | +4.445 (+2.27%) |
| Settled private bytes, MiB (first pair) | 367.324 | 396.238 | +28.914 (+7.87%) |
| Peak sampled private bytes, MiB (first pair) | 367.363 | 396.277 | +28.914 (+7.87%) |
| Idle CPU, % of one logical core (first pair) | 0.232 | 2.630 | +2.398 percentage points |

Native idle sampling uses separate release builds with `--features demo`, launched
with `--demo --demo-voice`; the standard package deliberately excludes demo fixtures.
Both run the same synthetic call scene, without camera or microphone access, with
10 seconds warmup then 20 one-second process samples. The initial viewport is
1120×760 logical pixels; renderer is wgpu. Hardware: Windows 11 Home 10.0.26200,
Ryzen 7 7800X3D (16 logical processors), 33,410,678,784 bytes visible RAM, Rust 1.98.1.
Windows process WorkingSet64 and PrivateMemorySize64 are sampled separately;
CPU is process CPU time divided by elapsed wall time (100% means one logical core).
Backend adapter, actual display scale and GPU memory were not measured. No scripted
interaction was possible because the native computer-use pipe was unavailable.

The first pair sampled 20.1945/20.1962 seconds and 0.046875/0.53125 CPU seconds.
A repeat changed-build run (same 10-second warmup and 20 samples) settled at
157.102 MiB working set / 377.000 MiB private bytes and 0.232% of one core.
The second baseline process exited during sampling, so that incomplete run was
discarded. The visible variation prevents a stable idle-regression or improvement
claim; the initial higher private-memory sample is reported rather than hidden.
The changed process loaded the system MF/MFCORE libraries; their individual cost
was not isolated. No camera capture was started in any measurement.

Active capture memory/CPU, sustained video bandwidth, frame latency and Linux native
runtime are unmeasured. These idle samples cannot establish camera performance or
Discord interoperability. Native screenshots could not be inspected or exported:
the tool reported `native pipe is unavailable ... (os error 2)` after retry/reset.


### Inline video debug playback and range buffering (September 12, 2026)

Baseline `67561d3`; macOS 27.0 (26A428), Apple M1 Pro, 16 GiB RAM, Rust 1.98.1.
The reported problem used `cargo run`, so these decoder measurements intentionally use
**debug** builds. Targeted optimization now covers platform frame copies, Symphonia AAC/core,
and egui pixel conversion (epaint/ecolor), following the existing voice dev-profile policy.
Application logic remains unoptimized. No dependency was added.

A generated 12-second H.264/AAC clip contains 360 1920x1080 frames at 30 fps and
577,536 mono PCM frames at 48 kHz. Both revisions decode every frame and seek to the midpoint.
One warmup plus five measured executions of the same native decoder test, with compilation
finished; elapsed decode time comes from the test, CPU time and maximum RSS from macOS
`/usr/bin/time -l`. Hardware decoder/system-service CPU is not included in process CPU time.

| Debug decoder metric (five-run median) | Baseline | After | Delta |
| --- | --- | --- | --- |
| Decode elapsed, ms | 7,062 | 1,645 | -5,417 (-76.7%) |
| Test-process user + system CPU, seconds | 5.87 | 0.46 | -5.41 (-92.2%) |
| Test-process maximum RSS, bytes | 65,388,544 | 64,143,360 | -1,245,184 (-1.9%) |

Elapsed samples (ms): baseline 7062, 7050, 7140, 7063, 7057;
after 1571, 1645, 1669, 1677, 1630. These are local decoder measurements, **not**
whole-app CPU, GPU usage, frame latency, or live CDN playback measurements. Small memory
differences are noisy. A separate muted debug player check played the clip through EOF in
12.341 seconds; pause/seek/cancel, silent video and video continuing after short audio passed.

The credential-free HTTP reader now retains eight 256 KiB ranges (2 MiB maximum,
including the incoming range), replacing one 16 KiB slot. A loopback HTTP regression reads
64 alternating 4 KiB samples from two tracks 4 MiB apart, imposing 10 ms latency on each
response. It checks returned bytes as well as request count. One warmup and five measured
runs: baseline median 823.958 ms / 64 requests; after median 28.323 ms / 2 requests
(-96.6% elapsed, -96.9% requests). Fetched payload falls from 1 MiB to 512 KiB for that
256 KiB read workload. This is synthetic latency evidence; the memory-only cache still
refills synchronously, so sustained slow networks can still buffer. Eviction, malformed
responses and cancellation are covered by the source tests.

Linux polling now yields when an appsink has no sample, allowing the sibling bounded
queue to drain instead of blocking the player. The Linux fixture test exercises asymmetric
track consumption; native Linux execution is not established by the macOS measurements.
The worker requests clock-only repaints at 10 Hz, preserves immediate frame/state updates,
and releases its update lock before UI texture conversion. The redundant unrotated macOS
frame copy is removed. No layout change or new media persistence is introduced.

Reproduction (FFmpeg 9.0.1 used here; generated content only):

```sh
ffmpeg -f lavfi -i testsrc2=size=1920x1080:rate=30 \
  -f lavfi -i sine=frequency=440:sample_rate=48000 -t 12 \
  -c:v libx264 -preset ultrafast -crf 25 -pix_fmt yuv420p \
  -c:a aac -movflags +faststart /tmp/serein-video-1080p.mp4
SEREIN_VIDEO_SAMPLE=/tmp/serein-video-1080p.mp4 cargo test --locked -p platform \
  decodes_local_sample -- --ignored --nocapture
SEREIN_VIDEO_SAMPLE=/tmp/serein-video-1080p.mp4 cargo test --locked -p serein \
  --features demo local_video_keeps_up_with_realtime -- --ignored --nocapture
cargo test --locked -p serein alternating_tracks_reuse_buffered_ranges -- --nocapture
```

For medians, build once and invoke each emitted test executable directly, one warmup and
five measured runs. The player test is explicitly ignored by default because it opens the
local output device at zero volume. It never uses an account or microphone.

## Friend requests — September 12, 2026

Baseline `179484c7353a1b1379df1b8581f8e0c3c940ca0a`, compared with the friend-request
implementation on Windows / Ryzen 7 7800X3D / approximately 32 GB RAM. Pinned release
toolchain, no dependencies added. Build `cargo replay` once per revision, then run
`target/release/replay-bench.exe` directly: one warmup and five measured runs.
Baseline median 40.8538 ms; after 39.9599 ms (-0.8939 ms / -2.19%). Samples span
39.4142–41.0634 ms before and 39.3734–42.0728 ms after: overlapping ranges, not a
performance improvement claim. Both retain 236,992–237,477 estimated timeline bytes
and 500 records after 100,000 synthetic messages. This existing workload does not
measure friend-request throughput, UI latency or process RSS.

Pending request profiles are separately bounded to 4,000 entries / 2 MiB; only visible
72-point rows are rendered. Native before/after screenshot control and scripted process
sampling are unavailable in this session, so CPU, RSS and frame metrics are unmeasured.

Standard `cargo xtask package` including voice, separate baseline/after snapshots:

| Metric (bytes) | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 62,854,656 | 62,923,264 | +68,608 (+0.109%) |
| Installed package | 73,055,652 | 73,126,738 | +71,086 (+0.097%) |
| PowerShell Compress-Archive ZIP | 43,465,392 | 43,488,770 | +23,378 (+0.054%) |

Both snapshots contain 823 generated files, excluding an unrelated obsolete `dist/voice`
directory left untouched. Voice is compiled into the standard executable. Source/notices
and documentation are included; these measurements precede adding this result table.
The separate debug `--demo --demo-friends` process launched and remained responsive;
that establishes startup only, not visual correctness or native interaction coverage.

## Additional system message types — September 12, 2026

Baseline `f359777a542807b483a7c9e40b00522e6fb13888`, Windows / Ryzen 7 7800X3D /
approximately 32 GB RAM / Rust 1.98.1. Standard `cargo xtask package`, including voice;
separate baseline/after snapshots, compressed with PowerShell `Compress-Archive`.

| Metric (bytes) | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 62,923,264 | 62,928,384 | +5,120 (+0.008%) |
| Installed package | 73,127,597 | 73,133,926 | +6,329 (+0.009%) |
| ZIP | 43,489,018 | 43,491,438 | +2,420 (+0.006%) |

Both snapshots exclude an unrelated obsolete `dist/voice` directory; voice is linked
in the executable. Measurements precede this result table. No dependencies, retained
fields, caches, wire parsing or reducer logic change: only bounded system descriptions
and existing gutter-icon selection. The synthetic UI check covers 12 rows at 280/900-point
widths, light/dark, in a tall viewport for full-fixture coverage. Native screenshot/control
is unavailable; process CPU/RSS/frame and visual comparison remain unmeasured. No speed
improvement or live Discord interoperability is claimed.

## Private contact editors and status menus — September 12, 2026

Baseline `073f0d23adb85c37b9a3779bd30da05ff05d3480`; Windows, Ryzen 7 7800X3D,
33,410,678,784 bytes installed RAM, Rust 1.98.1. Standard release package includes voice.
Separate baseline/after snapshots each contain 823 files; the unrelated obsolete
`dist/voice` directory was excluded and left untouched. Measurements precede this table.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 62,928,384 | 63,074,304 | +145,920 (+0.232%) |
| Installed bytes | 73,135,118 | 73,283,035 | +147,917 (+0.202%) |
| PowerShell Compress-Archive ZIP bytes | 43,491,755 | 43,539,208 | +47,453 (+0.109%) |
| 100,000-event reducer replay median | 41.4574 ms | 41.2044 ms | -0.2530 ms (-0.610%; noise) |
| Retained timeline estimated bytes | 236,992–237,477 | 236,992–237,477 | unchanged, 500 records |

Replay binaries were built from their respective revisions in isolated Cargo targets.
One warmup each, then five alternating baseline/after runs, with no concurrent task build.
Baseline samples (ms): 42.0041, 43.5626, 41.2310, 41.4574, 40.9729.
After samples (ms): 41.1566, 47.1019, 40.1776, 41.2044, 41.5186.
The small median difference is not a speed improvement claim. This existing reducer
workload does not exercise note saves, status menus, UI timing or process RSS.
Native CPU/memory/frame and before/after visual inspection are unmeasured: native
application controls are unavailable. Offline interaction tests are not screenshots
or proof of real Discord persistence.

## Account-type badges - September 12, 2026

Baseline `72c24cd`; Windows, Ryzen 7 7800X3D, 33,410,678,784 bytes RAM,
Rust 1.98.1. Standard release packages include voice. Separate snapshots exclude
the unrelated obsolete `dist/voice` directory, left untouched.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 63,087,104 | 63,095,808 | +8,704 (+0.014%) |
| Installed bytes | 73,302,329 | 73,312,974 | +10,645 (+0.015%) |
| Compress-Archive ZIP bytes | 43,542,936 | 43,546,477 | +3,541 (+0.008%) |
| 100,000-event reducer median | 42.1168 ms | 42.6705 ms | +0.5537 ms (+1.315%) |
| Retained estimated bytes | 236,992-237,477 | 236,992-237,477 | unchanged, 500 records |

Isolated revision-specific release binaries; one warmup each then five alternating
runs with no concurrent task build. Baseline samples (ms): 42.1168, 41.9057,
40.3950, 42.2955, 42.3955. After: 42.2008, 44.7333, 42.6705, 46.0183, 42.6672.
This small difference is within observed run variability, not a speed claim.
The reducer workload does not measure badge rendering or SQLite migration.
Native `--demo` startup was responsive; it is not visual proof of the new badges.
Native screenshot/control is unavailable, so before/after inspection, CPU/RSS and
p95 UI frame comparisons remain unmeasured. Synthetic UI tests cover both surfaces,
long names and light/dark layouts; no live Discord compatibility is claimed.

## Member badge row overlap - September 12, 2026

Baseline `d7d96b5`; Windows, Ryzen 7 7800X3D, 33,410,678,784 bytes RAM,
Rust 1.98.1. Rebuilt standard release packages include voice; separate snapshots
exclude the unrelated obsolete `dist/voice` directory, which was left untouched.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 63,095,808 | 63,095,808 | 0 |
| Installed bytes | 73,314,466 | 73,314,466 | 0 |
| Compress-Archive ZIP bytes | 43,546,839 | 43,546,583 | -256 (-0.0006%) |

The only runtime change gives the member name/badge line an explicit 18-point
height instead of the 32-point interactive minimum. Virtualized 42-point rows,
retained state, caches and dependency set are unchanged. A synthetic geometry
regression test reproduced a subtitle ending 10 points beyond its row before the
fix and passed after it at 180/240-point widths, light/dark and hover/no-hover.
This is not a native screenshot or timing benchmark. Native application control
is unavailable; CPU/RSS/p95 frame comparisons and native visual inspection remain
unmeasured. No speed or live-service compatibility improvement is claimed.

## DM sidebar scrolling and stacked group avatars - September 12, 2026

Baseline `0e355da`; Windows, Ryzen 7 7800X3D, 33,410,678,784 bytes RAM,
Rust 1.98.1. Standard release packages include voice; separate snapshots exclude
the unrelated obsolete `dist/voice` directory, which remains untouched.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 63,095,808 | 63,097,856 | +2,048 (+0.0032%) |
| Installed bytes | 73,315,659 | 73,317,707 | +2,048 (+0.0028%) |
| Compress-Archive ZIP bytes | 43,546,895 | 43,547,211 | +316 (+0.0007%) |

Friends and the DM section heading are two prefix rows in the existing virtualized
list. No additional scroller, cached row collection or per-frame full-list traversal.
Groups without custom icons paint at most two member avatars via the existing bounded
avatar cache/request path; cold groups may fetch those two images. No new dependencies,
cache limits, storage fields or network transports. Synthetic tests check pinned-search
geometry, Friends navigation, bounded visible rows and stacked artwork at 24/32/144 points.
Native screenshot/control is unavailable, so visual inspection and CPU/RSS/p95 frame
comparisons remain unmeasured. These tests do not establish native smoothness or live
Discord interoperability; no speed improvement is claimed.

## Clickable hand cursor - September 12, 2026

Baseline `ec0ce95`; Windows, Ryzen 7 7800X3D, 33,410,678,784 bytes RAM,
Rust 1.98.1. Standard release packages include voice; separate snapshots exclude
the unrelated obsolete `dist/voice` directory, left untouched.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 63,097,856 | 63,100,928 | +3,072 (+0.0049%) |
| Installed bytes | 73,319,066 | 73,322,138 | +3,072 (+0.0042%) |
| Compress-Archive ZIP bytes | 43,547,590 | 43,548,491 | +901 (+0.0021%) |

Standard buttons use egui's cursor style. The stateless custom-control hook is
registered once per context and returns immediately when an explicit cursor is
already chosen; otherwise it inspects only egui's small hovered-widget set.
No full-widget traversal, retained cache, new dependency or repaint request.
Synthetic tests verify hand/default/text/resize/grab output in both themes,
including disabled controls and repeated theme application. Native cursor capture,
CPU/RSS and p95 frame measurements are unavailable because native application
control is disabled. No native latency or speed improvement is claimed.

## Solo call camera and screen previews - September 12, 2026

Baseline `ec0ce9553dac516fdccd678958be94d1cf1996cf`; Windows 11 Home,
Ryzen 7 7800X3D, 33,410,678,784 bytes RAM, Rust 1.98.1. Separate clean
`cargo xtask package` snapshots use standard release builds including voice,
without default features. Package snapshots include the behavior documentation
but precede these refreshed measurement values. After-build includes main's
cursor/Homebrew changes through `02a859f`; the delta includes those changes. ZIPs use PowerShell Compress-Archive.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 63,097,856 | 63,091,200 | -6,656 (-0.0105%) |
| Installed package bytes | 69,337,029 | 69,334,506 | -2,523 (-0.0036%) |
| Compress-Archive ZIP bytes | 41,683,608 | 41,688,126 | +4,518 (+0.0108%) |

These small size changes are not a performance improvement claim. Shared
DM/guild tiles replace the separate DM avatar renderer. Local screen capture now
runs after the user starts sharing even without a peer; its worker retains one
preview up to 640x360 RGBA (921,600 bytes), replaces it at most 10 times per second,
and uses the existing bounded native raw-frame channel. UI texture/upload copies
are additional. Encoding/transmission still wait for secure readiness.

Synthetic checks cover both preview textures in empty DM/guild rosters at
320/900-point widths, light/dark, waiting/connected/failed, preview byte/color
validation, and camera rekeys. Native screenshot/interaction tools were unavailable
(missing Computer Use pipe; no Orca CLI), so native CPU, RSS, GPU, startup and frame
timings are unmeasured. No real device or Discord call was exercised.

## Extension shop previews - September 12, 2026

Baseline `d00bd04`, after `ae697f4` (includes main through `65c5f4b`);
Windows 11, Ryzen 7 7800X3D, Rust 1.98.1, release/wgpu. Standard packages
include voice. Package snapshots precede this note; development evidence and
preview PNGs are not bundled in the standard executable/package.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 65,071,104 | 65,398,272 | +327,168 (+0.503%) |
| Installed bytes | 71,389,398 | 71,719,314 | +329,916 (+0.462%) |
| Compress-Archive ZIP bytes | 42,431,635 | 42,538,459 | +106,824 (+0.252%) |
| Native capture workload peak WorkingSet64, median bytes | 232,865,792 | 234,192,896 | +1,327,104 (+0.57%) |
| Sampled process CPU, median ms | 406.250 | 437.500 | +31.250 |

Native method: existing `profile_preview --demo --page=extensions`, 1120x760
logical / 1400x950 framebuffer, 125% display scale. One warmup per variant,
then five alternating launches, 100 ms WorkingSet64/TotalProcessorTime samples
until the fixture exits. This measures synthetic startup, rendering, GPU readback
and PNG writing together, not idle CPU, interactive startup p95 or frame p95.
Both executables ran from C:, with no concurrent Cargo builds. GPU allocations
and child processes are not included. Completion was verified using each saved
1400x950 PNG and stdout marker; PowerShell's polled ExitCode was unavailable.
Sampled peaks ranged from 160 to 234 MB across the series; 100 ms sampling
can miss brief PNG-write peaks. The median difference is within this noise,
not evidence of a precise RAM regression or speed improvement.

The baseline example harness seeds the same starter catalog without changing
application UI. The after fixture preloads all three local thumbnails; production
loads only visible cards. The shop keeps up to eight 640x360 RGBA thumbnails
(7,372,800 pixel bytes), rejects oversized inputs, and evicts offscreen images.
It uses the existing worker/HTTPS client and adds no dependency or idle animation.
These component limits are not whole-process RAM or GPU measurements.

Native dark/light, narrow and theme-page screenshots were captured and inspected
using the existing wgpu framebuffer example. Keyboard/consent/cache behavior is
covered by offline egui tests; no OS input automation or live Discord claim is
made. Raw measurements: [metrics.json](pr-evidence/extension-shop/metrics.json).

## Community extensions - September 12, 2026

Baseline `0e355da`, implementation `46e81a1`; Windows 11 (10.0.26200),
Ryzen 7 7800X3D, 33,410,678,784 bytes RAM, Rust 1.98.1, release/wgpu.
Standard packages include voice. Package snapshots precede this measurement
note; development evidence and SDK/package examples are not bundled.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 63,095,808 | 65,076,736 | +1,980,928 (+3.14%) |
| Installed package bytes | 69,333,622 | 71,385,534 | +2,051,912 (+2.96%) |
| ZIP bytes (Compress-Archive) | 41,682,785 | 42,425,224 | +742,439 (+1.78%) |
| Settled working set, median bytes | 169,791,488 | 170,258,432 | +466,944 |
| Settled private bytes, median | 398,897,152 | 397,594,624 | -1,302,528 |
| Process CPU ms per ~10 s, median | 15.625 | 0 | -15.625 |

Native method: three alternating baseline/after launches with `--demo
--demo-settings=appearance`, all community plugins disabled. Each process used
five seconds warmup, then twenty 500 ms samples (~10.2 seconds). Both exposed a
native window; Start-Process used Hidden on both. No child processes were
reported. No concurrent task builds ran during the measured series. Values are
Get-Process WorkingSet64, PrivateMemorySize64 and TotalProcessorTime, not GPU
allocations. The CPU range was 0-46.875 ms per sample window; its small difference
is timer quantization/noise, not a speed improvement. Private-byte variation
also exceeds the small median delta. A preceding exploratory launch had a
209 MB baseline working set, so it is not included in these three-run medians.
Display scale, interactive startup p95, scrolling frame p95 and visual correctness
remain unmeasured: the native computer-use helper failed to connect (OS error 2).

`cargo run --locked --release -p extensions --example benchmark` executes real
committed Rust Wasm packages against short synthetic Unicode input. After one
warmup, median parse+invoke of five samples was 2.525 ms (uppercase) and 2.190 ms
(word count); median of 100 invocations, each with a fresh runtime, was 1.210 ms
and 1.093 ms. These have no predecessor feature baseline. They do not measure
UI frame latency or prove exact process-memory reclamation. Wasm/module/store
objects are invocation-owned, and host cleanup tests verify installed files and
data removal; shared host code and allocator reserves remain possible costs.

Raw numeric samples: [metrics.json](pr-evidence/community-extensions/metrics.json).
No live Discord, microphone, macOS native or Linux native UI claim is made.

## Notification preference save retries - September 12, 2026

Baseline `d00bd04`, branch `fix/remember-notifications`; Windows 11 Home
10.0.26200, Rust 1.98.1. Both snapshots use `cargo xtask package`, standard
release features including voice, before adding this measurement note.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 65,071,104 | 65,071,104 | 0 |
| Installed package bytes | 71,389,398 | 71,389,398 | 0 |
| ZIP bytes (Python zipfile, DEFLATE level 9, sorted paths) | 42,280,563 | 42,280,541 | -22 (-0.000052%) |

One package per revision; these sizes do not establish a speed improvement.
Only a rejected preference enqueue is retried on subsequent UI frames. The
existing sixteen-command cache queue and one outstanding preference write remain
bounded. A synthetic regression fills the queue, drains it, retries through the
desktop's save path, and reopens SQLite with a fresh UI for both enabled and
disabled notifications. The old behavior fails that regression; the fix passes.
Native restart interaction, CPU/RSS and save latency are unmeasured: the native
Computer Use pipe was unavailable (OS error 2), and Orca CLI was absent. No live
Discord session or system notification was exercised for this verification.


## Server role settings - September 12, 2026

Baseline `cba6894`, implementation `5b63b80`, Windows 11 Home 10.0.26200,
Ryzen 7 7800X3D (16 logical processors), 33,410,678,784 physical RAM bytes,
Rust 1.98.1. Standard packages include voice; no dependency or feature-flag
change. The separate process samples enable the existing `demo` feature on both
revisions and never load an account.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable bytes | 66,586,112 | 66,858,496 | +272,384 (+0.409%) |
| Installed package bytes | 72,918,003 | 73,192,802 | +274,799 (+0.377%) |
| Sorted DEFLATE-9 ZIP bytes | 42,757,346 | 42,860,548 | +103,202 (+0.241%) |
| Reducer replay median, ms | 41.9324 | 42.0395 | +0.1071 (+0.255%) |
| Peak working-set bytes | 172,265,472 | 170,835,968 | -1,429,504 (-0.830%) |
| Peak private bytes | 399,007,744 | 397,062,144 | -1,945,600 (-0.488%) |

Package sizes use one `cargo xtask package` output per revision. ZIP uses Python
zipfile DEFLATE level 9, sorted relative paths and fixed timestamps; installed size
sums all 234 standard package files. After files were staged using the unchanged
packager manifest from the clean baseline, excluding unrelated stale root dist
files (an old voice executable and legacy docs); those files were preserved.
Both package snapshots precede this measurement note. Reducer replay uses `cargo replay`, one warmup then five direct
release executable runs without concurrent builds. Both retain 500 records and
236,992-237,477 estimated timeline bytes. A preliminary after series overlapped
Clippy compilation and was excluded; the recorded after series ran at idle.
This is a generic synthetic reducer check, not role-request latency or process RSS.
The 0.26% difference is within sample variation, not a demonstrated regression.

Process comparison: one release `--features demo` process per revision,
`--demo --demo-server-settings`, default 1120x760 logical window, Wgpu renderer,
five seconds warmup then twenty samples spaced 500 ms apart. Start-Process used
Hidden consistently; no scripted native input was available. The before process
settled at 172,199,936 working-set / 398,938,112 private bytes; after settled at
170,835,968 / 397,062,144 bytes. TotalProcessorTime increased by 0 ms between
first and last samples in both runs; timer resolution does not prove zero CPU use.
The small memory difference from one pair is not evidence of an improvement.

A separate new role-editor fixture (`--demo-server-page=role-editor`) used the same
sampling: peak/settled working set 170,749,952 bytes, private bytes 397,332,480,
0 ms measured CPU delta. There is no predecessor Roles view for comparison.
Child/GPU allocation, frame-time p95, startup latency, and runtime display-scale
interaction remain unmeasured. Framebuffer exports were inspected at 1440x1000
and 800x900 logical sizes (125% pixel output); native Computer Use failed to
connect to its pipe (OS error 2) and Orca CLI was absent. No live Discord result
is inferred. Raw samples: [metrics.json](pr-evidence/server-roles/metrics.json).


## Server invite settings - September 12, 2026

Baseline `b47c490`, implementation `7ddf670`, Windows 11 Home 10.0.26200,
Ryzen 7 7800X3D (16 logical processors), 33,410,678,784 physical RAM bytes,
Rust 1.98.1. Standard packages include voice; no dependency/feature-policy change.
The baseline is the clean task starting commit; later unrelated main changes are
not included in either side of this comparison.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable bytes | 66,963,456 | 67,083,264 | +119,808 (+0.179%) |
| Installed package bytes | 73,301,095 | 73,422,980 | +121,885 (+0.166%) |
| DEFLATE-9 ZIP bytes | 42,895,158 | 42,943,475 | +48,317 (+0.113%) |
| Reducer replay median, ms | 43.8178 | 40.7485 | -3.0693 (-7.005%) |
| Peak working-set bytes | 175,968,256 | 170,913,792 | -5,054,464 (-2.872%) |
| Peak private bytes | 403,664,896 | 398,708,736 | -4,956,160 (-1.228%) |

Final `cargo xtask package` outputs were staged separately. Installed size sums
all 234 files from the unchanged packager manifest; the changed output was staged
using the clean baseline's relative paths to exclude unrelated stale root dist
files, which were preserved. ZIP uses sorted paths, fixed timestamps and Python
zipfile DEFLATE level 9. Both snapshots precede this measurement note.

Reducer replay: one warmup plus five direct release executable runs, 100,000
synthetic events each, no concurrent local builds. Both retain 500 messages and
236,992-237,477 estimated timeline bytes. Baseline samples range 41.6375-53.4924 ms;
after 40.0589-42.4159 ms. This generic reducer workload is not invite operation
latency; the difference from one series is not evidence of a feature speedup.

Process samples use separate release builds with the existing `demo` feature,
`--demo --demo-server-settings`, Wgpu, default 1120x760 logical window. Both used
Start-Process Hidden, five seconds warmup and twenty samples 500 ms apart, without
concurrent builds or native scripted input. Working set and private bytes are
Windows Get-Process metrics, not GPU allocations. Raw samples are committed below.

- Baseline: peak/settled working set 175,968,256/175,968,256 bytes; peak/settled private bytes 403,664,896/403,664,896; measured CPU delta 1093.75 ms.
- After: peak/settled working set 170,913,792/170,913,792 bytes; peak/settled private bytes 398,708,736/398,708,736; measured CPU delta 0 ms.
- Invites: peak/settled working set 170,651,648/170,651,648 bytes; peak/settled private bytes 397,152,256/397,152,256; measured CPU delta 31.25 ms.

The additional Invites sample adds `--demo-server-page=invites` and uses eight new
synthetic rows with visible one-second countdowns. No predecessor Invites page
exists. Baseline CPU activity and the one-pair memory differences are reported,
not attributed to this feature; timer resolution does not prove zero idle CPU.
Helper/GPU allocation, frame-time p95 and startup latency remain unmeasured.

Inspected offline framebuffer exports at 1440x1000 and 800x900 logical sizes
(125% pixel output), dark/light themes. Native Computer Use could not connect to
its pipe (OS error 2). These captures and egui click tests are not native input,
accessibility or live Discord proof. Raw evidence: [metrics.json](pr-evidence/server-invites/metrics.json).


## Server integrations - September 12, 2026

Baseline `6178e0f`, implementation branch `feat/server-integrations` (exact source
fingerprint in the linked metrics), Windows 11 Home 10.0.26200, Ryzen 7 7800X3D
(16 logical processors), 33,410,678,784 RAM bytes, Rust 1.98.1. Standard packages
include voice with no dependency or feature-policy changes.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable bytes | 67,125,760 | 67,291,136 | +165,376.0000 (+0.246%) |
| Installed package bytes | 73,468,894 | 73,636,897 | +168,003.0000 (+0.229%) |
| ZIP bytes (CompressionLevel.Optimal) | 43,117,724 | 43,178,867 | +61,143.0000 (+0.142%) |
| Reducer replay median, ms | 41.8779 | 44.2123 | +2.3344 (+5.574%) |
| Profile peak working-set bytes | 172,142,592 | 170,872,832 | -1,269,760.0000 (-0.738%) |
| Profile peak private bytes | 397,127,680 | 397,238,272 | +110,592.0000 (+0.028%) |

Package measurements use clean staging of the same 234 packager paths, preserving
unrelated stale root dist files. Both ZIPs use .NET ZipFile.CreateFromDirectory
with CompressionLevel.Optimal and no enclosing directory. Sizes precede this
measurement note; ZIP timestamps are not normalized.

Reducer replay uses one warmup and five direct release runs of 100,000 synthetic
events, without concurrent builds. Both retain 500 records and 236,992-237,477
estimated timeline bytes. Baseline runs span 40.2218-45.1117 ms; after runs span
41.5264-52.8766 ms. The median increase is reported, not attributed to integration
operations: this generic workload does not measure their latency.

Process comparison uses release builds with the existing demo feature and
`--demo --demo-server-settings`, Wgpu, default 1120x760 logical window at the
125% environment scale. Both used Start-Process Hidden, five seconds warmup and
twenty samples 500 ms apart, without concurrent builds or scripted native input.
Window visibility/occlusion and actual GPU adapter were not verified. The baseline
demo needed the same missing Command::ForumPosts match arm repaired in this change;
its standard package was built from unchanged main before that demo-only repair.

- Baseline Profile: peak/settled working set 172,142,592 bytes; private bytes 397,127,680; measured CPU delta 15.625 ms.
- After Profile: peak/settled working set 170,872,832 bytes; private bytes 397,238,272; measured CPU delta 0 ms.
- New Integrations view: peak/settled working set 170,119,168 bytes; private bytes 398,082,048; measured CPU delta 0 ms.

The new-view sample adds `--demo-server-page=integrations` with two apps, two
incoming webhooks and one followed channel. No predecessor Integrations page
exists. No child processes were observed. Timer resolution does not prove zero
idle CPU, and one pair of memory samples does not establish an improvement.
GPU allocations, frame-time p95 and startup latency remain unmeasured.

Final offline framebuffer exports were inspected in dark/light themes at
1440x1000 and narrow 800x900 logical sizes (125% pixel output), including webhook
rows. Computer Use could not connect to its native pipe (OS error 2); Orca CLI
was absent. Native interaction, accessibility and live Discord remain unverified.
Raw samples: [metrics.json](pr-evidence/server-integrations/metrics.json).

## Server channel context menu - September 13, 2026

Baseline `f1edbcf200fcf2bf051b0d7d378b310c678a5a4f`, implementation branch
`feat/server-channel-menu`. Windows 11 Home 10.0.26200, Ryzen 7 7800X3D
(16 logical processors), 33,410,678,784 RAM bytes, Rust 1.98.1 MSVC.
Standard release packages include voice, with no dependency changes.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable bytes | 67,291,136 | 67,620,864 | +329,728 (+0.490%) |
| Installed package bytes | 73,640,266 | 73,972,032 | +331,766 (+0.451%) |
| ZIP bytes | 43,179,702 | 43,286,379 | +106,677 (+0.247%) |
| Reducer replay median, ms | 46.0853 | 42.4090 | -3.6763 (-7.977%) |
| Peak/settled working-set bytes | 169,218,048 | 167,342,080 | -1,875,968 (-1.109%) |
| Peak/settled private bytes | 397,578,240 | 395,743,232 | -1,835,008 (-0.462%) |

Package sizes sum fresh, separate `cargo xtask package` outputs. ZIPs use
PowerShell `Compress-Archive -CompressionLevel Optimal` without an enclosing
directory; timestamps are not normalized. Sizes precede this measurement note.

Reducer replay uses direct release binaries: one warmup plus five measured runs
of 100,000 synthetic events, without concurrent builds. Baseline samples are
46.0853, 42.8627, 47.3063, 49.8822 and 41.0222 ms; after samples are 42.0104,
45.7572, 41.5661, 46.6492 and 42.4090 ms. Both retain 500 records and
236,992-237,477 estimated timeline bytes. The overlapping sample ranges do not
establish a speedup; this workload does not measure channel action latency.

Native process samples use release builds with `--features demo`, launched with
`--demo --demo-chat` and `Start-Process -WindowStyle Hidden`: five seconds warmup,
ten samples one second apart, no concurrent builds or scripted native input.
The app configures Wgpu and a default 1120x760 logical window. Actual GPU adapter,
display scale and window visibility/occlusion were not verified. Working set and
private bytes are Windows process metrics, not GPU allocation. Measured CPU delta
over ten seconds was 187.5 ms baseline and 0 ms after; timer resolution does not
prove zero idle CPU, and one process pair does not establish an improvement.

The new menu is covered by synthetic egui mouse/keyboard, permission and narrow
light/dark tests. Native menu interaction, screenshots, helper/GPU memory,
frame-time p95 and startup latency are unmeasured: Orca is unavailable and the
available Computer Use API disables native surfaces. These idle samples are not
menu performance or live Discord evidence. Shortcut storage has a separate
enforced limit of 256 IDs, 4 KiB vector capacity and 8 KiB serialized bytes.
Raw samples: [metrics.json](pr-evidence/server-channel-menu/metrics.json).

## Server audit log - September 13, 2026

Baseline `f1edbcf`, branch `feat/server-audit-log`; exact source fingerprint and
raw samples are in [metrics.json](pr-evidence/server-audit-log/metrics.json).
Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical processors),
33,410,678,784 RAM bytes, Rust 1.98.1. Both standard packages include voice.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable bytes | 67,291,136 | 67,436,032 | +144,896 (+0.215%) |
| Installed package bytes | 73,640,266 | 73,787,169 | +146,903 (+0.199%) |
| ZIP bytes (CompressionLevel.Optimal) | 43,179,719 | 43,230,371 | +50,652 (+0.117%) |
| Reducer replay median, ms | 47.0764 | 64.0595 | +16.9831 (+36.076%) |
| Profile peak working-set bytes | 172,224,512 | 170,233,856 | -1,990,656 |
| Profile peak private bytes | 397,463,552 | 395,927,552 | -1,536,000 |

Packages use clean staging of the same 234 paths, .NET ZipFile with Optimal
compression and no enclosing directory. Sizes precede this measurement note;
ZIP timestamps are not normalized. No dependencies changed.

Replay uses one warmup and five direct release runs of 100,000 synthetic events,
retaining 500 records and 236,992-237,477 estimated timeline bytes. Baseline runs
span 45.3208-47.3546 ms; after runs span 57.2998-104.8310 ms. Concurrent compiler
processes were observed during both sets. These timings are confounded and do
not establish a regression caused by the audit log or measure its request latency.

Process samples use release demo builds, Wgpu, default 1120x760 logical window
at the 125% environment scale, Start-Process Hidden, five seconds warmup and
twenty samples 500 ms apart. Profile uses `--demo --demo-server-settings`;
the new Audit Log sample adds `--demo-server-page=audit-log` with 50 loaded
synthetic events from a 75-entry fixture. There is no predecessor Audit Log page.

- Baseline Profile: peak/settled working set 172,224,512 bytes; private bytes 397,463,552; CPU delta 15.625 ms.
- After Profile: peak/settled working set 170,233,856 bytes; private bytes 395,927,552; CPU delta 0 ms.
- New Audit Log: peak/settled working set 170,745,856 bytes; private bytes 396,906,496; CPU delta 0 ms.

No child processes were observed. Concurrent builds were present during all
process samples. Single-run differences and timer resolution do not establish
memory improvements or zero idle CPU. Visibility/occlusion, actual GPU adapter,
GPU allocations, frame-time p95 and startup latency remain unverified.

Inspected synthetic offline framebuffer exports in dark/light themes at
1440x1000 and narrow 800x900 logical sizes (125% output), with expanded invite
details. Native computer APIs are disabled in this session, so native input and
accessibility validation remain unavailable. These exports, layout tests and
local HTTP fixtures do not prove live Discord interoperability.
# Keybinds settings package comparison (September 13, 2026)

Baseline `b2b8a50c0c1699601fb4c53ba6ad0bc48a7a5d7c`, compared with the
keybinds settings implementation. Windows 11 Home 10.0.26200, Ryzen 7 7800X3D,
approximately 31 GiB usable RAM, pinned Rust 1.98.1. Both revisions used
`cargo xtask package`: release, no default features, voice included, unsigned.
The baseline UI crate finished compiling before the implementation was wired in.
Separate copies of the full `dist` tree were measured and compressed using
PowerShell `Compress-Archive`, one package per revision.

| Metric (bytes) | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 68,865,024 | 68,877,824 | +12,800 (+0.019%) |
| Full package | 142,024,109 | 142,036,909 | +12,800 (+0.009%) |
| Compressed package | 82,125,289 | 82,130,844 | +5,555 (+0.007%) |

Native screenshots and scripted interaction measurements were unavailable:
the Orca CLI is not installed and the available computer-use API disables native
applications. The standard release package excludes demo support and exited
before an attempted synthetic idle sample; no process measurements from that
attempt are valid. CPU, memory, frame timing, display scale and renderer behavior
remain unmeasured. Offline egui layout checks cover narrow/wide and dark/light
states; they are not native visual or live Discord evidence.

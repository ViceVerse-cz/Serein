# Voice default-device polling — October 7, 2026

Baseline runtime source: `884fb73fa68bffda70b1f308c903ece239281522`.
The voice audio worker now retains the CPAL host with its active device set and
reuses it for one-second default-device checks and microphone retries. Failed
default lookups preserve healthy streams until a different default is confirmed.
Stream failure recovery still replaces the device set and host.

An offline release probe uses the pinned `pulseaudio` 0.3.1 dependency with fake
protocol replies over Unix socket pairs. It compares the original fresh-client
polling pattern with one retained client. After one warmup, all five measured
runs on macOS 27.0 (26A428), Apple M1, 16 GiB RAM, Rust 1.98.1 produced:

| Metric / method | Baseline pattern | Reuse pattern | Delta |
| --- | ---: | ---: | ---: |
| Clients created for 300 metadata polls | 300 | 1 | -299 / -99.67% |
| Peer sockets still connected after the polling batch | 300 | 1 | -299 / -99.67% |

The baseline clients were dropped after each query; the reused client remained
active through the batch. The probe also observed one connection remaining after
the reused client's final drop. This confirms polling-driven resource retention
in the dependency, not an upstream shutdown fix or a whole-process memory bound.
The probe explicitly closes its synthetic peers afterward. Source, locked
dependencies, exact reproduction command and samples are in
[the evidence directory](pr-evidence/voice-host-reuse/README.md).

These are dependency experiment counts, not Linux application socket/RSS samples.
Each query includes a 2 ms pause to let the reactor park; concurrent compilation
and deliberate pauses make the recorded elapsed times unsuitable for speed claims.
No Discord call, microphone, physical device, Linux/PipeWire client-limit behavior,
callback latency, frame timing or production CPU/RSS comparison was measured.

Both revisions also completed the standard `cargo xtask package` build on that
Mac, including voice and disabling default/demo features. File sizes were summed
from the installed `.app`; `ditto -c -k --keepParent dist <archive>` compressed the
complete distribution directory. Both ad-hoc signatures passed verification.

| Package metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Desktop executable | 68,180,992 B | 68,180,992 B | 0 B |
| Installed app bundle | 74,201,013 B | 74,201,013 B | 0 B |
| Distribution directory | 74,265,778 B | 74,265,778 B | 0 B |
| Distribution ZIP | 48,039,907 B | 48,039,944 B | +37 B / +0.00008% |

The ZIP difference is negligible packaging variation, not a runtime regression.
Raw sizes and the changed runtime source hash are in the evidence directory.

# CPU/RAM deep dive — October 2, 2026

Baseline `f16bc92fde374b91c5482daf802992f2373ee74c`, compared with the runtime
changes delivered alongside this report. Both revisions were measured on macOS
27.0 (26A428), Apple M1 MacBookAir10,1, 16 GiB RAM, pinned Rust 1.98.1 and locked
dependencies. Main advanced during the audit by a Homebrew cask-only release
update; measurements retain the recorded starting runtime source. Compiler work
was serialized and stopped during measurements.
All workloads are synthetic/offline; no saved account, network session, microphone
or camera was used. Raw samples are in
[`pr-evidence/cpu-memory/measurements.json`](pr-evidence/cpu-memory/measurements.json).

The audit covered timeline/reducer/navigation work, retained histories and cache
bounds, image worker cancellation, stream frame ownership, SQLite working sets,
protocol projection, fonts and voice/media allocation paths. Three issues were
fixed:

- Timeline tail queries used forward iteration even though the ordered timeline
  supports reverse iteration. Reducer reconciliation, read/live-edge/forward
  cursors and UI tail queries now search from the newest end. A fully loaded
  selected history checks its newest live row against the read marker, instead
  of scanning every live row. Retained deleted rows are still skipped; empty and
  partial histories retain their previous unread/navigation behavior.
- Aborting an async image job does not stop a running `spawn_blocking` decoder.
  Replacement workers previously had fresh admission limits while retired
  decoders could still run. Eight process-wide slots now belong to the actual
  blocking closures until they finish. Waiting admission is cancellable; existing
  image byte budgets, download concurrency and result budgets are unchanged.
- Screen watching allocated a new CPU image before dropping the previous
  undisplayed frame. New frames reuse that pending allocation. Upload moves the
  image out as before, preserving immutable pending uploads and retaining no
  extra CPU frame after upload. Resolution growth avoids geometric over-allocation;
  a major shrink releases oversized capacity. Invalid dimensions/lengths are
  rejected without replacing the last valid frame.

Existing byte/item cache budgets, shared/lazy fonts, bounded SQLite page cache,
resident-history ownership and preallocated voice buffers did not reveal another
confirmed issue in this audit. These checks are neither a proof of zero leaks nor
an application-wide RAM cap. Encoded inputs, decoder scratch, completed results,
pending uploads, textures and driver/framework memory have separate lifetimes.
There was no cache-quality reduction or dependency change.

## Release CPU and RAM measurements

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Reducer: 100,000 events, median | 153.985 ms | 53.504 ms | -100.481 ms / -65.25% |
| Cursor triples: full history, median | 263.428 ms | 6.948 ms | -256.481 ms / -97.36% |
| Cursor triples: ten deleted tail rows, median | 341.132 ms | 16.075 ms | -325.057 ms / -95.29% |
| 120 frame conversions: all coalesced | 147.600 ms | 147.330 ms | -0.271 ms / -0.18% |
| Frame test peak RSS: all coalesced | 34.406 MiB | 26.516 MiB | -7.891 MiB / -22.93% |
| Frame test peak footprint: all coalesced | 26.501 MiB | 18.579 MiB | -7.922 MiB / -29.89% |
| 120 frame conversions: upload every third frame | 148.299 ms | 147.927 ms | -0.372 ms / -0.25% |
| Frame test peak RSS: upload every third frame | 34.438 MiB | 34.438 MiB | 0.000 MiB / +0.00% |
| Frame test peak footprint: upload every third frame | 26.516 MiB | 26.501 MiB | -0.016 MiB / -0.06% |
| Standard desktop executable | 62,006,096 bytes | 62,006,112 bytes | 16 bytes / +0.00% |
| Complete installed bundle | 68,016,525 bytes | 68,016,541 bytes | 16 bytes / +0.00% |
| Complete distribution ZIP | 43,249,803 bytes | 43,251,982 bytes | 2,179 bytes / +0.01% |

Reducer method: build once per revision with `--release --locked`, run the
preserved `replay-bench` directly for one warmup followed by five alternating
baseline/after pairs. Baseline elapsed range was 152.415..154.912 ms; after
52.668..54.331 ms. The 100,000-event workload retained 500 rows and
331,992..332,477 estimated timeline bytes in both builds. Elapsed time is a CPU
workload proxy, not native frame latency or process RSS.

Cursor method: identical test-only workload in each revision's release core test
binary, one warmup and five batches per case. Each batch performs 100,000
live-edge/forward/unread query triples with 500 retained rows, read marker 500 and
latest metadata 501. Full-history elapsed ranges were 251.867..276.234 ms before
and 5.455..9.996 ms after. With ten retained deleted tail rows, ranges were
340.092..342.216 ms before and 15.997..17.787 ms after. Retained estimates were
unchanged at 776,996 and 778,508 bytes respectively. This deliberately isolates a
hot lookup; it does not predict that percentage improvement for the entire UI.

Frame method: the changed release desktop test executable contains an exact
original allocation-per-frame comparator selected by
`SEREIN_WATCH_FRAME_LEGACY=1`; unset uses production reuse. Five alternating
fresh-process pairs per upload scenario, each with one warmup and five measured
batches of 120 opaque synthetic 1920×1080 frames. Input is 8,294,400 bytes in both
modes. `/usr/bin/time -l` reports the whole test process's lifetime maximum RSS
and peak physical footprint; the table uses medians across the five processes.
Elapsed values are medians of each process's five batches, then across processes.
Upload interval 0 holds all frames undisplayed. Interval 3 models ownership moving
out every third frame and a pending CPU upload; it does not run a GPU or codec.
Release frame elapsed distributions overlap; no conversion CPU improvement is
claimed. Coalesced-frame peak RSS falls by about one 1080p frame; peak RSS with
third-frame uploads is unchanged because producer and upload ownership still
overlap. A fully consuming UI still needs a new frame allocation after each
ownership transfer. The production transport already bounds frames to 1920 per
side and 1920×1080 total pixels.

Debug builds use egui's optimized conversion in chunks with at most 64 KiB scratch.
The same coalesced-frame workload in one fresh process per mode measured
190.149 → 210.654 ms per 120 frames (+10.8%, about +0.171 ms/frame), while peak
RSS fell from 38,076,416 to 30,392,320 bytes (-20.2%). This debug CPU/RAM tradeoff
is separate from the release result; no universal CPU improvement is claimed.

## Native process and lifecycle observations

| Native focused idle metric | Baseline | After | Delta / interpretation |
| --- | ---: | ---: | --- |
| Mean CPU, one logical core | 1.180% | 1.275% | +0.096 percentage points; no improvement claim |
| Post-warmup sampled peak RSS | 124.500 MiB | 127.219 MiB | +2.719 MiB / +2.18%; RSS, not an allocation budget |
| Settled sampled RSS | 108.766 MiB | 111.484 MiB | +2.719 MiB / +2.50%; last five sample median |
| Settled / peak physical footprint | 82.1M / 100.6M | 82.4M / 113.9M | +0.3M / +13.3M; lifetime peak difference reversed in repeat |

The native release fixture used `--no-default-features --features demo`, Metal
on Apple M1, a 1120×760 logical window at display scale 2, and
`--demo --demo-friends --demo-frame-sample=10,20`. `caffeinate -d -u` kept the
display awake. After a ten-second warmup, twenty one-second `ps` samples recorded
process CPU-time deltas and RSS; settled RSS is the median of the last five.
CPU is percentage of one logical core. Centisecond CPU-time precision gives
roughly one-percentage-point quantization at this interval. Both fixtures focused
the search/caret and completed their diagnostic interval; no scripted typing or
scrolling was injected and no helper children were present. An earlier sleeping
display run produced no completed frame interval and was excluded.

Baseline/after diagnostic intervals completed 40/40 callbacks over
20596/20139 ms, all without input and with search/viewport focus.
Maximum measured callback wall time was 1801/1906 µs.
Callback instrumentation excludes tessellation and presentation. Startup latency,
full-frame p95, GPU memory and real message/media workloads remain unmeasured.
The first changed idle process had a higher lifetime peak physical footprint
(113.9M versus 100.6M), while its settled footprint was close (82.4M versus
82.1M). An additional matched fresh-process pair measured settled/peak physical
footprints of 82.1M/113.7M before and
82.2M/100.5M after. Its mean CPU was
1.374%/1.422% and settled RSS
111.172/111.266 MiB. The raw file retains
both pairs; lifetime peaks include startup and can vary independently of settled
RSS. The higher changed-build peak did not repeat. Two launches per revision do
not isolate a startup-memory cause. These changes target loaded timelines, decoder
replacement and coalesced video; the focused idle fixture does not exercise all
three. No native idle CPU/RAM improvement is claimed.

Lifecycle process peak RSS was 17.688 → 17.719 MiB;
peak physical footprint 16.485 → 16.532 MiB.
Baseline/after completed 2,092/3,625 passes, 66,944/116,000 channel visits,
40,166,400/69,600,000 inserts and 9/15 logout cycles in about 60.013 seconds.
Steady retained history estimates matched: 856,392..9,786,824 bytes for the small
case and 6,178,192..13,395,456 bytes for the large case.

The one-minute release lifecycle workload alternates row- and byte-pressure
channel visits, insert/eviction and logout cycles. It has no SQLite, renderer or
audio devices. Retained-byte assertions remained within their existing budgets;
process peaks are measurements, not the retained estimates. One soak per revision
cannot establish long-session leak freedom or allocator behavior on other systems.

## Package, checks and reproduction

Both standard voice-enabled `cargo xtask package` builds passed without
`demo`/`developer-session`; baseline and changed distributions were preserved
separately. Installed bytes sum all regular files in the complete bundle,
including notices/assets; both contain 206 files. ZIPs use
`ditto -c -k --sequesterRsrc`. Packages are locally ad-hoc signed and not notarized.

Focused core tests passed (128 tests, two ignored), along with six frame ownership,
resize, invalid-input and alpha regression tests and both decoder cancellation
regressions. The frame regressions and both decoder cancellation tests also
passed in release. `cargo xtask check` passed formatting and strict
workspace/all-target Clippy but stopped at unchanged loopback fixtures.
The final `cargo test --workspace --locked --no-fail-fast` run recorded 998 passing,
three failing and 25 ignored test executions. `cargo xtask policy` and the standard
`cargo check --locked -p serein --no-default-features` passed.

Failures were the Gateway identify/resume and outgoing-activity loopback tests
(`WrongHttpMethod`) and the voice mixed-audio/resume test (an unexpected accepted
connection). A separate ephemeral loopback listener received an unsolicited HEAD
request without any test client. The pinned WebSocket server rejects that method;
the fixture client sends GET. The same host interference was already documented
in the earlier September audit. No transport tests were disabled or unrelated
transport changes made. The PR stays draft for this host test blocker; CI status
is reported separately. Linux, Windows and live Discord/media behavior are untested.

The baseline revision predates `timeline_cursor_workload`. For the cursor
comparison, copy only that new ignored test function into the baseline
`read_state::navigation_tests` module before building its core test executable;
the existing `state`/`message` helpers suffice. Keep all baseline production
code unchanged. Confirm the exact filtered invocation runs one test rather than
zero. This is the same test-only port used for the recorded baseline. The frame
comparator runs only in the changed desktop test binary, which contains both the
original allocation path and production reuse.

To reproduce, build each revision once, preserve its executables and run without
concurrent compiler work:

```bash
cargo build --locked --release -p replay-bench
cargo test --locked --release -p client-core --no-run
cargo test --locked --release -p serein --no-default-features --bin serein --no-run
cargo build --locked --release -p serein --no-default-features --features demo
cargo xtask package
```

Run the emitted core test executable with
`read_state::navigation_tests::timeline_cursor_workload --ignored --exact --nocapture`.
Run the desktop test executable with
`watch::tests::watch_frame_memory_workload --ignored --exact --nocapture` under
`/usr/bin/time -l`; compare legacy/unset and upload intervals 0/3 via
`SEREIN_WATCH_FRAME_UPLOAD_EVERY`. Run preserved replay executables directly,
then `/usr/bin/time -l <replay-bench> --soak 60` separately. The raw sample file
records workload counts, flags and metric units.

# Custom Rich Presence - September 28, 2026

Baseline `5dd38dde6432e7efe4484c450652a9c8849ec357`, compared with
`dcbc431b75adb5f03ce717c792610e0b8457db0b` on Windows 11 Home 10.0.26200,
Ryzen 7 7800X3D, 31.1 GiB RAM, pinned Rust 1.98.1. The updated revision includes
main's merged artwork fallback PR #442, so this is the complete branch delta,
not an isolated attribution of every byte to Custom RPC.

Both standard voice-enabled `cargo xtask package` builds passed with locked
dependencies and no demo/developer features. Baseline and changed `dist` folders
were separate. Complete portable ZIPs use .NET `ZipFile`, `CompressionLevel.Optimal`.
NSIS was unavailable, so installer executables were not generated.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Desktop executable bytes | 76,765,184 | 77,372,928 | +607,744 / +0.792% |
| Installed package bytes | 80,868,500 | 81,476,244 | +607,744 / +0.752% |
| Portable ZIP bytes | 44,197,608 | 44,345,286 | +147,678 / +0.334% |
| Synthetic reducer 100,000 events, median | 142.9011 ms | 142.4239 ms | -0.4772 ms / -0.334%; noise |
| Retained timeline estimate / rows | 331,992..332,477 bytes / 500 | 331,992..332,477 bytes / 500 | Unchanged |

Reducer method: build `replay-bench` once per revision with `--release --locked`,
then run the executable directly for one warmup and five measured samples.
Baseline range: 134.9423..150.2501 ms; after: 132.1846..152.1863 ms. Other task
builds ran on this machine; the distributions overlap and no speedup is claimed.
This is not RSS, UI latency or live Discord interoperability. Native CPU/memory,
startup/frame timing and scripted keyboard/scrolling verification remain
unmeasured because the native automation helper is unavailable.

The bundled plugin is 404,606 bytes (SHA-256
`aa4ac3855855708e65430e73000eda62b176301b47cc79ecad10bc4889cb05b0`).
The editor reuses native widgets and background artwork resolution. Configuration
and resolved activity are each capped at 3 KiB; latest requests replace earlier
ones. The combined Gateway event has a 4-KiB budget and omits secondary Spotify
when necessary, retaining the existing rate limit. These are enforced bounds,
not runtime measurements.

Earlier builds hit disk/paging-file exhaustion and LLVM out-of-memory; serialized
retries passed after resource pressure eased. A reused baseline extension artifact
was invalidated before the changed release build. Final `cargo xtask check`
passed with 1,159 passing test executions, strict Clippy and policy checks.

# Native-first plugin artwork fallback - September 26, 2026

Baseline `48e442715a0db51f54eedfabd99d1f8dba4369a3`, compared with `5f4dfdb` on
Windows 11 Home 10.0.26200, Ryzen 7 7800X3D, 31.1 GiB RAM and pinned Rust 1.98.1.
Both standard voice-enabled `cargo xtask package` builds used locked dependencies
without demo/developer features. Complete distribution ZIPs use .NET `ZipFile`
with `CompressionLevel.Optimal`; baseline and updated distributions were separate.
NSIS was unavailable, so installer executables were not generated. The updated
package's first final link hit MSVC `LNK1318` on its PDB; a serialized retry against
the same compiled artifacts passed.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Desktop executable bytes | 73,543,168 | 73,545,216 | +2,048 / +0.0028% |
| Installed package bytes | 77,645,994 | 77,648,042 | +2,048 / +0.0026% |
| Portable ZIP bytes | 43,363,325 | 43,363,214 | -111 / -0.0003% (compression noise) |

While the picker is open, eligibility and permission checks run for visible cells
on each frame, not only when artwork is chosen. The review follow-up reuses each
cell's known guild/emoji for composer eligibility instead of scanning all guild
emoji catalogs again. Actual selection still resolves the emoji ID against current
state; reaction eligibility retains the existing core validation path. No polling
or additional cache is introduced. Native CPU/RSS/frame timing remains unmeasured;
synthetic routing and multi-frame GIF tests do not establish live interoperability.

# Lazy Fluent catalog loading - September 27, 2026

Compared `a07ed34` before and after replacing Fluent's all-catalog static loader
with one replaceable bundle for the selected language. Windows x64, Ryzen 7
7800X3D, 32 GB RAM, Rust 1.98.1; release desktop with
`--features developer-session`, without demo. Each localization build ran beside
the same installed production executable after a 15-second warmup. Twenty samples
were taken two seconds apart with no compiler running. CPU is process CPU time as
a percentage of all 16 logical processors; memory is Windows `WorkingSet64` and
`PrivateMemorySize64`.

| Metric | All 11 catalogs | Selected catalog | Delta |
| --- | ---: | ---: | ---: |
| Average CPU | 0.894% | 0.935% | +0.041 percentage points; noise |
| Maximum CPU | 1.118% | 1.167% | +0.049 percentage points; noise |
| Average working set | 185.7 MiB | 162.9 MiB | -22.8 MiB / -12.3% |
| Maximum working set | 186.2 MiB | 162.9 MiB | -23.3 MiB / -12.5% |
| Average private memory | 370.8 MiB | 351.3 MiB | -19.5 MiB / -5.3% |
| Maximum private memory | 371.1 MiB | 351.4 MiB | -19.7 MiB / -5.3% |

The simultaneous installed-production controls measured 162.0/348.2 MiB
working/private memory during the baseline run and 163.9/352.7 MiB during the
updated run. The updated localization build therefore no longer has a measurable
idle-memory premium in this sample. Its 0.087-percentage-point average CPU excess
over the updated control is too small for a performance claim.

All eleven FTL files remain embedded in the executable for offline language
switching, but only the selected catalog is parsed into a Fluent bundle. Switching
language replaces and drops the previous bundle. The 76,410,880-byte developer
executable includes debug information; standard package, compressed distribution,
startup latency, frame timing, GPU memory, and cross-platform memory remain
unmeasured. Both live-session processes stayed responsive; no messages, calls,
microphone, or camera actions were performed.

# Thread browser review fixes - September 28, 2026

Compared PR #455 head `e06d72fd` with review fix `c4eef429` on Windows x64,
Ryzen 7 7800X3D, 32 GB RAM, Rust 1.98.1. One standard voice-enabled
`cargo xtask package` build per revision, without demo/developer-session features.
Both used the same detached worktree and release target, with
`CARGO_INCREMENTAL=0` and `CARGO_BUILD_JOBS=2`. Before/after distributions were
preserved separately; installed totals sum all 198 files. ZIPs use .NET
`ZipFile.CreateFromDirectory` with `CompressionLevel.Optimal` and no root folder.
NSIS was unavailable, so no installer executable was generated.

| Metric, bytes | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Desktop executable | 76,694,528 | 76,697,088 | +2,560 / +0.0033% |
| Full installed package | 80,797,844 | 80,800,404 | +2,560 / +0.0032% |
| Portable ZIP | 44,176,880 | 44,178,210 | +1,330 / +0.0030% |

These small artifact deltas are not a runtime-performance result. The changes
reuse the existing bounded archive request path without adding polling or caches.
Native screenshots and matched idle CPU/memory measurements were blocked by the
unavailable Computer Use native pipe (Windows error 2). Frame and startup latency
also remain unmeasured; synthetic UI tests are not native or live Discord proof.
The later documentation-only commit is outside these measured package trees.

# Animated profile review fixes - September 22, 2026

Compared the PR head `ffa38ae` with `dda91ab` on Windows x64, Ryzen 7 7800X3D,
32 GB RAM, Rust 1.98.1. One standard voice-enabled `cargo xtask package` build
per revision, without demo/developer features. ZIPs use .NET `ZipFile` with
`CompressionLevel.Optimal`; baseline and updated distributions are separate.
The updated revision also merges main through `1451b45`, so these deltas cannot
be attributed to the review fixes alone. A stale shared release-cache extension
artifact was cleared with `cargo clean --release -p extensions` before the
successful updated package. NSIS was unavailable; installer executables were
not generated.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Desktop executable bytes | 70,249,472 | 71,376,384 | +1,126,912 / +1.60% |
| Installed package bytes | 74,351,763 | 75,479,068 | +1,127,305 / +1.52% |
| Portable ZIP bytes | 42,442,802 | 42,839,294 | +396,492 / +0.93% |

Pending-request and full-size/partial-frame GIF regressions passed, as did the
full local check (1,063 tests passed, 20 ignored). These are behavioral checks,
not latency or throughput measurements. Animation decoding retains a bounded
48 MiB allocation budget for its three possible 2048-square RGBA buffers.
Native CPU/RSS/frame timing and screenshots were unavailable: the Windows
computer-use plugin could not connect to its native pipe (`os error 2`). No
native performance or live Discord interoperability improvement is claimed.

# App extension capabilities - September 22, 2026

Compared the preserved host/package at `e46351a` (runtime unchanged from
`5e31f5d`) with this PR's app-capability implementation. Windows x64, Ryzen 7
7800X3D, 32 GB RAM, Rust 1.98.1; standard voice-enabled `cargo xtask package`,
without demo/developer features. Complete distribution ZIPs use .NET `ZipFile`
with `CompressionLevel.Optimal`. One package was built per revision.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Desktop executable bytes | 70,147,584 | 70,361,600 | +214,016 / +0.3051% |
| Installed package bytes | 74,249,727 | 74,463,894 | +214,167 / +0.2884% |
| Portable ZIP bytes | 42,390,190 | 42,471,534 | +81,344 / +0.1919% |
| Protector rebuilt-module invocation median | 1,165.175 us | 1,237.120 us | +71.945 us / +6.17% |
| Image sharing rebuilt-module invocation median | 1,129.040 us | 1,119.350 us | -9.690 us / -0.86% |
| Counter create-event invocation median | 1,664.060 us | 1,692.555 us | +28.495 us / +1.71% |
| App Toolbox dashboard, 18,296-byte snapshot | Unavailable | 4,044.970 us | New capability |

The installed-package delta includes 151 added bytes in the bundled README.
NSIS was skipped because `makensis` is unavailable; these are unsigned portable
packages. The existing OpenH264 LNK4255 warning was nonfatal.

Timings use the release `sdk_check` runners with identical rebuilt legacy Wasm
modules: one warmup and five batches of 20 calls, median batch time per call.
Each call includes a fresh sandbox and module compilation, excluding package
parsing, process startup, snapshot construction, worker scheduling and storage IO.
Measurements ran sequentially after compilation finished. A reverse-order repeat
changed the protector comparison to 1,383.285 us baseline / 1,217.845 us after;
the unchanged committed image control varied by 25.2% between baseline runs.
These short local samples do not establish a stable speed change. The repeated
App Toolbox median was 3,970.445 us.

App Toolbox is an optional 173,786-byte Wasm / 503,584-byte JSON example, not
embedded in production. The real sandbox checks all 11 proposed action types,
six app event kinds, a 50-message UTF-8 snapshot, and the actual synthetic desktop
demo snapshot. The 5-million-fuel and 16-MiB Wasm limits remain unchanged.
Snapshots are capped at 64 KiB; coalesced app events share the existing 32-item /
64-KiB reactive queue and ten-starts-per-second budget. No new worker, timer,
runtime dependency or persistent cache is added. Native CPU/RSS/frame timing and
live Discord behavior remain unmeasured; native UI capture is unavailable.

Reproduce after building the standalone Wasm workspace:

```powershell
cargo run --locked --release -p extensions --example sdk_check -- examples/extensions/target/wasm32-unknown-unknown/release
```

# Reactive extension events - September 22, 2026

Compared the host at `8b1c798` in an isolated worktree with the event implementation
at `5e31f5d`, on Windows x64, Ryzen 7 7800X3D, 32 GB RAM and Rust 1.98.1.
Both standard `cargo xtask package` builds include voice, without demo or developer
features. Package directories were kept separate. ZIPs contain each complete
`dist` directory, using .NET `ZipFile` with `CompressionLevel.Optimal`.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Desktop executable bytes | 70,119,936 | 70,147,584 | +27,648 / +0.0394% |
| Installed package bytes | 74,222,109 | 74,249,727 | +27,618 / +0.0372% |
| Portable ZIP bytes | 42,378,074 | 42,390,190 | +12,116 / +0.0286% |
| Protector rebuilt-module invocation median | 1,128.045 us | 1,156.705 us | +28.660 us / +2.54% |
| Image sharing rebuilt-module invocation median | 1,143.920 us | 1,126.040 us | -17.880 us / -1.56% |
| Counter create-event invocation median | Unavailable | 1,646.860 us | New capability |

The installed-package comparison includes a 30-byte LF/CRLF difference in the
otherwise identical bundled Simple Icons license between checkouts. Both builds
produced a portable package; NSIS installer creation was skipped because
`makensis` is unavailable. The existing OpenH264 LNK4255 warning was nonfatal.

Each release `sdk_check` runner used one warmup and five batches of 20 calls,
reporting the median batch time per call. The same rebuilt legacy modules were
used with both hosts; their Wasm/package sizes remain those listed below. Each
call includes a fresh sandbox and module compilation, excluding package parsing,
process startup, worker scheduling and storage IO. No Cargo build ran during the
measurements. Unchanged committed-module controls varied by up to 7.5%, so no
stable speed improvement or regression is inferred from the small timing deltas.

The optional counter example is 122,576 Wasm bytes / 354,453 JSON-package bytes;
it is not embedded in the production desktop. Its create/update/delete, panel,
reset and corrupt-storage behavior passed in the real sandbox, including a
16 KiB UTF-8 text input. Pathological JSON escaping can still exhaust the fixed
execution budget before reaching byte limits; limits were not increased.
Delivery queues at most 32 calls / 64 KiB and starts at most ten event invocations
per second. Native frame timing, process RSS and live Discord behavior were not
measured; native capture is unavailable in this session.

Reproduce the invocation workload after building the standalone Wasm examples:

```powershell
cargo run --locked --release -p extensions --example sdk_check -- examples/extensions/target/wasm32-unknown-unknown/release
```

# Extension SDK bounded serialization - September 22, 2026

Compared SDK sources at `d231e90` with the bounded serializer and unchanged example
plugin sources, on Windows x64, Ryzen 7 7800X3D, 32 GB RAM and Rust 1.98.1.
Both builds used the standalone extension workspace's locked dependencies and
`--release --target wasm32-unknown-unknown` (size optimization, LTO, one codegen
unit). Baseline modules were saved separately before editing the SDK.

The release `extensions` example `sdk_check` loaded each module into the unchanged
host sandbox. Each measurement has one warmup and five batches of 20 invocations;
the table reports the median batch duration per call. Each call creates a new
runtime, including module compilation; package construction/parsing and process
startup are excluded. Baseline and changed modules were run sequentially using
the same executable, with no concurrent Cargo build.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Protector Wasm bytes | 70,629 | 78,746 | +8,117 / +11.49% |
| Protector JSON package bytes | 205,650 | 227,360 | +21,710 / +10.55% |
| Protector invocation median | 1,014.520 us | 1,160.400 us | +145.880 us / +14.38% |
| Image sharing Wasm bytes | 70,629 | 78,738 | +8,109 / +11.48% |
| Image sharing JSON package bytes | 205,642 | 227,312 | +21,670 / +10.54% |
| Image sharing invocation median | 989.660 us | 1,119.255 us | +129.595 us / +13.09% |

These are small local activation workloads, not UI latency, RSS or live Discord
measurements. Unchanged committed modules varied between runs, so the timing
deltas are observations, not a stable slowdown estimate. The extra code provides
bounded serialization and native SDK diagnostics. Serialized response buffers stop
at 256 KiB, including JSON escaping; plugin-owned output values still consume the
existing 16 MiB sandbox memory budget.

For the initial SDK-only step at `8b1c798`, the distributed desktop runtime and
committed plugin packages were unchanged. The
SDK is a host dev-dependency only; desktop executable, installed package and ZIP
sizes were not remeasured. Plugin packages above are the uncompressed portable
JSON artifact, with no separate compressed SDK distribution. Reproduce after a
standalone Wasm build with:

```powershell
cargo run --locked --release -p extensions --example sdk_check -- examples/extensions/target/wasm32-unknown-unknown/release
```

# Navigation caches and process scanning — September 22, 2026

Compared initial baseline `5fe88e52` with runtime commit `02e2acba` on macOS 27.0
(26A428), Apple M1 Pro, 16 GiB RAM, pinned Rust 1.98.1 and locked dependencies.
Both standard packages include voice and exclude demo/developer-session features.
Baseline sources and package output stayed in a separate worktree. Benchmark-only
test additions were identical on both revisions. Changed release crates were cleaned
before building the final benchmark executables to avoid shared-target reuse of an
older worktree artifact; the new regression-test names were verified in the executables.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| 200 picker frames, 50k emoji / one search hit, message churn | 104.630 ms | 11.910 ms | -92.720 ms (-88.62%) |
| 200 sidebar frames, 10k account channels / 100 visible-guild rows, message churn | 32.105 ms | 11.500 ms | -20.605 ms (-64.18%) |
| Same sidebar, unchanged state | 11.242 ms | 11.150 ms | -0.093 ms (-0.82%, small) |
| 100 scans of 4,096 process paths, matching only | 231.388 ms | 116.444 ms | -114.945 ms (-49.68%) |
| 100,000-event reducer replay | 53.495 ms | 54.204 ms | +0.709 ms (+1.33%) |
| Retained timeline estimated bytes / records | 339,992–340,477 / 500 | 339,992–340,477 / 500 | Unchanged |
| Standard release executable bytes | 55,992,336 | 55,992,336 | 0 |
| Installed app bundle bytes | 61,937,873 | 61,937,873 | 0 |
| Compressed app ZIP bytes | 41,206,777 | 41,208,325 | +1,548 (+0.004%, noise) |

Component timings are medians of five measured batches after one warmup batch;
UI workloads also run ten initial warmup frames. Picker and sidebar churn apply
real synthetic message events and include reducer work, unlike the earlier manual
revision-bump benchmark. They render at 900×700 and 280×700 respectively, excluding
GPU presentation, network requests and whole-app frame latency. The matcher uses
equal groups of misses, basename hits, longer suffix hits and macOS bundle hits;
it excludes OS process enumeration. Replay binaries ran alternately, one warmup
and five measured runs per revision. The small reducer-only increase is reported
as a trade-off, not a speedup; its sample ranges overlapped (53.228–60.186 ms before,
52.930–55.099 ms after). The large UI/matcher gains repeated in an earlier paired run,
which had unrelated host builds active during part of the sample. Final timings
were taken serially with no Cargo build observed running at their start.

Reproduce the component workloads with:

```sh
cargo test --release --locked -p ui custom_picker_frame_benchmark -- --ignored --nocapture
cargo test --release --locked -p ui channel_list_frame_benchmark -- --ignored --nocapture
cargo test --release --locked -p discord-api process_matcher_benchmark -- --ignored --nocapture
cargo replay
# Then run target/release/replay-bench directly: one warmup and five measured runs.
```

One standard `cargo xtask package` output per revision was measured after local
ad-hoc signing; neither is a notarized distribution. Bundle size sums regular-file
lengths under `Serein.app`; compression uses `ditto -c -k --keepParent`. Executable
hashes differ despite equal file sizes. Package contents and dependency notices are
unchanged apart from the executable. The small ZIP difference is not a performance gain.

Native checks used separate release builds with `--features demo`, launched with
`--demo --demo-chat`, default viewport/appearance, wgpu on the same macOS display
at 2× scale. Each of two launches per revision warmed up for ten seconds, then used
30 main-process `ps` samples at one-second intervals (about 30.38 seconds elapsed).
Settled RSS was 151,936 / 142,128 KiB before and 145,488 / 143,232 KiB after.
CPU from process-time deltas was 0% / 0% before and 0.889% / 0% after; the first
after increase did not repeat. These short samples do not establish an idle CPU
or RAM improvement. GPU/driver and helper memory, startup latency and frame percentiles
were not measured. The isolated sidebar process's peak RSS also changed direction
between paired runs, so no process-RSS saving is claimed from that workload.

The deterministic memory changes are smaller transient indexes/row buffers and a
513-byte Linux command-line read limit. READY's temporary reference vector uses at
most 1 MiB of element storage on 64-bit; old/new validated account snapshots still
overlap. Cache ceilings, video resolution/buffer reuse, packet pacing and process-scan
intervals are unchanged. Native Linux/Windows enumeration and live Discord/media
performance were not tested. The separately landed Spotify feature is outside this
comparison. There is no visible UI change, so before/after screenshots are not applicable.

# Optional smooth scrolling - September 19, 2026

Baseline: `d160c3e`. After: this change on that baseline. One standard Windows x64
`cargo xtask package` per revision, Rust 1.98.1 MSVC, locked dependencies,
release profile and voice included. Builds ran serially using the same Cargo
target; each completed six-file `dist` directory was copied aside before the
next build. ZIP uses PowerShell `Compress-Archive -CompressionLevel Optimal`.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 66,424,320 | 66,430,464 | +6,144 (+0.0092%) |
| Full portable package, bytes | 66,486,654 | 66,492,798 | +6,144 (+0.0092%) |
| ZIP, bytes | 37,656,202 | 37,658,338 | +2,136 (+0.0057%) |

The disabled path replaces egui's smoothed wheel delta with the current bounded
raw wheel event sum and removes the two timeline transition animations. No new
dependency, background task or retained message data is added. Native CPU,
memory and frame-time sampling was unavailable, so no speed claim is made. Both
packages completed with the existing nonfatal OpenH264 LNK4255 warning. NSIS was
unavailable, so installer binaries were not produced.

# Shared extension repository - September 21, 2026

Baseline: `1a5b30d`; after: this change. Windows x64, Rust 1.98.1.
Normal builds fetch the shared theme/plugin catalog when either shop page opens.
The existing worker handles downloads and parsing; local actions cancel metadata
refreshes instead of waiting for the network.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Package bytes embedded in normal builds, exact source file lengths | 556,980 | 0 | -556,980 |
| Persistent public catalog budget | 0 | 1 MiB / 256 entries | +1 MiB maximum |

These are payload sizes and limits, not executable-size, process-memory or latency
measurements. Test/demo builds retain the existing offline package fixtures.
Native screenshot/interaction evidence is unavailable because the computer-use
native pipe cannot connect (`os error 2`); matched native CPU/RSS/frame timings
and baseline package-size comparisons were not measured. No speedup is claimed.

# Original Discord sound assets — September 21, 2026

Baseline: `932dc60`; after: this change. Windows x64, Rust 1.98.1.
The classic pack now preserves original 44.1 kHz MP3 bytes and adds outgoing-ring,
camera-on, screen-share-start, call-join and participant-leave cues. Asset sizes
are exact file measurements, not CPU/RSS or installed-package measurements.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Classic MP3 files, bytes (`git ls-tree -lr` / file lengths) | 140,928 | 583,331 | +442,403 |
| Unique classic MP3 bytes embedded (message/current-channel share a cue) | 134,400 | 565,409 | +431,009 |

The existing lazy worker, one-slot queue, 128 KiB per-asset cap, six-second
decoded ceiling and 192 kHz output ceiling are unchanged. No extra background
worker or network fetch is added. Native frame timing, CPU/RSS and matched
before/after release-package sizes were not measured; no runtime speedup is claimed.

# UI frame work and stopped-video cleanup — September 21, 2026

Baseline: `86027564`; after: this PR. macOS 27.0 (26A428), Apple M1 Pro,
16 GiB RAM, pinned Rust 1.98.1, locked dependencies and the standard release
profile. Identical benchmark-only additions were applied to the preserved baseline
worktree. Each executable was built once and copied separately; samples ran
serially without concurrent task builds. Medians use one warmup and five measured
batches. A second run of both executables confirmed the substantial differences.

| Release component workload / median elapsed time | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| 2,000 passes, 200 reused non-CJK text jobs | 288.646 ms | 16.879 ms | -271.767 ms (-94.15%) |
| 200 picker frames, one server / 500 emoji | 20.426 ms | 19.919 ms | -0.507 ms (-2.48%, small) |
| 200 picker frames, 50,000 emoji / one search hit | 104.914 ms | 11.467 ms | -93.447 ms (-89.07%) |
| 200 picker frames, 50,000 emoji / search miss | 33.919 ms | 4.525 ms | -29.394 ms (-86.66%) |
| 200 picker frames, 50,000 emoji / capped broad matches | 26.476 ms | 21.277 ms | -5.199 ms (-19.64%) |
| One-hit picker search, state changes every frame | 103.895 ms | 103.816 ms | -0.079 ms (-0.08%, noise) |
| 1,000 frames / 12 cached thumbnails | 25.063 ms | 20.347 ms | -4.716 ms (-18.82%) |
| 1,000 frames / 12 cached viewer renditions | 41.758 ms | 28.383 ms | -13.375 ms (-32.03%) |
| 1,000 frames / 12 cached media, animation enabled | 27.513 ms | 21.027 ms | -6.486 ms (-23.57%) |

The font workload paints pre-laid-out galleys with Latin text, accented characters
and punctuation, isolating repeated end-pass detection plus paint-list bookkeeping.
It excludes text layout, tessellation and GPU presentation. The new bounded weak
job cache avoids rescanning unchanged text; shapes are still traversed. First-use
CJK decoding and font coverage are unchanged. Its isolated process peak RSS was
29,163,520 → 29,097,984 bytes, a small noisy difference, not a RAM improvement claim.

The picker uses its actual popup renderer at 900×700 with ten initial warmup frames,
then six 200-frame batches. Cross-server cases contain 100 synthetic guilds with
500 emoji each. Results refresh on state revision, generation, account, server or
query changes. The churn case advances the state revision each frame and shows
no material improvement or regression; unrelated accepted events still invalidate.
The extra retained search data is at most 16,000 index bytes plus 256 query bytes
and fixed metadata, without duplicating catalog strings.

Media workloads render twelve cached 4×2 synthetic textures at 1200×300, with
signed URL metadata describing 4096×2048 images. They exercise thumbnail, viewer
and animation-key paths, not downloading, decoding, animated playback or GPU
upload. URL parameter order/encoding, signatures, source selection, dimensions,
request bounds and thumbnail fallback have behavioral coverage. No media cache
or rendition limit changes.

Repeat medians (baseline → after, ms): font 290.228 → 16.642; picker server
20.588 → 20.388, hit 104.214 → 11.884, miss 33.809 → 4.671, broad matches
26.059 → 21.553, churn 104.294 → 104.934; media thumbnail 23.542 → 20.155,
viewer 40.381 → 28.413, animation-enabled 24.976 → 21.091. Small server/churn
differences are noise; these component results are not whole-app frame percentiles.

| Standard voice-enabled macOS package | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 55,547,584 | 55,547,600 | +16 (+0.00003%) |
| Installed app, bytes | 61,489,380 | 61,489,396 | +16 (+0.00003%) |
| App ZIP, bytes | 40,676,887 | 40,678,512 | +1,625 (+0.00400%) |

Both revisions used `cargo xtask package`, without demo/developer features, and
passed `codesign --verify --strict` (local ad-hoc signing, not notarization).
Installed totals sum all 199 regular bundle files; the file set is unchanged.
ZIPs use `ditto -c -k --keepParent` on separately preserved `Serein.app` bundles.
Only the executable, compiled icon asset catalog and code-signature resources
differ; the latter two are regenerated by packaging. Size is effectively unchanged,
not a package-reduction claim. No dependencies, bundled fonts or emoji assets were removed.

Native sampling used release `demo` builds and
`--demo --demo-friends --demo-frame-sample=8,15`, configured WGPU rendering,
1120×760 logical pixels and 2× display scale. `ps` sampled RSS and cumulative
process CPU every 200 ms after the instrumented eight-second warmup. Child PIDs
were checked separately. Both revisions include the same synthetic member fixture
repair required to compile release demos; the standard packages do not enable it.
This repairs the demo-build blocker recorded in the historical section below.

The first baseline run completed 73 samples over 15.068 seconds: 0.929% of one CPU
core, settled RSS 146,931,712 bytes, sampled peak RSS 147,062,784 bytes, no children.
All 30 UI callbacks were inputless with the viewport and search focused; callback
wall-time buckets contained 17 below 1 ms and 13 below 2 ms (maximum 1,661 µs).
These callback timings exclude tessellation and presentation.

**No valid native before/after idle comparison was obtained.** The first head run
and a later baseline retry started sampling but failed to emit a completion frame
within the 60-second watchdog. The instrumentation deliberately does not schedule
repaints. A separately identified, ad-hoc-signed demo bundle rendered the synthetic
Friends screen, but its completed head sample was input-disturbed (559 callbacks,
only 266 inputless and none with search focused), so its CPU/RSS/timing results are
excluded. No runtime changes were made merely to force benchmark callbacks. The
release post-menu/forum synthetic checks passed on both executables. Whole-app
idle improvement, startup latency and end-to-end frame percentiles remain unverified;
native evidence is a draft-PR blocker, not a claim of zero regression.

The video regression exercises the shared announcement path with eight real
software decoders: stopping one source releases its decoder and lets a ninth
participant decode. A zero top-level SSRC with an active `streams[]` source keeps
the decoder. Full-queue and rapid off/on tests reject obsolete queued frames;
stopping the final lifetime releases scratch capacity. The 16-source, eight-decoder,
16-item / 16-MiB queue and 1080p limits remain unchanged. No downscaling or quality
reduction is introduced; codec/driver or whole-process RAM savings are unmeasured.

Reproduce the headless workloads by building once, then invoking the produced
test executables individually with `--ignored --nocapture --test-threads=1`:

```sh
cargo test --release --locked -p ui --lib --test startup_memory --no-run
# startup_memory-<hash>: settled_font_frames
# ui-<hash>: custom_picker_frame_benchmark, media_frame_benchmark
cargo test --locked -p discord-voice --lib decoder_cleanup
```

# Reviewed RAM findings — September 21, 2026

Baseline: `b3130c37`; after: this PR. macOS 27.0 (26A428), Apple M1 Pro,
16 GiB RAM, Rust 1.98.1, locked dependencies and the standard release profile.
The same benchmark-only `crates/ui/tests/startup_memory.rs` was added to the
baseline before runtime edits. Baseline executables and the signed package were
preserved separately. Measurements ran serially without task builds.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Font-set installation median, ms | 96.035 | 0.009 | -96.026 (-99.99%) |
| Font-set installation process peak RSS, bytes | 67,584,000 | 2,932,736 | -64,651,264 (-95.66%) |
| Atlas installation median, ms | 26.613 | 30.072 | +3.459 (+13.00%) |
| Atlas installation process peak RSS, bytes | 41,844,736 | 25,395,200 | -16,449,536 (-39.31%) |
| 100,000-event reducer median, ms | 50.672 | 50.298 | -0.374 (-0.74%) |
| Standard release executable, bytes | 54,528,000 | 54,528,000 | +0 (+0.00%) |
| Installed app, bytes | 60,466,497 | 60,466,497 | +0 (+0.00%) |
| App ZIP, bytes | 40,237,564 | 40,236,598 | -966 (-0.00%) |

Each component test creates a fresh egui context per installation, with one
warmup and five measured calls in a separate process. Elapsed time excludes
context construction/destruction; `/usr/bin/time -l` reports maximum process RSS
across all six installations. Font timing covers definition installation, not
first glyph rasterization. Atlas timing includes PNG decoding, premultiplication
and queuing the texture; no GPU upload or native window runs in this harness.
These are isolated component process peaks, not whole-app RAM or additive savings.
The atlas trades about 3.46 ms of worker initialization (+13%) for about 15.69 MiB
less peak component RSS. A reversed-order repeat confirmed the tradeoff: baseline
27.765 ms / 41,877,504 bytes versus after 30.399 ms / 25,378,816 bytes. This one-time
cost per atlas load is retained for the memory saving; no rendering-speed claim is made.

Font samples (ms): [98.126, 99.103, 93.948, 93.158, 96.035] →
[0.034, 0.01, 0.009, 0.009, 0.008].
Atlas samples (ms): [27.168, 26.817, 26.613, 26.527, 26.341] →
[31.357, 30.002, 30.008, 30.147, 30.072].
Reducer samples (ms): [50.660667, 50.672459, 51.051333, 50.243792, 50.794125] →
[49.542625, 49.921333, 50.384125, 50.29825, 50.565542]. The reducer is unchanged;
its small timing difference is treated as noise. Retained timeline remains
323,992–324,477 estimated bytes / 500 records. It does not exercise compressed
Gateway input.

The deterministic changes remove the unnecessary 16,467,736-byte CJK decode at
startup and one 16,515,072-byte atlas conversion buffer. The Gateway regression
check sends a fragmented synthetic 512-KiB payload followed by dictionary-dependent
text: oversized pending capacity is released after completion, small packets
reuse capacity and the inflater continues decoding. Its 64-MiB wire/output
limits stay unchanged. Repeated large packets may allocate more often.

Both standard `cargo xtask package` builds include voice without demo/developer
features. The installed total sums all 198 bundle files; ZIP uses
`ditto -c -k --keepParent` on `Serein.app` for each revision. Both bundles pass
`codesign --verify --strict`; signatures are local ad-hoc, not notarized.
Small package-size differences include code layout and compression noise.

Reproduce the component benchmark by building once, then running each ignored
test separately in the produced executable:

```sh
cargo test --release --locked -p ui --test startup_memory --no-run
/usr/bin/time -l target/release/deps/startup_memory-<hash> font_install --ignored --nocapture
/usr/bin/time -l target/release/deps/startup_memory-<hash> emoji_install --ignored --nocapture
cargo replay
# Run target/release/replay-bench once to warm up, then five more times.
```

Native idle CPU/RSS, startup and frame latency remain unmeasured. The untouched
baseline fails `cargo build --release --locked -p serein --features demo` because
`apps/desktop/src/post_menu_demo.rs:166` calls `debug_member_search_check`, which
is exported only under `debug_assertions`. That pre-existing demo-only compile
error is left unchanged. No live account, microphone or media-device tests ran.
No visible UI change: atlas pixels match exactly and font fallback ordering stays
unchanged. Partial media texture updates were deferred because occluded video
continues polling without rendering; partial deltas would accumulate instead
of replacing the single pending frame.

# Reviewed performance findings — September 19, 2026

Baseline: `9fca898`, with the new benchmark-only test harness applied before runtime
edits. After: guild miss caching, ASCII BiDi bypass, indexed/cached SQLite channel
loads, and shared software-video scratch reuse. macOS 27.0 (26A428), Apple M1 Pro,
16 GiB RAM, pinned Rust 1.98.1, locked dependencies, standard release profile.
Baseline executables were preserved separately; final component measurements ran
serially without task builds, using the same harness and one warmup plus five
measured runs per revision. Earlier runs during background compilation were excluded.

| Component workload / median elapsed time | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| 10,000 hit/miss pairs, 1,000 guilds | 45.113 ms | 0.565 ms | -44.548 ms (-98.75%) |
| 100,000 ASCII BiDi calls, 292-byte text | 667.910 ms | 2.863 ms | -665.047 ms (-99.57%) |
| 100,000 styled ASCII BiDi calls, 296-byte text | 643.010 ms | 2.324 ms | -640.686 ms (-99.64%) |
| 200 SQLite loads, one row | 4.987 ms | 0.614 ms | -4.374 ms (-87.70%) |
| 200 SQLite loads, 50 rows | 13.967 ms | 9.211 ms | -4.755 ms (-34.05%) |
| 200 SQLite loads, 500 rows | 123.235 ms | 74.909 ms | -48.326 ms (-39.21%) |
| 1,000 settled timeline frames, 900px synthetic text | 228.1 ms | 211.4 ms | -16.7 ms (-7.3%) |
| 1,000 settled timeline frames, 360px synthetic text | 157.3 ms | 141.6 ms | -15.7 ms (-10.0%) |
| 120 alternating 1080p/720p software decodes | 491.037 ms | 475.301 ms | -15.736 ms (-3.20%) |
| Scratch length changes in that decode workload | 120 | 1 | -119 |
| Peak requested scratch capacity | 8,294,400 bytes | 8,294,400 bytes | 0 |
| 100,000-event reducer replay | 45.733 ms | 45.269 ms | -0.464 ms (-1.02%, noise) |
| Replay retained timeline | 284,992–285,477 bytes / 500 records | Same | 0 |

The lookup workload models repeated unknown-guild invite previews. SQLite uses
synthetic in-memory databases and includes row decoding/destruction; it measures
neither disk latency nor channel switching. Its normal channel window is bounded
to 500 rows. `EXPLAIN QUERY PLAN` confirms the expression index removes the temporary
ordering B-tree. The index adds disk/write overhead within the existing page ceiling;
write throughput was not measured. Unsigned IDs remain text, and secure deletion stays on.

Timeline measurements use 500 synthetic ordinary text rows, ten warmup frames and six
measured batches of 1,000 frames in the release UI test binary. The settled viewport
reuses current-dimension heights for clipped leading rows, while resize, state changes,
dynamic content and active selection retain the full measurement path. Hidden-row renders
fell from 4,000 to 0 across the 1,000-frame samples at both widths. Media, embeds, replies,
components, spoilers, timestamps, invite-like links and non-empty reactions are excluded
from reuse; the result is not a whole-app frame-time claim.

The BiDi numbers measure only direction analysis, not parsing, complete message
layout or native frame latency. Mixed RTL initially measured 700.684 → 723.470 ms;
a reversed-order repeat measured 699.148 → 699.886 ms (+0.11%). The plain ASCII repeat
was 668.086 → 1.346 ms. There is no consistent material RTL regression in these runs.
The five-sample ranges for the primary 500-row SQLite comparison were
122.296–126.581 → 74.313–76.612 ms. Video ranges were 478.539–504.481 →
473.184–476.857 ms: the small elapsed-time difference is not a live playback claim.
That workload repeatedly decodes two synthetic OpenH264 keyframes, without devices
or network. Buffer reuse is verified separately with alternating real software decodes;
its high-water allocation remains until the decoder worker exits.

Reproduce the component workloads with the following ignored tests. Each performs
its own warmup and five samples; for the old revision, apply only the benchmark
harness additions. Build once before measuring, then run the produced executables
without concurrent builds. `cargo replay` builds the reducer; run its binary once
to warm up and five more times for the reported median.

```sh
cargo test --release --locked -p client-core guild_lookup_benchmark -- --ignored --nocapture
cargo test --release --locked -p ui bidi_ascii_benchmark -- --ignored --nocapture
cargo test --release --locked -p local-store benchmark_channel_load -- --ignored --nocapture
cargo test --release --locked -p discord-voice compare_alternating_software_decode -- --ignored --nocapture
cargo test --release --locked -p ui leading_overscan_benchmark -- --ignored --nocapture
cargo replay
```

| Standard macOS package / bytes | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 53,624,384 | 53,624,384 | 0 |
| Installed app, sum of 197 files | 59,558,087 | 59,558,087 | 0 |
| App ZIP, `ditto -c -k --keepParent` | 39,702,953 | 39,704,308 | +1,355 (+0.0034%) |

Both `cargo xtask package` commands completed with voice and without demo/developer
features. The preserved baseline bundle subsequently failed resource-seal verification:
its icon files differed from the completed packaging resources. For a matched comparison,
the baseline bundle was reconstructed with its preserved baseline executable and the
after package's unchanged resources, then ad-hoc signed again. Both compared bundles
pass `codesign --verify --strict`; both have the same file set. These are locally
ad-hoc-signed, unnotarized packages. ZIP differences include compression/metadata noise;
there is no package-size improvement claim.

| Offline native idle sample | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Process CPU, cumulative CPU-time delta / elapsed time | 0.05% | 0.00% | -0.05 percentage points |
| Peak / settled sampled RSS, KiB | 205,280 / 205,280 | 203,728 / 203,728 | -1,552 KiB (-0.76%) |
| Child processes | 0 | 0 | 0 |

Separate release builds with `--features demo` launched explicitly with `--demo`,
using the default initial scene and no interaction on both revisions. Each had
30 seconds of warmup, followed by 21 `ps -p PID -o time=,rss=` observations at
one-second intervals (20 intervals). Renderer logs identify Apple M1 Pro / Metal;
the built-in display is 3024×1964 Retina (2×), with the default requested 1120×760
window and demo zoom. No builds ran during sampling. RSS excludes driver/GPU
allocations and its peak covers only the sample window, not startup. These single
idle samples and the CPU clock's coarse resolution do not establish a CPU or memory
improvement. Interactive frame percentiles, startup latency and live media latency
remain unmeasured. No accounts, microphone, calls or network media were used.

The overscan follow-up repeated the same idle sample with the settled current binary:
CPU was 0.00% and sampled peak/settled RSS was 203,952 KiB, versus the prior matched
sample's 0.00% and 196,944 KiB. An earlier run reached 16.86% while loading local demo
content, so neither run is treated as a whole-app CPU or memory claim.

# Reaction tooltip loading - September 17, 2026

Baseline: `ef0c61a`. After: the reaction-tooltip follow-up on that baseline.
One standard Windows x64 `cargo xtask package` per revision, Rust 1.98.1 MSVC,
locked dependencies, release profile and voice included. Builds ran serially
using the same Cargo target; each completed `dist` was copied to a separate
directory before the next build. Package bytes sum all 188 files; ZIP uses
PowerShell `Compress-Archive -CompressionLevel Optimal`.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 66,171,392 | 66,170,368 | -1,024 (-0.0015%) |
| Full portable package, bytes | 70,244,189 | 70,243,165 | -1,024 (-0.0015%) |
| ZIP, bytes | 40,472,299 | 40,471,778 | -521 (-0.0013%) |

Native screenshots and matched hover CPU, memory and frame-time sampling were
unavailable because the computer-use runtime exposed no Windows application
surface. An installed authenticated Serein instance was already running and was
left untouched. No runtime performance improvement is claimed. Both packages
passed with the existing nonfatal OpenH264 LNK4255 warning; NSIS was unavailable,
so Windows installer binaries were not produced.

# Discord chat links - September 16, 2026

Baseline: clean `afb2a3e`. After: chat-link navigation on that baseline.
One standard Windows x64 `cargo xtask package` per revision, Rust 1.98.1 MSVC,
locked dependencies, release profile and voice included. Builds ran serially
using a shared Cargo target and separate worktree distribution directories.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 72,636,416 | 72,641,024 | +4,608 (+0.0063%) |
| Full portable package, bytes | 76,702,882 | 76,707,490 | +4,608 (+0.0060%) |
| ZIP, bytes | 43,319,355 | 43,320,194 | +839 (+0.0019%) |

Package bytes sum all 186 files; ZIP uses .NET
`System.IO.Compression.ZipFile.CreateFromDirectory` with default compression.
The baseline comparison copy excludes one obsolete 1,075-byte libpulse-sys
license left in the existing, non-cleaned dist directory by an earlier build.
All 185 current non-executable package files have identical SHA-256 hashes;
the original distribution remains untouched. This is a size comparison only.

Native before/after screenshots and matched CPU, memory and frame-time samples
are unavailable: native computer control is disabled in this session and Orca
is not installed. No runtime performance improvement is claimed. Both portable
packages passed with the existing nonfatal OpenH264 LNK4255 warning; NSIS is not
installed, so Windows installer binaries were not produced.

# Channel creation types - September 15, 2026

Baseline: clean `544a3f8`. After: the channel-creation-types changes on that base.
One standard Windows x64 `cargo xtask package` per revision, Rust 1.98.1 MSVC,
locked dependencies, release profile and voice included. Separate worktree `dist`
directories preserve both packages. Package bytes sum all files; ZIP uses .NET
`System.IO.Compression.ZipFile.CreateFromDirectory` with its default compression.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 72,637,440 | 72,638,976 | +1,536 (+0.0021%) |
| Portable package bytes | 76,703,564 | 76,705,100 | +1,536 (+0.0020%) |
| ZIP bytes | 43,318,150 | 43,318,988 | +838 (+0.0019%) |

Native CPU, memory and frame timing remain unmeasured: Windows Computer Use
reported an unavailable native pipe (OS error 2), and Orca is not installed.
No speed improvement is claimed. The production build was launched for owner
testing; this is not synthetic screenshot or live interoperability evidence.
Packaging passed; the existing OpenH264 LNK4255 warning was nonfatal. NSIS is
unavailable, so these are unsigned portable packages rather than installers.
# Stream audio call-playback exclusion - September 15, 2026

Baseline `a28b646` and the exclusion implementation in PR #230 were packaged with
`cargo xtask package`, standard release with voice and no demo/developer features,
Rust 1.98.1, macOS 27.0 (26A428), Apple M1 Pro / 16 GiB. Baseline output was copied
to a separate directory before edits. One package measurement per revision; both
local ad-hoc signatures passed verification.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Packaged executable, bytes | 66,716,336 | 66,716,336 | 0 |
| Installed bundle, sum of files | 72,623,733 | 72,626,189 | +2,456 (+0.0034%) |
| ZIP, `ditto -c -k --keepParent` | 43,389,158 | 43,390,952 | +1,794 (+0.0041%) |

The macOS executable is unchanged; package growth is notices/provenance. This does
not measure Linux/Windows binary size, capture CPU, RSS or end-to-end A/V latency.
Those native desktop measurements remain unavailable, and no speed improvement is claimed.

Linux now owns at most 32 application monitors with 100 ms / 38,400 bytes of pending
PCM each, plus the existing four-chunk transport queue. Discovery admits at most 256
entries and one in-flight request. Mixing uses a fixed 10 ms cadence on one worker;
with no eligible apps it sends no audio and waits up to 100 ms for native events.
Windows uses one native process-loopback worker and the existing bounded audio queue.
Native server/driver allocations are additional; these are payload limits, not RSS.

An additional temporary harness compiled the actual Linux adapter against a private
PulseAudio 17 server on this Mac, loading only `module-null-sink` and a private UNIX
protocol socket (`-n`, no default or hardware modules). `pacat` supplied synthetic
48 kHz float stereo: game `(0.125, 0.25)`, Serein `(0.5, -0.5)`, 100 ms playback latency
and 20 ms process time. The final run, after three seconds warmup, delivered
144,000 stereo frames in three seconds, all at the game's expected amplitude. No sample exceeded
the game-only bounds when Serein playback appeared. Rekey, application removal,
Serein-only idle and stop also passed. This verifies the Pulse API with synthetic
signals; it is not Linux/PipeWire hardware, Discord interoperability or a latency benchmark.

# Indexed message channel lookups - September 15, 2026

Historical measurements from the original PR revision, before its September 21
rebase onto `81b472d5`. A fresh release replay on macOS failed while compiling
`client-core` with `No space left on device`; these figures do not measure the
rebased revision.

Baseline: fetched `origin/main` at `490be9c`; after: that revision plus the
message-path channel-index substitutions on `perf/message-channel-index`.
Windows 11 10.0.26200 x64, Ryzen 7 7800X3D (16 logical processors),
33,410,678,784 bytes RAM, pinned Rust 1.98.1 MSVC, locked release profile.
Both standard `cargo xtask package` builds included voice without demo or
developer-session features. They ran serially in one isolated worktree with
the same E: Cargo target; the baseline `dist` was copied aside before rebuilding.
The portable packages each contain the same 186 file paths. `makensis` was
unavailable, so no installer was measured. The OpenH264 LNK4255 warning was
nonfatal in both builds.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Synthetic 10,000-message / 20,001-channel reducer, median | 711.2295 ms | 177.1066 ms | -534.1229 ms (-75.1%) |
| Existing 100,000-message reducer, median | 45.1406 ms | 44.0192 ms | -1.1214 ms (-2.5%; noisy) |
| Release executable bytes | 72,463,872 | 72,463,872 | 0 |
| Full portable package bytes | 76,529,996 | 76,529,996 | 0 |
| ZIP bytes, Compress-Archive Optimal | 43,259,755 | 43,260,003 | +248 (+0.0006%) |

`cargo replay` built each release workload once. The existing `replay-bench`
fixture gained `--wide-channels`: it clones synthetic navigation to 20,001
channels and applies 10,000 messages to the last channel. One warmup per binary
preceded five alternating baseline/after runs with no concurrent build.
Wide baseline: 688.5916, 699.4779, 712.9667, 717.0727, 711.2295 ms.
Wide after: 170.8742, 187.2572, 171.2247, 182.9038, 177.1066 ms.
Both retained 500 records / 261,477 estimated bytes. The existing replay
baseline: 55.4731, 47.1661, 42.6606, 45.1406, 43.8635 ms; after:
51.7992, 43.3525, 44.0192, 43.2530, 45.6519 ms. Its ranges overlap, so
no general reducer speedup is claimed. It retained 500 records and
260,992..261,477 estimated bytes on both revisions.

ZIPs compressed each copied package's contents with the same path layout.
The wide case measures channel lookup work under a synthetic large navigation
set; it does not measure actual account startup, native UI CPU/RSS/frame timing,
network latency or live Discord compatibility.

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

## Selected-channel search shortcut (October 2, 2026)

Eight actual egui search tests and fresh full checks passed on corrected source
`e94640f5`, including cross-guild numeric submission and remembered-channel
eligibility changes. Actual native captures use exact parent `47a81035` and
corrected source `e94640f5`; the temporary synthetic Command+F fixture is excluded
from shipping and measurement builds.

Measured on macOS 27, Apple M1 / 16 GiB, Rust 1.98.1 and locked dependencies.
The standard voice-inclusive packages use unchanged fat LTO, no development
features, and local ad-hoc signatures. Parent package source `ef9cd5d1` is
application-identical to parent `47a81035`; installed bytes sum regular files,
and ZIP uses `ditto -c -k --sequesterRsrc` over the complete distribution.
Both standard packages passed; these are isolated feature revisions, not the
newer main aggregate containing unrelated features.

| Metric | Parent | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable, bytes | 62,088,304 | 62,088,304 | 0 (0%) |
| Installed distribution, bytes | 68,098,733 | 68,098,733 | 0 (0%) |
| Complete ZIP, bytes | 43,286,187 | 43,286,492 | +305 (+0.0007%) |
| Common native median process CPU | 0.0% | 0.0% | 0 percentage points |
| Peak process RSS, KiB | 125,552 | 125,664 | +112 (+0.089%) |
| Settled process RSS, KiB | 125,504 | 125,616 | +112 (+0.089%) |

Both optimized native binaries use matching process-only thin LTO:
`CARGO_PROFILE_RELEASE_LTO=thin CARGO_BUILD_JOBS=2 cargo build --release --locked -p serein --features demo`.
This does not change the repository fat-LTO profile. All runtime workspace
dependencies compiled from the intended worktree; binaries contain no capture
hooks. Both runs use identical flags recorded in the raw evidence, a 1120×760
logical viewport at 2× scale and Metal. A five-second warmup precedes ten
one-second macOS `ps` samples; settled RSS is the final-five median. All other
compilers, tests, helpers and native demos were paused during the paired sample.
Small RSS differences and quantized idle CPU are noise, with no improvement claim.
Action/request latency, GPU memory and frame/startup latency remain unmeasured.
Native synthetic input proves application handling, not physical OS routing or
live Discord compatibility. Source identities, hashes, package sizes, capture
metadata and all samples: [channel-search/measurements.json](pr-evidence/channel-search/measurements.json).

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

## Linux and Windows stream audio — September 15, 2026

Compared baseline `0628052` with stream-audio commit `79d1bc0` on macOS 27.0
(26A428), Apple M1 Pro, 16 GiB RAM, Rust 1.98.1. Both use `cargo xtask package`:
the standard release build including voice, without demo/developer-session features.
Baseline output was preserved in a detached worktree before building the changed tree.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Packaged macOS executable bytes | 66,409,328 | 66,410,496 | +1,168 (+0.0018%) |
| Installed app bundle bytes | 72,316,725 | 72,317,893 | +1,168 (+0.0016%) |
| Compressed app ZIP bytes | 43,275,334 | 43,277,237 | +1,903 (+0.0044%) |

One build per revision; executable file size after the packaging strip/sign step.
This measures the shared audio transport changes on macOS, not the size or runtime
cost of the Linux/Windows adapters. Installed size sums regular files in `Serein.app`;
ZIP uses `ditto -c -k --keepParent`. Compression varies with binary content and metadata;
these tiny deltas do not establish a runtime improvement. No Rust dependency was added.

Capture uses four bounded PCM chunks (up to 38,400 bytes each); the sender reserves
38,400 PCM bytes and sends one 20 ms stereo Opus frame per tick. Linux adds four
bounded appsink buffers and a separate event-driven audio worker. These are component
limits, not RSS measurements. Synthetic checks exercise audio gates, rekey generation
tags, queue pressure, malformed PCM, pacing and cancellation without opening devices.

Linux/Windows hardware capture CPU, RSS, A/V latency and native before/after UI
screenshots remain unmeasured because those desktop sessions are unavailable here.
The macOS demo does not execute either new native adapter; no native performance
improvement or live interoperability is claimed. Windows cross-checking on this Mac
also stopped in existing native Opus/OpenH264 build scripts (missing Visual Studio
generator / incompatible host C++ flags), before checking the Windows adapter.

The follow-up after owner testing moves Windows frame admission ahead of D3D11
readback. Capture is capped at the selected frame rate, and no new staging texture,
GPU-to-CPU copy, or raw-frame allocation is performed while the one-frame queue is
occupied. Windows OpenH264 uses its low-complexity mode. Before this change those
costs ran for every compositor callback and frame-rate/queue dropping happened only
after readback. At 1920x1080 BGRA, each avoided readback and subsequent copy is
8,294,400 bytes; a 3840x2160 source is 33,177,600 bytes. These are buffer sizes and
work bounds derived from the dimensions, not throughput measurements.

Native Windows frame time, CPU/RSS, GPU copy load and viewer FPS remain unmeasured on
this macOS host. The owner observed severe lag at 1080p60 before this follow-up; the
new result requires another Windows measurement.

The next follow-up prefers a Windows Media Foundation hardware H.264 transform and
falls back to the existing OpenH264 encoder when hardware activation, encoding, or a
forced keyframe fails. It keeps the existing bounded CPU BGRA-to-NV12 conversion and
GPU readback, so this offloads H.264 compression but is not a zero-copy pipeline.
The native transform and fallback source both produce the same bounded Annex-B stream.
Actual NVIDIA encoder selection, viewer FPS, sender CPU and stop/start stability still
require owner testing on Windows hardware.

The standard macOS release package at the preceding `0e7d2a3` revision versus this
follow-up changed from 66,716,336 to 66,716,480 executable bytes (+144), from
72,626,189 to 72,626,333 installed bundle bytes (+144), and from 43,390,952 to
43,393,402 ZIP bytes (+2,450). One package per revision used the same host and
`cargo xtask package`; ZIP compression noise is not a speed improvement or regression.

The hardware-encoder follow-up, compared with that preceding package, is 66,716,320
executable bytes (-160), 72,626,173 installed bundle bytes (-160), and 43,393,952
ZIP bytes (+550). The Windows-only Media Foundation module is excluded from this
macOS package; these measurements cover only the small shared encoder selection change.


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


## 2026-09-16: profile preview Rich Presence cards

Baseline: `a426298` (the initial account-preview implementation), compared with this card refinement on Windows x64, Rust 1.98.1 MSVC, Ryzen 7 7800X3D, 32 GB RAM, standard voice-enabled release builds. These numbers measure the refinement, not the initial addition relative to main.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 72,653,312 | 72,684,032 | +30,720 (+0.042%) |
| Full portable package, bytes | 76,719,778 | 76,750,498 | +30,720 (+0.040%) |
| ZIP, Compress-Archive Optimal, bytes | 43,330,075 | 43,344,292 | +14,217 (+0.033%) |
| 100,000-event reducer replay, median ms | 43.6106 | 43.3468 | -0.2638 (-0.605%) |
| Retained timeline estimated bytes / records | 260,992-261,477 / 500 | 260,992-261,477 / 500 | Unchanged |
| Native UI CPU, memory, frame time | Unmeasured | Unmeasured | Unmeasured |

One standard package per revision. The baseline was built in a clean detached worktree and preserved separately. Both measured distributions contain the same 186 files. The after distribution was staged from those generated paths, leaving one pre-existing obsolete `libpulse-sys-1.23.0-LICENSE-MIT` notice in root dist untouched and excluded from both measurements. ZIP uses `Compress-Archive -LiteralPath <dist> -CompressionLevel Optimal`. Both packages passed; OpenH264 LNK4255 was nonfatal. NSIS/makensis is unavailable, so these are unsigned portable packages, not installers.

Replay uses `cargo replay` followed by one warmup and five measured runs of the release executable, with no concurrent task builds during measurement. Replay crates were rebuilt for the after source to avoid shared-target worktree cache ambiguity. Baseline: 45.1570, 43.6106, 42.9670, 43.6495, 43.1553 ms. After: 42.8025, 43.3468, 42.8042, 45.1077, 47.1907 ms. The small median difference is noise, not a claimed speed improvement. This workload is a synthetic message reducer, not activity-card rendering, process RSS or live interoperability.

Native screenshots and matched UI CPU/memory/frame-time measurements are unavailable because native desktop capture/control is disabled and Orca is absent. No UI performance claim is made. Package sizes precede this performance-note-only edit.

## 2026-09-16: joined invite navigation

| Metric / method | Baseline `2283600` | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 72,683,520 | 72,684,544 | +1,024 (+0.001%) |
| Full portable package, bytes | 76,751,643 | 76,752,667 | +1,024 (+0.001%) |
| ZIP, Compress-Archive Optimal, bytes | 43,344,538 | 43,345,148 | +610 (+0.001%) |
| Native UI CPU, memory, frame time | Unmeasured | Unmeasured | Unmeasured |

Matched Windows x64, Rust 1.98.1 MSVC, standard voice-enabled packages contain the same
187 files. ZIP uses `Compress-Archive -LiteralPath <dist> -CompressionLevel Optimal`.
Both builds passed with the same nonfatal OpenH264 LNK4255 warning. `makensis` is unavailable,
so these are unsigned portable distributions. Native UI measurements remain unavailable because
desktop capture/control is disabled and Orca is absent; no runtime performance claim is made.
## Linux and Windows stream audio — September 15, 2026

Compared baseline `0628052` with the stream-audio implementation on macOS 27.0
(26A428), Apple M1 Pro, 16 GiB RAM, Rust 1.98.1. Both use `cargo xtask package`:
the standard release build including voice, without demo/developer-session features.
Baseline output was preserved in a detached worktree before building the changed tree.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Packaged macOS executable bytes | 66,409,328 | 66,410,496 | +1,168 (+0.0018%) |
| Installed app bundle bytes | 72,316,725 | 72,317,893 | +1,168 (+0.0016%) |
| Compressed app ZIP bytes | 43,275,334 | 43,277,237 | +1,903 (+0.0044%) |

One build per revision; executable file size after the packaging strip/sign step.
This measures the shared audio transport changes on macOS, not the size or runtime
cost of the Linux/Windows adapters. Installed size sums regular files in `Serein.app`;
ZIP uses `ditto -c -k --keepParent`. Compression varies with binary content and metadata;
these tiny deltas do not establish a runtime improvement. No Rust dependency was added.

Capture uses four bounded PCM chunks (up to 38,400 bytes each); the sender reserves
38,400 PCM bytes and sends one 20 ms stereo Opus frame per tick. Linux adds four
bounded appsink buffers and a separate event-driven audio worker. These are component
limits, not RSS measurements. Synthetic checks exercise audio gates, rekey generation
tags, queue pressure, malformed PCM, pacing and cancellation without opening devices.

Linux/Windows hardware capture CPU, RSS, A/V latency and native before/after UI
screenshots remain unmeasured because those desktop sessions are unavailable here.
The macOS demo does not execute either new native adapter; no native performance
improvement or live interoperability is claimed. Windows cross-checking on this Mac
also stopped in existing native Opus/OpenH264 build scripts (missing Visual Studio
generator / incompatible host C++ flags), before checking the Windows adapter.
## 2026-09-15: stream packet markers and keyframe recovery

| Metric / method | Baseline `0a8f0ab` | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable bytes | 72,557,568 | 72,576,000 | +18,432 (+0.0254%) |
| Full portable package bytes | 76,626,644 | 76,645,076 | +18,432 (+0.0241%) |
| ZIP bytes, Optimal | 43,288,437 | 43,294,686 | +6,249 (+0.0144%) |

Standard voice-enabled `cargo xtask package` on Windows 11 Home build 26200,
AMD Ryzen 7 7800X3D, 33,410,678,784 bytes RAM, pinned Rust 1.98.1 MSVC.
One package per revision; 187 files each. Baseline `0a8f0ab` was built from the
unchanged checkout and preserved before editing. The after column is this follow-up.
Package size sums regular files; ZIP uses PowerShell Compress-Archive Optimal.
Both packages built successfully. The after release build took 2m 38s.
OpenH264 linker LNK4255 warnings were nonfatal; missing `makensis` means these
are unsigned portable packages, without an NSIS installer.

Soundshare marking adds eight bytes per audio RTP packet. The idle screen recovery
retains one current raw snapshot, bounded to 33,177,600 bytes; fitted 720p/1080p
snapshots use 3,686,400/8,294,400 bytes. These are component bounds, not RSS measures.
No dependency was added. Ordinary idle screens do not encode extra frames.

Native media CPU/RSS, GPU use and end-to-end audio/video latency remain unmeasured:
native desktop control/capture is disabled in this session, and no owner-operated
live stream was run. The two-endpoint encrypted localhost test checks media delivery,
not capture or speakers. No speed, hardware-capture or live interoperability claim
follows from package sizes or the passing test.

### 2026-09-16: stream diagnostic follow-up

| Metric / method | Baseline `b445aae` | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable bytes | 72,576,000 | 72,578,560 | +2,560 (+0.0035%) |
| Full portable package bytes | 76,645,076 | 76,647,636 | +2,560 (+0.0033%) |
| ZIP bytes, Optimal | 43,294,686 | 43,295,117 | +431 (+0.0010%) |

Same Windows host, pinned toolchain, standard voice-enabled package and size method
as above; one package per revision, 187 files each. The baseline package was preserved
before this edit. The follow-up package built in a separate checkout in 3m 20s;
NSIS remains unavailable. No dependency change. Diagnostic reports retain their
eight-item queue and process-wide 128-report / 64-KiB output limits. Disabled
diagnostics still evaluate a few state flags/atomic loads on the 50-Hz stream tick.
CPU/RSS and end-to-end media latency remain unmeasured; no speed improvement is claimed.

### Windows loopback recreation follow-up

Against the preserved `14456e9` package, the same standard Windows release package
has a 72,580,608-byte executable (+2,048), 76,649,684 total package bytes (+2,048),
and a 43,295,212-byte Optimal ZIP (+95); still 187 files. Native capture clients are
released and recreated sequentially on encryption epoch changes, preserving existing
packet and queue bounds. No dependency change. The owner confirmed audible shared
browser audio; the release sender log records about 50 audio packets/s and 19–22
video frames/s after negotiation. These are sender counters from one owner test,
not a controlled performance comparison or proof of smooth viewer playback.
The owner still reports intermittent lag; the subsequently supplied viewer log stops
accepting audio and video while the main call continues.

### Idle media UDP keepalive follow-up

Against preserved `be42b72`, the release executable is 72,583,680 bytes (+3,072),
the portable package totals 76,652,756 bytes (+3,072), and its Optimal ZIP is
43,297,676 bytes (+2,464); still 187 files. Same host, toolchain and compression
method as above, one package per revision. Compilation/linking completed, but
`cargo xtask package` could not replace the running `target/release/serein.exe`
(Windows access denied). The newly linked `target/release/deps/serein.exe` was
copied into the existing standard `dist` resources and zipped; SHA-256 verified
that the packaged executable matches the linked output. The running build was
left untouched. This is a manually refreshed portable package, not a successful
rerun of the standard packaging command.

The keepalive adds eight UDP payload bytes per connection every five seconds
after discovery (1.6 bytes/s, excluding network headers), with no queue, extra
thread or dependency. The localhost test verifies repeated idle-viewer pings and
subsequent encrypted audio/video delivery. Live freeze recovery, CPU/RSS and
end-to-end latency remain unmeasured; no playback improvement is claimed yet.

# Theme transparency and blur — September 19, 2026

Baseline: `9fca8980`. After: this rebased transparency branch. Standard
voice-enabled macOS packages and release demo builds used Rust 1.98.1 on macOS
27.0, Apple M1 Pro, 16 GiB RAM, Metal, 2x display scale. Disabled transparency
uses the same opaque native window and GPU surface selection as baseline.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 53,624,384 | 53,640,848 | +16,464 (+0.031%) |
| Installed app bundle, bytes | 59,558,087 | 59,574,551 | +16,464 (+0.028%) |
| ZIP, `ditto --keepParent`, bytes | 39,702,949 | 39,711,200 | +8,251 (+0.021%) |
| Disabled idle CPU, median of 3 × 30 s after 10 s warmup | 0.067% | 0.100% | +0.033 percentage points |
| Disabled settled RSS, median | 199,248 KiB | 199,088 KiB | -160 KiB (-0.08%) |
| Disabled physical footprint, median | 158,090,320 B | 158,925,856 B | +835,536 B (+0.53%) |

The CPU samples are quantized by the short process-time interval, and unrelated
Cargo builds ran elsewhere on the host during sampling. The small CPU and memory
differences are therefore treated as noise, not an improvement or regression.
The disabled path performs no compositor calls, repaint scheduling, allocations,
or extra draw passes; it exits window-effect synchronization before theme lookup.
No helper processes were present. Frame callback timing is unmeasured because an
idle event-driven window did not produce enough callbacks for a useful comparison.

## Scalable bundled jumbo emoji — October 2, 2026

The isolated comparison is parent `47a81035` (runtime-identical to the preserved
`ef9cd5d1` package) versus vector runtime `f7b8a99c` / packaging-only icon-path
repair `173477ec`. Final aggregate `6afe8190` normally integrates main `66cc09d2`.
The complete source and raw samples are in
[the evidence record](pr-evidence/scalable-jumbo-emoji/measurements.json).

| Metric / method | Parent47 | Vector173 | Delta |
| --- | ---: | ---: | ---: |
| Standard voice executable | 62,088,304 B | 66,948,784 B | +4,860,480 B (+7.828%) |
| Installed standard package | 68,098,733 B | 73,031,808 B | +4,933,075 B (+7.244%) |
| ZIP, same ditto method | 43,286,187 B | 47,500,504 B | +4,214,317 B (+9.736%) |
| Package files | 206 | 220 | +14 license files |
| Native thin-LTO demo executable | 66,727,920 B | 71,628,704 B | +4,900,784 B |
| Idle process CPU, median | 0% | 0% | 0 percentage points |
| Sampled peak RSS | 125,664 KiB | 125,472 KiB | −192 KiB (−0.153%) |
| Settled RSS | 125,616 KiB | 125,424 KiB | −192 KiB (−0.153%) |

Native samples used macOS 27.0, Apple M1, 16 GiB RAM, Metal, 2× display scale,
the same `--demo --demo-chat`, five-second warmup and ten one-second `ps` samples.
All team compilers/apps paused and the unrelated host build finished. Both samples
used the identical process-only thin-LTO override; standard shipping packages
retain normal fat LTO. Settled RSS is the median of the final five samples.
The small RSS difference is noise, without an improvement claim.

The final aggregate standard package passed: executable 67,113,232 B,
installed 73,196,256 B, ZIP 47,575,522 B, 220 files.
These aggregate bytes include unrelated incoming work; the isolated table above
reports the vector change separately. Fresh full workspace checks passed on the
isolated sources and measured aggregate revision `6afe8190`; its final actual
native frame was inspected. These package checks and measurements retain that
exact runtime identity after later integrations. Documentation-only evidence
commits do not change the measured runtime.

Later normal integration `2d754e25` (main `dc7e9f00`) passed fresh full checks
with 369 UI tests and cross-worktree-ID workspace cache pruning. Its standard
voice package passed: executable 67,113,568 B, installed 73,196,592 B,
ZIP 47,577,622 B, 220 files. These latest aggregate bytes include unrelated
incoming work; the isolated vector comparison above retains its original source.
An independently captured and inspected current native frame is byte-identical
to the prior `6afe8190` frame (SHA-256 `6d8c4d00ae8b793e…`); hooks were removed
byte-exactly. Saved build logs show all twelve runtime workspace crates compiling
fresh from the exact worktree for each recorded baseline/vector release build.

The trusted compressed SVG bundle is 4,244,356 B. Rasterization runs off-thread
at 64/128/256 physical pixels with a 64 KiB expansion/window limit, eight shared
decode permits and the existing 1,024-item / 16 MiB emoji texture cache. There is
no runtime artwork download or SVG external image resolver. All 4,009 cells pass
synthetic rasterization. The idle sample does not add the jumbo screenshot
fixture; active decode/frame/startup latency and GPU memory remain unmeasured.
No live service or Windows/Linux native appearance was tested.

## Unicode mathematical-letter fallback — September 20, 2026

Package baseline: `4c3c53a`. After: this branch. The idle sample compared
`f0cb74d` with the same font patch before its clean rebase. Both comparisons used
Rust 1.98.1 on Windows 11 Home build 26200, AMD Ryzen 7 7800X3D,
33,410,678,784 bytes RAM, WGPU/DX12, the same `--demo --demo-chat` fixture,
default viewport, and the standard voice-enabled `cargo xtask package` profile.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 67,598,848 | 68,078,080 | +479,232 (+0.709%) |
| Full portable package, bytes | 71,690,121 | 72,173,873 | +483,752 (+0.675%) |
| ZIP, `Compress-Archive` Optimal, bytes | 40,980,135 | 41,329,028 | +348,893 (+0.851%) |
| Idle CPU, one 10 s window after 15 s warmup | 0.155% | 0.010% | -0.145 percentage points |
| Peak working set, 11 samples at 1 s | 318,853,120 | 314,937,344 | -3,915,776 (-1.23%) |
| Settled working set | 318,853,120 | 314,929,152 | -3,923,968 (-1.23%) |

One package was built per revision. The preserved baseline executable and base
notice set were combined with the otherwise unchanged final staging tree to compare
the complete 194-file baseline package with the 195-file package that adds the OFL
notice. `makensis` was unavailable, so neither measurement includes an NSIS installer.
The OpenH264 LNK4255 warning was nonfatal in both package builds.

The CPU and memory differences come from one short idle sample and are treated as
noise, not an improvement. The deterministic cost is the 479,308-byte bundled
Noto Sans Math face plus its notice and small integration changes. Native screenshot
capture was unavailable because the Windows computer-use helper failed to initialize
with OS error 3; no visual, frame-time, startup, or live Discord claim is made.

## Large settings-proto responses — September 20, 2026

Baseline: `0ffd9b3`. After: this branch. Both used the standard voice-enabled
`cargo xtask package` profile with Rust 1.98.1 on Windows. One 195-file package
was built per revision; ZIPs use PowerShell `Compress-Archive` Optimal.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable, bytes | 68,147,200 | 68,147,200 | 0 |
| Full portable package, bytes | 72,243,085 | 72,243,085 | 0 |
| ZIP, `Compress-Archive` Optimal, bytes | 41,352,545 | 41,352,563 | +18 bytes (noise) |

The response cap grows from 1 MiB to 6 MiB so Discord's documented 5 MiB encoded
settings value fits with its JSON envelope. Responses below the old cap follow the
same path; there is no dependency, worker, persistent allocation or retained-layout
change. A maximum-size accepted response can transiently use up to 5 MiB more input
storage than before. CPU/RSS and live-account latency are unmeasured because no
authenticated account was used. The OpenH264 LNK4255 warning remained nonfatal;
`makensis` was unavailable, so both are unsigned portable packages.


## Linux sign-in window teardown, September 19, 2026

Compared baseline `d160c3e` with `f6498ce` on CachyOS Linux 7.2.6-1-cachyos,
AMD Ryzen 5 7600, 30 GiB reported RAM, Rust 1.98.1, GTK 4.22.5 and WebKitGTK
2.52.6. Both used `cargo xtask package --format dir`, the standard release
configuration including voice, without demo or developer-session features.
Baseline output stayed in a detached worktree; the changed platform crate was
cleaned before the final build to avoid reusing the baseline artifact from the
shared target directory. One package was measured per revision.

| Metric | Baseline bytes | After bytes | Delta |
| --- | ---: | ---: | ---: |
| Installed executable | 68,832,568 | 68,836,792 | +4,224 / +0.0061% |
| Installed regular-file total | 73,267,947 | 73,272,171 | +4,224 / +0.0058% |
| Compressed installation tree | 43,473,374 | 43,473,493 | +119 / +0.0003% |

The installed total sums regular-file sizes under `dist/linux-root`, including
notices. Compression used GNU tar with `--sort=name --mtime=@0 --owner=0
--group=0 --numeric-owner -C dist/linux-root -czf <archive> .` for each tree.
These are artifact sizes, not runtime performance or reproducible-build claims.
The packaged executable's linked libraries all resolved with `ldd`.

An isolated offline GTK/X11 diagnostic observed the test window from a second
X11 connection after `gtk_window_destroy`, without another GLib iteration.
The baseline window still existed; flushing GDK after destruction removed it.
This verifies buffered native destruction, not live Discord login or Wayland.
The login pump retains its 16-iteration / 2-ms callback budget and now flushes
queued display requests. Login CPU, RSS and frame/teardown latency are unmeasured;
separate offline Wayland validation confirmed teardown behavior without measuring
latency. That check used debug demo builds, a local HTML page and a synthetic
XHR-header handoff without sending the request, on Weston 15 inside an isolated
1280×960 Xvfb display. The native window remained after handoff on the baseline
and disappeared with the fix. This was not a live Discord login test.

## SDK account/channel data and invalidation events - September 22, 2026

Baseline: previous SDK head `e1a403b`. After: runtime source `3d94c76`.
Windows x64, Ryzen 7 7800X3D, 32 GB RAM, Rust 1.98.1, serialized Cargo builds.
The after revision also integrates main through `14e72bf`; this is a branch
comparison, not an isolated attribution of size or timing to the four new grants.

Release sandbox workload: `cargo run --locked --release -p extensions --example
sdk_check -- <wasm-directory>`. One warmup, five batches of 20 calls, median per
call. Each call creates a fresh sandbox and compiles its module; package parsing,
process startup, snapshot construction and worker IO are excluded. No Cargo build
ran during the timed calls. Baseline used the committed baseline modules extracted
to a temporary directory; only its committed-module rows are compared below.

| Committed-module workload | Before, us | After, us | Delta |
| --- | ---: | ---: | ---: |
| Protector activation | 1,066.635 | 1,171.655 | +105.020 / +9.85% |
| Image-sharing activation | 1,007.975 | 1,061.295 | +53.320 / +5.29% |
| Counter create event | 1,572.145 | 1,669.315 | +97.170 / +6.18% |
| Toolbox dashboard, 18,296-byte snapshot | 3,772.155 | 4,312.075 | +539.920 / +14.31% |

The first three committed modules are unchanged. Toolbox grew from 173,786 to
198,370 Wasm bytes and now builds the additional data summaries; its JSON package
is 574,992 bytes. It remains optional, not embedded in the production app.
The after rebuilt Toolbox run measured 4,036.745 us for identical Wasm, illustrating
run-order/host noise. These single-session samples show no established stable
regression or improvement; the observed dashboard median rose about 0.54 ms.
New account/server/channel groups and all 11 event kinds were separately checked
in the real sandbox; the timed dashboard uses the same legacy snapshot shape.

The collector retains its 64 KiB serialized snapshot limit, allocating from
already-loaded data only at invocation. Per-group byte/item bounds and the shared
32-item / 64 KiB event queue remain explicit. Detailed events coalesce per kind;
no background timer or persistent plugin process was added. Native screenshots,
CPU/RSS and frame latency are unavailable: native automation is disabled, `orca`
is absent, and browser CUA initialization fails with OS error 3. No live-account
or native UI performance claim is made.

Both standard `cargo xtask package` builds passed, including voice and excluding
demo/developer-session features. One package per revision; .NET ZipFile Optimal
compression of the full `dist` directory. Affected release crates were rebuilt
from each worktree to avoid stale shared-target artifacts.

| Artifact, bytes | Before | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 70,361,600 | 71,060,992 | +699,392 / +0.9940% |
| Installed package | 74,463,927 | 75,163,319 | +699,392 / +0.9392% |
| Portable ZIP | 42,471,539 | 42,708,512 | +236,973 / +0.5580% |

The OpenH264 LNK4255 warning was nonfatal in both builds. `makensis` is unavailable,
so these are unsigned portable packages, with no NSIS installer measurement.

## SDK message metadata and relationships - September 22, 2026

Baseline: `bf66cf4` (runtime identical to `3d94c76`). After: `20e47b2`.
Both contain main through `14e72bf`. Windows x64, Ryzen 7 7800X3D, 32 GB RAM,
Rust 1.98.1, serialized Cargo builds. The verified baseline package was preserved
before editing; changed extensions, UI and desktop crates rebuilt from this worktree.

Release `sdk_check` invocation medians use one warmup and five batches of 20 calls,
each with a fresh sandbox/module compilation. Package parsing, process startup,
snapshot collection and worker IO are excluded. No concurrent Cargo build ran
during timed calls. The same legacy input shapes are compared at both revisions;
new metadata/relationship groups and all 13 app events passed separate sandbox
checks, including the desktop collector's all-grants fixture.

| Committed-module workload | Before, us | After, us | Delta |
| --- | ---: | ---: | ---: |
| Protector activation | 1,102.360 | 1,119.970 | +17.610 / +1.60% |
| Image-sharing activation | 982.155 | 1,043.825 | +61.670 / +6.28% |
| Counter create event | 1,629.025 | 1,644.250 | +15.225 / +0.93% |
| Toolbox dashboard, 18,296-byte snapshot | 4,236.245 | 4,241.240 | +4.995 / +0.12% |

The first three committed modules are unchanged. Toolbox grows from 198,370 to
231,762 Wasm bytes; its optional JSON package is 671,753 bytes, not embedded in
production. Its identical rebuilt-module repeat measured 4,163.915 us. These
single-session variations do not establish a stable timing change.

New message details have an 8-KiB/20-record ceiling, nested rows share 4 KiB per
message, and relationships have a 4-KiB/100-record ceiling. Both consume the
remaining shared 64-KiB snapshot budget. When message details are also granted,
the text timeline uses 20 rows instead of 50; its 20-KiB byte budget is unchanged.
This lets the all-grants fixture fit the unchanged 5,000,000-fuel sandbox budget;
valid wire size alone still cannot guarantee arbitrary plugin execution. Existing
timeline-only grants retain their 50-row ceiling. Queue limits remain 32 items /
64 KiB, with ten starts per second and no new worker or timer.

Native screenshot/CPU/RSS/frame evidence remains unavailable: native automation
is disabled, `orca` is absent, and browser CUA initialization fails with OS error 3.
These are synthetic sandbox measurements, not live Discord or native UI evidence.

Standard voice-enabled `cargo xtask package` passed at both revisions, without
demo/developer-session features. One package per revision, .NET ZipFile Optimal
compression over the complete `dist` tree:

| Artifact, bytes | Before | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 71,060,992 | 71,107,072 | +46,080 / +0.0648% |
| Installed package | 75,163,319 | 75,209,399 | +46,080 / +0.0613% |
| Portable ZIP | 42,708,512 | 42,731,126 | +22,614 / +0.0530% |

OpenH264 LNK4255 was nonfatal. `makensis` is unavailable, so no NSIS installer
was produced; the unsigned portable distribution was measured.

### Typed-manifest authoring follow-up

The follow-up to `44f46d2` adds SDK metadata types, an offline manifest checker,
contract tests and docs only. The desktop uses this SDK as a development
dependency; no production host code, dependency, invocation or UI behavior changes.
The preceding native package measurements remain the runtime evidence; no new
native package or UI performance claim is made for authoring-only changes.

All four example plugins rebuilt and passed the existing release sandbox checks.
Three Wasm modules were byte-identical to the pre-edit artifacts. Message Counter
remained 122,576 bytes with a different hash; both its committed and rebuilt
modules passed the event/storage checks. Shipped package files were unchanged.

## SDK channel and member coverage - September 22, 2026

Baseline: `8861300` (production runtime identical to `20e47b2`). After: the
channel/member coverage follow-up on PR #373. Windows x64, Ryzen 7 7800X3D,
32 GB RAM, Rust 1.98.1, serialized shared-target Cargo builds. The verified
baseline package was copied before edits; its executable SHA-256 was
`92dfb897c6ff8719b61029289cd24e50f8be5b3ebca2d08338af372dd45fc710`.

The release `sdk_check` workload uses one warmup and five batches of 20 calls,
a fresh sandbox/module compilation per call, and unchanged committed plugins
and input shapes. Package parsing, startup, snapshot collection and worker IO
are excluded; no concurrent Cargo build ran during these timed calls.

| Committed-module workload | Before, us | After, us | Delta |
| --- | ---: | ---: | ---: |
| Protector activation | 1,178.240 | 1,097.925 | -80.315 / -6.82% |
| Image-sharing activation | 1,042.590 | 989.220 | -53.370 / -5.12% |
| Counter create event | 1,682.165 | 1,606.670 | -75.495 / -4.49% |
| Toolbox dashboard, 18,296-byte snapshot | 4,268.025 | 4,131.795 | -136.230 / -3.19% |

These single-session variations do not establish a stable speed improvement.
The rebuilt Toolbox measured 4,291.690 us; its Wasm grew from 231,762 to
274,763 bytes as the SDK gained optional types. The committed Toolbox stays
unchanged. New Guild Inspector is a separate optional 267,510-byte Wasm module
in a 773,922-byte JSON package, not embedded in the production executable.
Both committed/rebuilt Inspector packages passed real sandbox invocation with
new data and event kinds. An immutable older App Toolbox compiled at `3d94c76`
also passed current-host invocation without rebuilding its Wasm.

The two new data groups each have a 6-KiB wire ceiling. Member details additionally
cap members at 20, role IDs per member at 32 and catalog roles at 32; the collector
charges item/nested storage against its group budget. Groups consume remaining
space in the existing 64-KiB snapshot. New event kinds share the existing 32-item /
64-KiB queue and ten starts/second. No new cache, dependency, worker or timer.
No lifecycle tests were added. Native screenshots, CPU/RSS and frame timings remain
unavailable; these synthetic checks do not establish live Discord compatibility.


The standard voice-enabled `cargo xtask package` passed at coverage source
`ac48e1c`, without demo/developer-session features. One package per revision;
.NET ZipFile Optimal compression of the full `dist` directory:

| Artifact, bytes | Before | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 71,107,072 | 71,197,184 | +90,112 / +0.1267% |
| Installed package | 75,209,399 | 75,299,511 | +90,112 / +0.1198% |
| Portable ZIP | 42,731,126 | 42,757,192 | +26,066 / +0.0610% |

The changed executable SHA-256 is
`bcc962c1ea5c936e22ce32b5eed785faba5f9f4b5e55a38a3974a5293c1d1151`.
OpenH264 LNK4255 was nonfatal. `makensis` is absent, so the unsigned portable
package was measured; no NSIS installer was produced.


## SDK discovery and rich data - September 22, 2026

Baseline: `7d3def0` (production runtime identical to `ac48e1c`). After: the
host-discovery/rich-data follow-up on PR #373. The final branch also integrates
main through `f3a165f`; the package comparison includes those intervening UI/core
changes and must not be attributed solely to SDK code. The invocation timings
below isolate the unchanged extension host inputs/modules across the SDK change.
Windows x64, Ryzen 7 7800X3D,
32 GB RAM, Rust 1.98.1, serialized shared-target Cargo builds. The previous
verified package was copied before edits; its executable SHA-256 was
`bcc962c1ea5c936e22ce32b5eed785faba5f9f4b5e55a38a3974a5293c1d1151`.

Release `sdk_check` medians use one warmup and five batches of 20 calls, each
with a fresh sandbox/module compilation. Package parsing, process startup,
snapshot collection and worker IO are excluded. No other Cargo build ran during
timed calls. Committed modules and their app inputs are unchanged; the current
host additionally injects its public support catalog into each invocation.

| Committed-module workload | Before, us | After, us | Delta |
| --- | ---: | ---: | ---: |
| Protector activation | 1,138.095 | 1,136.195 | -1.900 / -0.17% |
| Image-sharing activation | 1,002.415 | 1,066.020 | +63.605 / +6.35% |
| Counter create event | 1,650.570 | 1,893.125 | +242.555 / +14.70% |
| Toolbox dashboard, 18,296-byte app snapshot | 4,210.395 | 4,212.230 | +1.835 / +0.04% |

The counter median increased about 0.24 ms in this session; the committed
Toolbox dashboard was nearly unchanged. These are observed overheads, not a
stable cross-machine regression estimate. Rebuilt Toolbox measured 4,731.850 us
(previously 4,225.275 us); its expanded SDK module grew from 274,763 to 322,343
Wasm bytes. Existing committed packages remain unchanged. Conversation Inspector
is a new optional 312,941-byte Wasm / 905,809-byte JSON package, not embedded in
production. All six committed/rebuilt plugin pairs and the immutable legacy
App Toolbox passed real sandbox checks, including new data/discovery/events.

Rich content is bounded to 10 rows / 8 KiB, forum data to 10 threads / 6 KiB,
and activity to eight typing IDs plus twenty pin IDs / 2 KiB. They share the
64-KiB app snapshot cap. Discovery counts against the existing 256-KiB invocation
cap, slightly reducing space for other input fields. Queue/rate/fuel limits
are unchanged; no new dependency, cache, worker or timer. Poll detail and forum
tag data remain unsupported by core state. No lifecycle tests were added.
Native screenshot/CPU/RSS/frame evidence remains unavailable; synthetic sandbox
measurements do not establish live Discord compatibility.


The standard voice-enabled `cargo xtask package` passed at runtime source
`a248c79`, without demo/developer-session features. Full `cargo xtask check`
passed after integrating main through `f3a165f`: 1,055 tests passed, 20 ignored,
strict workspace Clippy, no-default desktop compilation and policy checks passed.
One package per revision; .NET ZipFile Optimal compression of the full `dist`:

| Artifact, bytes | Before | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 71,197,184 | 71,307,264 | +110,080 / +0.1546% |
| Installed package | 75,299,511 | 75,409,591 | +110,080 / +0.1462% |
| Portable ZIP | 42,757,192 | 42,799,245 | +42,053 / +0.0984% |

The changed executable SHA-256 is `36f3d1c8329246f19ba33253576efdba6d38e421b13f32911a623ca9222c8292`.
This comparison includes the intervening main changes described above.
OpenH264 LNK4255 was nonfatal. `makensis` is absent, so no NSIS installer
was produced; measurements describe the unsigned portable package.


## Partial user profiles ? September 22, 2026

Compared clean baseline `fa10fec` with runtime change `d025e84` on Windows 11
Home 10.0.26200, Ryzen 7 7800X3D, 31.1 GiB RAM, Rust 1.98.1. One standard
`cargo xtask package` per revision, voice included, no demo/developer-session
features; full portable directory compressed with .NET ZipFile Optimal.

| Artifact, bytes | Before | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 71,307,264 | 71,311,360 | +4,096 / +0.0057% |
| Installed package | 75,409,561 | 75,413,657 | +4,096 / +0.0054% |
| Portable ZIP | 42,799,281 | 42,801,211 | +1,930 / +0.0045% |

Changed executable SHA-256:
`159a23116d5d9bce5a1f7d22d1189439df6cda6c9a8de603b2d9cceb57df4ccb`.
Both packages passed; OpenH264 LNK4255 was nonfatal. NSIS is unavailable,
so these are unsigned portable packages, not installer measurements.

`cargo replay`, one warmup then five direct executable runs per phase:
before 56.7939, 56.5201, 57.8881, 67.8447, 62.4075 ms; after 60.0030,
61.8068, 61.2891, 59.8946, 60.7432 ms. Median 57.8881 ? 60.7432 ms
(+2.8551 ms / +4.93%). Both retain 339,992?340,477 estimated bytes / 500
records after 100,000 events. The reducer dependencies are unchanged, so the
same reducer binary was reused. Variation under concurrent build load is not
evidence of a profile performance regression or improvement; replay does not
exercise profile decoding or UI rendering.

Native before/after interaction, CPU/RSS and frame timing remain unmeasured:
Computer Use could not connect to its native pipe (`os error 2`). Headless
profile UI tests passed but do not establish native or live Discord behavior.


## SDK app actions - September 22, 2026

Compared the SDK host at `e761bf0` with the app-action expansion on Windows,
Rust 1.98.1, release `sdk_check`: one warmup followed by five samples of twenty
fresh-runtime invocations, reporting the median. Both use the same 18,296-byte
synthetic Toolbox snapshot. The task also incorporates main through `6367d3d`;
that intervening hover change does not affect the extension host crate.

| Metric | Before | After | Delta |
| --- | ---: | ---: | ---: |
| Committed Toolbox invocation | 5,597.285 us | 6,007.710 us | +410.425 us / +7.33% |
| Rebuilt Toolbox invocation | 5,555.735 us | 5,981.995 us | +426.260 us / +7.67% |
| Rebuilt Toolbox Wasm | 334,225 bytes | 359,380 bytes | +25,155 bytes / +7.53% |

The host now advertises ten additional capabilities; rebuilt SDK code includes
35 typed actions and two optional preference snapshots. This is measurable
invocation overhead, not a claimed performance improvement. Timing is a single
local comparison under ordinary development load, not a cross-machine guarantee,
UI latency or snapshot-construction measurement. Existing committed plugins and
all rebuilt examples pass the real sandbox check at unchanged fuel/memory limits.

Conversation Actions is a new optional example: 346,486 Wasm bytes and a
1,003,107-byte JSON package. It is not added to the production bundle or catalog.
One foreground proposal remains capped at 8 KiB, snapshots at 64 KiB, ABI buffers
at 256 KiB, and local participant overrides at 64 slots. There is no new dependency,
worker, timer, network API or cache. Native screenshots/CPU/RSS/frame measurements
are unavailable because Computer Use cannot connect to its native pipe (`os error 2`).
Both standard voice-enabled Windows packages built successfully with
`cargo xtask package`, using one build job and the same shared dependency cache.
The changed crates were cleaned before each build to prevent stale cross-worktree
artifacts. Baseline `7a64a4611067a11308f8e99f300631b760573608` and implementation
`a0dbc87ccf9875f5b34ada0433a3992c7816444e` outputs were retained separately.

| Package metric | Before | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 71,403,008 bytes | 71,844,864 bytes | +441,856 / +0.62% |
| Installed directory | 75,505,692 bytes | 75,947,548 bytes | +441,856 / +0.59% |
| Portable ZIP | 42,842,855 bytes | 42,970,627 bytes | +127,772 / +0.30% |

Installed size sums all files in each fresh `dist` directory. ZIPs use .NET
`ZipFile.CreateFromDirectory` with Optimal compression and no enclosing directory.
Both builds reported the existing OpenH264 LNK4255 debug-information warning;
NSIS was unavailable, so no Windows installer was produced. Packaging does not
establish live Discord interoperability.


The real native demo snapshot also exposed a pre-existing App Toolbox fuel
failure, reproduced on clean `7a64a46`. The collector now limits combined
`timeline` plus `message_details` to 12 rows each and reports truncation.
Timeline-only reads retain 50 rows, metadata-only reads retain 20. The unchanged
real-Wasm demo regression test and all ten desktop SDK integration tests pass
with the same 5,000,000-fuel limit. The timing table above uses its original fixed
synthetic snapshot; it does not measure this collector reduction.

## Startup parsing, sidebar scans and package trims - September 24, 2026

Baseline: `f164494e`. macOS 27.0, Apple M1 Pro, 16 GiB RAM, Rust 1.98.1
aarch64-apple-darwin. Both packages come from `cargo xtask package` (voice included, no
demo features) in separate worktrees and target directories.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 58,113,312 | 57,537,488 | -575,824 (-0.99%) |
| Installed `dist` bytes (205 files) | 64,119,376 | 63,543,552 | -575,824 (-0.90%) |
| `ditto -c -k --sequesterRsrc dist` ZIP bytes | 42,031,473 | 41,752,881 | -278,592 (-0.66%) |
| READY permission projection, 9.5 MiB / 200 guilds / 20,000 channels, ms | 27.8-28.1 | 17.2-18.0 | about -37% |
| Sidebar badge scan, 200 guilds / 20,000 channels / 50 roles, ms | 23.8 | 6.1 | -74% |
| `:` emoji suggestion refresh, 100 servers x 100 custom emoji, ms | 0.92-1.31 | 0.42-0.58 | about -58% |

The READY row times `Envelope::permissions` on a synthetic release-mode payload, three
runs each. The permission projection now decodes each guild while splitting the array
rather than collecting raw values and parsing every guild again, so each guild's JSON is
scanned once. `navigation` (17.5-19.1 ms) and the envelope split (10.3-10.7 ms) are unchanged.

The sidebar row times the rail badge loop (`lights_guild_rail` plus `mention_count` for
every channel) over 20 release-mode iterations. It runs on the UI thread after every
rail-revision change, such as a message in any channel. With 4,000 cached decisions, one
scan of more than 4,000 channels evicted its own entries and recomputed every role map.
The cache now holds 32,768 decisions (7.7 ms), and `mention_count` checks the mention count
before permissions (6.1 ms). Real per-entry size is about 72 bytes plus B-tree overhead,
within the 128-byte estimate. Admission still reserves only 4,000 entries; the cache
grows beyond that only into permission budget left free by the admitted metadata.

The emoji row times `mentions::Menu::refresh` in release mode, 200 iterations per query
(`:sm`, `:smile`, `:zzq`). ASCII names are now compared case-insensitively in place instead of
through a lowercase copy for each of the roughly 16,000 candidate names. Refresh runs twice
per frame while a `:` query is open.

Of the size delta, 88,889 bytes come from re-encoding `assets/icons/atlas.png` with
`oxipng -o max --strip all` (pixel-identical). The remaining 486,935 bytes come from compiling
out dependency `log` calls in release builds (nothing ever installs a logger) together
with dropping unused SQLite extensions (FTS3/4/5, R-tree, dbstat, soundex, STAT4) through
`LIBSQLITE3_FLAGS`. These two were measured together. Mach-O page alignment makes byte
deltas under 16 KiB invisible.

The decoded Phosphor atlas (512x832 RGBA, 1,703,936 bytes) is no longer kept in a static
after upload. That figure is computed from its dimensions, not measured as process RSS.
On Windows, the unread-badge recount no longer arms a 1 s repaint while the window is idle.
It runs at most 1 s after a frame instead. That Windows path was not run locally, and
idle CPU was not measured on any platform.

Rejected after measurement: a zstd raw-RGBA Twemoji atlas would save 929 KB but decodes in
44.5 ms against 24.3 ms for the PNG at startup. Writing zlib output straight into the
growing buffer saved 0.3 ms per 8 MiB. No live Discord session was used.

## Interface zoom down to 50% (October 2, 2026)

Source `8fc9d4df7c6819d77828ceb83a7c279fd87603b9` has the same application runtime
as `9f0126b7`; its additional assertions exercise the real SDK host. Parent
`47a81035` is runtime-identical to the preserved `ef9cd5d1` baseline. Both native
binaries use pinned Rust 1.98.1, repository fat LTO, default features plus `demo`;
the standard package separately includes voice with no default features.

On Apple M1/macOS 27.0 (26A428), 16 GiB, native Metal and 2× display scale, both
apps opened `--demo --demo-settings=appearance` at the unchanged 100% default.
All other builds/apps were paused. Each sequential sample used a five-second
warmup and ten one-second macOS `ps` CPU/RSS readings (15.22 seconds total),
then stopped by SIGINT. Settled RSS is the last-five median.

| Metric | Parent | After | Delta |
| --- | ---: | ---: | ---: |
| Native median CPU | 0% | 0% | 0 percentage points |
| Sampled peak RSS | 132,336 KiB | 132,432 KiB | +96 KiB (+0.073%) |
| Settled RSS | 132,288 KiB | 132,384 KiB | +96 KiB (+0.073%) |
| Standard executable | 62,088,304 B | 62,088,304 B | 0 B |
| Full installed package | 68,098,733 B | 68,098,733 B | 0 B |
| Compressed distribution | 43,286,187 B | 43,286,963 B | +776 B (+0.0018%) |
| Default + demo fat executable | 63,988,160 B | 63,988,160 B | 0 B |

Standard packages contain 206 regular files; ZIPs use the same
`ditto -c -k --sequesterRsrc` method without an enclosing directory. The small
RSS difference is noise, with no improvement/regression claim. This idle check
does not time changing zoom or establish frame/startup latency. Actual native
matched chat captures separately show 80% before and 50% after; all temporary
instrumentation is removed. No live account or audio device was used.

Fresh full checks, the constrained old-schema migration/reopen/default INSERT,
focused model/UI/settings checks, all tutorial/catalog builds/tests/lints and
real committed/rebuilt SDK host boundary checks pass. The nineteen-page authoring
wiki is published from `8fc9d4df` as preview, not released. Raw samples, hashes,
source comparisons and screenshot provenance are in
[zoom measurements](pr-evidence/smaller-interface-zoom/measurements.json).

## Optional diagnostics shortcut (October 2, 2026)

Focused diagnostics, composer-typing and preference compatibility tests plus
fresh full checks passed on runtime source `9275aad5`. Package source `ec22a013`
adds only the absolute macOS icon output path and leaves application code unchanged.
Actual native captures compare exact parent `47a81035` with runtime `9275aad5`,
using the same keybinds window and synthetic scroll to the bottom.

Measured on macOS 27, Apple M1 / 16 GiB, Rust 1.98.1 and locked dependencies.
The standard voice-inclusive packages use unchanged fat LTO, no development
features, and local ad-hoc signatures. Parent package source `ef9cd5d1` is
application-identical to parent `47a81035`; installed bytes sum regular files,
and ZIP uses `ditto -c -k --sequesterRsrc` over the complete distribution.
Both standard packages passed; these are isolated feature revisions, not the
newer main aggregate containing unrelated features.

| Metric | Parent | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable, bytes | 62,088,304 | 62,088,304 | 0 (0%) |
| Installed distribution, bytes | 68,098,733 | 68,098,733 | 0 (0%) |
| Complete ZIP, bytes | 43,286,187 | 43,289,301 | +3,114 (+0.0072%) |
| Common native median process CPU | 0.0% | 0.0% | 0 percentage points |
| Peak process RSS, KiB | 128,944 | 128,880 | -64 (-0.050%) |
| Settled process RSS, KiB | 128,896 | 128,832 | -64 (-0.050%) |

Both optimized native binaries use matching process-only thin LTO:
`CARGO_PROFILE_RELEASE_LTO=thin CARGO_BUILD_JOBS=2 cargo build --release --locked -p serein --features demo`.
This does not change the repository fat-LTO profile. All runtime workspace
dependencies compiled from the intended worktree; binaries contain no capture
hooks. Both runs use identical flags recorded in the raw evidence, a 1120×760
logical viewport at 2× scale and Metal. A five-second warmup precedes ten
one-second macOS `ps` samples; settled RSS is the final-five median. All other
compilers, tests, helpers and native demos were paused during the paired sample.
Small RSS differences and quantized idle CPU are noise, with no improvement claim.
Action/request latency, GPU memory and frame/startup latency remain unmeasured.
Native synthetic input proves application handling, not physical OS routing or
live Discord compatibility. Source identities, hashes, package sizes, capture
metadata and all samples: [issue-diagnostics-shortcut/measurements.json](pr-evidence/issue-diagnostics-shortcut/measurements.json).

## Windows WebM container admission - September 25, 2026

Baseline: `7bdf862`. Windows x86_64, Rust 1.98.1. Both standard release packages include
voice and contain 198 files. ZIPs use PowerShell `Compress-Archive -CompressionLevel Optimal`.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| `dist/serein.exe` | 73,463,296 bytes | 73,463,808 bytes | +512 (+0.0007%) |
| Installed `dist` bytes | 77,566,012 | 77,566,632 | +620 (+0.0008%) |
| Portable ZIP bytes | 43,341,577 | 43,341,880 | +303 (+0.0007%; compression noise) |

A three-second 320x180 VP9/Opus WebM synthesized from the existing fixture decoded its
first video frame through Media Foundation with 48 kHz audio metadata. The existing MOV
decode/seek tests also passed. Native UI CPU, memory, frame timing and screenshots were
not measured because desktop capture/control is unavailable; no performance improvement
or universal Windows codec coverage is claimed.

## Windows rounded corners - September 25, 2026

Compared clean baseline `9013b20` with the Windows DWM corner-preference change on Windows
x64, Rust 1.98.1. Both standard `cargo xtask package` builds include voice and contain 198
files. ZIPs use .NET `ZipFile` with Optimal compression over the complete `dist` directory.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| `dist/serein.exe` | 73,463,808 bytes | 73,463,808 bytes | 0 |
| Installed `dist` bytes | 77,566,604 | 77,566,604 | 0 |
| Portable ZIP bytes | 43,341,873 | 43,341,978 | +105 (+0.0002%; compression noise) |

Native UI CPU, memory, frame timing and before/after screenshots remain unmeasured because
the untouched baseline's offline demo does not compile: existing fixtures omit the new
`Member.clients` field and a demo-only slider check is not exported to the binary. The
standard authenticated build was not launched for evidence. No performance change is claimed.

## Short initial-history pagination (October 2, 2026)

Runtime source `9a5c7a0f8b0224a7ee168fba47325c7dc5168933` integrates parent
`456fdc1ff3925b9a84cae1295e5605b3c60a34b5`. Explicit upward input over a short
message page can request older history; idle/downward input does not drain pages.
Pending frames retain browsing intent, and an arriving older page preserves the
existing message's rendered position. Final scroll offsets remain nonnegative.

Fresh `cargo xtask check`, the standard voice-enabled release package and the
normal fat-LTO default+demo build passed. The real egui regression covers input,
pending frames, an actual older history event and rendered message restoration
within 2px; it also proves intentional return-to-latest still works. No UI design
change is illustrated with an unrelated screenshot.

| Metric | Comparator 343c6d48 | Changed 9a5c7a0f | Delta |
| --- | ---: | ---: | ---: |
| Standard executable | 62,137,600 B | 62,137,616 B | +16 B |
| Installed package, 206 regular files | 68,148,029 B | 68,148,045 B | +16 B |
| Same-method ZIP | 43,311,255 B | 43,311,312 B | +57 B |
| Normal fat-LTO default+demo executable | 64,037,760 B | 64,037,760 B | 0 B |
| Native idle CPU median | 0% | 0% | 0 percentage points |
| Native sampled peak RSS | 124,624 KiB | 124,480 KiB | -144 KiB (-0.116%) |
| Native settled RSS | 124,592 KiB | 124,416 KiB | -176 KiB (-0.141%) |

The preserved comparator is the Control-click source 343c6d48. Parent 456fdc1f
additionally includes PR #513 active-server emoji ordering. The only runtime source
differences from the comparator are that additive emoji-picker change and this
fix, verified by a scoped diff; the picker is closed in this workload. These are
honest starting-comparator measurements, not isolated per-feature cost figures.

Both instrument-free builds use identical normal release flags and
`--demo --demo-chat` on Apple M1/macOS 27, 16 GiB, native Metal. All task owners paused
compilers/apps for the pair: five-second warmup, ten one-second process CPU/RSS
samples, settled RSS from the final five samples, intentional SIGINT termination.
The small RSS difference is noise and is not an improvement claim. Active history
request/response latency, p95 frames, GPU memory and live service behavior are
unmeasured. Raw samples, hashes and provenance are in
`docs/pr-evidence/short-history-pagination/measurements.json`.

## Compact IRC chat layout (October 2, 2026)

Exact parent `71ebbc1c0393a0ba4f4e6c93ae9b7b0bd3e06d35` compared with
`e5b98f831568094d40ca779267601ee279ed13bb`. The runtime difference is limited to
compact message authors/spacing, leading-block layout and English/Czech labels.
Both revisions use the same pinned toolchain, lockfile and voice-inclusive shipping package.
Workspace-name artifacts were invalidated across worktree PackageIDs before fresh builds;
all twelve runtime crates compiled from their own worktrees. Immutable copied packages
passed strict/deep ad-hoc signature verification; they are not notarized releases.

| Same-method standard package | Parent71 | Compact e5 | Delta |
| --- | ---: | ---: | ---: |
| Executable | 62,269,488 B | 62,269,504 B | +16 B (+0.00003%) |
| Installed files | 68,279,917 B | 68,279,933 B | +16 B (+0.00002%) |
| ZIP | 43,363,423 B | 43,364,562 B | +1,139 B (+0.00263%) |
| File count | 206 | 206 | 0 |

The package uses normal fat LTO. A separate matched native pair uses instrument-free
**default-plus-demo thin LTO** builds on Apple M1/macOS 27/16 GiB/Metal; the process-only
`CARGO_PROFILE_RELEASE_LTO=thin` override and other release flags are identical.
The native workload is ordinary `--demo --demo-chat`, with five seconds warmup and ten
one-second `ps` samples. All four build owners explicitly paused compilers, native apps
and heavy IO before the pair. Both native processes stopped with the intended SIGINT.

| Quiet native idle | Parent71 | Compact e5 | Delta |
| --- | ---: | ---: | ---: |
| Median process CPU | 0.0% | 0.0% | 0.0 percentage points |
| Peak RSS | 125,616 KiB | 125,952 KiB | +336 KiB |
| Settled RSS (last-five median) | 125,568 KiB | 125,904 KiB | +336 KiB (+0.268%) |

This small RSS difference is process-to-process noise; no improvement is claimed.
Active compact frame timing is unmeasured. Two actual egui glyph/height regressions
verify ordinary/pending authors, shorter rows, density preservation, leading quote/code
and long-author bounds at 900/440 points. Fresh full workspace/strict/policy checks passed
(369 UI and 168 desktop tests). All thirteen current native after frames were captured
and inspected; historical before-frame source IDs remain explicit in the capture record.
Hooks were removed byte-exactly before packaging. Linux/Windows native appearance and
live service compatibility are unmeasured. Raw samples, sizes, hashes and build provenance
are in `docs/pr-evidence/compact-chat/measurements.json`; per-image identities are in
`docs/pr-evidence/compact-chat/capture.json`.

## History copies and decoded-image backpressure — September 26, 2026

Baseline `dad3c26c`, compared with this PR on macOS 27.0 (26A428), Apple M1 Pro,
16 GiB RAM, pinned Rust 1.98.1 and locked dependencies. Component timing uses
release builds, one warmup and five measured batches, with no concurrent Cargo
build during sampling. The image queue workload measures allocated pixel capacity
in debug tests; it is not a timing or desktop-process RSS comparison.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Recent history completion, µs/page | 193.722 | 108.413 | -85.309 / -44.04% |
| Older history completion, µs/page | 101.516 | 12.237 | -89.279 / -87.95% |
| Append history completion, µs/page | 102.810 | 15.397 | -87.413 / -85.02% |
| 200 incremental SQLite saves, ms | 303.007 | 249.976 | -53.031 / -17.50% |
| Paused image consumer, queued pixel bytes | 536,870,912 | 130,023,424 | -406,847,488 / -75.78% |
| Queue workload process peak RSS, bytes | 555,302,912 | 151,552,000 | -403,750,912 / -72.71% |
| 100,000-event reducer replay, ms | 158.133 | 155.741 | -2.392 / -1.51%, small/noise |
| Retained replay timeline, estimated bytes / rows | 331,992–332,477 / 500 | 331,992–332,477 / 500 | Unchanged |

The history workload starts with 500 rows, each carrying the maximum 512 author
roles and a synthetic nickname, then completes a 50-row recent/older/append page.
Each sample averages 100 completions, excluding fixture construction. This is a
membership-heavy stress case, not a claim about typical chats. Incoming rows now
inherit directly from the prior timeline before replacement or eviction. This
removes a map containing 2,048,000 bytes of copied role IDs plus nickname/node
allocations per page in this fixture; it is not a measured RSS reduction.

The storage workload uses an in-memory SQLite database with 500 rows / 3,736,500
estimated message bytes, toggling one row's edited flag for every save. It includes
loading, validation, comparison and SQL work; it excludes filesystem latency.
Borrowed rows replace a second owned message window. Full saves add only a bounded
vector of at most 500 references. Transactions, account isolation and eviction
limits are unchanged.

The paused-consumer workload offers up to 128 separate 1024×1024 RGBA results.
Its explicit legacy comparator bypasses byte admission to reproduce the original
128-item channel. The new sender admits 31 results (124 MiB of pixels, 130,027,919
charged bytes including metadata), then waits on the next result. The workload
cancels that waiting send before draining. The production queue permits at most
128 items / 128 MiB of charged allocations, including results being consumed;
optional previews drop under pressure, final results wait cancellably. Eight
active/completed jobs, one coordinator result awaiting admission, decoder scratch,
UI/GPU caches and allocator overhead remain additional. This is not a 128 MiB
whole-app cap. The queue now requests another frame when a partial UI drain leaves
results behind.

Queue peak RSS uses macOS `/usr/bin/time -l` around the emitted `serein` debug test
executable, invoked directly with
`avatars::tests::decoded_result_queue_workload --ignored --exact --nocapture`.
There was one separate process per mode, with `SEREIN_IMAGE_QUEUE_LEGACY=1` only
for the original item-only comparator; no compiler was running. This includes the
test runtime and the next producer allocation before it blocks, unlike the queued
pixel count. It is an isolated component process, not the running desktop app.

Replay uses one warmup and five direct executable runs after building each revision.
Its ordinary message stream scarcely exercises these changes; the small timing
variation is not claimed as a general reducer improvement. No live Discord account,
voice call, microphone or camera was used. There is no new dependency, schema or
asset change. The bundle audit retained existing fat LTO, stripping, compressed CJK
fonts and Twemoji artwork rather than reducing language/emoji coverage.

Reproduce the focused workloads:

```sh
cargo test --locked --release -p session-cache page_membership_benchmark -- --ignored --nocapture
cargo test --locked --release -p local-store benchmark_changed_row_save -- --ignored --nocapture
SEREIN_IMAGE_QUEUE_LEGACY=1 cargo test --locked -p serein avatars::tests::decoded_result_queue_workload -- --ignored --exact --nocapture
cargo test --locked -p serein avatars::tests::decoded_result_queue_workload -- --ignored --exact --nocapture
cargo replay
```

For historical history/storage comparisons, apply only the added benchmark tests
to `dad3c26c`; keep its production implementations unchanged. The queue comparator
runs from the new source and uses identical pixel allocations in both modes.

Both standard voice-enabled `cargo xtask package` builds passed, without demo or
developer-session features. Baseline and runtime commit `dbe18e56` outputs were
preserved separately. Installed size sums all 205 files in each complete `dist`;
ZIPs use `ditto -c -k --sequesterRsrc` over that directory, without `--keepParent`.
The macOS bundles are locally ad-hoc signed and verified, not notarized releases.

| Package metric, bytes | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 57,932,160 | 57,915,712 | -16,448 / -0.0284% |
| Full installed package | 63,938,384 | 63,921,936 | -16,448 / -0.0257% |
| Compressed distribution | 41,948,935 | 41,932,334 | -16,601 / -0.0396% |

These are small artifact deltas, not a substantive bundle-size optimization;
archive metadata and compression can vary between builds. Licenses/notices and
runtime assets are unchanged.

The native idle control uses release builds with `--features demo`, launched with
`--demo --demo-friends --demo-frame-sample=1,1`: 1120×760 logical pixels, 2× scale,
Apple M1 Pro Metal backend. Each revision has one launch, a ten-second warmup and
twenty `ps -p PID -o %cpu=,rss=` samples one second apart, with no input after
launch and no concurrent build. Settled RSS is the median of the last five samples;
peak RSS covers the sampling interval, not startup. Only the demo PID is included.

| Native idle metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Median sampled CPU | 0.75% | 0.45% | -0.30 percentage points; noisy |
| Sampled peak RSS, KiB | 146,192 | 146,352 | +160 / +0.11% |
| Settled RSS, KiB | 130,032 | 130,224 | +192 / +0.15% |

Idle RSS is essentially unchanged. The CPU difference is not claimed as a stable
improvement from these short runs. The demo disables downloaded-image workers, so
it controls for idle regressions rather than measuring the queue fix. System/GPU
resources are not fully represented by process RSS. Frame/startup latency remains
unmeasured; the frame diagnostic only confirmed matching viewport and scale.

## Flatpak WebKit locale preflight (October 2, 2026)

The Linux-only login/verification preflight checks the existing Flatpak marker
and environment presence, then GLib's effective encoding after GTK initialization.
It runs only on explicit window creation, before WebKit construction, and creates
no worker, cache, retained payload, retry or persistent setting. Non-Flatpak paths
retain their behavior. Existing synthetic authentication-handoff and four offline
Flatpak preparation tests passed; a Linux-only subprocess test uses actual GLib
encoding for native ASCII, Flatpak ASCII rejection, and Flatpak UTF-8 admission
without GTK display, WebKit, credentials or global environment mutation. A
separate display-backed regression repeats the cases after actual GTK
initialization under Xvfb in the Linux native CI job; it constructs no WebKit.

The development host is macOS 27 / Apple M1 / 16 GiB; this code is excluded from
its compiled runtime; desktop error callers reuse the existing static
`Failure::label()` mapper so the repair guidance remains visible. Native Linux
Flatpak startup/CPU/RSS and affected Linux
package deltas are unmeasured here, and no improvement is claimed. Linux CI and
reporter confirmation remain required. The preserved exact-parent `1107d904`
standard macOS package is 62,154,064 executable / 68,164,493 installed / 43,322,199
ZIP bytes (206 files), a host baseline rather than a Linux comparison. Full source
and standard-package checks are recorded in the task PR as they complete.

## Development data isolation (September 27, 2026)

Windows x86_64 package measurements compare clean `672ee68` with
`fix/isolate-development-storage`, both built using Rust 1.98.1 and
`cargo xtask package`. The package command disables default features, so this measures the
shipping OS-data-directory path rather than the worktree-local development path. NSIS was
unavailable; the installed-directory total covers the complete unsigned `dist` tree and the
compressed total is an optimal PowerShell ZIP of that same tree.

| Metric | Baseline | After | Delta | Method |
|---|---:|---:|---:|---|
| Packaged executable | 76,558,336 B | 76,556,800 B | -1,536 B (-0.002%) | `dist/serein.exe` file size |
| Installed package directory | 80,661,162 B | 80,660,045 B | -1,117 B (-0.001%) | Sum of 198 files under `dist` |
| Compressed distribution | 44,140,603 B | 44,141,033 B | +430 B (+0.001%) | PowerShell `Compress-Archive -CompressionLevel Optimal` |

The deltas are immaterial build/link/compression noise. CPU, RSS and rendering measurements are
not applicable because the change only selects the persistent-data root before those existing
workers open their files; it adds no polling, queue, network request or render work.

## Live stream preview (September 27, 2026)

Windows x86_64 package measurements compare `origin/main` at `1ecf8d16` with
`feat/live-stream-preview` at `54c34e66`, both built using Rust 1.98.1 and
`cargo xtask package`. Each installed-directory total covers the same 198 files; ZIPs use
PowerShell `Compress-Archive -CompressionLevel Optimal`. NSIS was unavailable.

| Metric | Baseline | After | Delta |
|---|---:|---:|---:|
| Packaged executable | 76,688,384 B | 76,739,072 B | +50,688 B (+0.0661%) |
| Installed package directory | 80,791,700 B | 80,842,358 B | +50,658 B (+0.0627%) |
| Compressed distribution | 44,171,707 B | 44,184,003 B | +12,296 B (+0.0278%) |

The preview adds no polling or closed-popover rendering work: one visible hover starts one
latest-wins request capped at 4 KiB, and the still uses the existing 512-pixel media bounds.
Native CPU/RSS and frame timing were not measured because desktop capture/control was unavailable.

## Windows transparent caption controls — September 28, 2026

The earlier transparency-layer workaround did not fix the reported duplication.
Its measurements have been removed: they do not describe the final native-caption
fix. The final change suppresses `WS_SYSMENU` only for a blurred custom frame and
keeps that suppression across winit style rewrites. Native title-bar mode restores
the system controls. No dependency, worker, timer, or asset was added.

The comparison baseline is `f6e7cfb2` (before the working caption fix), Windows x64,
Rust 1.98.1, locked dependencies and standard voice-enabled `cargo xtask package`.
The baseline package was preserved separately; the measured package is `dd5af5f3`.
Installed size sums all 198 files. Both portable ZIPs were produced using .NET
`ZipFile.CreateFromDirectory` with `CompressionLevel.Optimal` and no enclosing
directory. This compares the final correction, not the complete branch against main.

| Package metric, bytes | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 76,688,896 | 76,692,480 | +3,584 / +0.0047% |
| Full installed package | 80,792,212 | 80,795,796 | +3,584 / +0.0044% |
| Portable ZIP | 44,171,877 | 44,173,235 | +1,358 / +0.0031% |

Both standard voice-enabled package builds passed. `makensis` is unavailable, so
the optional NSIS installer was skipped. Final matched release CPU, working-set,
frame/input-latency measurements are unavailable; no runtime performance
improvement is claimed from the old debug measurements or from screenshots.

Native synthetic evidence in `docs/pr-evidence/windows-transparency/` uses
`--demo --demo-transparency --demo-friends`, DX12/DirectComposition, 125% scale,
and a 1500 x 900 pixel window. Before is `f6e7cfb2`; after is the caption fix
following maximize, restore, minimize and restore. The owner also confirmed the
normal non-demo debug build works. These are visual checks, not benchmarks or
proof of live Discord interoperability.

The subsequent fullscreen review correction retains the latest requested system-menu
bit instead of always restoring it. Its expanded native-window regression test,
focused Clippy, and normal debug build passed, followed by owner confirmation.
The package sizes and screenshots above predate that correction; release sizes
and performance have not been remeasured for the review follow-up.

## Stream preview review fixes (September 28, 2026)

The comparison is the pre-review branch at `d65e5bfa` versus the five review fixes
at `cb26f2a9`, not the complete feature versus main. Both standard voice-enabled
packages passed `cargo xtask package` using Rust 1.98.1 on Windows 11 x64, a Ryzen
7 7800X3D and 32 GB RAM. The same isolated worktree/target built both revisions;
the baseline package was preserved before rebuilding. NSIS was unavailable.
Installed totals include all 198 files; portable ZIPs use .NET `ZipFile` with
Optimal compression and no enclosing directory.

| Package metric, bytes | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 76,755,968 | 76,756,992 | +1,024 / +0.00133% |
| Installed directory | 80,859,284 | 80,860,308 | +1,024 / +0.00127% |
| Portable ZIP | 44,192,527 | 44,192,549 | +22 / +0.00005% |

The release `replay-bench` executables process 100,000 synthetic message events.
Each batch below discards one warmup per revision and takes the median of five
direct executable runs; the second batch alternates baseline/after to reduce
time-varying machine load. No build was started by this task during sampling;
other desktop/background activity was not controlled.

| Replay timing, ms | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| First, separate batches | 132.50 | 145.87 | +13.37 / +10.09% |
| Paired rerun | 138.50 | 137.27 | -1.23 / -0.89% |
| Paired five-run range | 135.83-142.11 | 135.95-149.05 | overlapping |

The inconsistent timing delta does not establish a speedup or a repeatable
regression. Every run retained 500 records and 331,992-332,477 estimated timeline
bytes. This is a generic reducer control, not preview latency, process RSS, UI
frame timing or live Discord evidence. The fixes add no dependency, worker or
polling and preserve the one-request, 4 KiB response and 512-pixel media bounds.
Native interaction screenshots and CPU/RSS measurements remain unavailable:
the Computer Use module could not connect to its native pipe (`os error 2`).

# DX12 allocation policy - September 28, 2026

Compared clean baseline `5dd38dde6432e7efe4484c450652a9c8849ec357` with
implementation `dd4f4e191f79c241cea9e3d04332010d7d62b565`. The desktop now
preserves eframe's adapter-specific device descriptor and changes only the DX12
memory hint from `Performance` to `MemoryUsage`. This permits smaller allocation
blocks; it does not cap resource sizes, reduce texture limits, or change image
quality settings. Other backends retain their inherited hint. This measurement
applies to this NVIDIA/DX12 configuration; savings on other adapters are unmeasured.

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (8 cores / 16 logical processors),
31.116 GiB usable RAM, RTX 5070 Ti / DX12, NVIDIA driver 32.0.15.9186, Rust 1.98.1.
Both native executables used `cargo build --release --locked -p serein --features
demo`, retaining the default development-data feature and always-built voice.
Neither contains the earlier DHAT/allocator instrumentation. The release profile,
lockfile and toolchain were unchanged; the retry used one Cargo job to reduce
concurrent compiler memory pressure.

Five alternating baseline/after launches used `--demo --demo-friends
--demo-frame-sample=8,15`. Each requested an 8-second warmup, then sampled Windows
process counters for approximately 15 seconds at a requested 250 ms interval
(58 samples per run). GPU Process Memory counters were read for the exact PID
after each timed process sample and memory-map inventory. The requested viewport
was 1120 x 760; nine start records confirm that size and scale 1.25, while baseline
run 4 produced no frame marker. Background build/test processes were present in
all runs. These are memory observations, not a controlled CPU/latency benchmark.

[Per-run measurements and frame records](pr-evidence/dx12-memory/measurements.json)
include executable hashes and the number of background build processes at both
sampling endpoints. The GPU readings had status 0 on both adapter instances;
the table sums the exact process's instances. OS memory columns use the final
sample, not the window average. Peak below means the largest sampled value after
warmup, not a startup peak. MiB = 1,048,576 bytes.

| Median of five runs | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Dedicated GPU usage | 255.344 MiB | 115.344 MiB | -140.000 MiB / -54.83% |
| Shared GPU usage | 27.535 MiB | 27.535 MiB | unchanged |
| GPU total committed | 282.883 MiB | 142.883 MiB | -140.000 MiB |
| Process private commitment, final / sampled peak | 320.633 MiB | 181.434 MiB | -139.199 MiB / -43.41% |
| Process working set, final / sampled peak | 111.969 MiB | 111.852 MiB | -0.117 MiB; no resident-RAM benefit established |

Dedicated GPU usage ranged from 244.453-255.344 MiB before and 99.332-115.344 MiB
after. Individual paired reductions ranged from 129.109 to 156.012 MiB; the
median reduction was 140 MiB. Process private commitment ranged from
320.223-337.133 MiB before and 165.969-183.367 MiB after. Working-set ranges were
109.750-142.633 MiB and 111.695-112.230 MiB respectively. Driver/process accounting
overlaps these GPU counters: do not add the private-commit reduction to the GPU
reduction, or describe either as an equivalent reduction in resident system RAM.
No current heap-by-type or exact live GPU-resource occupancy comparison was made.

Observed process CPU medians were about 0.104% of one logical core in both
variants. The ranges were 0-0.104% before and 0-2.915% after. Counter quantization,
background work and missing matched focus/input windows prevent a CPU speedup or
no-regression claim. The OS sampling window is separate from the application's
frame-marker window. Only after runs 1 and 4 produced complete frame records:
after-1 was focus/input-valid (30 of 30) but had one **507.355 ms elapsed logic/UI
sample**; its other 29 samples were below 1 ms. After-4 had disturbed focus/input
(7 of 10 viewport-focused, 9 of 10 input-free) and a 13.300 ms maximum. None of
the baseline runs completed a frame window. The 507.355 ms observation remains
unresolved and must not be discarded. Pinned eframe calls logic and UI back to
back; this is wall time that can include blocking or descheduling, not GPU or
whole-frame latency. There is no matched baseline that attributes it to the hint.

Additional 15-second offline process checks launched chat, a static 640 x 360
stream texture, and a static 320 x 240 camera texture in both versions. All six
processes remained alive, Windows reported them responsive, and stderr identified
RTX 5070 Ti / DX12 without an additional error. These are initialization/static
texture checks, not inspected visual results, active codecs, sustained uploads,
scrolling, resizing, animation stress, or live Discord tests. Computer Use could
not connect to its native Windows pipe (`os error 2`), so interactive verification
remains unavailable. Keep the change in draft for hands-on checks, including the
unresolved timing observation. Smaller allocation blocks may increase allocation
work under churn; no claim of regression-free rendering is made.

`cargo test --locked -p serein gpu::tests` passed all five tests, including the
all-backend policy/descriptor check. The complete `cargo xtask check` passed
formatting, strict workspace Clippy, 1,146 tests (24 ignored), the desktop build
without default features, and policy checks. Its initial attempt hit MSVC linker
`LNK1102: out of memory` during competing builds; the one-job retry passed.

Both standard voice-enabled `cargo xtask package` builds use no default features
and exclude demo. Packages were preserved separately; installed totals include
198 files. ZIPs use .NET `ZipFile.CreateFromDirectory`, `CompressionLevel.Optimal`,
without a root folder. NSIS was unavailable, so no installer executable was made.

| Standard package, bytes | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 76,765,184 | 76,766,720 | +1,536 / +0.00200% |
| Installed directory | 80,868,500 | 80,870,036 | +1,536 / +0.00190% |
| Portable ZIP | 44,197,653 | 44,197,914 | +261 / +0.00059% |

The documentation/evidence commit follows these measured binaries and does not
change application code. Full raw captures, samplers, build logs and separately
preserved binaries are local at
`E:/codex-builds/serein-dx12-memory-evidence-20260928`.
## Custom Rich Presence editor and catalog follow-up (September 28, 2026)

Final source `e4a7c8c524a1e0f6eaf1b6b873284ccb8454d2d9` removes the shipped
plugin from the client binary and uses the separately published Serein-extensions
package. The standard voice-enabled release package passed on the same Windows
machine/toolchain as above. NSIS remains unavailable; the complete portable ZIP
uses .NET ZipFile with Optimal compression and no enclosing folder.

| Package metric, bytes | Original baseline `5dd38dde` | Final | Delta |
| --- | ---: | ---: | ---: |
| Executable | 76,765,184 | 76,983,808 | +218,624 |
| Installed directory | 80,868,500 | 81,087,124 | +218,624 |
| Portable ZIP | 44,197,608 | 44,282,657 | +85,049 |

Compared with the previous Custom RPC build `dcbc431b`, executable and installed
size decrease by 389,120 bytes; ZIP size decreases by 62,629 bytes. This is a
package-size measurement, not a runtime speed or memory claim. No new reducer
measurement was needed for this UI/package-only follow-up.

The native helper is now available: dark 1120x900 and light 800x760 synthetic
windows were inspected, with section navigation, editable text, scrolling and
composer-button absence checked. Captures are in `docs/pr-evidence/custom-rpc/`
(`native-after.jpg`, `native-light.jpg`, `native-scrolled.jpg`). A matched native
before capture was not collected. CPU/RSS/frame timing and live Discord
interoperability remain unmeasured.
## RAM allocation audit — September 28, 2026

This audit separates live application allocations, allocator retention, process RSS,
and GPU/driver memory. Component budgets are admission ceilings, not reservations
or an application-wide RAM cap. A smaller executable does not establish lower RAM.
Apple's [memory-footprint guidance](https://developer.apple.com/library/archive/technotes/tn2434/_index.html)
likewise distinguishes heap/anonymous-VM attribution from a single process total.
The code review used the locked dependencies, including image 0.25.10 and egui
`fe6d63ef`, rather than assuming APIs from their latest releases.

### Findings and decisions

| Area | Finding | Action |
| --- | --- | --- |
| Still-image decoding | Already-sized RGBA8 images occupied an intermediate RGBA buffer and a second egui pixel buffer simultaneously. A 4096² image needs 64 MiB per buffer. | Decode directly into the final pixel allocation and premultiply alpha with at most 64 KiB of conversion scratch. Preserve dimension, encoded-byte and decoder-allocation validation; other formats and resizing retain their established path. |
| Message updates | `clear` retained allocations and `clone_from` reused oversized allocations after large fields became small. These capacities remained charged to timeline budgets. | Release explicit null/empty fields; compact a shortened nonempty field only when its allocation exceeds 1 KiB and four times its new length. Ordinary edits retain allocation reuse. |
| Fonts and bundled artwork | CJK inflation is already lazy; imported font weights share bytes; system fallback uses memory-mapped files. Twemoji already decodes into one final pixel buffer and icon pixels are released after upload. | Preserve language coverage, artwork and existing sharing. No new dependency or allocator. |
| History and SQLite | Active/dormant timelines are moved; at most two dormant windows share the resident-history budget. Incremental SQLite saves borrow rows and the page-cache target is already 2 MiB. | Preserve limits, history previews, transactions and persistence semantics. |
| Animation residency | Legacy and inline animation caches each permit 128 MiB and can retain offscreen clips. Viewer pixels already release after the viewer stops painting. | Future candidate: measured offscreen grace-period eviction, compared against revisit latency, decode CPU and download activity. No arbitrary cache-cap reduction here. |
| Concurrent decoding | Eight jobs can hold encoded input, frame collections and decoder scratch outside the 128 MiB completed-result queue. | Future candidate: byte admission before expensive decode while preserving overlapping downloads. Measure mixed small-avatar/large-picture workloads before choosing a budget. |
| Retired image workers | Started [Tokio blocking decoders](https://docs.rs/tokio/1.53.1/tokio/task/fn.spawn_blocking.html) cannot be stopped by aborting their async waiter. Old/new worker decodes can overlap during replacement. | The eight-job limit is per worker, not a global bound across retired workers. Cancellation-aware decode and rapid-replacement stress tests merit separate work. |
| Stream rendering | `watch.rs` constructs a new ColorImage for each received stream frame; other video paths already reuse Arc buffers. | This is allocation churn, not proof of a leak. Profile a synthetic producer/renderer workload before changing frame ownership. |
| Large-account startup | Borrowed protocol projection and moved channel vectors avoid full copies, but old/new account state overlaps during validation with wire/decompression buffers. | Keep atomic validation and supported-account ceilings. A replacement-READY heap profile is needed before redesign. |

Rust documents that [`Vec::clear`](https://doc.rust-lang.org/std/vec/struct.Vec.html#method.clear)
retains capacity. Applying `shrink_to_fit` indiscriminately would trade repeated
allocations for tiny savings, so the patch compacts only empty or substantially
shrunk fields. Stale patches, absent fields, immutable forwarded bodies and history
reconciliation retain their existing behavior. Nested payload accounting remains
unchanged; this is not a new global compactor.

The direct image path preserves image 0.25.10's explicit output-buffer reservation
before decoding: `ImageReader::into_decoder` alone does not perform the reservation
that `ImageReader::decode` does. Pixel-equivalence tests cover every alpha value,
PNG/WebP/GIF, RGB/grayscale/16-bit input, JPEG, resized output and conversion
chunk boundaries. Bounded chunks use the existing optimized egui conversion
routine, including in debug builds; scratch is additional to decoder reservations. Decoder failure
releases the partially filled buffer. No unsafe conversion or reduced image quality
is introduced.

### Component measurements

Baseline `2d5345a` versus this change, macOS 27.0 (26A428), Apple M1, 16 GiB RAM,
Rust 1.98.1, locked dependencies. The clean starting checkout `dc82f22` was
fast-forwarded to the remote default branch before building. Baseline package,
demo, replay and cache-test executables were preserved before production edits.
Only the cache benchmark's test code was added to the baseline. No compiler ran
during measurement; all inputs were synthetic, with no account, microphone,
camera or live media session.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| 4096² RGBA PNG release decode, median process peak RSS, bytes | 146,554,880 | 79,364,096 | -67,190,784 / -45.85% |
| Same workload, release decode time, median ms | 23.823 | 23.132 | -0.691 / -2.90% |
| Final retained pixel allocation, bytes | 67,108,864 | 67,108,864 | Unchanged |
| 500 messages shortened from 7,000 to 6 bytes, estimated timeline bytes | 3,736,500 | 239,500 | -3,497,000 / -93.59% |
| Same workload, content string capacities, bytes | 3,500,000 | 3,000 | -3,497,000 / -99.91% |
| Same workload, release µs per 500 edits | 133.052 | 144.874 | +11.823 / +8.89% |
| Ordinary 64-to-6-byte edits, release µs per 500 edits | 132.277 | 137.433 | +5.156 / +3.90% |
| Ordinary edit workload, estimated timeline bytes | 268,500 | 268,500 | Unchanged |
| 100,000-event release reducer replay, median ms | 152.027 | 153.925 | +1.898 / +1.25%; overlapping ranges |
| Replay retained timeline, estimated bytes / rows | 331,992–332,477 / 500 | 331,992–332,477 / 500 | Unchanged |
| 60-second lifecycle soak, process peak RSS, bytes | 18,612,224 | 18,612,224 | Unchanged |

The image measurement uses the emitted **release** desktop test executable directly
under macOS `/usr/bin/time -l`: one warmup per mode, then five alternating legacy/
direct pairs, each in a fresh process. The test-only legacy comparator reproduces
the original decoder in the same executable. Its pregenerated 338,205-byte PNG
contains 4096 identical rows with all 256 alpha values. Fixture generation happens
in a separate process. The final 64 MiB pixel allocation is identical; the saving
replaces the full-frame conversion buffer with at most 64 KiB of scratch. This is
isolated decoder peak RSS, not desktop RSS, GPU memory or an eight-worker peak test.
Raw legacy peaks: 146,554,880; 146,538,496; 146,538,496; 146,554,880; 146,554,880.
Direct peaks: 79,364,096; 79,364,096; 79,396,864; 79,364,096; 79,380,480 bytes.
Legacy decode times: 24.099, 23.814, 24.326, 23.823, 23.646 ms. Direct: 23.240,
22.867, 23.131, 23.222, 23.132 ms. This small timing change is workload-specific;
the primary improvement is temporary memory, not a general image speedup.

A matching debug check (one warmup and five alternating pairs) measured median
peak RSS 150,175,744 → 82,935,808 bytes and decode time 45.211 → 44.407 ms.
Using the existing optimized egui conversion in bounded chunks avoids moving
per-pixel conversion into unoptimized application code in debug builds.

The cache workload uses release builds, one warmup batch and five measured
batches of 100 windows. Timing covers 500 patch applications per window,
excluding message/patch construction. All 500 messages survive and contain the
same edited text. Capacity/estimated-byte results are deterministic, not RSS.
Compacting large fields costs about 24 ns more per edit in this workload; this
is a measured memory/time tradeoff. The ordinary case preserves its allocation
reuse and shows no RAM saving. These microbenchmarks do not establish visible
UI latency differences.

Replay uses one warmup per revision and five alternating direct-executable pairs.
Baseline times: 151.558, 152.027, 154.554, 152.045, 151.720 ms. After: 153.925,
152.214, 154.824, 157.826, 152.611 ms. This ordinary insertion workload does not
meaningfully exercise the fixes; no general reducer speedup is claimed.
One 60-second lifecycle soak per revision exercised 66,400 / 66,688 channel
visits and 39,840,000 / 40,012,800 inserts, each with nine logout cycles. Both
kept identical steady resident-history estimate ranges: 856,392–9,786,824 bytes
under row pressure and 6,178,192–13,395,456 under byte pressure. All lifecycle
and bound assertions passed. This is bounded synthetic pressure, not proof
against every leak or live-account workload.

Reproduce the component workloads:

```sh
cargo test --locked --release -p session-cache tests::edited_message_capacity_workload -- --ignored --exact --nocapture
cargo test --locked --release -p serein avatars::tests::image_decode_memory_workload --no-run
# Run the emitted desktop test executable directly, not Cargo, under /usr/bin/time -l:
SEREIN_IMAGE_DECODE_FIXTURE=/tmp/serein-ram-4096.png SEREIN_IMAGE_DECODE_LEGACY=1 /usr/bin/time -l PATH_TO_TEST_BINARY avatars::tests::image_decode_memory_workload --ignored --exact --nocapture
SEREIN_IMAGE_DECODE_FIXTURE=/tmp/serein-ram-4096.png /usr/bin/time -l PATH_TO_TEST_BINARY avatars::tests::image_decode_memory_workload --ignored --exact --nocapture
cargo replay
# After building, run target/release/replay-bench directly for timing/peak RSS:
/usr/bin/time -l target/release/replay-bench --soak 60
```

For the historical cache comparison, add only `empty_patch` and
`edited_message_capacity_workload` from this change to `2d5345a` and preserve
its production implementation. Generate the image fixture separately using
only Python's standard library:

```python
import struct, zlib
def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
row = b'\0' + bytes(v for x in range(4096) for v in (x % 256, 255 - x % 256, 83, x % 256))
compressor = zlib.compressobj(6)
with open('/tmp/serein-ram-4096.png', 'wb') as output:
    output.write(b'\x89PNG\r\n\x1a\n')
    output.write(chunk(b'IHDR', struct.pack('>IIBBBBB', 4096, 4096, 8, 6, 0, 0, 0)))
    for _ in range(4096):
        data = compressor.compress(row)
        if data:
            output.write(chunk(b'IDAT', data))
    output.write(chunk(b'IDAT', compressor.flush()))
    output.write(chunk(b'IEND', b''))
```

### Native idle control

Both preserved release demo builds used `--no-default-features --features demo`
and were launched with `--demo --demo-friends --demo-frame-sample=1,1` on the same
Apple M1 Metal renderer. The built-in display was 2560×1600 Retina and asleep;
this is an **occluded/display-asleep idle control**, not active rendering evidence.
The default 1120×760 viewport was requested; actual window size/display scale
were not independently verified. No interaction was injected.

After a ten-second warmup, `ps -p PID -o %cpu=,rss=` sampled each owned process
20 times at one-second intervals. One launch per revision; settled RSS is the
median of the final five samples. `vmmap -summary PID` was captured afterward.
No child/helper processes were found. Only the owned synthetic processes were
terminated after sampling.

| Native control metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Median idle CPU | 0.0% | 0.0% | Unchanged |
| Peak / settled RSS, KiB | 136,432 / 136,432 | 135,760 / 135,760 | -672 / -0.49% |
| Physical footprint, vmmap display | 73.0M | 72.3M | -0.7M |
| Peak physical footprint, vmmap display | 90.9M | 90.5M | -0.4M |

These small native differences are within uncontrolled launch/allocator variation;
no general desktop RAM improvement is claimed. The idle fixture does not exercise
the large-image or shrinking-message workloads. Frame markers did not complete
while the display was asleep, so p95 frame time, startup latency, active scrolling
and GPU-wide memory remain unmeasured.

### Standard package and verification

Both revisions passed `cargo xtask package`, including voice, with default/demo
features disabled. The complete outputs were preserved in separate directories.
Each package contains the same 205 file paths; installed size sums those files.
ZIPs use `ditto -c -k --sequesterRsrc` over each complete `dist` directory without
an enclosing directory. These are locally ad-hoc signed and verified macOS
bundles, not notarized releases.

| Package metric, bytes | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 61,003,200 | 61,003,216 | +16 / less than 0.001% |
| Full installed package | 67,010,392 | 67,010,408 | +16 / less than 0.001% |
| Compressed distribution | 42,808,529 | 42,809,918 | +1,389 / +0.0032% |

These negligible artifact differences do not establish a package-size improvement.
Licenses, notices, runtime assets and dependency versions are unchanged.

Workspace/fuzz formatting, strict workspace Clippy, focused image/cache tests
(including release image tests), the standard feature-disabled build check and
policy checks passed. The final `cargo test --workspace --locked --no-fail-fast`
run had **1,148 passed, six failed and 24 ignored**. `cargo xtask check` is
consequently **not green**. All failures are in unchanged network fixtures:

- Gateway `local_socket_identify_ack_drop_resume_and_invalid_session` and
  `outgoing_activity_waits_for_ready_coalesces_clears_and_resumes`, plus voice
  `local_guild_voice_waiting_mixed_audio_and_resume`: `Protocol(WrongHttpMethod)`.
- API `invites_http_pause_preserves_features_and_revoke_reconciles_without_retry`,
  `integrations_http_permission_scopes_mutation_reconciliation_and_no_retry` and
  `upload_cancel_before_write_and_redirect_never_send_message`: a loopback listener
  accepted a connection during a 50–80 ms no-request assertion.

A prior full run had 1,150 passed, four failed and 24 ignored: the two Gateway
tests and two voice tests (`local_voice_websocket_udp_dave_and_opus_exchange`
also failed then). Gateway failures reproduced individually. An independent
loopback listener with no client launched received an unsolicited HTTP `HEAD`
after 1.336 seconds; only the method was recorded, with no headers or payloads.
The extra API connections are consistent with that observed probe interference,
but their methods were not captured. No API/Gateway/voice code or test was changed,
and no check was disabled or production handshake weakened.

The PR stays draft while full verification is blocked. Windows/Linux runtime
behavior and live Discord were not measured. Screenshots are not applicable
because this has no visible UI change.

## Per-server notification settings — September 29, 2026

Baseline `306bccdbb4d28fa83dac09260772917d3d8b0018`; after is the
server-notification-settings implementation. Same Apple M1, 16 GiB RAM,
macOS 27.0, Rust 1.98.1 (Homebrew), pinned lockfile and release profile.
Both standard packages include voice with default/demo features disabled and
passed `cargo xtask package`, including local ad-hoc signing verification.
The baseline package was built and preserved at this exact commit earlier in
this delivery session for issue #452; its verified artifact was reused. The
changed package was rebuilt here. Both have the same 205 file paths. Installed
size sums file bytes; complete `dist` directories were compressed separately
with `ditto -c -k --sequesterRsrc`, without an enclosing directory.

`cargo replay` builds the unchanged synthetic reducer workload. The baseline
binary was built from a disposable worktree at the recorded commit and preserved
before changes. Final direct runs alternate the two binaries, reversing order
on each pair: one warmup and five measured runs per revision. Host scheduling
remains uncontrolled. This is reducer elapsed time and retained timeline accounting,
not process memory, notification delivery latency or UI frame time.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable, bytes | 61,003,216 | 61,036,080 | +32,864 / +0.0539% |
| Installed package, bytes | 67,010,408 | 67,043,272 | +32,864 / +0.0490% |
| ZIP distribution, bytes | 42,809,900 | 42,822,815 | +12,915 / +0.0302% |
| Reducer median, ms / 100,000 events | 172.244000 | 163.930750 | -8.313250 / -4.83% |
| Retained timeline, estimated bytes / records | 331,992–332,477 / 500 | 331,992–332,477 / 500 | Unchanged |

Five measured samples per revision:

- baseline: 160.914292, 162.024583, 174.693666, 172.244000, 173.763500 ms.
- after: 163.401625, 160.369042, 178.079000, 173.128042, 163.930750 ms.

The ranges overlap and the baseline itself drifted from a pre-edit median of
154.611708 ms (five runs: 156.027625, 154.611708, 155.423708, 154.219167,
152.081875; warmup 196.209500) to 172.244000 ms during paired sampling.
The observed median difference does not establish a performance improvement.
Paired warmups were 161.528041 ms baseline and 161.668084 ms after. Retained
bounds are unchanged. No dependencies, runtime assets or bundled notices changed.

Native synthetic renders were inspected in dark/light mode on Metal at a
1120×760 viewport and 2× display scale. Headless input tests cover a 320×550
viewport and scrolling. Native OS input automation was unavailable
(`AXIsProcessTrusted=false`; targeted event posting had no effect), so matched
interaction CPU/RSS, startup and p95 frame timing remain unmeasured; no UI
performance claim. Live Discord and Windows/Linux behavior were not exercised.
The PR remains draft for this missing native evidence.

`cargo xtask check` passed (911 tests, 23 ignored; formatting, strict Clippy,
standard app check and policy checks), as did the focused notification/API/UI
checks, final demo build and standard release package.

## Search navigation — September 29, 2026

Baseline: `306bccdbb4d28fa83dac09260772917d3d8b0018`; after: this search-navigation
change. macOS 27.0, Apple M1, 16 GiB RAM, pinned Rust 1.98.1. Both standard
`cargo xtask package` builds include voice and omit demo/developer features.
Separate copies of the complete `dist` directories were measured: executable
file length, sum of installed regular-file lengths, and ZIP size from
`ditto -c -k --sequesterRsrc DIST OUTPUT.zip`. Both contain 205 files.
These are locally signed packages, not notarized releases.

After building each revision with `cargo replay`, `target/release/replay-bench`
ran once for warmup and five times for measurement. Each run processes 100,000
synthetic reducer events; this is not a search-latency benchmark.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable, bytes | 61,003,216 | 61,019,632 | +16,416 / +0.027% |
| Full installed package, bytes | 67,010,408 | 67,026,824 | +16,416 / +0.024% |
| ZIP distribution, bytes | 42,809,900 | 42,817,252 | +7,352 / +0.017% |
| 100,000-event reducer median, ms | 164.532 | 214.415 | +49.882 / +30.32% |
| Alternating baseline/after reducer control, median ms | 223.211 | 223.475 | +0.264 / +0.12% |
| Retained timeline, estimated bytes | 331,992–332,477 | 331,992–332,477 | Unchanged; 500 records |

Measured replay samples, milliseconds:

- Baseline: 164.208291, 165.308000, 164.532041, 163.146209, 169.510250.
- After: 214.414500, 210.785000, 215.487125, 217.813542, 205.935208.

The initial sequential samples showed +30.32% elapsed time. To investigate,
the exact baseline replay was rebuilt in a clean worktree, preserving the changed
binary. Each binary then received one warmup and five measured runs, alternating
baseline/after order each pair. Baseline was 221.487–244.230 ms;
after was 220.689–244.861 ms. Medians differed by +0.12%
with overlapping ranges. The baseline slowdown in the later control demonstrates
substantial run-to-run host variation; the initial +30.32% is not established as
a code regression. Neither comparison establishes a speed improvement.

Alternating control samples, milliseconds:

- Baseline: 244.230333, 221.487167, 223.256500, 221.950292, 223.210833.
- After: 233.071917, 244.860625, 221.811375, 223.474584, 220.689167.

Retained timeline estimates are not process RSS. Dependencies, licenses and
bundled notices are unchanged.

Native before/after captures used debug `--features demo` builds, launched with
`--demo --demo-search=synthetic`, at the same 1120×760 viewport and 2× display
scale on the Metal renderer. Native input automation was unavailable:
`AXIsProcessTrusted=false`, and targeted event posting had no effect. Matched
native interaction CPU/RSS, startup and p95 frame timing are therefore unmeasured;
no UI speed or memory claim is made. Headless egui tests exercise keyboard/click
input and narrow light/dark pager layouts, but do not validate OS input routing.
Screenshots contain only synthetic app content and are development evidence,
not bundled assets. No live-account or audio-device workload was run.

## macOS Control-click context menus (October 2, 2026)

Measured runtime `343c6d48` against exact parent `2118f7d9`. Parent runtime is identical
to preserved GIF-sync `43e1215c` across application/crate/assets/tool/vendor sources,
macOS packaging, Cargo manifests/lockfile, toolchain and licenses (`git diff --quiet`
passed). This reuses its immutable standard package and optimized default-plus-demo
binary, with recorded hashes verified before sampling.

The input plugin maps macOS Control-primary presses to secondary presses and remembers
the chosen button through release, even when Control is released first. It adds one
state flag and a scan over the existing native input event list; no cache, queue or
background worker is introduced.

| Metric / method | Parent | Changed | Delta |
| --- | ---: | ---: | ---: |
| Standard voice-enabled executable | 62,137,600 B | 62,137,600 B | 0 B (0%) |
| Full installed macOS package, 206 regular files | 68,148,029 B | 68,148,029 B | 0 B (0%) |
| Distribution ZIP, `ditto -c -k --sequesterRsrc`, no enclosing directory | 43,310,944 B | 43,311,255 B | +311 B (+0.000718%) |
| Native idle process CPU, sample median | 0% | 0% | 0 percentage points |
| Native sampled peak process RSS | 124,880 KiB | 124,576 KiB | −304 KiB (−0.2434%) |
| Native settled process RSS, median of final five samples | 124,832 KiB | 124,528 KiB | −304 KiB (−0.2435%) |

Both optimized native binaries use the normal repository fat-LTO release profile,
default features plus `demo`, and identical `--demo --demo-chat` arguments. Neither
contains capture instrumentation. macOS 27.0 (26A428), Apple M1 (eight logical CPUs,
16 GiB RAM), native Metal renderer, 2× display scale, 1120×760-point window. Five-second
warmup followed by ten one-second `ps` samples, one process at a time, with all other
agent builds/native apps paused. SIGINT/exit −2 ends each successful sample intentionally.
One build and one sample per revision: quantized zero CPU and the small RSS difference
are not evidence of a performance improvement. Event-handling latency, p95 frame time,
startup, GPU memory and live Discord behavior are unmeasured.

The fresh full `cargo xtask check`, standard `cargo xtask package`, and optimized demo
build passed. Actual native before/after WGPU images show the existing message menu
opening after injected Control-click; before source `47a81035` precedes additive GIF
changes outside this visible scene. The after image waits 500 ms for the actual popup
fade to finish. Native capture hooks were removed byte-exactly before release builds.
Synthetic egui input is not proof of macOS OS event routing; Accessibility automation
is unavailable on this host. Raw package/build hashes, ten-sample records, screenshot
metadata and limits are in
[the task measurements](pr-evidence/macos-control-click/measurements.json).

## Chat paste, mention recency and image galleries (October 2, 2026)

Runtime source `5193f09e0a0aceb0955101407371ea7a299bc47f` includes this task and a normal merge of main
`71ebbc1c0393a0ba4f4e6c93ae9b7b0bd3e06d35`. Its original parent comparator is
`47a81035047695fe1d81edc4cb7efe46c14d87a8`. These are aggregate measurements:
incoming quiet/GIF, voice, diagnostics/search, zoom, camera, Linux appearance,
media and Flatpak changes are included, so the deltas do not isolate gallery cost.

On Apple M1/macOS 27/16 GiB/Metal at 2× scale, instrument-free default+demo releases
used identical process-only thin-LTO flags; the shipping package kept normal
fat-LTO. After inspected workspace-name cache invalidation across worktree IDs,
all twelve runtime crates freshly compiled from the exact source. Both builds
passed, and the preserved standard app passed strict signature verification.
Parent47 and the preserved standard comparator `ef9cd5d1` have identical runtime,
notices and macOS packaging sources; Linux packaging differences are excluded.

| Metric / method | Parent47 | Aggregate5193 | Delta |
| --- | ---: | ---: | ---: |
| Standard executable, bytes | 62,088,304 | 62,269,488 | +181,184 (+0.292%) |
| Installed contents, bytes | 68,098,733 | 68,279,917 | +181,184 (+0.266%) |
| ZIP, same ditto method, bytes | 43,286,187 | 43,366,411 | +80,224 (+0.185%) |
| Idle CPU, median 10×1s after 5s warmup | 0% | 0% | 0 percentage points |
| Sampled peak RSS, KiB | 125,136 | 125,168 | +32 |
| Settled RSS, median last 5, KiB | 125,088 | 125,120 | +32 |

The native pair used the same ordinary `--demo --demo-chat` fixture, with all team
compilers/native apps and heavy IO held. Both processes stopped after sampling.
The small RSS difference is noise; no improvement is claimed. Gallery screenshots
use a separate opt-in bounded five-image fixture. Eight actual WGPU before/after
frames cover wide/narrow and dark/light layouts; temporary hooks were removed
byte-for-byte. Actual egui pointer tests cover embed entry/lifecycle and row heights.
Fresh full checks passed 373 UI/168 desktop tests plus all workspace strict/policy
checks. Frame timing, active decode, native OS paste routing, Linux/Windows native
rendering and live Discord interoperability remain unmeasured. Raw source/build,
sample and capture records are in `docs/pr-evidence/chat-parity/`.

## Complete large-guild subscriptions — September 29, 2026

Baseline: `400ac8cb060757b6b775284356324f22e5968158`; after: this
large-guild subscription change. Windows 11 Home 10.0.26200, AMD Ryzen 7
7800X3D, 31.1 GiB RAM, pinned Rust toolchain. Both revisions used the standard
`cargo xtask package` command. `makensis` was unavailable, so the command
produced the unsigned package executable and complete `dist` directory but no
Windows installer. Separate ZIPs were created from each complete `dist`
directory with .NET `ZipFile` optimal compression and no enclosing directory.

After building each revision with `cargo replay`, its `replay-bench.exe` ran
once for warmup and five times for measurement. Each run processes 100,000
synthetic reducer events; the changed gateway subscription packet is outside
this workload.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable, bytes | 77,560,832 | 77,560,832 | Unchanged |
| Full installed package, bytes / files | 81,664,148 / 198 | 81,664,148 / 198 | Unchanged |
| ZIP distribution, bytes | 44,510,112 | 44,510,240 | +128 / +0.0003% |
| 100,000-event reducer median, ms | 133.5614 | 130.6347 | -2.9267 / -2.19% |
| Retained timeline, estimated bytes / records | 331,992–332,477 / 500 | 331,992–332,477 / 500 | Unchanged |

Measured replay samples, milliseconds:

- Baseline: 131.9832, 130.0590, 133.5614, 142.3409, 139.4035.
- After: 126.1190, 130.6347, 133.9218, 126.7004, 135.1862.

The samples overlap, so the median difference does not establish a speed
improvement. The 128-byte ZIP difference with identical installed files is
archive metadata variation, not package growth. Retained timeline estimates
are not process RSS. No dependency, bundled asset, or license changed. This is
a nonvisual gateway packet fix, so screenshots and renderer measurements are
not applicable. Live large-guild acceptance remains unverified.

## Niri portal timestamp compatibility (September 29, 2026)

Baseline `306bccdbb4d28fa83dac09260772917d3d8b0018` and the Niri timestamp fix
were built on the same CachyOS x86_64 host (Linux 7.2.8-1-cachyos, Ryzen 7 7800X3D,
32 GiB RAM), using pinned Rust 1.98.1 and the locked default release configuration
with voice and no default features. One package per revision; no repeated-size or
CPU benchmark. Separate checkout/dist directories retained both builds. The native
Arch package smoke checks passed for both revisions.

| Metric | Baseline | Niri fix | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 78,201,432 | 78,204,376 | +2,944 (+0.0038%) |
| Installed payload bytes (.PKGINFO) | 82,646,916 | 82,649,860 | +2,944 (+0.0036%) |
| Compressed Arch package bytes | 43,629,154 | 43,630,731 | +1,577 (+0.0036%) |

The baseline executable was built by `cargo xtask package`; Debian dependency
metadata is unavailable on this Arch host, so its packaging was completed with the
repository's Arch packager. The final build used `cargo xtask package --format arch`.
Sizes are filesystem byte counts and the package's installed-size metadata; tiny
compressed deltas include packaging metadata noise and are not performance claims.

The existing offline capture example reproduced constant-PTS starvation before
the fix. Normal and Niri timestamp regression modes now require three advancing
preview samples and five advancing video samples after secure readiness. These
are synthetic checks, not a native Niri or Discord interoperability benchmark.
Capture CPU, RSS, frame latency and real hardware encoder performance remain
unmeasured: no live source was captured, and the native demo does not exercise
portal capture. The fix adds timestamp metadata handling only on Niri portal
frames, with no new frame queue or dependency.


## REST API proxy plugin host (September 30, 2026)

Baseline `92a4bb66` versus the API proxy host change, pinned Rust 1.98.1 on
Windows. Both used the standard voice-enabled `cargo xtask package`; NSIS was
unavailable, so no installer was generated. Complete portable folders were
compressed separately with .NET ZipFile Optimal, without an enclosing directory.

| Metric | Baseline bytes | After bytes | Delta bytes |
| --- | ---: | ---: | ---: |
| Executable | 77,564,928 | 77,601,280 | +36,352 |
| Complete portable folder | 81,668,244 | 81,704,596 | +36,352 |
| ZIP distribution | 44,510,947 | 44,529,776 | +18,829 |

The external API Proxy package is 920,490 bytes, including its 328,895-byte Wasm
module and dependency notices; it is downloaded from the community catalog and
is not bundled with Serein.

The existing release sandbox benchmark used one warmup, five parse-plus-invoke
samples and 100 invoke samples. Message Delete Protector measured 2,762/1,220 us
before and 2,697/1,145 us after (parse-plus-invoke / invoke). The actual external
API Proxy package measured 6,818/2,974 us. Each invocation creates a fresh bounded
Wasm runtime; plugin execution happens on the extension worker, outside rendering.
Runs overlapped release compilation, so these noisy timings establish neither an
improvement nor native frame timing or proxy network latency. The real package's
activation restore, passive Open and explicit Apply were also checked offline.

Routing uses a bounded coalescing configuration watch and a cached HTTP client
pool rebuilt only when its selected route changes, outside rendering. Existing
requests retain their selected route. Native screenshots, process RSS, UI frame
time, real proxy latency and live/cross-platform compatibility remain unverified.


### Proxy authentication follow-up

Final source `7d048921c50792ef91aacf94dcfcb96549f5ba9e` adds host-owned HTTP Basic
proxy credentials and clears credential input undo history. The final standard
voice-enabled `cargo xtask package` passed on the same Windows host; no NSIS
installer was available. Relative to the credential-free host at `f1557f00`:

| Metric | Before bytes | With authentication bytes | Delta bytes |
| --- | ---: | ---: | ---: |
| Executable | 77,601,280 | 77,702,144 | +100,864 |
| Complete portable folder | 81,704,596 | 81,805,430 | +100,834 |
| ZIP, .NET Optimal | 44,529,776 | 44,551,343 | +21,567 |

The updated external package is 920,979 bytes; its inspected synthetic egui
catalog preview is 69,881 bytes at 640 x 360, downloaded separately. This is an
actual offline framebuffer render, not an OS window capture or proxy performance
measurement. The host's OS credential IO runs on at most one blocking job;
active credentials share zeroizing ownership without per-frame secret copies.
Real credential-store latency, proxy latency, RSS and native frame timing remain
unmeasured. Previous Wasm timings above concern the earlier package only.


### Proxy authentication reviewer follow-up

Source `8468df98477f286ac5a9f4a0fe65199ad45ab95d` pauses new REST client
acquisition immediately when credential Save/Remove is accepted, preserves a
rejected Save draft, and warns about unencrypted HTTP proxy authentication.
The same Windows voice-enabled release packaging and .NET Optimal ZIP procedure
passed; NSIS remains unavailable. Compared with authentication source `7d048921`:

| Metric | Before bytes | After review fixes bytes | Delta bytes |
| --- | ---: | ---: | ---: |
| Executable | 77,702,144 | 77,703,680 | +1,536 |
| Complete portable folder | 81,805,430 | 81,806,966 | +1,536 |
| ZIP, .NET Optimal | 44,551,343 | 44,551,885 | +542 |

Credential drafts are copied only on explicit Save and remain zeroizing and
bounded; accepted jobs clear the draft. Existing in-flight requests retain their
previous route. No extra background job, queue or dependency was added. These
package sizes do not establish OS credential-store latency, proxy latency, RSS
or native frame timing; those remain unmeasured.


## Discord poll layout (September 30, 2026)

The starting poll implementation `6b901def` and this layout follow-up used Rust
1.98.1 on Ubuntu 26.04.1, AMD Ryzen 5 7535U (6 cores / 12 threads), 14,657 MiB RAM.
Both standard voice-enabled `cargo xtask package` builds and Debian smoke checks
passed. Installed bytes sum regular files extracted from each `.deb`.

| Metric | Baseline | After | Absolute / percent delta |
| --- | ---: | ---: | ---: |
| Standard executable, bytes | 79,609,520 | 79,634,608 | +25,088 / +0.0315% |
| Full installed package, bytes | 84,055,258 | 84,080,346 | +25,088 / +0.0298% |
| Compressed Debian package, bytes | 40,157,332 | 40,161,928 | +4,596 / +0.0114% |
| Synthetic reducer replay median, ms | 156.306 | 154.591 | -1.715 / -1.10% |

Replay binaries from both source revisions ran one warmup and five measured
100,000-event runs without concurrent compilation. Both retained 500 messages /
331,992–332,477 estimated timeline bytes. The small timing delta is noisy and is
not an improvement claim, process RSS or UI latency.

The baseline native release demo used Vulkan llvmpipe (LLVM 21.1.8), Xvfb
1120 x 760, scale 1, dark theme, and the unvoted synthetic poll after opening and
closing the creator. Eight seconds of warmup preceded thirty one-second `/proc`
CPU/VmRSS samples: 0.0666% of one logical CPU and 232,252 KiB peak/settled RSS.
No child/helper processes or active voice session were observed. The after demo
release built successfully, but after-process sampling was not collected before
the owner's expedited push request. Native CPU/RSS deltas, hardware performance,
frame/startup latency and live account/audio behavior remain unmeasured.


## Release validation CI repair (October 1, 2026)

Baseline `44f4719adfcc4fb9781f2057be211d61efc97f61` and repaired build
`5b4d7ec325f36628c8852876d2093c508ee1d4a5` used pinned Rust 1.98.1 on
Ubuntu 26.04.1 x86_64, AMD Ryzen 5 7535U, approximately 14 GiB RAM. Each ran
one standard voice-enabled `cargo xtask package`, `CARGO_BUILD_JOBS=2`, with
`--release --locked -p serein --no-default-features`. Separate worktrees and
independent copies of the dependency cache were used; all workspace packages
were cleaned only in those copies to force a fresh build of each source revision.
Both Debian package smoke checks passed, including installed contents, ownership,
desktop metadata and the host shared-library closure.

| Metric | Baseline bytes | After bytes | Absolute / percent delta | Method |
| --- | ---: | ---: | ---: | --- |
| Standard executable | 79,645,232 | 79,645,232 | 0 / 0% | Packaged executable length |
| Full installed Debian payload | 84,090,970 | 84,093,528 | +2,558 / +0.003042% | Sum of extracted regular files |
| Compressed Debian distribution | 40,166,644 | 40,165,972 | -672 / -0.001673% | `.deb` file length |

Installed files increased from 210 to 211. The complete payload increase is the
2,195-byte upstream `yoke-derive` license and 363 added notice bytes. Executable
hashes differ despite identical size; these are one build per revision and the
tiny compressed delta is not an improvement claim. The compatible derive macro
patch changes build-time string construction; Gateway, voice teardown and fuzz
repairs affect only synthetic development tests. Runtime CPU, RSS, frame/startup
latency and live account/audio behavior were not measured. Flatpak source-preparation repairs
and this measurement documentation do not change the native installed payload.


## Quiet-message prefix — October 2, 2026

Runtime source `28eac6bb` on parent main `47a81035`; package source `462d7208`
adds only the verified absolute macOS icon output path. The preserved parent
package/native source `ef9cd5d1` is runtime-identical to `47a81035`. macOS 27,
Apple M1, 16 GiB, native Metal at 2× scale; pinned toolchain/lockfile. Standard
packages use no default development features and include voice; native binaries
use default features plus demo and the unchanged optimized fat-LTO profile.

| Metric | Parent | Quiet prefix | Delta |
| --- | --- | --- | --- |
| Signed standard executable bytes | 62,088,304 | 62,088,304 | 0 |
| Installed package bytes, 206 regular files | 68,098,733 | 68,098,733 | 0 |
| Distribution ZIP bytes, identical ditto method | 43,286,187 | 43,287,077 | +890 (+0.0021%) |
| Common synthetic chat median process CPU | 0.0% | 0.0% | 0 percentage points |
| Sampled peak process RSS, KiB | 128,768 | 128,592 | -176 (-0.1367%) |
| Settled process RSS, KiB | 128,720 | 128,544 | -176 (-0.1367%) |
| 100,000-event reducer median, ms | 53.308083 | 52.984917 | -0.323166 (-0.6062%) |
| Retained timeline estimate, bytes / records | 331,992–332,477 / 500 | 331,992–332,477 / 500 | Unchanged |

Both native binaries launch `--demo`: five seconds warmup, then ten one-second
macOS `ps` CPU/RSS samples; settled RSS is the median of the last five. All
compilers, tests and other native demos were stopped. This measures idle overhead
in the common synthetic chat, not send latency or real notification suppression.
Fresh parent/after replay binaries receive one warmup and five alternating runs;
ranges overlap (parent 52.960291–54.733875 ms, after 52.482000–53.390875 ms).
Small differences and quantized CPU are not improvement claims. Package ZIP
changes include regenerated native icon/signature resources. No dependency,
worker or retained-message copy is added; prefix handling borrows existing text.
GPU memory and frame/startup latency remain unmeasured. Raw samples, hashes,
source identities and methods are in
`docs/pr-evidence/quiet-messages/measurements.json`; preserved artifacts are in
`target/issue-sweep/silent/462d7208`. Local full workspace checks, standard package,
optimized demo build and native/replay sampling passed. No live message,
account, provider request, microphone or camera was used.

## Public Catbox attachment hosting (October 2, 2026)

Baseline `eab1961ae80d2822a49629fb5d182b2c8e18e9d1` and runtime source
`ef9cd5d1d846335713af71f9f1d9cd895483d59d` used macOS 27.0 (26A428),
Apple M1 (8 logical CPUs), 16 GiB RAM, Rust 1.98.1 and native Metal. Both
standard voice-enabled `cargo xtask package` builds passed without command-line
feature overrides. The xtask internally builds with `--no-default-features`;
voice is included. Full installed bytes sum every regular file in the complete
portable folder. ZIPs use `ditto -c -k --sequesterRsrc` over that folder without
an enclosing directory; both packages contain 206 files.

| Metric | Baseline | After | Absolute / percent delta |
| --- | ---: | ---: | ---: |
| Standard executable, bytes | 62,006,112 | 62,088,304 | +82,192 / +0.1326% |
| Full installed payload, bytes | 68,016,541 | 68,098,733 | +82,192 / +0.1208% |
| Complete ZIP, bytes | 43,252,563 | 43,286,187 | +33,624 / +0.0777% |
| Release demo median process CPU | 0.0% | 0.0% | +0.0 percentage points |
| Sampled peak process RSS, KiB | 123,584 | 123,872 | +288 / +0.2330% |
| Settled process RSS, KiB | 123,536 | 123,856 | +320 / +0.2590% |

Native process measurements use separate optimized builds of
`cargo build --release --locked -p serein --features demo`, with default features
enabled, launched `--demo --demo-chat`. Five seconds of warmup precede ten
one-second macOS `ps` process CPU/RSS samples; settled RSS is the median of the
last five samples. No other native demo was running, and no Cargo compilation
was active during the after sample. Ambient compiler activity during baseline
and after work can differ. This is one build/sample per revision; small size/RSS
deltas and quantized 0.0% CPU readings are not improvement claims.

The public-host transport streams at most 200,000,000 file bytes in 64 KiB chunks,
bounds replies to 4 KiB, and retains one upload task/latest progress value.
Those are admission/storage limits, not measured live transfer performance. No
real file was uploaded to Catbox and no live Discord session or audio/device
action was used. Active-upload throughput, GPU memory and frame/startup latency
remain unmeasured. Native screenshots use actual synthetic app-owned Metal
framebuffers; OS input routing and other-platform interaction remain unverified.
Raw build sizes, hashes and process samples are retained in
`docs/pr-evidence/external-upload/measurements.json`. The evidence-only follow-up
changes no runtime source from the measured commit.

## Active server in the emoji picker (October 2, 2026)

Parent `47a81035047695fe1d81edc4cb7efe46c14d87a8` and runtime source
`942bf21c92240d4fa0a8dbf2392ae9b159ed2fba` used pinned Rust 1.98.1 on
macOS 27.0 (26A428), Apple M1 (8 logical CPUs), 16 GiB RAM and native Metal
at 2× display scale. The preserved standard package at `ef9cd5d1` has identical
runtime and host-build inputs to parent `47a81035`, verified across apps,
crates, assets, tools, vendor, macOS packaging, Cargo files, notices, license
and toolchain. Intervening Arch/repository recipes do not affect this host package.
Both normal voice-enabled `cargo xtask package` builds passed without profile
or feature overrides; xtask internally uses `--no-default-features`. Installed
bytes sum all regular files; ZIPs use `ditto -c -k --sequesterRsrc` without an
enclosing directory. Both packages contain 206 files.

| Metric | Baseline | After | Absolute / percent delta |
| --- | ---: | ---: | ---: |
| Standard executable, bytes | 62,088,304 | 62,088,304 | 0 / 0% |
| Full installed payload, bytes | 68,098,733 | 68,098,733 | 0 / 0% |
| Complete ZIP, bytes | 43,286,187 | 43,286,600 | +413 / +0.000954% |
| Release demo median process CPU | 0.0% | 0.0% | +0.0 percentage points |
| Sampled peak process RSS, KiB | 131,360 | 131,504 | +144 / +0.1096% |
| Settled process RSS, KiB | 131,312 | 131,472 | +160 / +0.1218% |

The native process comparison uses matched
`cargo build --release --locked -p serein --features demo` builds with the
identical process-only `CARGO_PROFILE_RELEASE_LTO=thin` override and
`CARGO_BUILD_JOBS=2`, in an independent cache. This changes no repository
profile and is separate from the standard fat-LTO package comparison above.
Both uninstrumented binaries run `--demo --demo-emoji`, with five seconds of
warmup followed by ten one-second macOS `ps` CPU/RSS samples. Settled RSS is
the median of the last five samples. All four build owners held compilation,
and no other Serein demo ran during either sample. Both processes were stopped
by intentional SIGINT after collection. One build/sample per revision, small
RSS/ZIP differences and quantized 0.0% idle CPU do not establish an improvement.

The performance workload opens the ordinary offline picker. Actual native
before/after frames separately use the same temporary fixture inserting one
server before the active server; instrumentation was removed before committing.
The rail remaps visible indices without copying catalogs, and the regression
clicks the displayed first guild and restores an actually scrolled 42-server
rail to its top after the active guild changes. Active scrolling, GPU memory,
frame/startup latency, OS input routing and live Discord behavior remain
unmeasured. Raw hashes, package sizes and process samples are retained in
`docs/pr-evidence/active-server-emoji/measurements.json`. The evidence-only
follow-up changes no runtime source from the measured commit.

## AUR binary recipe payload (October 2, 2026)

The local packaging pass used the published Arch x86_64 package from
`v1.0.0-nightly.20261001.53`, verified against that release's `SHA256SUMS.txt`
(`5efa3f71216cced0da707753b96007add5efc9bad256c390ecc020c5a238dfc5`).
The recipe's `package()` function copied its extracted `usr` payload on this macOS
host without installing or launching Serein. SHA-256 comparison verified all 211
original files remained byte-identical. This is a packaging comparison against a
verified release asset, not a new application build or runtime measurement.

| Metric | Published Arch payload | AUR recipe payload | Delta / method |
| --- | --- | --- | --- |
| Installed regular-file bytes | 84,480,993 | 84,492,467 | +11,474 bytes (+0.0136%); sum of file sizes |
| Regular files | 211 | 213 | +2 license copies under `usr/share/licenses/serein-bin` |
| Executable | Existing released binary | Byte-identical | 0 bytes; SHA-256 comparison |
| Compressed distribution | 46,847,463 bytes | Unmeasured locally | Native `makepkg` archive creation is delegated to Arch CI |

No compiler options, application dependencies or runtime code changed. CPU, RSS,
frame latency and native Arch startup were not measured. The manual AUR build
repackages an existing binary; it does not compile Rust.

## Voice session ownership — initial historical comparison, October 2, 2026

This comparison covers the initial implementation before transport-confirmed
negotiation and scoped queue-pressure review corrections. The corrected aggregate
comparison below supersedes it; these original measurements remain historical.

Baseline `eab1961` and the call-takeover change were measured on the same macOS
27.0 (26A428), Apple M1 MacBookAir10,1 / 16 GiB machine, Rust 1.98.1 and locked
dependencies. Voice remains in the standard package; no dependencies were added.
Raw samples: [`voice-call-takeover/measurements.json`](pr-evidence/voice-call-takeover/measurements.json).

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard desktop executable | 62,006,112 B | 62,022,528 B | +16,416 B / +0.03% |
| Complete installed package, 206 regular files | 68,016,541 B | 68,032,957 B | +16,416 B / +0.02% |
| Distribution ZIP, `ditto -c -k --sequesterRsrc` | 43,252,563 B | 43,258,951 B | +6,388 B / +0.01% |
| Native idle CPU, median 10 samples | 0.0% | 0.0% | 0 percentage points |
| Native peak RSS | 123,584 KiB | 124,064 KiB | +480 KiB / +0.39% |
| Native settled RSS, last-five median | 123,536 KiB | 124,016 KiB | +480 KiB / +0.39% |
| Reducer 100,000 events, alternating-five-pair median | 53.328 ms | 52.654 ms | -0.674 ms / -1.26% |
| Estimated retained timeline, 500 records | 331,992–332,477 B | 331,992–332,477 B | unchanged |

Native samples used optimized `--features demo` builds, `--demo --demo-chat`,
Metal, a 1120×760 logical native viewport at 2× display scale (2240×1520
physical screenshot pixels), a 5-second warmup and ten 1-second
macOS `ps` samples. Settled RSS is the last-five median; CPU is the process
percentage, not GPU usage or frame latency. Compilation and other native demos
were stopped. No microphone, camera or live account was used.

Reducer measurements used the immutable baseline and changed release binaries,
one warmup each and five alternating pairs. Baseline range 52.906–53.823 ms;
after 52.444–54.572 ms. The earlier isolated baseline was 65.504 ms and the first
after run 52.316 ms; the paired rerun demonstrates timing variation rather than
a 20% improvement. The paired ranges overlap, and this ownership fix does not
optimize message reduction; no speedup is claimed. Neither workload measures
actual call takeover latency or live service behavior.

The Gateway retains one owner-session identity, at most 2 KiB in a redacted,
zeroizing secret, and one latest `(channel, request)` watch value. Takeover drops
local media and the matching initial ring worker without sending an account-wide
hangup. No new persistent cache, queue or background worker was added.


## Voice session ownership — corrected aggregate, October 2, 2026

Runtime source `3f96877f553bbe50eab91d31611cd97ad94b64b8` normally integrates
main `2118f7d9`. Its comparator is preserved GIF source
`43e1215cce0e69ee10a9998b9aaba8cfcf179c96`, whose application runtime source is
byte-identical to that main revision. Later main `8e177c3b` and `456fdc1f` add
other features and are not relabeled as this baseline. macOS 27.0 (26A428),
Apple M1 MacBookAir10,1 / 16 GiB, Rust 1.98.1, locked dependencies, voice included.
Raw samples and binary hashes:
[`voice-call-takeover/final-measurements.json`](pr-evidence/voice-call-takeover/final-measurements.json).

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard desktop executable | 62,137,600 B | 62,154,064 B | +16,464 B / +0.0265% |
| Installed package, 206 regular files | 68,148,029 B | 68,164,493 B | +16,464 B / +0.0242% |
| Distribution ZIP, ditto | 43,310,944 B | 43,321,311 B | +10,367 B / +0.0239% |
| Optimized native CPU, ten-sample median | 0.0% | 0.0% | 0 percentage points |
| Optimized native peak RSS | 124,720 KiB | 124,640 KiB | −80 KiB / −0.064% |
| Optimized settled RSS, last-five median | 124,672 KiB | 124,592 KiB | −80 KiB / −0.064% |
| Reducer 100,000 events, alternating-five-pair median | 53.559125 ms | 53.624750 ms | +0.065625 ms / +0.123% |
| Estimated retained timeline, 500 records | 331,992–332,477 B | 331,992–332,477 B | unchanged |

Actual standard `cargo xtask package` passed; its standard build uses
`cargo build --release --locked -p serein --no-default-features`, including voice. Deep/strict ad-hoc signature
verification passed. ZIP used `ditto -c -k --sequesterRsrc`, without an enclosing directory.
All 206 regular paths match, 203 SHA-256 hashes are identical, and all 199 bundled
license/notice files are unchanged. Only the executable, regenerated Assets.car
and ad-hoc CodeResources differ. Native icon compilation now receives a canonical
Resources path so actool cannot reuse another worktree's relative destination.

Both optimized native executables use the default release profile (FAT LTO,
codegen-units=1), `--features demo` and identical `--demo --demo-chat`, 1120×760
logical viewport at 2× scale, Metal. Samples use 5-second warmup plus ten one-second
macOS ps readings. Both apps stopped before reducer measurement; all compilers,
tests and other native apps remained stopped during the team-confirmed quiet
window. One reducer warmup per immutable binary preceded five alternating pairs.
Baseline range 52.855959–54.114708 ms and after 52.966000–54.040583 ms overlap.
The 80 KiB RSS difference is tiny noise; no improvement is claimed. These workloads
do not measure UI frame latency, GPU, session replacement or live media performance.
No live account, service call, microphone, camera or OS picker was used.

Negotiation retains one replaceable validated credential candidate and a scoped
u64 revision under the original 30-second deadline. Media/ringing readiness waits
for an exact transport confirmation and Gateway acknowledgement. An unconfirmed
attempt abandons locally; confirmed replacement clears the old local scope without
an account-wide hangup. Existing eight-slot control admission now retains one
fixed-metadata pending Abandon under pressure; a new Join cannot overtake cleanup,
and an unsent full-queue Join fails only its own attempt. No persistent cache or
new background worker is introduced. Server acceptance establishes a submitted
candidate, not a physical-client identity.


## Account GIF favorite synchronization (October 2, 2026)

Runtime source `43e1215cce0e69ee10a9998b9aaba8cfcf179c96` was compared with
parent `47a81035047695fe1d81edc4cb7efe46c14d87a8` on macOS 27.0 (26A428),
Apple M1 (8 logical CPUs), 16 GiB RAM, Rust 1.98.1 and native Metal at 2× scale.
The parent's app, crate, asset, tool, lockfile and toolchain sources are identical
to measured Catbox source `ef9cd5d1d846335713af71f9f1d9cd895483d59d`; its
preserved package and optimized demo binary provide the size/native baseline.
Reducer replay was freshly compiled from the actual parent, after cleaning only
its affected workspace packages. The changed build passed `cargo xtask check`,
standard voice-inclusive `cargo xtask package`, default-plus-demo optimized build,
and `cargo replay`.

| Metric | Parent baseline | After | Absolute / percent delta |
| --- | ---: | ---: | ---: |
| Standard executable, bytes | 62,088,304 | 62,137,600 | +49,296 / +0.0794% |
| Full installed payload, bytes | 68,098,733 | 68,148,029 | +49,296 / +0.0724% |
| Complete ZIP, bytes | 43,286,187 | 43,310,944 | +24,757 / +0.0572% |
| Favorites picker median process CPU | 0.0% | 0.0% | +0.0 percentage points |
| Sampled peak process RSS, KiB | 129,968 | 130,544 | +576 / +0.4432% |
| Settled process RSS, KiB | 129,920 | 130,496 | +576 / +0.4433% |
| 100,000-event reducer median, ms | 53.028459 | 52.678375 | -0.350084 / -0.6602% |
| Retained timeline estimate, bytes | 331,992–332,477 | 331,992–332,477 | 0; 500 records |

Standard packages use the xtask's normal `--no-default-features` voice build;
both contain 206 regular files and are locally ad-hoc signed, not notarized.
Installed bytes sum all regular files; ZIP uses identical
`ditto -c -k --sequesterRsrc` over the complete portable contents without an
enclosing directory. Native binaries use
`cargo build --release --locked -p serein --features demo`, default features and
the unchanged fat-LTO release profile. Both launch
`--demo --demo-gifs=favorites` with five synthetic favorites. Five seconds of
warmup precede ten one-second macOS `ps` CPU/RSS samples; settled RSS is the
median of the last five. No compiler, other native demo or helper child process
was active during sampling.

Replay uses one warmup and five measured runs per revision, alternating revision
order. Parent elapsed samples span 52.816708–56.014250 ms; changed samples span
52.631750–54.123583 ms. Their ranges overlap; the small timing/RSS differences
and quantized idle CPU are not improvement claims. Replay measures a bounded
synthetic reducer, not GIF network synchronization, process RSS or UI latency.
GPU memory, frame/startup latency and real-account interoperability remain
unmeasured. Raw source identities, binary hashes, sizes and all samples are in
`docs/pr-evidence/gif-favorite-sync/measurements.json`. This evidence follow-up
changes no measured runtime source.


## DM recipient ringing controls (October 2, 2026)

Runtime `8a4272d606465eba1cc4513e39cd7b905a8d733f` was compared with the
immutable confirmation-pressure dependency `9cdad91c`. Its standard package uses
production source `5724cf5b`; the later `9cd` changes are cfg(test)-only and leave
production code unchanged. Both feature sources use the recorded main110 base;
these measurements are not labeled as a later aggregate main revision.
Environment: macOS 27.0 (26A428), Apple M1 (8 logical CPUs), 16 GiB RAM,
Rust 1.98.1, aarch64-apple-darwin, native Metal at 2× scale.

Current source passed focused metadata/worker regressions and fresh
`cargo xtask check` (178 desktop and 354 UI tests, plus unchanged ignored
workloads), strict lint/format/policy, the standard voice-inclusive package,
default-plus-demo optimized build and reducer build. Standard release workspace
artifacts were removed by package name across all worktree PackageIDs; all 12
runtime workspace crates freshly compiled. The optimized demo immediately
followed that unchanged same worktree. All five reducer workspace crates then
freshly compiled; no capture hooks were present in these release builds.

| Metric | Confirmation-pressure baseline | Ring controls | Absolute / percent delta |
| --- | ---: | ---: | ---: |
| Standard executable, bytes | 62,154,064 | 62,219,840 | +65,776 / +0.1058% |
| Installed payload, bytes | 68,164,493 | 68,230,269 | +65,776 / +0.0965% |
| Complete ZIP, bytes | 43,321,986 | 43,346,860 | +24,874 / +0.0574% |
| Median process CPU | 0.1% | 0.1% | 0.0 percentage points |
| Sampled peak process RSS, KiB | 127,248 | 127,344 | +96 / +0.0754% |
| Settled process RSS, KiB | 127,200 | 127,296 | +96 / +0.0755% |
| 100,000-event reducer median, ms | 53.315542 | 53.678583 | +0.363041 / +0.6809% |
| Retained timeline estimate, bytes | 331,992–332,477 | 331,992–332,477 | 0; 500 records |

Both standard packages use the xtask's release `--no-default-features` path,
including voice. Installed size sums all 206 regular files; complete portable
ZIPs use identical `ditto -c -k --sequesterRsrc` without an enclosing directory.
All paths match, 203 file hashes match, and all 199 license/notice files remain
unchanged. Deep/strict local ad-hoc signatures verify; packages are not notarized.

Both native binaries use the unchanged release profile (FAT LTO,
codegen-units=1), default features plus demo, and identical `--demo --demo-call`.
The viewport is 1120×760 logical at 2× scale. A 5-second warmup precedes ten
one-second macOS ps readings; settled RSS is the median of the last five.
Root, UI and External agents explicitly held all builds/tests/native apps during
sampling. Both measured apps stopped before one reducer warmup per binary and
five alternating pairs. Baseline reducer range 53.011416–53.704667 ms and after
53.145250–54.971958 ms overlap. The 96 KiB RSS difference and small timing
differences are noise; no improvement is claimed. GPU memory, frame/startup
latency and live ringing permissions, delivery or latency remain unverified.
No Discord account, HTTP call-control write or media device was used.

Recipient dispatch retains one targeted HTTP worker, 64 observed calls and one
active record. Each record owns two bounded 64-ID vectors: 66,560 allocated
ID-buffer bytes total. This figure excludes the fixed 64-entry
Option<RecipientCall> Vec, active record headers and Arc/Mutex metadata. Core
ringing metadata is bounded to 64 calls × 64 IDs (32 KiB); the bounded membership
map uses MAX_NAV channels × 64 allocated IDs (512 bytes/channel). Current scope,
service metadata, membership and worker revision are revalidated; no persistent
cache, retry loop or schema is added. Raw source IDs, hashes, samples and build
provenance are recorded in
`docs/pr-evidence/dm-ring-controls/measurements.json`.

## Voice confirmation queue admission — historical pre-watch comparison (October 2, 2026)

Historical pre-watch feature source `9cdad91c86139543a370f3658e5f92257a9064fe` is compared with
main `1107d9045fb9d98980d6d8e9987c96a362b4f9ab`. The standard feature package
was built from `5724cf5be34f17a62ee1b5fc07f2cd2653be3d79`; `9cd` adds only
`cfg(test)` live-negotiation coverage and leaves production source unchanged.
Later main features are outside this recorded comparison. Environment: macOS
27.0 (26A428), Apple M1 MacBookAir10,1 / 16 GiB, Rust 1.98.1, locked dependencies.
Raw samples, source identities, binary hashes and the reducer identity proof:
[`voice-confirmation-pressure/measurements.json`](pr-evidence/voice-confirmation-pressure/measurements.json).

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Standard executable | 62,154,064 B | 62,154,064 B | unchanged |
| Installed package, 206 regular files | 68,164,493 B | 68,164,493 B | unchanged |
| Full distribution ZIP | 43,322,199 B | 43,321,986 B | −213 B / −0.0005% |
| Optimized native CPU, ten-sample median | 0.0% | 0.0% | 0 percentage points |
| Optimized native peak RSS | 123,888 KiB | 124,768 KiB | +880 KiB / +0.710% |
| Optimized settled RSS, last-five median | 123,856 KiB | 124,720 KiB | +864 KiB / +0.698% |
| Reducer 100,000 events, alternating-five-pair median | 53.379833 ms | 52.975458 ms | −0.404375 ms / −0.758% |
| Estimated retained timeline, 500 records | 331,992–332,477 B | 331,992–332,477 B | unchanged |

Both actual standard `cargo xtask package` builds passed, including voice and
bundled notices. The xtask internally uses `--no-default-features`; the repository
release profile is unchanged. Deep/strict ad-hoc signature verification passed.
Installed bytes sum regular files; ZIP uses `ditto -c -k --sequesterRsrc` over the
complete contents without an enclosing directory. All 206 paths match, 203 hashes
and all 199 bundled license/notice files are identical. Only the executable, regenerated
Assets.car and ad-hoc signature metadata differ.

Both optimized native executables use default-plus-demo features, FAT LTO,
codegen-units=1 and identical `--demo --demo-chat`, with no capture hooks. Native
Metal uses a 1120×760 logical viewport at 2× scale. A 5-second warmup precedes ten
one-second macOS `ps` samples; settled RSS is the final-five median. All team
compilers, tests and other native apps were paused during the matched pair, and
both apps stopped before reducer replay. The 864 KiB settled RSS difference is
small idle variation; no performance improvement is claimed.

Reducer replay uses one warmup per binary and five alternating pairs. The
preserved baseline was built from `3f96877f`; all 96 tracked files in its complete
model, client-core, session-cache, test-support and replay-bench trees plus
workspace manifests, lockfile, toolchain and Cargo configuration are byte-identical
to main `1107d904`. Features and release profile also match. This equivalence
applies only to the pure reducer, not the application or UI. Baseline samples span
52.768833–53.726709 ms; after samples span 52.405167–54.155959 ms. The ranges overlap.
These checks measure neither queue latency nor UI frames, GPU or live media.

Those samples describe the older event-queue implementation only. Current
correction `5ecd04f7fb495a1d574197eb85ad6006b4fdb5a3` changes production delivery
to one optional fixed-size failure watch, independent of reliable account-event
capacity. The report retains generation, channel, request, revision and a static
diagnostic: at most 64 bytes plus fixed watch synchronization metadata. It has
no allocated payload or credentials, retry worker or additional command slot.
Reports remain unseen until the reliable FIFO drains; a final report survives
publisher shutdown and is consumed once. The original 30-second negotiation
deadline remains.

The corrected source passed nine focused pressure/FIFO/closed-publisher/retirement
regressions, strict desktop all-target lint and the full workspace check
(174 desktop / 367 UI tests; six / five existing ignored). Its fresh standard
package compiled all 12 runtime workspace crates after all-worktree-ID release
invalidation, passed in 11m46s, and passed deep/strict ad-hoc signature verification.
It contains intervening main features, so these are aggregate package sizes,
not an isolated watch correction delta:

| Current aggregate metric | Corrected source 5ecd |
| --- | ---: |
| Standard executable | 62,269,504 B |
| Installed package | 68,279,933 B / 206 files |
| Full distribution ZIP | 43,364,502 B |

Old native/reducer values above are not measurements of this corrected source.
No new layout changed, so no new screenshots were required; the existing local
candidate-error component is used. Queue latency and physical/live media remain
unmeasured. Current source, hashes, verification and bounds are recorded separately
in the same measurements JSON. No live account, service call, microphone, camera
or OS picker was used.


## Server settings polish and rail motion (October 2, 2026)

Baseline `eab1961` and this branch were built separately with
`cargo build --release --locked -p serein --features demo` on an Apple M1 (16 GB,
macOS 27.0, Metal). Each was launched once with `--demo --demo-friends`, no
interaction was injected, and after a ten-second warmup `ps -p PID -o %cpu=,rss=`
sampled the process 20 times at one-second intervals. Settled RSS is the median
of the final five samples. No child processes were found.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release demo executable, bytes | 63,905,488 | 64,004,752 | +99,264 / +0.16% |
| Median idle CPU | 0.0% | 0.0% | Unchanged |
| Peak / settled RSS, KiB | 122,448 / 122,448 | 123,120 / 123,120 | +672 / +0.55% |

The RSS difference is one launch each and within normal allocator/launch
variation. New motion (rail pill, icon morph, badges, switches, sidebar rows,
page fade) uses egui's `animate_*` helpers, which request repaints only while a
value is moving, so the idle result is the expected one. Active-animation frame
time, p95 latency and the standard package size were not measured.

## Attachment URL admission (October 2, 2026)

Compared baseline `69b7ad9b8d45ab544cb759f862f74e0ec500f27c` with source
`b93626b9b0269b4bf09d4f365180f1f146c6b07f` on Ubuntu 26.04.1 x86_64,
AMD Ryzen 5 7535U / 14 GiB RAM, pinned Rust 1.98.1. Both standard locked
release packages include voice and disable default/demo features.

| Metric | Baseline | After | Delta | Method |
| --- | ---: | ---: | ---: | --- |
| Stripped release executable | 80,195,504 B | 80,195,504 B | 0 B / 0% | Logical file size of each package's `dist/serein` |
| All 211 installed Debian file payloads, logical bytes | 84,643,561 B | 84,643,561 B | 0 B / 0% | `dpkg-deb --extract`, then sum regular-file sizes |
| Compressed Debian distribution | 40,326,056 B | 40,326,284 B | +228 B / +0.0006% | Logical file size of each `.deb` |
| 100,000 legacy URL admissions, median | 72.159942 ms | 70.750168 ms | -1.409774 ms / -1.95% | Release component harness; `Instant`, one warmup, five batches |
| Message-scoped URLs admitted per 100,000 attempts | 0 | 100,000 | Newly supported form | Count successful admissions in each component batch |

Both `cargo xtask package` runs passed the native Debian smoke check, including
installed contents and host shared-library closure. Payload bytes exclude filesystem
allocation overhead. Package-size differences do not establish runtime memory use.

The [component harness](pr-evidence/voice-attachment-playback/benchmark.py) snapshots
the production validator and compiles it with `rustc -O` and the package build's
release model/URL dependencies. It uses synthetic signed URLs and bounded voice
metadata, `black_box`, one warmup and five measured 100,000-attempt batches per path.
No compiler ran during measured batches. Legacy samples were
71.737262/70.918062/72.180922/72.159942/72.627516 ms before and
71.178538/70.598705/70.750168/70.412517/71.634009 ms after. The small difference
on this shared workstation is noise, not a claimed speed improvement.

The previously rejected message-scoped path took a 49.470710 ms median; after
passing the additional admission guards it took 80.415707 ms. Those timings perform
different work. This is URL admission, not network, rendering or playback latency.
Native audio CPU/RSS, UI frame timing, device latency and live CDN behavior remain
unmeasured. No audio device, account, microphone or live media request was used.

Reproduce after packaging either revision, with the same `CARGO_TARGET_DIR` used
for that build: `python3 docs/pr-evidence/voice-attachment-playback/benchmark.py`.
The script also accepts a baseline-worktree path as its first argument.


## Gateway outage recovery verification (October 2, 2026)

Exact main `71ebbc1c0393a0ba4f4e6c93ae9b7b0bd3e06d35` is compared with recovery
runtime `d6bf2c8cb6077fd5c45b1348ab0cfcf46508f8c8`. The final evidence commit changes
only documentation, screenshots and records. The pinned Rust 1.98.1 toolchain,
lockfile and voice-inclusive standard packaging are unchanged. Both standard
packages freshly compiled all twelve runtime crates after scoped invalidation
across worktree PackageIDs and passed deep/strict local ad-hoc signing checks;
they are not notarized releases. Current full workspace tests, formatting,
strict Clippy and policy passed (368 UI / 168 desktop), including actual synthetic
Refresh/Send/Reconnect input and local READY/RESUMED recovery regression checks.
The synthetic authentication-handoff check also passed.

| Standard default FAT package | Main71 | Recovery D6 | Delta |
| --- | ---: | ---: | ---: |
| Executable | 62,269,488 B | 62,285,936 B | +16,448 B (+0.0264%) |
| Installed files | 68,279,917 B | 68,296,365 B | +16,448 B (+0.0241%) |
| Complete ZIP | 43,363,423 B | 43,364,981 B | +1,558 B (+0.0036%) |
| File count | 206 | 206 | 0 |

A separate matched native pair uses instrument-free default-plus-demo builds on
Apple M1/macOS 27/16 GiB/Metal, with the same process-only thin-LTO override and
jobs2; other repository release settings are unchanged. All four build owners
explicitly held compilers, native apps and heavy IO, and host process inspection
confirmed the quiet window. Each ordinary `--demo --demo-chat` run used five
seconds warmup and ten one-second `ps` samples, then stopped with SIGINT.

| Quiet native idle | Main71 | Recovery D6 | Delta |
| --- | ---: | ---: | ---: |
| Median CPU | 0.0% | 0.0% | 0.0 percentage points |
| Peak RSS | 125,920 KiB | 125,856 KiB | -64 KiB |
| Settled RSS (last-five median) | 125,872 KiB | 125,808 KiB | -64 KiB (-0.0508%) |

The same quiet window ran instrument-free default-FAT reducer binaries: one
warmup each, then five alternating pairs of 100,000 synthetic events. Median
elapsed time was 53.615000 to 53.792709 ms (+0.177709 ms); ranges
53.371750–54.605125 and 53.614125–54.215333 ms overlap. Both retained
331,992–332,477 estimated timeline bytes / 500 records. These are reducer
regression observations, not RSS or UI timing. Idle quantization, the small RSS
difference and overlapping reducer ranges support no performance improvement
claim. The ordinary demo does not exercise a real outage. Queue latency, active
frame/startup timing, GPU memory, physical sleep/wake, live RESUME and delivery
remain unmeasured.

Eight actual native Metal screenshots were inspected: exact before/after,
dark/light and wide/narrow synthetic disconnected states with a kept draft.
Temporary capture hooks were removed byte-exactly before standard/optimized
builds. The pure UI fixture restores demo mode before desktop dispatch and
constructs no account, transport, cache or media workers. No live account,
message, upload, microphone, camera or system capture picker was used.
Recovery retains one coalesced notification and one explicit-send pulse per
outage; REST writes retain their existing bounds and are never automatically
replayed after failed or ambiguous delivery. Raw samples, source/binary/image
identities and seven bounded actual build/check logs are in
[`resume-send/measurements.json`](pr-evidence/resume-send/measurements.json) and
[`resume-send/capture.json`](pr-evidence/resume-send/capture.json).

## Voice and streaming reliability audit (October 3, 2026)

Baseline `074ba3a158b72ddb9b7bc327fba64ca4b93975e2` is compared with the
runtime source hashes in
[`voice-stream-reliability/package-measurements.json`](pr-evidence/voice-stream-reliability/package-measurements.json).
Both freshly built standard packages include voice, use pinned Rust 1.98.1 and
the unchanged lockfile, and pass local ad-hoc signing. They are not notarized.
Host: macOS 27 / Darwin 27.0 arm64, Apple M1 Pro, 16 GiB RAM; no demo or
developer-session features. Installed bytes sum all regular package files;
complete ZIPs use sorted paths and Python `ZIP_DEFLATED`, compression level six.

| Standard package metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 67,261,440 B | 67,245,024 B | −16,416 B (−0.0244%) |
| Installed files | 73,345,536 B | 73,329,120 B | −16,416 B (−0.0224%) |
| Complete ZIP | 47,536,458 B | 47,535,475 B | −983 B (−0.0021%) |
| File count | 220 | 220 | 0 |

A separate release harness compiles the exact baseline/current Linux native
decoder modules with the same surrounding type/limit shims. Linux aarch64,
Debian 12, GStreamer 1.22.0, Rust 1.98.1; container limited to two CPUs and 2 GiB.
One warmup runs five native tests; five serial measured runs offer 32 Annex-B
filler access units, each 2,162,688 bytes, to a paused native pipeline.

| Median stalled native queue | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Retained compressed payload | 69,206,016 B | 8,650,752 B | −60,555,264 B (−87.5%) |
| Queued access units | 32 | 4 | −28 (−87.5%) |

Every sample produced the same counts. The old nonblocking appsrc admitted all
offered units despite `max-bytes`; the new admission check enforces four units
and 8,650,752 bytes before copying. This is native queue accounting under pressure,
not process RSS, decoded/GPU memory, throughput or a live-latency improvement.
The outer worker queue and buffers already consumed by the pipeline are separate.
The older-than-1.20 software fallback was reviewed and guarded in tests but not
executed on an old native runtime. No runtime dependency was added or upgraded.

Reproduce using the commands and small harness script in the
[evidence README](pr-evidence/voice-stream-reliability/README.md); raw runs and
source/runtime identities are in
[`release-summary.json`](pr-evidence/voice-stream-reliability/release-summary.json).
Focused voice tests pass (60 tests), as do non-UI workspace tests, strict Clippy,
formatting, policy and production checks. `cargo xtask check` is blocked by UI
failures, including an abort reproduced on unchanged baseline main; the PR stays
draft. Standard packaging succeeds before and after.

Ordinary offline demo idle sampling does not exercise these media paths. Active
CPU/RSS, callback timing, sender-clock A/V sync and live Linux portal/GPU behavior
remain unmeasured. No account, microphone, camera or desktop capture was used.
Synthetic delivery and bounded recovery do not prove resolution of intermittent
official-client error 2012. Very large keyframes at low feedback bitrates still
need owner-controlled investigation; no automatic resolution adaptation was added.

## Native Windows installer smoke (October 3, 2026)

Installer source `4be6d629` replaces setup/uninstall PowerShell with native Windows
process enumeration and shortcut property-store calls. Run
`python packaging/windows/test_installer.py` in a disposable Windows user with
pinned Rust 1.98.1 and NSIS. The script compiles an optimized, std-only offline
fixture; it never runs Serein, connects an account or opens audio devices.

| Synthetic fixture metric | Windows x64 | Windows ARM64 |
| --- | ---: | ---: |
| Compressed setup EXE | 870,573 B | 874,551 B |
| Fresh silent installation | 254.489 ms | 1,159.990 ms |
| First / second silent upgrade | 208.480 / 250.311 ms | 377.350 / 337.698 ms |

One CI run per architecture, no warmup: elapsed wall time around the setup
subprocess includes extraction, process checks, shortcut and registry writes.
These three samples perform different work and are not a latency benchmark or
before/after speed claim. [x64 evidence](https://github.com/ViceVerse-cz/Serein/actions/runs/37088681680/job/111104156410)
and [ARM64 evidence](https://github.com/ViceVerse-cz/Serein/actions/runs/37088681680/job/111104156236)
also verify the running-app guard, Unicode paths, shortcut target/working directory/
AppUserModelID, legacy-script removal and uninstall cleanup.

Production Windows executable, full voice-inclusive installed package and
distribution sizes and baseline installation timings remain unmeasured on the
macOS host. No Rust runtime or voice dependency changed. The standard host
voice-inclusive package passes; full workspace tests remain blocked by unchanged
UI failures. Antivirus acceptance and Windows signing are separate, unverified
release concerns.

## Light controls with window transparency (October 3, 2026)

Baseline `f3f244c97f223280b49966ac96284aa72fd03a1f` is compared with the
runtime source hashes in
[`light-transparency-surfaces/measurements.json`](pr-evidence/light-transparency-surfaces/measurements.json).
Host: Ubuntu 26.04.1 / Linux x86_64, AMD Ryzen 5 7535U, 12 logical CPUs,
15,369,359,360 bytes RAM, native eframe WGPU/Vulkan (RADV REMBRANDT), display
scale 1. Both revisions use pinned Rust 1.98.1 and the unchanged lockfile.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Idle CPU, mean / median of one core | 0.00% / 0.00% | 0.05% / 0.00% | +0.05 / 0.00 percentage points |
| Sampled peak / settled RSS | 130,904,064 B | 129,536,000 B | −1,368,064 B (−1.05%) |
| Standard executable | 85,350,064 B | 85,352,368 B | +2,304 B (+0.0027%) |
| Installed regular files | 89,864,185 B | 89,866,489 B | +2,304 B (+0.0026%) |
| Complete compressed DEB | 44,192,928 B | 44,194,608 B | +1,680 B (+0.0038%) |

Native sampling uses the existing offline `profile_preview` harness at
1120 × 760, Standard/Light, 100% palette transparency: the Appearance modal
opens once and then idles. Both optimized builds use the same final-crate
`-C lto=off` override. After eight seconds of warmup and three seconds of
settling, psutil collects twenty one-second process CPU/RSS samples; settled
RSS is the median of the final five. No helper processes were present and no
build ran during sampling. Small CPU/RSS differences are sampling, allocator
or driver noise; this is not a performance improvement claim. Frame/startup
latency, startup memory peaks, GPU memory and live traffic are unmeasured.

Standard voice-inclusive packages use `cargo xtask package`, normal fat LTO
and no default/demo features. Both pass the Debian package smoke checks
(225 files). Installed bytes sum regular files extracted from each DEB;
compressed bytes measure the complete DEB distribution.

The [evidence README](pr-evidence/light-transparency-surfaces/README.md)
records reproduction commands and native before/after captures. The opaque
preview viewport validates control rendering, not desktop alpha/blur or
Windows/macOS compositors. Focused transparency tests, remaining workspace
packages, formatting, strict Clippy, policy and the production check pass.
`cargo xtask check` stops at the unchanged native video pressure test
`stalled_native_queue_rejects_pressure_at_item_and_byte_limits`; the same
failure reproduces on baseline main, so the PR remains draft.

## Markdown quote row boundaries — October 4, 2026

Compared renderer baseline `61a1a55001763b512e8a9813aeda74777122cb1d` with the
quote row-boundary fix using the exact announcement fixture in the existing native
`profile_preview --demo --page=markdown` harness. The same fixture page was added
to both builds; the baseline renderer was unchanged. The preview uses production
message widgets and synthetic state, with no desktop account or media adapters.

Ubuntu 26.04.1, AMD Ryzen 5 7535U (12 logical CPUs), 14.3 GiB RAM, Xvfb/X11,
1400×900 logical pixels at scale 1. Both builds used Rust 1.98.1, release,
`--no-default-features --features demo`, and WGPU/Vulkan with the Mesa lavapipe
ICD forced and its mapped driver library verified. After an identical neutral
sidebar click and an eight-second warmup, one fresh process per revision received
20 one-second psutil samples. CPU is percent of one logical core; peak RSS is
post-warmup, and settled RSS is the last five samples' median. No helper children
were present during sampling.

| Native idle metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Mean CPU, one logical core | 0.00% | 0.00% | 0.00 percentage points |
| Sampled peak RSS | 171.883 MiB | 175.074 MiB | +3.191 MiB / +1.86% |
| Settled RSS | 171.883 MiB | 175.074 MiB | +3.191 MiB / +1.86% |

This single launch pair does not separate layout cost from allocator/driver
variation. Startup, full-frame p95, GPU memory and live workloads were not measured.
Standard voice-enabled Debian package sizes, hashes and raw process samples are
recorded in [the task measurements](pr-evidence/markdown-wrapped-quote/measurements.json).

Reproduce the native fixture with:

```bash
cargo build --release --locked -p serein --no-default-features --features demo --example profile_preview
./target/release/examples/profile_preview --demo --interactive --page=markdown --width=1400 --height=900
```

The committed pair also covers a 760×900 light viewport (`--light`). A 760×520
native view was scrolled to inspect the long quote. Geometry regressions cover
three sources, 220/560/1260-point widths, light/dark themes and scales 1/2.
The exact announcement failed on the baseline because the following text began
at y=76 while its quote rail extended to y=216; explicit row boundaries pass.
`cargo xtask check` and strict Clippy for the demo preview passed.

## Compact message baselines — October 6, 2026

Compared baseline `2423f600ad3cdb266c4c5ea8a840b2b2ade4795c` with runtime
commit `05af1e6aee17a44f5821f8abe5d20350722121dc`. Both native previews use the
same `--compact` harness addition, pinned Rust 1.98.1, release,
`--no-default-features --features demo`, and separate Cargo target directories.
The [measurements](pr-evidence/compact-message-alignment/measurements.json) record
source/binary hashes, package sizes and raw process samples; the
[README](pr-evidence/compact-message-alignment/README.md) gives reproduction commands.

Host: Windows 11 Home 10.0.26200, Ryzen 7 7800X3D (16 logical CPUs),
33,410,678,784 bytes RAM, NVIDIA RTX 5070 Ti / driver 591.86, native WGPU/Vulkan.
Both processes mapped `vulkan-1.dll` and NVIDIA's `nvoglv64.dll`. Display scale is
1.25; dark and light framebuffer captures are 1750×1125 and 950×1125 pixels.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Mean idle CPU, one logical core | 0.2172% | 1.4059% | +1.1887 percentage points |
| Median idle CPU, one logical core | 0.00% | 0.00% | 0.00 percentage points |
| Sampled peak / settled working set | 144,941,056 B | 149,811,200 B | +4,870,144 B (+3.36%) |
| Standard voice-enabled executable | 84,548,096 B | 84,562,944 B | +14,848 B (+0.0176%) |
| Installed regular files | 88,721,292 B | 88,736,140 B | +14,848 B (+0.0167%) |
| Complete ZIP, .NET Optimal compression | 49,426,763 B | 49,432,817 B | +6,054 B (+0.0122%) |

Sampling uses one fresh interactive preview per revision at 1400×900 logical
pixels, `--demo --interactive --compact --page=member-tags`, default bottom
position. After eight seconds of warmup, twenty approximately one-second samples
measure process CPU time deltas and `WorkingSet64`; settled working set is the
last five samples' median. No builds run during sampling. Observed `conhost.exe`
children are excluded from these process-only measurements. This single launch
pair includes five nonzero CPU intervals after and one before; it does not
separate layout cost from input, allocator or driver variation. No performance
improvement or causal regression is established. Full-frame p95, startup peaks,
GPU memory and live traffic remain unmeasured.

Both standard voice-inclusive packages pass `cargo xtask package` with normal
fat LTO and no demo feature (213 installed files each). ZIPs use
`ZipFile.CreateFromDirectory`, `CompressionLevel.Optimal`, without a base-directory
entry. These packages precede this evidence-only documentation appendix. NSIS is
unavailable locally, so the optional Windows installer binary is not generated.

The committed captures use the same scene with `--scroll=-1200`, wide dark and
narrow light, and show aligned compact timestamp/author/body text. They are actual
native WGPU framebuffer exports, not OS screenshots or native-input proof; the
Computer Use helper cannot connect to its native pipe (`os error 2`). Workspace
tests (1,153 passed, 27 ignored), strict Clippy, policy and production checks pass.
`cargo xtask check` remains blocked by pre-existing formatting in
`crates/discord-api/src/forum.rs`, reproduced on the untouched baseline. Linux,
macOS and live Discord behavior remain unverified locally.

## Emoticon review fixes — October 6, 2026

Compared rebased PR baseline `20c2bb5f` with runtime `50873692` on Apple M1 Pro,
16 GiB RAM, macOS 27.0, pinned Rust 1.98.1. The std-only component harness
compiles each exact converter with `rustc -O`; one warmup and five alternating
measured batches each convert 100,000 synthetic messages using `black_box` and
`Instant`. These timings include output allocation.

| Message / bytes | Before, median µs | After, median µs | Delta |
| --- | ---: | ---: | ---: |
| Plain emoticons / 1,280 | 2.850 | 3.320 | +0.470 |
| Code and links / 1,710 | 3.950 | 5.042 | +1.093 |
| Escaped/unmatched ticks / 1,160 | 2.293 | 3.164 | +0.871 |

The scanner now indexes matching delimiters and preserves unmatched inline
markers as literal text. Samples were taken on a shared host with compiler
activity, so these are observational timings, not an uncontended comparison or
a performance improvement claim. Conversion occurs on submission. Native
frame timing and process CPU/RSS remain unmeasured. The baseline standard release package completed successfully (67,589,904-byte
executable) as verification was stopped at the user’s request. The changed release
package was not built; executable, installed package and distribution size
comparisons remain unmeasured. No dependencies
were added. Reproduction, raw samples and inspected synthetic native captures
are in [the evidence directory](pr-evidence/emoticon-review/README.md).


# Attachment storage selectors — October 7, 2026

Baseline `884fb73fa68bffda70b1f308c903ece239281522`, compared with the
`backend=b2`/`backend=b3` URL admission change delivered alongside this report.
Both standard `cargo xtask package` builds completed on macOS 27.0 (26A428),
Apple M1, 16 GiB RAM, Rust 1.98.1 with the lockfile and default/demo features
disabled; voice remains included. Both ad-hoc signatures verified successfully.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Desktop executable | 68,180,992 B | 68,180,992 B | 0 B |
| Installed app, sum of files | 74,201,013 B | 74,201,013 B | 0 B |
| Distribution directory, sum of files | 74,265,778 B | 74,265,778 B | 0 B |
| Complete ZIP, `ditto -c -k --keepParent dist` | 48,039,907 B | 48,040,389 B | +482 B / +0.0010% |

This changes one bounded URL query predicate. Metadata remains limited to 2,048
URL bytes; download limits, decoding, rendering, caches, queues and dependencies
are unchanged. Nine focused download tests and all 188 non-ignored desktop tests
passed. No CPU/RSS, native frame-time or decoder-throughput comparison was run;
the separate playback CPU complaint in issue #556 is not resolved by URL admission.
The small archive difference supports no runtime-performance claim. Raw package
sizes and the changed source hash are in
[the size record](pr-evidence/attachment-storage-selectors/package-sizes.json).

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

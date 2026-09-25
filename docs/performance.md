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

# Performance findings

Recent synthetic/offline measurements are workload-specific. They do not establish live Discord
performance, universal device results or application-wide memory bounds. The older upstream PR screenshot,
log and per-run evidence archive has been removed; the summaries below retain the useful results.

## Member cache follow-up

The follow-up compares `65e69baf` with the member-cache and resident-history changes in
this PR. On Linux x86_64 (Xeon Platinum 8573C), Rust 1.98.1 release builds used opt-level
3, fat LTO, one codegen unit and CPU 0 affinity. Results are medians; neither workload
measures GUI latency, live Discord traffic or cold disk I/O.

| Synthetic workload | Before | After | Method |
| --- | ---: | ---: | --- |
| 100 in-memory SQLite saves, 512 members, one nickname changed per save | 403.883 ms | 80.576 ms | One warmup, five alternating runs; actual `LocalStore` calls |
| 2,048 unchanged observations across 128 members | 58.595 ms; 2,048 snapshots | 1.321 ms; zero snapshots | One warmup, 15 alternating runs; assumes the worker finishes between observations |
| Reducer, 100,000 messages | 83.103 ms | 80.073 ms | Same 15-run batch; 500 retained records |
| Wide channels, 10,000 messages | 9.835 ms | 8.689 ms | Same 15-run batch; 500 retained records |

The unchanged-observation workload models a worst case for repeated snapshot construction;
real bursts may already coalesce. A preceding five-run batch showed the wide-channel median
increase by 6.43%, while the 15-run batch above moved in the other direction. The samples
were variable, so neither establishes a general reducer improvement or regression. The
SQLite workload uses an in-memory database; persistent storage and end-to-end impact remain
unmeasured. The measured source files have SHA-256 prefixes `8c8d24a5` (member identity),
`48efe02f` (resident history), and `96135681` (SQLite member storage).

The `65e69baf` native release baselines are 100,638,720 / 228,369,171 / 137,111,298 bytes
for the Windows executable, installed payload and installer; 98,777,240 / 227,363,188 /
141,899,312 bytes for the Linux executable, payload and DEB; and 79,212,416 / 202,656,528 /
136,670,762 bytes for the macOS executable, app bundle and ZIP. Updated package sizes and
native display, idle CPU, RSS and live synchronization measurements require native CI or
owner-controlled testing.

## Conversation and member loading

The October 29 follow-up compares frozen `af865308` with application source `d854f5fe`.
Desktop-only recovery follow-ups leave the measured core/model code unchanged.
The same loading harness was compiled against each revision's pinned Rust 1.98.1 release
libraries using optimization 3 and one codegen unit. Each batch uses one warmup pair and
five alternating measured pairs on Linux x86_64, pinned to CPU 0. It includes 1,000
busy 50-row channel switches and 100 member-sidebar cycles. Service responses are withheld
until after availability observations; their delay is not included in CPU timings.

| Loading fixture | Baseline | After |
| --- | ---: | ---: |
| Immediate busy-chat previews | 0 / 1,000 | 1,000 / 1,000 |
| Known nickname hits | 0 / 1,000 | 1,000 / 1,000 |
| Author lookup commands | 1,001 | 1 |
| Cached sidebar-page nickname hits | 0 / 100 | 100 / 100 |
| Sidebar reopen cache hits | 0 / 100 | 100 / 100 |
| Authoritative history requests | 1,000 | 1,000 |
| Live sidebar subscription commands | 300 | 300 |
| Estimated retained core bytes | 226,463 | 227,784 (+1,321) |
| Initial median core workload | 36.240 ms | 37.875 ms (+4.51%) |
| Same-binary repeat median | 35.606 ms | 36.028 ms (+1.19%) |
| Final-source median core workload | 38.809 ms | 39.204 ms (+1.02%) |

Both timing batches are retained. The initial difference exceeds 3%; it did not reproduce
above that threshold in the second batch. This is not evidence of unchanged native frame
latency. The after fixture also retains useful identity/range data which the baseline drops.
CPU-clock samples of separate, symbolized diagnostic builds show the existing timeline,
permission and message-accounting paths; they do not isolate a cause for timing differences.
Shared-library samples remain unresolved. The measured request counts are commands from
the core, not observed live Discord request counts.

The final source's [native comparison](https://github.com/MCShotty/Serein/actions/runs/40838872889)
uses Rust 1.98.1, optimization 3, thin LTO, one codegen unit, stripped symbols and
panic abort, with one warmup and five order-rotated samples. The current published nightly
is pinned to `v1.0.0-nightly.20261028.68` (`1b2f8942`). Linux x64 and Windows Server
2025 x64 use one-CPU affinity; macOS 26.6.2 ARM64 uses its platform scheduler.

| Runner / median milliseconds | Nightly | Frozen baseline | Implementation | Baseline delta |
| --- | ---: | ---: | ---: | ---: |
| Linux / reducer | 68.469 | 68.766 | 77.642 | +12.9% |
| Linux / wide channels | 217.717 | 8.295 | 8.475 | +2.2% |
| Windows / reducer | 107.118 | 110.497 | 110.389 | -0.1% |
| Windows / wide channels | 171.348 | 11.685 | 11.727 | +0.4% |
| macOS / reducer | 65.947 | 60.707 | 63.542 | +4.7% |
| macOS / wide channels | 92.461 | 6.320 | 6.406 | +1.4% |

The Linux and macOS reducer increases exceed 3% in this batch; the cause is not
established. A matched local diagnostic build measured -2.2% for reducer and +4.3%
for wide channels after a prior +8.3% / +0.6% batch. Diagnostic binaries retain
symbols and are labelled separately from CI and packages. Earlier `d22ad5d7` / `9089b23d`
macOS wide-channel results were +40.4% / -8.7% with byte-identical benchmark binaries.
Neither overlap nor a favorable repeat alone establishes a speedup or resolves every
platform regression. Subsequent same-code CI outcomes are tracked in PR #32.

A candidate split of dormant-preview mutation from dispatch was also measured with
one warmup and five rotated triples: baseline reducer 66.441 ms, `d854f5fe` 67.641 ms,
candidate 67.721 ms; wide channels 7.101 / 6.912 / 7.294 ms. The candidate did not
demonstrate a benefit and was removed. This preserves the preload/deletion safety
paths and avoids retaining a speculative optimization.

The new `cargo replay --loading` runs in native CI alongside the existing replay and
soak. Normal package sizes are recorded with artifact/source provenance in the
[loading evidence](pr-evidence/chat-member-loading/README.md) and PR #32. Native preview
latency, cold Windows storage, idle CPU, process RSS, avatar decoding and live synchronization
remain unmeasured in this Linux workspace.

## Live member identity follow-up

Compared final application source `1607575e` against frozen #30 `dff256ff` and
published nightly `v1.0.0-nightly.20261027.67` (`2d0165bf`) in
[source CI](https://github.com/MCShotty/Serein/actions/runs/40608770724).
Rust 1.98.1 and committed locks; release optimization 3, thin LTO, one codegen
unit, line-table debug information, stripped symbols and panic abort. Separate
source/target directories, one warmup and five alternating measured repetitions.
Linux 6.17 x64/glibc 2.39 and Windows Server 2025 x64 have four logical CPUs and
use single-CPU affinity. macOS 26.6.2 ARM64 has three logical CPUs and uses its
platform scheduler.
These are the existing core replay workloads, timed with their monotonic timer.

| Runner / median milliseconds | Nightly | Frozen baseline | Implementation | Baseline delta |
| --- | ---: | ---: | ---: | ---: |
| Linux / reducer | 56.525 | 58.137 | 57.705 | -0.7% |
| Linux / wide channels | 99.621 | 6.473 | 6.429 | -0.7% |
| Windows / reducer | 108.196 | 110.633 | 108.980 | -1.5% |
| Windows / wide channels | 172.316 | 11.637 | 11.677 | +0.3% |
| macOS / reducer | 68.023 | 64.722 | 65.831 | +1.7% |
| macOS / wide channels | 102.371 | 7.027 | 6.933 | -1.3% |

All six medians are within the 3% target in this batch. Small deltas are not an
optimization claim. The earlier `1a688389` macOS regression and its unisolated
cause remain documented below.
Nightly differences include earlier fork features. These measurements do not
establish UI latency, recovery latency, idle CPU, process RSS or live Discord
synchronization. Native macOS profiling remains unavailable in this Linux workspace.

The downloaded [Linux](https://github.com/MCShotty/Serein/actions/runs/40608770724/artifacts/12781773601),
[Windows](https://github.com/MCShotty/Serein/actions/runs/40608770724/artifacts/12781349440)
and [macOS](https://github.com/MCShotty/Serein/actions/runs/40608770724/artifacts/12781128341)
artifacts match GitHub's SHA-256 digests and record the full source revisions,
host information, binary hashes and individual samples.

Normal packages use the unchanged repository release profile (fat LTO, one codegen
unit, stripped symbols and line-table debug information), including voice and
excluding demo/developer features. The verified baseline artifacts at `f47ac7dc`
match the application source of `dff256ff`; only three evidence files differ.
All downloaded archives match their GitHub SHA-256 digests.

| Package metric / method | Frozen application baseline | Implementation | Delta |
| --- | ---: | ---: | ---: |
| Linux executable | 98,385,816 B | 98,518,296 B | +132,480 B (+0.13%) |
| Linux staged payload, excluding DEB | 226,971,764 B | 227,104,244 B | +132,480 B (+0.06%) |
| Linux DEB installed size, control metadata | 222,246 KiB | 222,376 KiB | +130 KiB (+0.06%) |
| Linux DEB | 141,754,980 B | 141,844,556 B | +89,576 B (+0.06%) |
| Windows executable | 100,094,464 B | 100,318,720 B | +224,256 B (+0.22%) |
| Windows installed source payload | 227,824,915 B | 228,049,171 B | +224,256 B (+0.10%) |
| Windows NSIS, normal packager | 136,993,389 B | 137,048,329 B | +54,940 B (+0.04%) |
| macOS executable | 78,867,184 B | 79,015,120 B | +147,936 B (+0.19%) |
| macOS app bundle | 202,311,296 B | 202,459,232 B | +147,936 B (+0.07%) |
| macOS artifact ZIP | 136,493,891 B | 136,561,623 B | +67,732 B (+0.05%) |

Linux retains 289 staged files and 304 installed regular files. Windows retains
289 installed source files, excluding setup scripts and the generated uninstaller.
macOS retains 281 app-bundle files; its artifact ZIP includes four top-level
license/README files. Compressed sizes include archive metadata. The separately
verified Windows installer is linked in the connection evidence; its compression
output is distinct from the normal-packager size above.

The current Linux lifecycle soak completed 3,778 passes, 120,896 channel visits,
72,537,600 live inserts and 15 logout cycles in 120.017 seconds, with the shipping
row/byte assertions passing. The native Windows and macOS 120-second soaks also
pass. This is bounded synthetic core pressure, not process RSS or live networking.
See [connection evidence](pr-evidence/connection-reliability/README.md) for current
checks and package provenance.

## Connection recovery and queue pressure

Compared frozen #30 `dff256ff` and implementation `1a688389` with published
nightly `v1.0.0-nightly.20261027.67` (`2d0165bf`) in
[native CI](https://github.com/MCShotty/Serein/actions/runs/40591903694).
Rust 1.98.1, committed locks, release optimization 3, thin LTO, one codegen unit,
line-table debug information, stripped symbols and panic abort. Each revision used
a separate source/target directory, one warmup and five alternating measured runs.
Linux 6.17 x64/glibc 2.39 and Windows Server 2025 x64 have four logical CPUs and
single-CPU affinity; macOS 26.6.2 ARM64 has three and uses the platform scheduler.
Times are median milliseconds from the existing replay workloads' monotonic timer.

| Runner / workload | Nightly | Frozen baseline | Implementation | Baseline delta |
| --- | ---: | ---: | ---: | ---: |
| Linux / reducer | 63.069 | 61.498 | 61.817 | +0.5% |
| Linux / wide channels | 73.919 | 6.725 | 6.544 | -2.7% |
| Windows / reducer | 106.675 | 110.075 | 110.282 | +0.2% |
| Windows / wide channels | 224.446 | 11.482 | 11.740 | +2.2% |
| macOS / reducer | 100.630 | 87.103 | 89.398 | +2.6% |
| macOS / wide channels | 139.161 | 7.714 | 8.007 | **+3.8%** |

A second macOS batch at the same source, baseline, nightly, compiler and release
settings used one warmup and five alternating samples on another hosted runner.
It rebuilt byte-identical benchmark executables. Its own paired comparison is:

| macOS repeat / workload | Nightly | Frozen baseline | Implementation | Baseline delta |
| --- | ---: | ---: | ---: | ---: |
| Reducer | 90.348 | 80.547 | 83.136 | **+3.2%** |
| Wide channels | 140.430 | 7.516 | 7.829 | **+4.2%** |

The macOS wide-channel cost exceeds the 3% target in both current-source batches;
the repeat reducer also misses it. This remains an unresolved regression, with no
performance pass claimed. Overlapping ranges do not establish equivalence. The
earlier `02f5b468` batch was +35.1% on a different hosted run; its absolute
timings cannot establish an improvement. The benchmark workloads are unchanged;
code inspection has not isolated the cause. Further local profiling was blocked
when the execution service disconnected. No speculative optimization was applied.
[Repeat artifact](https://github.com/MCShotty/Serein/actions/runs/40591903694/artifacts/12778266987)
retains the independent samples; the original batch remains separately archived.

Nightly deltas include prior fork features. These core workloads do not measure
Gateway/HTTP recovery latency, idle CPU, process RSS, presented frames or media.
Those measurements and owner-controlled network/sleep acceptance remain unavailable.

Package baselines were inspected from immutable native artifacts at `f47ac7dc`,
whose application source matches `dff256ff` (only three evidence files differ).
ZIP digests matched GitHub artifact metadata. Normal package outputs include voice
and use the repository release profile, separate from replay settings.

| Package metric / method | Frozen application baseline | Implementation | Delta |
| --- | ---: | ---: | ---: |
| Linux executable | 98,385,816 B | 98,458,264 B | +72,448 B (+0.07%) |
| Linux staged payload, excluding DEB | 226,971,764 B | 227,044,212 B | +72,448 B (+0.03%) |
| Linux DEB installed size, control metadata | 222,246 KiB | 222,317 KiB | +71 KiB (+0.03%) |
| Linux DEB | 141,754,980 B | 141,805,240 B | +50,260 B (+0.04%) |
| Windows executable | 100,094,464 B | 100,254,720 B | +160,256 B (+0.16%) |
| Windows installed source payload | 227,824,915 B | 227,985,171 B | +160,256 B (+0.07%) |
| Windows NSIS, normal packager | 136,993,389 B | 137,020,063 B | +26,674 B (+0.02%) |
| macOS executable | 78,867,184 B | 78,949,360 B | +82,176 B (+0.10%) |
| macOS app bundle | 202,311,296 B | 202,393,472 B | +82,176 B (+0.04%) |
| macOS artifact ZIP | 136,493,891 B | 136,534,737 B | +40,846 B (+0.03%) |

Linux retains 289 staged files and 304 installed files. Windows source-payload
counts exclude setup scripts and the generated uninstaller. The macOS artifact ZIP
also includes four top-level license/README files. Small compressed-size deltas
include archive metadata and do not establish a runtime improvement.
The current Linux 120-second lifecycle soak completed 4,056 passes, 129,792 channel
visits, 77,875,200 live inserts and 16 logout cycles in 120.019 seconds; the shipping
row/byte assertions passed. This is bounded core pressure, not process RSS.
See [connection evidence](pr-evidence/connection-reliability/README.md) for coverage,
installer provenance and remaining limitations. Historical measurements below retain
their original scope.

## Server folder mark-as-read — October 15, 2026 (PR #672)

Compared base `104b1aa5` with folder mark-as-read `2b02b49e`. Standard unsigned Windows x64
portable packages use Rust 1.98.1 and `cargo xtask package` (voice included, demo disabled).
Each installed tree contains 216 files; ZIP output uses .NET `Compress-Archive`. NSIS was
unavailable, so no installer was measured.

| Package metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| `serein.exe` | 87,399,936 B | 87,405,568 B | +5,632 B (+0.0064%) |
| Installed tree | 91,593,225 B | 91,598,857 B | +5,632 B (+0.00615%) |
| ZIP archive | 51,350,178 B | 51,354,785 B | +4,607 B (+0.00897%) |

Release demo builds at both revisions ran the same offline `--demo` workspace with the
synthetic folder context menu open in a 1120×760 window. The after menu shows the new row disabled
because this fixture has no unread guild in that folder. On Windows 11 Pro 10.0.26300, Ryzen 7
7800X3D (8 cores / 16 logical), 31.4 GiB visible RAM and AMD Radeon RX 7900 GRE (DX12),
process counters were sampled once per revision after a five-second warmup, 21 samples about one
second apart. CPU is the `Get-Process` CPU-time delta divided by actual elapsed wall time, where
100% is one logical core. Memory reports peak and mean of the final five samples.

| Native demo metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| CPU, one-core percentage | 1.4795% | 0.3893% | -1.0902 percentage points |
| Working set, peak / final-five mean | 185,057,280 B | 167,518,208 B | -17,539,072 B |
| Private bytes, peak / final-five mean | 204,038,144 B | 185,970,688 B | -18,067,456 B |

These single samples are descriptive only; no runtime improvement is claimed. Startup and frame
latency were not measured. The synthetic screenshots, raw samples and reproduction details are in
[`docs/pr-evidence/folder-mark-as-read/`](pr-evidence/folder-mark-as-read/). Live Discord behavior
and clicking the action against a folder containing unread guilds remain outside this offline run.
A later revision of the PR replaced the per-guild queue with the home tile's bulk acknowledgement
scoped to the folder's servers; it was not re-measured.

## Personal notification and navigation controls — October 17, 2026

Compared current upstream nightly `v1.0.0-nightly.20261014.58` (`3a4bf8d`),
fork baseline `f9e39f7`, and implementation `8205c36d931b`. Each release replay
was built with Rust 1.98.1, the committed lockfile, default replay features, fat
LTO and one codegen unit, using a separate source/target directory. Linux 6.18.44
x86-64/glibc 2.41 container, Xeon Platinum 8573C, five visible CPUs and
18,440,136 KiB visible RAM. Builds and tests were idle during sampling. One warmup
and five measured direct-binary runs per revision/workload, rotating revision order,
with the measurement process and its children pinned to CPU 0; elapsed times are
reported by the workload's `Instant` timer. No account, network, storage or audio device.

| Release workload / median | Current nightly | Fork baseline | After | After vs nightly | After vs fork |
| --- | ---: | ---: | ---: | ---: | ---: |
| Reducer, 100,000 events | 108.718 ms | 124.482 ms | 122.868 ms | +13.0% | −1.3% |
| Wide-channel reducer, 10,000 messages / 20,001 channels | 166.945 ms | 196.989 ms | 174.953 ms | +4.8% | −11.2% |
| Steady retained timeline estimates, 500 records | 393,492–393,977 B | 401,492–401,977 B | 401,492–401,977 B | +8,000 B | unchanged |

Scheduling variation is substantial: reducer ranges were 104.234–218.455 ms
(nightly), 110.811–216.428 ms (fork), and 112.729–185.926 ms (after); wide-channel
ranges were 146.976–175.158, 169.186–227.320, and 162.341–221.322 ms respectively.
An initial unpinned trial gave +5.2% / +33.2% against nightly, with a noisy
106.586–406.827 ms nightly wide-channel range, prompting the affinity-controlled
comparison above. No speedup is claimed. The nightly comparison includes earlier
video, reliability and upstream changes already in the fork; it does not isolate
this milestone. The feature's retained timeline estimates match its fork baseline.

The 120-second release lifecycle soak completed 1,444 passes / 46,208 channel
visits / 27,724,800 live inserts and six logout cycles in 120.896 seconds, with
its shipping row/byte assertions passing. This measures synthetic core pressure;
native UI CPU/RSS, frame latency and live synchronization remain unmeasured.

Current-nightly package baselines were downloaded from the immutable release tag
and checked against its published SHA256SUMS. Payload bytes sum regular archive
entries; Linux uses the Debian data archive, Windows its portable ZIP, and macOS
the complete ZIP including the app bundle and notices.

| Nightly package | Executable bytes | Full payload bytes | Published archive bytes |
| --- | ---: | ---: | ---: |
| Ubuntu 26.04 x64 DEB | 87,746,736 | 92,280,770 | 44,864,244 |
| Windows x64 ZIP | 86,937,600 | 91,130,889 | 51,171,600 |
| macOS ARM64 ZIP | 69,149,632 | 75,120,529 | 48,274,698 |

Current-head native application/package checks and matching after-package
measurements are tracked in the PR. No local production package, native Windows/macOS
UI performance comparison or live OS-notification check is claimed here.

## Name styles and bundled name fonts - October 15, 2026

Compared baseline `5f83077e` (`main`) with the name-styles branch on Linux (CachyOS, x86_64),
both built with `cargo xtask package` in separate worktrees and target directories. On this
host the `.deb` stage fails in `dpkg-shlibdeps` for both revisions, so the measurement uses
the release executable and the staged `dist` payload produced before that stage. Compressed
size is `zstd -19` of the executable alone.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Release executable bytes | 88,266,920 | 91,822,792 | +3,555,872 (+4.03%) |
| Staged `dist` payload bytes (no archives) | 92,445,973 | 96,065,437 | +3,619,464 (+3.92%) |
| Executable, `zstd -19` bytes | 42,744,679 | 44,296,091 | +1,551,412 (+3.63%) |

The growth is the 14 embedded display-name fonts (3,493,132 bytes) and their license texts.
The faces are embedded but parsed only when a visible styled name first needs one; with name
styles off (the default) none is parsed. CPU, process memory and frame timings were not
sampled, and no runtime improvement is claimed.

## Game mention catalog and review fixes - October 14, 2026

Compared baseline `7759f5b5` with game mentions and review fixes at `75c4dfe0`.
Both unsigned Windows portable packages use Rust 1.98.1 and `cargo xtask package`
(`--release --locked --no-default-features`, voice included). The baseline `dist`
was preserved in its separate worktree. The after measurement uses a clean copy
of the generated package, excluding an obsolete `ruzstd-0.9.0-LICENSE` left in the
reused output directory; that original file was preserved. Both measured packages
contain 216 files. ZIP uses Python deflate level 9 with sorted relative paths.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 87,321,088 | 87,412,736 | +91,648 (+0.105%) |
| Full portable package bytes | 91,514,377 | 91,605,995 | +91,618 (+0.100%) |
| ZIP distribution bytes | 50,312,736 | 50,338,891 | +26,155 (+0.052%) |

NSIS was unavailable, so installer size was not measured. Native UI testing was
left to the owner at their request; CPU, process memory and frame timings were
not sampled. These package sizes do not measure catalog latency or live Discord
behavior, and no runtime performance improvement is claimed.

## Hover preload review repairs - October 14, 2026

Compared the original PR #636 head `08a9ae01` with runtime repair `d4b27dd3`, which also
integrates `main` at `cf5452cd`. This comparison does not isolate the five fixes from main changes.
Both fresh unsigned Windows portable packages use Rust 1.98.1 and `cargo xtask package`
(`--release --locked --no-default-features`, voice included), on Windows 11 Home 10.0.26200,
Ryzen 7 7800X3D, 33,410,678,784 bytes visible RAM. Separate worktree `dist` outputs were
preserved. Installed bytes sum all 216 files; ZIP uses Python deflate level 9 with sorted paths.
The built release replay ran directly: one warmup and five measured runs of 100,000 events.

| Metric / method | Original PR head | Review repair | Delta |
| --- | ---: | ---: | ---: |
| Package executable | 86,960,640 B | 87,321,088 B | +360,448 B (+0.41%) |
| Installed tree (216 files) | 91,153,929 B | 91,514,377 B | +360,448 B (+0.40%) |
| ZIP, deflate level 9 | 50,180,156 B | 50,312,732 B | +132,576 B (+0.26%) |
| Reducer replay, median of 5 | 73.1705 ms | 68.7076 ms | -4.4629 ms (-6.1%) |
| Retained timeline estimate | 393,492..393,977 B | 393,492..393,977 B | 0 |
| Navigation revalidation requests, 10,000 reselections | 0 | 0 | 0 |
| Hover first visits opened without waiting / requests | 12/12 / 12 | 12/12 / 12 | 0 |
| 28-row hover sweep requests | 12 | 12 | 0 |

The replay run ranges overlap (before 69.0651..76.5154 ms, after 66.9713..71.3841 ms); no speedup is claimed. Navigation retains 2 dormant windows / 150 rows / 328,148 estimated bytes on both revisions; the hover pass retains 8 windows / 983,809 estimated bytes. Core selection median/p95 is 300/300 ns on both, near timer resolution. These are bounded allocation estimates, not RSS.

NSIS is unavailable, so no installer was produced. The existing OpenH264 linker debug-info
warning appears in both builds. Native interaction capture and CPU/RSS sampling were unavailable
(no native computer-use API, no `orca` executable). No live Discord or microphone test was used.
The original feature screenshots are synthetic framebuffer exports, not native interaction proof.
[Per-run outputs and executable hashes](pr-evidence/hover-preload/measurements-review.json)
and [reproduction scenarios](pr-evidence/hover-preload/README.md) retain the evidence boundaries.

## Hover preloading — October 13, 2026

Release builds of baseline `2df8256c` (main) and the hover-preload branch with default features
(voice included), Rust 1.98.1, Windows 11 (build 26200), Intel Core i9-13900K, 32 GiB RAM. The
built `replay-bench` ran directly: one warmup and five measured runs, medians shown. These are
synthetic reducer workloads; they count requests and core selection time, not network latency,
display latency or RSS.

| Metric / method | Baseline | Hover preload | Delta |
| --- | ---: | ---: | ---: |
| Package executable (`xtask package`) | 86,803,968 B | 86,869,504 B | +65,536 B (+0.08%) |
| Installed tree (`dist/`, 216 files) | 90,997,257 B | 91,062,793 B | +65,536 B (+0.07%) |
| Installed tree, zip deflate level 9 | 50,161,038 B | 50,183,621 B | +22,583 B (+0.05%) |
| `replay-bench`: 100,000 events | 58.1 ms | 58.5 ms | noise (+0.6%) |
| `replay-bench`: retained timeline | 393,492..393,977 B | 393,492..393,977 B | 0 |
| `--navigation`: revalidation requests, 10,000 reselections of three 50-message conversations | 10,000 | 0 | −10,000 |
| `--navigation`: select median / p95 | 200 / 300 ns | 200 / 200 ns | at timer resolution |
| `--navigation`: dormant windows / rows / estimated bytes | 2 / 150 / 328,052 B | 2 / 150 / 328,148 B | +96 B |
| Hover pass: first visits that opened without waiting | — | 12/12, 12 requests | new pass |
| Hover pass: requests for a 28-row sweep without clicks | — | 12 | capped at 12/min |

The baseline has no hover pass; there a first visit sends the same one request on click and waits
for it. In the hover pass synthetic responses arrive before the click, so it shows that a click
uses the preload without a second request, not how much latency is hidden; that depends on the
dwell and the network round trip. The three-conversation workload keeps two dormant windows on
both revisions, so the larger eight-window limit is not exercised here. The profile-extras cap
(2 s to 600 ms), image connection reuse and off-coordinator disk reads were not measured: they need
a network or disk-cache fixture this bench does not have.
## Embed styling review - October 14, 2026

Compared the original PR #650 head `c01461ed` with the review fixes in `9354335e` on
Windows 11 Home 10.0.26200, Ryzen 7 7800X3D and 33,410,678,784 bytes physical RAM.
Both standard unsigned portable packages were built with Rust 1.98.1,
`cargo xtask package` (`--release --locked --no-default-features`), including voice.
The installed-tree metric sums file lengths; both ZIPs use .NET `ZipFile` with
`CompressionLevel.Optimal`. Separate worktree `dist` directories were used. Baseline package
artifacts were later removed during a build-cache cleanup; the figures below were recorded
before that removal, and the changed portable tree was rechecked afterwards.

| Metric | Original PR head | Review fixes | Delta |
| --- | ---: | ---: | ---: |
| Package executable | 86,917,632 B | 86,925,824 B | +8,192 B (+0.0094%) |
| Installed portable tree | 91,110,921 B | 91,119,113 B | +8,192 B (+0.0090%) |
| Portable tree, ZIP | 50,316,564 B | 50,320,128 B | +3,564 B (+0.0071%) |

Release `profile_preview --demo --page=embeds --width=1120 --height=900 --interactive`
used the same synthetic fixture in both revisions, wgpu and 125% display scaling. Each
run had a five-second warmup and 20 one-second `Get-Process` samples. Memory is the observed
peak/final sample; CPU is process CPU time divided by elapsed wall time (100% = one core).
The console helper is separate. Other compilation was active during sampling, and there
is only one run per revision, so these small differences are not an improvement claim.

| Native process metric | Original PR head | Review fixes | Delta |
| --- | ---: | ---: | ---: |
| Preview peak / settled working set | 174,346,240 B | 174,428,160 B | +81,920 B |
| Preview peak / settled private bytes | 388,874,240 B | 389,730,304 B | +856,064 B |
| Preview idle CPU, one core | 0.162% | 0.405% | +0.243 percentage points |
| `conhost` peak / settled working set | 10,629,120 B | 10,670,080 B | +40,960 B |
| `conhost` peak / settled private bytes | 1,622,016 B | 1,740,800 B | +118,784 B |
| `conhost` idle CPU, one core | 0% | 0% | 0 percentage points |

Both release builds emit the existing OpenH264 `LNK4255` debug-info warning. NSIS is
unavailable on this host, so packaging skips the installer executable. These measurements
do not establish live Discord interoperability, startup latency or frame percentiles.
The [synthetic native fixture and reproduction procedure](pr-evidence/embed-styling-review/README.md)
describe the capture environment and package method. Dark/light and narrow native PNGs
were inspected. Manual native clicks and keyboard interaction were stopped by the user's
physical Escape key; source destinations and media actions have synthetic interaction tests.

## Profile cosmetics — October 11, 2026

Release builds of baseline `178bde35` and the cosmetics branch on Linux 7.2.9 (CachyOS),
AMD Ryzen 7 7800X3D, 30 GiB RAM, under headless gamescope at 1440 x 900 and scale 1.0.
Default features were used for the package; `--features demo` for the processes. Each run
had a 5 s warmup followed by 20 s sampled from `/proc/<pid>/status` at 1 s intervals; the
table shows the median of three runs. Both binaries were freshly copied first, so their
pages were equally cached. An earlier run against the just-linked branch binary showed
18 MiB more resident file pages from page-cache warmth. Heap and anonymous mappings were
identical in that run, so it is a measurement artifact.

| Metric | Baseline | Cosmetics | Delta |
| --- | ---: | ---: | ---: |
| Package executable | 87,314,600 B | 87,555,752 B | +241,152 B (+0.28%) |
| Installed tree (`xtask package --format dir`) | 91,848,634 B | 92,089,786 B | +241,152 B (+0.26%) |
| Installed tree, `tar` + `zstd -19` | 46,156,864 B | 46,280,238 B | +123,374 B (+0.27%) |
| `serein --demo` settled / peak RSS | 134 / 149 MiB | 134 / 149 MiB | 0 |
| `serein --demo` idle CPU (one core) | 0.25% | 0.25% | noise |
| `profile_preview --page=collectibles` settled / peak RSS | 94 / 110 MiB | 95 / 111 MiB | about +1 MiB |
| `profile_preview --page=collectibles` CPU | 0.25% | 0.25% | noise |

CPU did not change on the fixture page, which suggests the headless window was not playing
the effect (playback pauses while unfocused). Playback CPU was therefore not measured. Decoding two real Discord effect layers (450 x 880
APNG) through the worker's effect budget took 96–138 ms and produced 25–31 frames at
261 x 512. That is 13–16 MiB of frames per layer, against a 24 MiB layer budget and the
existing 128 MiB animation pool.

## GPU colour conversion for decoded video — October 11, 2026

Decoders now hand over native 4:2:0 planes (VideoToolbox NV12, GStreamer NV12/I420,
openh264 I420, Windows live NV12) and one render pass converts them on the GPU. Baseline
`178bde35` (main) and the changed tree were compared with Rust 1.98.1 on the same machines.
Native samples used the synthetic `--demo-video-playing` fixture replaced by a generated
clip for the run only, release builds with LTO off, 5–6 s warm-up, then 10 s of process CPU
time and peak RSS, alternating builds three times.

These measurements cover the parent project's native decoder path. Shared FFmpeg
decoding retains SDR RGBA or HDR P010 with explicit color metadata; that path is
outside these measurements.

| Workload | Before | After | Delta |
| --- | ---: | ---: | ---: |
| macOS M1 Pro, 1080p30 H.264 High + AAC, inline playback CPU | 38.9% | 32.5% | −16% |
| macOS same run, peak RSS | 203.9 MiB | 177.0 MiB | −26.9 MiB |
| Asahi Linux (M1, Vulkan, labwc), 720p30 H.264 Baseline via openh264dec, CPU | 97.0% | 60.4% | −38% |
| Asahi same run, peak RSS | 294.9 MiB | 269.7 MiB | −25.2 MiB |
| openh264 decode + handoff, 120 alternating 1080p/720p frames (release, median) | 473.2 ms | 208.1 ms | −56% |
| openh264 shared scratch capacity | 8,294,400 B | 3,110,400 B | −62% |
| macOS standard executable (`cargo xtask package`) | 68,706,864 B | 68,723,648 B | +16,784 B |
| macOS dist directory / zipped app | 74,814,816 / 48,280,440 B | 74,831,600 / 48,288,340 B | +16,784 / +7,900 B |

Rendered colour bars matched VideoToolbox's own BGRA conversion byte for byte on macOS and
were identical on Linux. The macOS inline gain is smaller because VideoToolbox already
converted in hardware; Windows live NV12 previously converted per pixel on the CPU, but
Windows CPU, GPU memory and frame latency were not measured. Software numbers come from the
ignored `compare_alternating_software_decode` workload. Live calls and streams were not used.

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


## Soundboard — historical upstream evidence, October 7, 2026

The following measurements were recorded by upstream #579, archived at `aa9c1dca`.
They do **not** measure this fork integration. The original synthetic screenshots remain
in the [immutable upstream evidence directory](https://github.com/ViceVerse-cz/Serein/tree/aa9c1dca6d24920ea7ef7d3a5c0836d54a43eabd/docs/pr-evidence).

Baseline `884fb73f` plus the one-line lockfile correction that lets it build on Windows,
compared with the soundboard playback, panel and server management changes. Windows 11 Pro
10.0.26200, AMD Ryzen 7 7800X3D, 32 GB RAM, Radeon RX 9070 XT, pinned Rust 1.98.1, locked
dependencies. Both revisions were built in separate target directories.

| Metric / method | Baseline | After | Delta |
| --- | --- | --- | --- |
| `serein.exe`, `cargo xtask package` | 85,350,400 B | 85,810,688 B | +460,288 B (+0.54%) |
| Installed `dist/` tree | 89,524,013 B | 89,984,421 B | +460,408 B (+0.51%) |
| `dist/` as one deflate-9 zip | 49,631,916 B | 49,767,265 B | +135,349 B (+0.27%) |
| Idle CPU, median of 3 | 1.25% of one core | 1.30% of one core | within run-to-run noise |
| Working set after sampling, median of 3 | 122.2 MB | 122.6 MB | within run-to-run noise |
| Private bytes after sampling, median of 3 | 152.2 MB | 148.3 MB | within run-to-run noise |

The process rows sample a release build with the `demo` feature running
`--demo --demo-voice` with no input: 15 seconds of warmup, then 30 one-second samples of
total processor time, working set and private bytes. Individual runs ranged over 1.15-1.41%
CPU and 147.9-156.1 MB private bytes across both builds, so no difference is claimed. The
zip is a local measurement of the package directory, not a published artifact.

Not measured: playback mixing cost in a live call, download and decode time of a real
sound, waveform rendering frame time and trim encoding time. None of those paths run in the
idle demo; the mixer and the Ogg Opus encoder have device-free tests but no timing
workload. Bounded additions: up to eight overlapping clips of at most six seconds in the
call mixer, a session cache of at most sixteen decoded clips (about 11.5 MB), and, only
while the upload review is open, up to 57.6 MB of decoded audio plus 30 KB of peaks.

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

## Voice participant controls (#592): Windows integration evidence - October 8, 2026

Fresh standard Windows x64 voice-enabled packages compare main `1b3e4a7b` with `d14edbb4` (measured 2026-10-08). Baseline/current file counts: 216/216.

| Metric | Main `1b3e4a7b` | Current integration | Delta |
| --- | ---: | ---: | ---: |
| Standard executable | 86,008,832 B | 86,089,216 B | +80,384 B (+0.0935%) |
| Installed directory | 90,199,948 B | 90,280,332 B | +80,384 B (+0.0891%) |
| Distribution ZIP | 50,001,798 B | 50,025,305 B | +23,507 B (+0.0470%) |

Method: `cargo xtask package`, Rust 1.98.1, standard release flags without demo; Windows 11 build 26200, Ryzen 7 7800X3D, 32 GiB RAM. Runtime workspace artifacts were invalidated before each feature build. Installed bytes sum every file in `dist`; ZIP uses whole-directory .NET Optimal compression. NSIS was unavailable, so no installer executable was built.

Alternating synthetic reducer replay (100,000 events): baseline median 76.5049 ms, range 74.0754-79.5180 ms; this change median 79.6071 ms, range 66.3717-83.4725 ms; median delta +3.1022 ms (+4.05%). One warmup each, then five baseline/controls/inbox rounds. All measured runs retained 500 records and 331,992-332,477 estimated timeline bytes (limit 4 MiB). The binaries were retained separately and workspace reducer artifacts rebuilt between variants. Concurrent compilation remained uncontrolled noise; overlapping ranges establish neither a performance improvement nor a regression.

Current native CPU, memory, frame/startup latency and affected-device behavior remain unmeasured because the native automation bridge is unavailable. Package size and synthetic reducer timing do not establish live Discord performance.

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

## Source-resolution regression checks — October 12, 2026

Baseline `574a8d2` and the source/aspect-ratio implementation use pinned Rust
1.98.1, the committed lockfile and separate release target directories. One warmup
per binary precedes five alternating measured runs of the 100,000-event synthetic
reducer. The release replay executables are byte-for-byte identical (SHA-256
`0d6060167876cc35dba30a368e3c36504420c869f42d8f3a4faa6d66c6491035`): screen
settings are outside this reducer workload.

| Metric / method | Baseline | Source implementation | Interpretation |
| --- | ---: | ---: | --- |
| Release replay median, five alternating pairs | 161.129 ms | 158.551 ms | Overlapping ranges; no performance claim |
| Retained timeline estimate / records | 331,992–332,477 B / 500 | 331,992–332,477 B / 500 | Unchanged reducer limits |

The 120-second offline lifecycle soak completes 1,326 passes, 42,432 channel
visits, 25,459,200 live inserts and six logout cycles in 120.072 seconds. It does
not measure media throughput, process/GPU memory or live Discord compatibility.

Source dimensions retain the existing 33,177,600-pixel and raw-byte envelope in
either orientation. Encoder bitrates interpolate between the existing preset
pixel-count/rate points and retain the 50 Mbit/s cap. Preset tests cover common
landscape, ultrawide, square, portrait and custom ratios; pending selection and
resize tests exercise synthetic RTP/DAVE transport.

Current native UI/display sampling is unavailable in this cloud session: no
native display session or window-capture tool. macOS now captures physical native
pixels before shared scaling, so default-preset capture CPU and memory need native
comparison; bounded worst-case allocation is unchanged. Current application and
clean/cached package size checks belong to the CI run for the delivered head.
Earlier package measurements above retain their historical revision labels.

## CodeRabbit audio review follow-up — October 12, 2026

Baseline `a65238d` and the review fixes use separately compiled native helpers
with `cc -O2`, the same private FFmpeg prefix and Linux host. Each helper feeds
5,000 stereo planar-float frames, 1,024 samples at 48 kHz, through the actual
attachment resampling boundary. One warmup precedes five alternating pairs.
These wall-clock samples include process startup and concurrent Rust build load;
their overlapping ranges do not establish a throughput regression or improvement.

| Metric / method | Baseline | Review fixes | Interpretation |
| --- | ---: | ---: | --- |
| Native helper median, five pairs | 11.364 ms | 12.396 ms | +1.031 ms; overlapping ranges |
| Native helper range | 9.067–19.189 ms | 11.642–13.683 ms | Startup/scheduler noise included |
| Native helper executable | 34,432 B | 34,816 B | +384 B; not an application/package measurement |

The workload checks stable-format PCM timing. Separate native regressions cover
stereo-to-mono, planar-to-packed, same-count channel-layout changes, rate changes,
invalid metadata, output guards and reset cleanup. Resampler configuration is
recreated only when input layout, sample format or rate changes. These helpers exclude demuxing,
decoding, audio devices, GUI and live Discord. Current native display sampling
is unavailable here; application and clean/cached package checks run in CI for
the delivered revision. Previous package measurements retain their original heads.

## Upstream media integration checks — October 13, 2026

The merge candidate combines video head `aa8a09a` with parent `61337f3` using
Rust 1.98.1 and the committed lockfile. Workspace formatting, strict Clippy,
tests, the minimal application build and policy pass. Exact 6144×2560 and PQ/HLG
RTP/DAVE regressions, native/HDR resource switching, the offscreen production
media shader and workspace/fuzz license checks pass. Native GPU/display and
Discord interoperability checks remain separate owner-controlled work.

The preserved `5b8ae38` release replay has unchanged reducer source through
`aa8a09a`; its compiler, profile and rustflags fingerprints match the merge
build. One warmup precedes five alternating pairs on the same Linux cloud host.
Scheduling noise remains; this synthetic reducer excludes media, UI and RSS.
The host is Linux 6.18.44/x86_64, Xeon Platinum 8573C, with a 16 GiB memory
limit. Offline delivery uses a Debian trixie container limited to four CPUs and
14 GiB; release settings retain fat LTO, one codegen unit and line-table debug
information. No native display session was available.

| Metric / method | Pre-sync replay | Merge candidate | Delta |
| --- | ---: | ---: | ---: |
| 100,000-event median, five pairs | 104.499 ms | 119.017 ms | +14.517 ms / +13.9% |
| Run range | 99.541–109.792 ms | 111.264–131.862 ms | Slower in all five pairs |
| Retained estimate, 500 messages | 331,992–332,477 B | 393,492–393,977 B | +61,500 B; not RSS |

The cause is not isolated by these measurements. The 120-second lifecycle soak
passes 1,923 cycles, 61,536 channel visits, 36,921,600 inserts and eight logout
cycles in 120.029 seconds, retaining bounded active/dormant histories. Navigation
and the 20,001-channel workload also pass.

Normal `cargo xtask package --format dir` builds the standard release without
demo/developer features and stages into an empty destination. Verification found
an omitted notification-sound provenance file; the allowlist was corrected and
the complete command rerun from an empty destination with caches. No payload was
manually repaired. The executable remained byte-identical across these runs.

| Linux package metric | Verified candidate | Method |
| --- | ---: | --- |
| Release executable | 87,856,520 B | Compiled, staged and installed hashes match |
| Installed payload | 132,622,544 B / 266 files | Sum of installed regular-file lengths |
| Compressed directory | 86,126,968 B | Python tarfile, gzip level 9 |

Executable SHA-256 is `eb703b348cdc54a9db0d616e340aa88eabe7309a80d81520c266faa17b712fd3`.
All six private libraries, corresponding-source files and notices match their
staging inputs; ELF runtime paths are relative and exclude the development FFmpeg
prefix. There is no matching `aa8a09a` package baseline, so no package delta is
claimed. Native distro package/runtime closure, Windows/macOS packages, display
transitions, playback throughput and settled process/GPU memory require CI or
native hardware; directory staging alone does not establish them.

## HDR capture/SPS review fixes — October 13, 2026

The `c47036c` baseline package was verified by executable hash before source
edits and preserved separately. The changed package measures the reviewed source
committed with this entry; the PR records its exact revision. Both use Rust
1.98.1, the committed lockfile and the same Linux 6.18.44/x86_64 cloud host
(Xeon Platinum 8573C, 16 GiB host limit), with a Debian trixie build container
limited to four CPUs and 14 GiB. Standard release includes voice and excludes
demo/developer features: fat LTO, one codegen unit, line-table debug information
and symbol stripping.

`cargo xtask package --format dir` starts with an empty output directory after
rebuilding xtask. Compiled, staged and installed executable hashes match; all
six private libraries, corresponding-source files and notices match their
staging inputs. ELF runtime paths are relative and omit the development FFmpeg
prefix. There is no manual staging repair. One package per source revision is
measured by file lengths; warmup is not applicable. Both compressed directories
use Python tarfile with gzip level 9.

| Linux package metric / method | Baseline `c47036c` | Review fixes | Delta |
| --- | ---: | ---: | ---: |
| Release executable | 87,856,520 B | 87,856,648 B | +128 B |
| Installed payload | 132,622,544 B | 132,622,672 B | +128 B |
| Compressed directory | 86,126,968 B | 86,126,955 B | -13 B |
| Installed regular files | 266 | 266 | +0 |

Changed executable SHA-256: `83e7f472230cab8fd369e83d42e9d69cc12fd6afaf453638428ecde381c9cac5`.

Workspace checking passes formatting, strict Clippy, 1,420 tests (31 ignored),
the minimal application build and policy. Login-handoff, cached-xtask workspace,
application/fuzz license checks and packaging fixtures pass. The actual Linux
capture pipeline retains P010/BT.2020/PQ through cropping and HDR→SDR→HDR
transitions. Its crop element requires planar 10-bit input, adding format
conversion work; no throughput or CPU improvement is claimed.

The fresh 120-second offline soak completes 1,488 passes, 47,616 channel visits,
28,569,600 inserts and six logout cycles in 120.043 seconds. This core workload
excludes capture, decoding, GUI and process/GPU memory. The historical +13.9%
reducer slowdown above retains its original revisions and unisolated cause;
it is not a fresh measurement of these media fixes.

Native display/window capture and suitable devices are unavailable here, so
media throughput, CPU and peak/settled process/GPU memory remain unmeasured.
Directory staging does not establish native application launch, full distro
runtime closure, physical HDR behavior or Discord interoperability. Current-head
Windows/macOS/Linux application/package CI and owner-controlled hardware checks
are reported separately in the PR. The retained settings screenshot pair is
historical evidence from `b1e3474`, predating the current control placement and
Source/8K additions.

## Current nightly comparison — October 13, 2026

This comparison supersedes the older baselines in the PR description. Its
baseline is the latest published upstream nightly,
[`v1.0.0-nightly.20261010.57`](https://github.com/ViceVerse-cz/Serein/releases/tag/v1.0.0-nightly.20261010.57),
source `cc0987e2d4505bdfb780045327bb14c3b0c0928e`; the changed source is
`4ad106c2348d5280a4cdf2ba840c2872aca14675`. These are matching local Linux
source rebuilds, using each revision's committed lockfile, Rust 1.98.1 and
standard voice-inclusive release without demo/developer features. Both profiles
use fat LTO, one codegen unit, line-table debug information and symbol stripping.
The Linux 6.18.44/x86_64 host is a Xeon Platinum 8573C with a 16 GiB limit;
the Debian trixie build/measurement container has four CPUs and a 14 GiB limit.
Separate worktrees and Cargo targets prevent reuse of workspace artifacts. The
existing PR package was hash-verified before reuse. These measurements compare
source rebuilds, not the published release's distribution archives.

| Metric / method | Nightly `cc0987e` | PR `4ad106c` | Delta |
| --- | ---: | ---: | ---: |
| Release executable | 87,185,832 B | 87,856,648 B | +670,816 B / +0.8% |
| Installed payload | 91,717,776 B | 132,622,672 B | +40,904,896 B / +44.6% |
| Compressed directory, gzip 9 | 53,094,741 B | 86,126,955 B | +33,032,214 B / +62.2% |
| Installed regular files | 228 | 266 | +38 |
| 100,000-event replay median, five pairs | 111.294 ms | 117.525 ms | +6.231 ms / +5.6% |
| Retained estimate, 500 messages | 331,992–332,477 B | 393,492–393,977 B | +61,500 B; not RSS |

Both packages use normal `cargo xtask package --format dir` from an empty
output directory. Compiled, staged and installed executable hashes match. File
lengths measure executable and installed payload sizes; Python tarfile with gzip
level 9 and the same archive basename measures both compressed directories.
There is one package per source revision and no warmup for file-size counters.
Most of the payload increase comes from bundled media libraries and their
corresponding-source archives. Nightly executable SHA-256:
`888ce8ddb9180be739f307b09c059aabd21923b0f39a5476ffb520de3e63d2c2`;
PR executable SHA-256:
`83e7f472230cab8fd369e83d42e9d69cc12fd6afaf453638428ecde381c9cac5`.

The release replay program is identical at both revisions. One warmup per
binary precedes five alternating pairs on the same host after compilation has
finished. Each sample processes 100,000 synthetic events and retains 500
messages. The nightly runs span 107.809–196.299 ms; PR runs span
111.828–147.282 ms. The PR is slower in four of five pairs, but the ranges
overlap and include scheduling noise. The measured median slowdown is not
attributed to a particular feature.

The comparison includes upstream changes made after the nightly, as well as
the retained video work; it does not isolate individual features. Replay
measures the core reducer, not media throughput, UI latency or process RSS.
Native media CPU/throughput, peak/settled process/GPU memory and physical HDR/
Discord interoperability remain unmeasured here. Older comparison sections
retain their original revisions and are historical evidence.

## Native Windows/macOS nightly comparison — October 13, 2026

Native CI source rebuilds compare the same published nightly
[`v1.0.0-nightly.20261010.57`](https://github.com/ViceVerse-cz/Serein/releases/tag/v1.0.0-nightly.20261010.57),
`cc0987e2d4505bdfb780045327bb14c3b0c0928e`, against PR source
`8a1ab4a2014c6660c86f8009407f396c36571480`. The latter differs from the
Linux measurement's `4ad106c` only in this performance document. These are
matching source rebuilds, not byte-identical copies of the published signed
release archives. Each OS uses its own hosted runner for both sides, with
separate clean checkouts and fresh Cargo targets, Rust 1.98.1 and each source's
committed lockfile. Standard release includes voice and excludes demo/developer
features; both profiles use fat LTO, one codegen unit, line-table debug
information and symbol stripping. No desktop, owner account, microphone or
live capture session is launched.

### Windows X64

Environment: Microsoft Windows Server 2025 Datacenter build 26100; AMD EPYC 7763 64-Core Processor; 16.0 GiB RAM, 4 logical CPUs. Runner image `win25-vs2026` / `20260925.250.1`.

| Metric / method | Nightly `cc0987e` | PR `8a1ab4a` | Delta |
| --- | ---: | ---: | ---: |
| Release executable | 86,203,392 B | 87,188,992 B | +985,600 B / +1.1% |
| Installed payload | 90,394,509 B | 137,153,849 B | +46,759,340 B / +51.7% |
| Compressed distribution ZIP | 50,912,221 B | 83,622,631 B | +32,710,410 B / +64.2% |
| Unsigned NSIS installer | 42,992,192 B | 73,600,334 B | +30,608,142 B / +71.2% |
| Installed regular files | 216 | 248 | +32 |
| 100,000-event replay median, five pairs | 105.887 ms | 117.777 ms | +11.890 ms / +11.2% |
| Retained estimate, 500 messages | 331,992–332,477 B | 393,492–393,977 B | +61,500 B; not RSS |

Replay ranges: nightly 104.890–107.109 ms; PR 116.095–119.153 ms. Ranges do not overlap in this sample; the cause of the measured difference is not isolated.

[Native comparison job](https://github.com/MCShotty/Serein/actions/runs/38408007242/job/115332603702) preserves the exact JSON results. Packaged executable SHA-256: nightly `feb9c6a7c4e2267d868dad376e30beb0f6628d4545775927c74d28cf1196f097`; PR `c99607af30ba96b2f2eb26fa28d66fe9818ff0d3bdca0a6aa47ba2eeefc7e3f8`.

Measured replay samples in pair order (ms): nightly `104.890, 105.887, 106.635, 105.681, 107.109`; PR `118.033, 117.411, 119.153, 116.095, 117.777`.

### macOS ARM64

Environment: macOS 26.6.2 build 25G83; Apple M1 (Virtual); 7.0 GiB RAM, 3 logical CPUs. Runner image `macos26` / `20260907.0351.1`.

| Metric / method | Nightly `cc0987e` | PR `8a1ab4a` | Delta |
| --- | ---: | ---: | ---: |
| Release executable | 68,638,560 B | 69,091,776 B | +453,216 B / +0.7% |
| Installed payload | 74,603,532 B | 95,585,421 B | +20,981,889 B / +28.1% |
| Compressed distribution ZIP | 48,052,367 B | 64,506,402 B | +16,454,035 B / +34.2% |
| Installed regular files | 223 | 248 | +25 |
| 100,000-event replay median, five pairs | 63.177 ms | 70.793 ms | +7.616 ms / +12.1% |
| Retained estimate, 500 messages | 331,992–332,477 B | 393,492–393,977 B | +61,500 B; not RSS |

Replay ranges: nightly 62.051–66.883 ms; PR 66.162–83.932 ms. Ranges overlap; scheduling noise prevents attributing the median difference to a feature.

[Native comparison job](https://github.com/MCShotty/Serein/actions/runs/38406359420/job/115327118696) preserves the exact JSON results. Packaged executable SHA-256: nightly `667a0fb5493ebdd349aa62cc74675658fcb29828dda639ce2d4df84e55268fb5`; PR `715c06bb6aff0df6307caf97fd565809dd4a7a73563f484753c5c501b4e858bd`.

Measured replay samples in pair order (ms): nightly `63.067, 64.794, 62.051, 66.883, 63.177`; PR `66.162, 67.295, 83.932, 82.592, 70.793`.

The [measurement helper](https://github.com/MCShotty/Serein/blob/6bad96ebcf66faf8c010a25b15368d98c6d181a9/scripts/measure-native-nightly.py) records the source revisions, build profile, environment, hashes and every replay sample.

Both sides run normal `cargo xtask package` after rebuilding xtask and start
with empty package output. Windows compiled/staged executable hashes match;
normal macOS packaging ad-hoc signs the staged copy, so its hash is recorded
separately and the bundle passes `codesign --verify --strict`. Windows ZIPs
use PowerShell/.NET Optimal compression; macOS ZIPs use
`ditto -c -k --sequesterRsrc`, matching each platform's release archive method.
Every archive passes a CRC check. One package per source is measured by file
lengths; size counters have no warmup. Windows installers are unsigned, and
macOS bundles are not Developer ID signed or notarized.

The identical release replay program receives one warmup per binary and five
alternating pairs after all compilation and compression finish. It measures
100,000 synthetic reducer events, not media throughput, UI latency or process
RSS. These comparisons include post-nightly upstream changes alongside the
video work and do not isolate individual features. Native encoding/playback
FPS, media CPU use, peak/settled process/GPU memory and physical HDR/Discord
interoperability remain unmeasured. Earlier Windows measurements above retain
their historical revision labels and are not reused as current results.

## Video encoding evidence for #567

The [compact evidence record](pr-evidence/video-encoding/README.md) retains
matched synthetic native captures, narrow/light camera and screen layouts, and
[unchanged historical samples](https://github.com/MCShotty/Serein/blob/918a4abe391a9e1619ba678af2b3f204f594cf6e/docs/pr-evidence/video-encoding/measurements.json).
Older helpers, logs and detailed build records remain in the
[immutable archive](https://github.com/MCShotty/Serein/tree/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence).
The archive commit identifies storage; each record retains its measured revision,
working-tree source hashes, environment, method and limitations. None of these
historical results validates the latest extracted PR head.

### Windows release package — October 8, 2026

Matched voice-inclusive releases compare `1b3e4a7b` with `eefa0fe5`, using Rust
1.98.1 on Windows 11 build 26200, Ryzen 7 7800X3D and 32 GiB RAM, without demo.
Installed size sums every file in `dist`; ZIPs use .NET's Optimal compression.

| Metric | Baseline | FFmpeg integration | Delta |
| --- | ---: | ---: | ---: |
| Release executable | 86,008,832 B | 86,554,112 B | +545,280 B |
| Installed package | 90,199,948 B | 124,070,028 B | +33,870,080 B |
| Complete ZIP | 50,001,798 B | 78,054,030 B | +28,052,232 B |

The historical package reused a stale xtask that omitted FFmpeg staging. Its
output was repaired by replaying `packaging/ffmpeg/bundle.py`; all 241
non-executable files then matched their expected staging sources, including
DLLs, corresponding source, recipes and notices. The complete archive passed
integrity checks. **No clean end-to-end package rerun is claimed.** These are
portable directory/ZIP sizes, not NSIS installer sizes. The
[original method](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/performance.md#windows-ffmpeg-integration-package---october-8-2026)
retains the exact executable identity and checks.

### Linux synthetic component and UI results — October 5–9, 2026

These are separate additions and workloads, not one whole-PR comparison.
Original record names in the consolidated JSON identify the source of each row.

| Historical workload | Baseline | After | Interpretation |
| --- | ---: | ---: | --- |
| Software FFmpeg camera, 300 frames | 993.070 ms | 1,024.365 ms | Five measured alternating pairs; overlapping ranges |
| Driver detection, fresh processes | 262.167 ms | 265.354 ms | Five measured pairs; no GPU drivers available |
| 8K controls, sampled peak/settled helper RSS | 162,140,160 B | 163,909,632 B | One helper pair, +1,769,472 B; no encoding/capture |
| Split-option setup, 60,000 contexts | 1,301.148 ms | 1,371.227 ms | Five measured pairs; codecs/drivers never opened |

The camera fixture warms 30 frames before timing 300 deterministic 640×480 RGB
frames; one process warmup precedes five measured pairs. Driver detection uses
one warmup and five alternating processes against real detector/native code;
all nine NVENC/AMF/QSV codec paths are unavailable on the recorded host. The 8K
helper warms six seconds, receives 100 wheel events and records fifteen
one-second CPU/RSS samples. Split setup uses matched optimized diagnostic
FFmpeg builds that differ from the standard distribution recipe. Full OS,
CPU/RAM, renderer, compiler, source identities, commands, samples and noise
limits are retained in the linked records and stage methods.

These workloads do not establish general speed/quality improvements. Helper
sizes are not package sizes; sampled RSS is not a memory ceiling. Physical
8K/60 fps, frame/startup p95, GPU memory, active hardware throughput and fresh
whole-app/package comparisons remain unmeasured in these historical records.
Current implementation checks are recorded below separately from these measurements.

### Shared ultrawide/HDR media — October 11, 2026

The baseline is `c4bc650`, with its unchanged normal FFmpeg recipe in a detached
worktree. The new shared-media recipe is built with NVENC, AMF and QSV enabled,
H.264/HEVC/AV1 decoding and dav1d 1.5.1. Each revision uses its own Cargo release
directory, pinned Rust 1.98.1 and the standard voice-inclusive, no-development-data
package command. No historical executable or manually repaired staging tree is
used for this comparison.

`cargo xtask check` passes in an offline Linux container using the build host's
native libraries, CA certificates and a private temporary directory. The container
avoids the managed sandbox's nonstandard root-directory ownership, which the
unchanged IPC permission test correctly rejects. Formatting, strict Clippy,
workspace tests and policy checks pass. Login-handoff and the six license-policy
fixtures pass. Locked application/fuzz license graphs, native query/preset/packet
fixtures, source-patch reproduction and packaging fixtures were checked separately.

The actual media shader passes offscreen software-Vulkan tests for SDR, PQ/HLG
highlights, rotation, odd-crop chroma, resource cleanup and device recreation.
The exact 6144×2560 picture is decoded through synthetic RTP/DAVE at 60 fps
timestamp cadence. Four HEVC/AV1 PQ/HLG transport fixtures and HDR attachment
audio/seek/cancellation fixtures pass. These establish picture/color/lifecycle
behavior, not sustained 60 fps throughput or live Discord compatibility.

Fresh measurements compare `c4bc650` with `53cd786` on Linux x86-64, Intel Xeon
Platinum 8573C, with a four-CPU quota and 16 GiB memory limit. Each uses its own
release target directory. The baseline normal `cargo xtask package --format dir`
completed from empty output without a staging repair: executable 87,744,200 bytes,
installed tree 125,169,707 bytes, and canonical sorted `gzip -n -6` distribution
82,836,259 bytes. Both include voice and corresponding-source/license material.

| Synthetic release replay | Baseline | Shared media | Method |
| --- | ---: | ---: | --- |
| 100,000-event median | 97.741 ms | 96.144 ms | One warmup, five alternating pairs after builds settled |
| Observed elapsed range | 80.842–113.114 ms | 88.715–106.501 ms | Overlapping samples; no improvement claimed |
| Retained timeline | 331,992–332,477 bytes / 500 records | Same | Reducer accounting, not RSS |

The replay executables have identical `.text` hashes; earlier samples taken
during compilation were noisy and are excluded from this comparison. The new
120-second lifecycle soak completes 2,145 passes / 68,640 channel visits and
41,184,000 inserts within the shipping row/byte limits, with nine logout cycles.
It uses no network, storage, renderer or audio devices and does not measure media
throughput or process memory.

Clean and cached current-package measurements and native CI results are reported
in [#567's Performance and Verification sections](https://github.com/ViceVerse-cz/Serein/pull/567).
CPU, frame-delivery rate and peak/settled memory during physical HDR playback remain
unmeasured here. Native window capture and a suitable HDR display are unavailable
in this environment. The owner has agreed to test the current CI Windows installer
with the 6144×2560 stream and HDR/SDR display transitions. Outgoing HDR captures
currently tone-map to SDR until receiver 10-bit compatibility is established.

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

## Post rename review repair (2026-10-14, PR #655)

Fresh standard voice-inclusive packages compare pre-repair `74ff7038` with
runtime repair `460d7117`; both include the same main integration (`c84b5998`).
Environment: Windows 11 x64 build 26200, Ryzen 7 7800X3D, 32 GB RAM, pinned
Rust 1.98.1. `cargo xtask package` builds without default, demo or developer
features. NSIS is unavailable, so these are unsigned directory packages and
ZIP archives, not installer validation.

| Metric / method | Baseline | Repair | Delta |
| --- | ---: | ---: | ---: |
| serein.exe file length | 87,251,456 B | 87,251,456 B | 0 B |
| Installed payload, 216 files | 91,444,745 B | 91,444,745 B | 0 B |
| ZIP, sorted paths / DEFLATE level 9 | 50,287,594 B | 50,287,599 B | +5 B |
| 100,000-event reducer replay, median of five | 68.47 ms | 66.50 ms | -1.98 ms / -2.89% |

Each replay executable was built once and verified through `cargo replay`,
then run directly with one warmup and five measured samples. Both retain
500 timeline records / 393,492..393,977 estimated bytes. The timing ranges
overlap (67.82..73.78 ms before, 64.52..70.47 ms after); this is not evidence
of a speed improvement. The workload measures the general synthetic reducer,
not rename-specific latency, process RSS, UI frame timing or live compatibility.

The repair retains existing bounded state and adds no dependency or cache.
Regressions cover revoked access, rename echoes with invalidated mutable details,
a fresh post-details reload and a later conflicting name. Full local
`cargo xtask check` and separate fuzz formatting passed. Native GUI capture
and owner-controlled live validation remain unavailable/unverified.
Raw samples, source revisions and executable hashes are in
[measurements.json](pr-evidence/post-rename-review/measurements.json).

## URL markup parsing (2026-10-14, issue #656)

[Measurements and source hashes](pr-evidence/url-underscores/measurements.json)
compare `fbcbbad3` with the URL protection change on Windows 11 x64 build 26200,
Ryzen 7 7800X3D, 32 GB RAM and pinned Rust 1.98.1. Standard packages include voice
and use `cargo xtask package`, without demo or developer features. Performance
notes and development evidence are not included in the installed payload.

| Metric / method | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 87,366,656 B | 87,381,504 B | +14,848 B / +0.0170% |
| Installed payload, 216 files | 91,559,945 B | 91,574,793 B | +14,848 B / +0.0162% |
| ZIP, Optimal compression | 50,470,238 B | 50,479,398 B | +9,160 B / +0.0181% |
| Plain message, median / 10,000 parses | 25.1799 ms | 26.1144 ms | +0.9345 ms / +3.71% |
| Underscore URLs, median / 10,000 parses | 28.7708 ms | 41.5100 ms | +12.7392 ms / +44.28% |

The plain-message sample ranges overlap; no speed improvement is claimed.
Protecting underscore URLs adds about 1.27 microseconds per parse in this workload.

The release component workload is `cargo test --release --locked -p ui --lib
url_underscore_parse_benchmark -- --ignored --nocapture`. Each built test binary
is then run directly, with one warmup and five samples of 10,000 parses for each
synthetic message. This measures parsing, not native frame latency, idle CPU,
RSS or live Discord behavior. Native desktop automation is unavailable here.

These numbers were measured on the underscore-only revision `e6f92b0f`. Protection
was later generalized to `*`, `~` and `&` entities, plus a trailing-opener check of
at most four passes. That revision was not re-measured.

Protection adds temporary passes only for candidate URLs containing `_`, `*`, `~`
or `&`; other messages use an early return. Source remains capped at 8 KiB, each
pass at 512 events (exhaustion falls back to bounded literal text), and stored
links at 16. No dependency or retained cache is added.

## PR #620 upstream conflict resolution — October 18, 2026

Runtime merge `3d1acc7968e228c64fbe683b0e13aa9cd0804502` combines the previous
PR head `4806c9bb4a0f2646bf3e43e048e08796843705b8` with parent main
`d9d87471f0ea2190da3f90db815e115045461703`. The evidence follow-up changes only
this document. Account cleanup retains upstream's opt-in saved-login storage
and pending-save protection. Upload-preview ownership, GIF hover fixtures and
cached-package font-license fixtures are reconciled with upstream.

Pinned Rust 1.98.1 `cargo xtask check` passes: **1,537 passed, 29 ignored**, with
formatting, strict workspace/all-target Clippy, minimal production configuration
and policy checks. Final strict Clippy, login-handoff, cached-xtask/workspace/package,
Linux/Flatpak packaging, repository-setup and mocked macOS signing fixtures pass.
Both locked license graphs and six license-policy fixtures pass. Bounded ASan
fuzzing completes 98,662 decode and 7,086 state runs (31 seconds per target,
512 MiB ceiling), with no finding. Release replay and the 120-second lifecycle
soak pass; the soak completes 1,866 passes and eight logout cycles. Replay ran
alongside compilation, so its timing is not a comparable performance sample.
These checks use synthetic/offline input and do not establish live compatibility.

Standard voice-enabled Linux directory packaging uses Debian 13 x86-64, the
pinned toolchain/lockfile, normal release fat LTO, two Cargo build jobs and a
task-owned native SDK. The complete `cargo xtask package --format dir` command
passes with an empty output directory, then passes again after preserving that
output and starting with another empty directory. Dependency caches are reused;
this is not a cold-cache measurement. No staging repair is performed.

| Footprint / repeatability | First output | Cached repeat | Delta |
| --- | ---: | ---: | ---: |
| Executable | 92,193,224 B | 92,193,224 B | 0 B |
| Installed payload (242 files) | 96,791,057 B | 96,791,057 B | 0 B |
| Normalized ZIP | 55,763,863 B | 55,763,863 B | 0 B |

The payload matches the current Cargo executable and allowlisted installation
manifest, modes and notices, including all 19 font licenses. ELF dependency
closure resolves with the task SDK; no development RPATH is embedded. The
application is not launched. The comparison ZIP is generated from each verified
directory payload using sorted paths, fixed timestamps, preserved 0755/0644
modes and DEFLATE level 9. This checks repeatability, not a PR performance delta.
Executable SHA-256: `2559f36f1c19738521712aa48ff2dbd123e3f8f0abce1bb6a8323e6249b88f6e`.
Normalized ZIP SHA-256: `21067951174a549de564eb5a5965a5ccc8da4c6fd67963b00492e7f3a54c56be`.
Both payload manifests and both ZIP hashes are identical.

Historical component evidence keeps its original revisions: the
[theme-helper fixture](https://github.com/MCShotty/Serein/blob/4e01c7c9526d17da292485e108abf18621dd5ae2/docs/pr-evidence/theme-command-timeout/README.md)
records a 1,013.038 → 60.032 ms median with one warmup and five samples.
The avatar reservation fixture compares exact `reserve_attempt` implementations
at `422e7ad` and `463abd7`: Rust 1.98.1 `-O`, offline Linux x86-64 (4 vCPUs,
12 GiB), one warmup and five samples of 100,000 calls with 2,047 fresh failed
entries; median 1,312.063 → 0.149 ms. Neither fixture measures whole-app speed.
The October 15 package SIGKILL remains a historical failed attempt; this pass
does not relabel that result.

No fresh matched runtime baseline, UI latency, CPU/RSS, physical-device, Windows
or macOS package measurement is claimed. Native display and live Discord checks
remain unverified. Parent native/security/license/fuzz CI jobs exclude draft PRs;
#620 retains its draft status at the owner's request. Mocked signing and local
Linux SDK closure do not establish native macOS or global Linux installation.


## Application-command follow-up — October 17, 2026

Based on fork `f9e39f7`. Focused command schema, discovery, API loopback, autocomplete
correlation and composer regressions pass. Separate replay and 120-second soak runs
passed (1,024 passes). The bounded ASan decoder/state smoke passed; dependencies and
lockfiles are unchanged. The final full-form stale-suggestion guard has focused Core
and UI coverage. Full workspace/native demo checks are recorded in the PR.

`scripts/compare-replay.py` compares the reviewed commit, the current published upstream
nightly and the previous fork on independent Windows/macOS/Linux CI workers. It uses
matching Rust 1.98.1 release overrides, separate worktrees/targets, one warmup and five
rotated samples per workload. Artifacts retain revisions, environment and sample ranges.
These synthetic reducer results are not UI latency or media throughput measurements.
Current-head executable, installed-payload and compressed-package comparisons await
native CI; UI CPU/RSS and live command performance remain unmeasured.


### Command review and native evidence at `564d8dd1` plus the field-style correction

The complete local `cargo xtask check` passed 1,676 tests (32 ignored), policy
checks and the minimal application build at `564d8dd1`. The later autocomplete
field correction only matches the existing proportional 14-point scalar-field
font, alignment and text color. Its existing composer regression, strict native
UI all-target/demo Clippy, formatting and native `profile_preview` rebuild pass.

The native before/after pair in `pr-evidence/application-commands` uses the same
1120 × 760, scale-1 dark X11 window and synthetic `/weather` form with `Pra`.
Before is fork `f9e39f7`; after is `564d8dd1` plus that three-line style fix.
The before build has two demo-only compilation repairs: remove duplicated avatar
helpers and supply the missing optional error argument to `pending::show`.
Neither changes the displayed feature. After opens the new synthetic suggestions
popup; this is feature-absence evidence, not live service data. Native light-mode
layout at the supported minimum 760 × 520 was also inspected; those extra captures
are not bundled. Capture uses explicit `--demo --interactive`, Xvfb, software
rendering and ImageMagick's application-window capture, with process-liveness checks.
These debug preview pictures do not establish release CPU/RSS or frame latency.

A submitted command's later HTTP failure still requires form re-entry. Only local
validation failures preserve that form; this limitation remains in the inventory.


### Cross-platform command replay at `564d8dd1`

[Run 39056323878](https://github.com/MCShotty/Serein/actions/runs/39056323878)
compares nightly `v1.0.0-nightly.20261014.58` (`3a4bf8d`), fork `f9e39f7`
and command code `564d8dd1`. The later field-style correction does not change
this core workload. Five rotated measured runs follow one warmup per build;
Rust 1.98.1, opt-level 3, thin LTO, one codegen unit and panic=abort match.
Linux/Windows pin one allowed logical CPU; macOS uses its platform scheduler.
Raw CI JSON records each worker's OS/CPU, full revisions, settings and ranges.
Values are milliseconds; each entry is median (minimum–maximum).

| Platform / workload | Nightly | Previous fork | Commands | Delta vs nightly / fork |
| --- | ---: | ---: | ---: | ---: |
| Linux-X64 / reducer | 80.517 (79.993–80.916) | 79.284 (78.504–80.101) | 79.093 (79.018–82.000) | -1.8% / -0.2% |
| Linux-X64 / wide channels | 226.205 (221.134–229.776) | 230.142 (224.395–234.305) | 226.427 (221.166–231.768) | +0.1% / -1.6% |
| Windows-X64 / reducer | 78.035 (77.959–101.676) | 79.240 (79.080–106.378) | 78.814 (78.539–79.393) | +1.0% / -0.5% |
| Windows-X64 / wide channels | 48.810 (48.105–49.212) | 69.543 (68.001–74.627) | 68.152 (67.905–68.479) | +39.6% / -2.0% |
| macOS-ARM64 / reducer | 55.292 (55.175–55.410) | 55.624 (55.409–56.302) | 55.943 (55.716–56.636) | +1.2% / +0.6% |
| macOS-ARM64 / wide channels | 83.693 (83.187–84.001) | 83.590 (83.116–84.013) | 82.957 (82.794–83.901) | -0.9% / -0.8% |

The Windows wide-channel result is 39.6% slower than nightly; the previous fork
is already slower. Its cause is unisolated. These are core reducer measurements,
not autocomplete, UI latency, media throughput or a causal improvement claim.


### Native package snapshot at `564d8dd1`

## Moderation and AutoMod — October 17, 2026

Based on fork `f9e39f7`. Focused permission/hierarchy, sparse rule preservation, bounded
parsing, loopback HTTP and native UI-state regressions pass, including whitespace
preservation on unrelated rule edits. Replay and the 120-second soak passed (1,531
passes). Login-handoff, cached-xtask workspace and license-policy fixtures pass; the
locked graph is unchanged. After sequencing builds, bounded ASan decoder/state smoke
passed at `91c95fdd` (112,105 and 3,638 runs, 31 seconds per target). The earlier compiler
OOM did not produce a fuzz result. Native application checks are recorded separately in the PR.

`scripts/compare-replay.py` compares the reviewed commit, the current published upstream
nightly and the previous fork on independent Windows/macOS/Linux CI workers. Matching
Rust 1.98.1 release overrides, separate worktrees/targets, one warmup and five rotated
samples produce revision-labelled JSON artifacts including sample ranges. Native
packages must separately establish executable, installed-payload and distribution
sizes. Native UI CPU/RSS, moderation latency and live service behavior remain unmeasured.


### Moderation release comparison at `91c95fdd`

[Native CI run 39056325676](https://github.com/MCShotty/Serein/actions/runs/39056325676)
compares published nightly `v1.0.0-nightly.20261014.58` (`3a4bf8d`), previous fork
`f9e39f7` and moderation code `91c95fdd`. Rust 1.98.1, opt-level 3, thin LTO, one
codegen unit, stripped symbols and panic=abort match across each worker's builds.
One warmup precedes five rotated samples of each bounded workload. Linux/Windows
pin one allowed logical CPU; macOS uses its platform scheduler. The run's per-OS JSON
artifacts include the exact OS/CPU, full revisions, settings and raw samples. These
are separate GitHub-hosted x64 Linux/Windows and ARM64 macOS workers; elapsed values
are milliseconds, with minimum–maximum ranges below.

| Platform / workload | Nightly median (range) | Previous fork | Moderation | Delta vs nightly / fork |
| --- | ---: | ---: | ---: | ---: |
| Linux-X64 / reducer | 48.866 (47.986–49.167) | 47.722 (47.084–48.275) | 47.769 (47.428–48.763) | -2.2% / +0.1% |
| Linux-X64 / wide channels | 61.633 (60.952–68.746) | 61.370 (60.250–66.805) | 62.140 (60.508–63.802) | +0.8% / +1.3% |
| Windows-X64 / reducer | 97.703 (95.286–130.104) | 97.112 (93.646–113.227) | 96.023 (94.101–144.464) | -1.7% / -1.1% |
| Windows-X64 / wide channels | 94.748 (81.093–118.035) | 116.428 (107.706–146.935) | 112.896 (109.315–161.461) | +19.2% / -3.0% |
| macOS-ARM64 / reducer | 79.794 (73.478–90.337) | 80.099 (61.130–91.886) | 80.526 (70.608–92.206) | +0.9% / +0.5% |
| macOS-ARM64 / wide channels | 137.806 (112.357–141.365) | 125.894 (113.885–155.284) | 130.810 (107.787–215.842) | -5.1% / +3.9% |

The Windows wide-channel result is slower than nightly; its cause is unisolated.
The workloads exercise core state, not moderation latency, native UI or media throughput.
No causal performance improvement or official-client equivalence is claimed.


### Native package snapshot at `91c95fdd`

Standard voice-inclusive native CI packages, compared with published nightly
`3a4bf8d` and previous fork `f9e39f7`. Installed size sums regular-file bytes;
distribution normalizes those files with Python 3.12 ZIP DEFLATE level 6 and
fixed metadata, excluding outer CI archives and installers. One artifact per
revision/platform; MiB = 1,048,576 bytes. Other native platforms remain pending.

| Platform / metric | Nightly | Previous fork | This code | Delta vs fork |
| --- | ---: | ---: | ---: | ---: |
| Linux-X64 / executable | 83.682 | 87.900 | 88.037 | +0.137 (+0.16%) |
| Linux-X64 / installed payload | 88.006 | 130.657 | 130.795 | +0.137 (+0.11%) |
| Linux-X64 / normalized distribution | 51.211 | 84.553 | 84.614 | +0.061 (+0.07%) |

## FFmpeg media playback — October 17, 2026

The audio/preview change starts from fork main `f9e39f7`. A native component
comparison uses that revision's Cargo-built libopus static library versus the
new `serein_audio_open/decode/close` wrapper and the existing pinned FFmpeg build.
Both consume the same 151 packets from the original three-second mono Opus
fixture at 48 kHz, recreating the decoder 200 times per sample. The temporary C
harness uses GCC `-O3 -DNDEBUG`; the shared Linux worker has a Xeon Platinum
8573C, five available CPUs and a 16 GiB memory limit. One warmup precedes five
alternating measured runs per backend; elapsed time uses `CLOCK_MONOTONIC`.
Other workspace compilation was running, so these are indicative component
measurements, not isolated native application benchmarks.

| Metric / method | `f9e39f7` libopus path | FFmpeg packet path | Delta |
| --- | ---: | ---: | ---: |
| Median elapsed per 200 full decodes | 1.5084 s | 0.8771 s | −0.6313 s (−41.9%) |
| Decoded mono samples, before demuxer trim | 28,992,000 | 28,992,000 | 0 |
| Release executable / installed payload / compressed package | Unmeasured | Unmeasured | Unavailable |

The component comparison excludes demuxing, Rust allocation, HTTP, output devices,
GUI rendering and the playback queue. Separate production-path regressions compare
trimmed duration, channel count and RMS level for WAV, MP3, Opus and Vorbis, and
check cancellation, SDR/HDR posters, rotation and bounded embedded-clip playback.
A full-clip regression also covers FFmpeg growing its input buffer: larger native
requests receive bounded 64 KiB reads instead of failing partway through playback.
No application CPU/RSS, startup, video throughput or physical display performance
claim follows from these results.
The shared audio wrapper also completed 200 decoder lifecycles with AddressSanitizer
and UndefinedBehaviorSanitizer; linked codec libraries were not instrumented.

Full local release packaging was not repeated: earlier production fat-LTO links
exceeded this worker's memory limit, and remaining disk space cannot support a
fresh matching package pair. Current native packages remain CI checks. No new
runtime library, codec recipe or license dependency is introduced by this change.


## Soundboard and playback integration — October 17, 2026

This feature builds on fork media PR #7 (`f3abb03`), not a merged fork dependency.
Focused soundboard model/protocol, request/call correlation and API loopback tests pass,
including a dropped POST response that is not retried. Separate synthetic replay and
120-second soak runs passed (1,168 passes), before the final read-only spinner guard.
Login-handoff, cached-xtask and license-policy fixtures pass; the dependency graph and
FFmpeg recipe are unchanged. Full native/UI/decoder checks and ASan results are
reported separately in the PR; earlier upstream measurements are historical evidence. Bounded ASan
decoder/state smoke subsequently passed (93,615 and 2,469 runs, 31 seconds each) on the
reviewed media implementation. The subsequent Windows-only test-argument correction
changes no fuzzed parser/reducer code.

The branch's release comparison jobs use the current published upstream nightly,
previous fork and reviewed commit on Windows/macOS/Linux. Matching Rust 1.98.1
release overrides, separate targets, one warmup and five rotated samples are recorded
in JSON with exact revisions, environment and sample ranges. Core reducer timing does
not measure audio stretching, media decode, native UI latency or CPU/RSS. Those media
measurements remain unavailable here. Current-head standard native packages provide
the executable, installed payload and compressed distribution comparisons.


### Current media release comparison at `a94983ab`

[Run 39058677973](https://github.com/MCShotty/Serein/actions/runs/39058677973)
compares published nightly `v1.0.0-nightly.20261014.58` (`3a4bf8d`), previous
fork `f9e39f7` and media code `a94983ab` on separate Windows x64, Linux x64 and
macOS ARM64 workers. Rust 1.98.1, opt-level 3, thin LTO, one codegen unit and
panic=abort match; one warmup precedes five rotated measured runs. Linux/Windows
pin one allowed CPU, while macOS uses the platform scheduler. Raw CI JSON records
OS/CPU, exact revisions, settings and ranges. Values are milliseconds, median
(minimum–maximum). This core-only workload does not exercise audio decoding,
stretching, soundboard mixing or GUI rendering.

| Platform / workload | Nightly | Previous fork | Media | Delta vs nightly / fork |
| --- | ---: | ---: | ---: | ---: |
| Linux-X64 / reducer | 63.295 (61.608–64.248) | 63.491 (61.648–64.701) | 64.237 (60.519–65.560) | +1.5% / +1.2% |
| Linux-X64 / wide channels | 112.988 (94.424–117.141) | 119.903 (116.312–121.558) | 121.310 (116.398–122.577) | +7.4% / +1.2% |
| Windows-X64 / reducer | 58.066 (57.603–60.780) | 58.144 (57.711–60.494) | 56.240 (55.600–57.943) | -3.1% / -3.3% |
| Windows-X64 / wide channels | 69.075 (64.426–84.789) | 64.319 (62.407–66.892) | 64.566 (63.838–68.376) | -6.5% / +0.4% |
| macOS-ARM64 / reducer | 67.099 (66.357–70.469) | 68.648 (66.939–70.439) | 68.725 (67.667–70.565) | +2.4% / +0.1% |
| macOS-ARM64 / wide channels | 100.793 (97.166–103.192) | 105.346 (103.715–108.621) | 100.973 (99.229–106.693) | +0.2% / -4.2% |

Linux wide-channel replay is 7.4% slower than nightly and 1.2% slower than the
previous fork; the cause is unisolated. No causal speed improvement is claimed.
The inherited FFmpeg-first decoder changes in prerequisite #7 are included here;
these comparisons do not isolate soundboard cost. Standard native package sizes
are recorded below; release media throughput, idle CPU, RSS and frame latency remain
unmeasured. This is not proof of official-client interoperability.


### Current media focused native checks

At `a94983ab`, strict native `ui`/`serein` all-target/demo Clippy and
formatting pass. Focused native checks pass four sound-editor UI cases, eight
audio/UI cases, twelve sound/worker/upload cases, one Ogg Opus round-trip and
two pitch-preserving speed cases. They cover bounded clips, stale-call worker
cancellation, deafen, released cached PCM, trim encoding, damaged audio, keyboard
controls, narrow cards and preserved unknown/custom emoji on unrelated edits.
These are offline fixtures, not a microphone or live-call test.

Native CI run 39058677973 passes all four application/package gates (Linux x64,
Windows x64/ARM64 and macOS ARM64), all three release replay comparisons,
x64/ARM64 installer fixtures, SDK, licenses and ASan fuzz.
Inherited security and separate Nix failures remain visible in the PR. The
Windows-only missing optional-effects test argument was corrected at `a94983ab`.

### Linux media package snapshot at `a94983ab`

Standard release including voice, compared with nightly `3a4bf8d` and previous
fork `f9e39f7`. Installed values sum regular-file bytes; distribution recompresses
the payload using Python 3.12 ZIP DEFLATE level 6 and fixed metadata, excluding
the outer CI archive/installers. One artifact per revision; MiB = 1,048,576 bytes.

| Metric | Nightly | Previous fork | Media | Delta vs fork |
| --- | ---: | ---: | ---: | ---: |
| executable | 83.682 | 87.900 | 88.282 | +0.382 (+0.43%) |
| installed payload | 88.006 | 130.657 | 131.039 | +0.382 (+0.29%) |
| normalized distribution | 51.211 | 84.553 | 84.700 | +0.147 (+0.17%) |

This includes prerequisite #7 and does not isolate the soundboard contribution. Windows/macOS package snapshots follow below.


### Current media native screenshots

The matched native pair in `pr-evidence/desktop-media` uses the synthetic server
settings fixture at 1120 × 760, scale 1, dark mode. Before is fork `f9e39f7`
with only the two demo-build repairs (duplicate avatar helpers and the missing
`pending::show` optional-error argument). Prerequisite #7 has no changes to these
UI/settings/preview paths, verified against `f3abb039`; the baseline represents
feature absence. After is media code `a94983ab`, not an upstream screenshot.
The supported minimum 760 × 520 light-mode soundboard layout was inspected too.

The native preview build passes. Captures launch explicitly with
`--demo --interactive` on Xvfb/software rendering, check process liveness and
capture only the app window with ImageMagick. No account, microphone, speaker
playback or service was used. Only one small current pair is committed. Debug
preview captures are not release CPU/RSS, frame latency or live acceptance evidence.


### Windows and macOS media package snapshots at `a94983ab`

Same standard voice-inclusive package and normalization method as Linux above.
One artifact per revision/platform; MiB = 1,048,576 bytes. Baselines are nightly
`3a4bf8d` and previous fork `f9e39f7`. CI checkout `d03ae140` has the exact Git
tree of source `a94983ab`, verified before these measurements were recorded.

| Platform / metric | Nightly | Previous fork | Media | Delta vs fork |
| --- | ---: | ---: | ---: | ---: |
| Windows-X64 / executable | 82.910 | 87.505 | 87.982 | +0.477 (+0.54%) |
| Windows-X64 / installed payload | 86.909 | 135.216 | 135.693 | +0.477 (+0.35%) |
| Windows-X64 / normalized distribution | 47.986 | 80.844 | 81.015 | +0.170 (+0.21%) |
| macOS-ARM64 / executable | 65.946 | 69.884 | 70.198 | +0.314 (+0.45%) |
| macOS-ARM64 / installed payload | 71.641 | 95.216 | 95.530 | +0.314 (+0.33%) |
| macOS-ARM64 / normalized distribution | 46.027 | 63.548 | 63.677 | +0.129 (+0.20%) |

Windows x64/ARM64 both pass clean and cached normal packaging from empty job-owned
output directories, archive validation and a bounded packaged driver-query launch
without development FFmpeg paths. No manual staging repair was used. The x64 ZIPs
are both 86,052,573 bytes with metadata-dependent hashes; bit-for-bit reproducibility
is not claimed. The query helper exits before account/UI/audio initialization,
so these checks do not establish an interactive launch or hardware encoding.
macOS nightly is signed and the CI bundle is unsigned. These are observed package
deltas, not isolated soundboard cost or release media/UI performance claims.

revision/platform; MiB = 1,048,576 bytes. The Windows x64 snapshot below completes this three-platform comparison.

| Platform / metric | Nightly | Previous fork | This code | Delta vs fork |
| --- | ---: | ---: | ---: | ---: |
| Linux-X64 / executable | 83.682 | 87.900 | 88.186 | +0.286 (+0.33%) |
| Linux-X64 / installed payload | 88.006 | 130.657 | 130.944 | +0.287 (+0.22%) |
| Linux-X64 / normalized distribution | 51.211 | 84.553 | 84.678 | +0.124 (+0.15%) |
| macOS-ARM64 / executable | 65.946 | 69.884 | 70.135 | +0.251 (+0.36%) |
| macOS-ARM64 / installed payload | 71.641 | 95.216 | 95.467 | +0.251 (+0.26%) |
| macOS-ARM64 / normalized distribution | 46.027 | 63.548 | 63.659 | +0.111 (+0.17%) |

macOS nightly is signed while CI is unsigned; these are observed package deltas, not an isolated feature cost.


### Current moderation native evidence

Code `91c95fdd` passes the local strict native `ui`/`serein` all-target/demo
Clippy run, three AutoMod UI regressions and the moderation cancellation UI
regression. The native preview rebuild also passes. These checks follow the
read-only ban-view permission correction and whitespace-preservation fix.
Native CI run 39056325676 passes the full application/package gate on Linux x64,
Windows x64/ARM64 and macOS ARM64, plus SDK, licenses, ASan fuzz, installer
fixtures and all three release replay comparisons. Inherited security/Nix
failures are reported separately; an earlier local resource-limited full-gate
attempt is not claimed as passed.

`pr-evidence/server-moderation` contains an actual native X11 window pair at
1120 × 760, scale 1, dark mode. The same synthetic server settings fixture shows
feature absence at fork `f9e39f7` and the AutoMod page at `91c95fdd`. The baseline
has only the two documented demo-build repairs (duplicate avatar helpers and
missing `pending::show` optional-error argument); its feature UI is unchanged.
The supported minimum 760 × 520 light-mode moderation layout was also inspected.
Captures use explicit `--demo --interactive`, Xvfb/software rendering, bounded
process-liveness checks and ImageMagick application-window capture. They do not
establish live moderation, release UI performance or a side-by-side official
client comparison. Only one small current pair is committed.


### Windows moderation package snapshot at `91c95fdd`

Same standard package and normalization method as the Linux/macOS snapshot above.
One artifact per revision; MiB = 1,048,576 bytes. Baseline nightly `3a4bf8d`,
previous fork `f9e39f7`, after `91c95fdd`.

| Metric | Nightly | Previous fork | Moderation | Delta vs fork |
| --- | ---: | ---: | ---: | ---: |
| executable | 82.910 | 87.505 | 87.870 | +0.365 (+0.42%) |
| installed payload | 86.909 | 135.216 | 135.581 | +0.365 (+0.27%) |
| normalized distribution | 47.986 | 80.844 | 80.979 | +0.135 (+0.17%) |

Windows x64/ARM64 CI validates both clean and cached normal packages from empty
output directories. The bounded packaged driver-query helper launches with the
development FFmpeg prefix removed from PATH; no manual staging repair was used.
This check stops before UI/account/audio initialization and is not a full
interactive launch or hardware-encoding result. Archive metadata may change hashes.

## Desktop Inbox — October 17, 2026

Code revision `2139384fcb259e504535bd9688c9de6cb2e4a727`, based on fork `f9e39f7`.
The current published upstream nightly was rechecked on October 17:
`v1.0.0-nightly.20261014.58` (`3a4bf8d`). The owner-supplied Discord Stable build is
recorded in the compatibility inventory; it is not the performance baseline.

Release reducer comparison: Rust 1.98.1, committed lockfiles and release profiles,
Linux 6.18.44 x86_64/glibc 2.41, Intel Xeon Platinum 8573C, 16 GiB container memory,
four-CPU quota. Each binary/workload received one warmup and five rotating measured
runs, pinned to CPU 0. Task compiler processes were paused during sampling and no
native preview was running. Values below are medians; ranges expose host variation.

| Workload / elapsed milliseconds | Current nightly | Previous fork | Inbox | Delta vs nightly / fork |
| --- | ---: | ---: | ---: | ---: |
| 100,000-event reducer | 152.105 (146.564–254.134) | 151.199 (139.729–213.125) | 149.063 (139.452–223.483) | −2.0% / −1.4% |
| Wide channel navigation | 185.019 (156.702–248.175) | 211.192 (195.419–287.684) | 219.871 (206.293–231.027) | +18.8% / +4.1% |

The wide-channel result is slower; its cause has not been isolated. The overlapping,
wide ranges do not establish a causal improvement or regression from the Inbox alone.
These are synthetic core workloads, not GUI latency, media throughput or process RSS.
The final 120-second soak passed 1,457 passes, 46,624 visits and six logout cycles
in 120.041 seconds, with the existing retained-history budgets unchanged.

`cargo xtask check` passed on the implementation before the final navigation/content
retirement corrections. Focused navigation and Inbox tests, strict Clippy and native
preview builds were rerun after those corrections; the initial follow-up Inbox suite has nine
passing cases, including dark/light narrow layouts and read-marker isolation.
The bounded ASan fuzz smoke passed 80,860 decoder inputs and 3,079 state transitions.
Login-handoff, cached-xtask workspace, license-policy and packaging fixtures passed.

The matched native screenshot pair in `pr-evidence/desktop-inbox` uses the same
1120 × 760 channel-links fixture, font setup and dark appearance. Before is fork
`f9e39f7` (with its required demo-only pending-upload tuple repair); after adds the
new synthetic Inbox state, not live account data. Light/narrow rendering and bookmark
navigation were inspected separately. The feature branches also remove byte-identical
duplicate demo helpers so their previews compile; production behavior is unaffected.

The initial native/package results are recorded below; subsequent review fixes require
their own current-head CI. Baseline fork artifacts are pinned to run `39021847864` at `f9e39f7`;
its native, installer, license and fuzz jobs passed, while security and separate Nix
checks failed. Native release UI CPU/RSS, frame latency, media throughput and physical
Windows/macOS/Discord interoperability remain unmeasured here. No debug-preview result
is represented as a release or official-client performance measurement.

### Initial Inbox package comparison

These are the initial feature packages at PR head `f19c412`, built by native CI
run [39048221415](https://github.com/MCShotty/Serein/actions/runs/39048221415).
GitHub checked out merge `37cd8be`; both revisions have the same Git tree
`f291a249939b99466b9ff42c68ee0620a3a243a2`. Later review corrections are not included
in these measurements; their current-head checks/results are reported in the PR.

Baselines are published nightly `v1.0.0-nightly.20261014.58` (`3a4bf8d`) and previous
fork `f9e39f7`. All are standard release packages including voice. Installed values
sum regular-file payload bytes; distribution values recompress that payload with
Python 3.12 ZIP DEFLATE level 6 and fixed metadata, excluding outer CI archives and
installers. Debian payloads are read without installation. Values below are MiB
(1,048,576 bytes); deltas are against the previous fork. One artifact per build/platform.

| Platform | Metric | Nightly | Previous fork | Initial Inbox | Delta vs fork |
| --- | --- | ---: | ---: | ---: | ---: |
| Linux-X64 | Executable | 83.682 | 87.900 | 88.274 | +0.374 (+0.43%) |
| Linux-X64 | Installed payload | 88.006 | 130.657 | 131.031 | +0.374 (+0.29%) |
| Linux-X64 | Normalized distribution | 51.211 | 84.553 | 84.710 | +0.157 (+0.19%) |
| Windows-X64 | Executable | 82.910 | 87.505 | 87.968 | +0.462 (+0.53%) |
| Windows-X64 | Installed payload | 86.909 | 135.216 | 135.678 | +0.462 (+0.34%) |
| Windows-X64 | Normalized distribution | 47.986 | 80.844 | 81.007 | +0.162 (+0.20%) |
| macOS-ARM64 | Executable | 65.946 | 69.884 | 70.198 | +0.314 (+0.45%) |
| macOS-ARM64 | Installed payload | 71.641 | 95.216 | 95.529 | +0.314 (+0.33%) |
| macOS-ARM64 | Normalized distribution | 46.027 | 63.548 | 63.683 | +0.135 (+0.21%) |

Windows x64 and ARM64 CI completed clean and cached packages from empty output
directories and validated their archives. The packaged driver-query helper exited
successfully with the development FFmpeg prefix removed from PATH. This is a bounded
load/query check before UI, account, capture or audio initialization; it is not a full
interactive application launch or a hardware encoding result. No staging repair was used.
A separate local release build was interrupted before staging for review corrections;
no completed local clean/cached package is claimed. Native playback throughput, UI
CPU/RSS and physical display/Discord checks remain unmeasured.

The sibling feature branches add `scripts/compare-replay.py` and three isolated CI jobs.
Each records the current published nightly, previous fork and reviewed head, Rust 1.98.1,
matching release overrides, OS/CPU, one warmup and five rotated samples per workload.
Linux/Windows sampling uses one allowed logical CPU when available; macOS reports the
platform scheduler. Raw results include ranges and are uploaded per OS/architecture.
These are synthetic core measurements, not UI/media performance or a parity claim.

### Inbox review corrections

The subsequent lifecycle review adds ten passing Inbox regressions, including service
echoes before/after REST confirmation, closing/reopening during accepted writes,
transient Gateway outages and a stopped connection retiring an unknown outcome.
Rust 1.98.1 formatting and native/demo all-target strict Clippy passed after these fixes.
The earlier full workspace, fuzz, replay and package results above retain their original
revision scope; they are not measurements of these corrections. The native connection
cancellation test awaits CI because its local code-generation run was deferred for the
workspace memory budget. Current revision results are linked in the PR.


### Current Inbox application and package verification at `3d20f9bb`

[Run 39054844003](https://github.com/MCShotty/Serein/actions/runs/39054844003)
passed all four native application/package jobs (Linux x64, Windows x64/ARM64,
macOS ARM64), installer fixtures, SDK, licenses, ASan fuzz and all three release
replay comparisons. Its merge checkout `071e456b` has the exact Git tree of
`3d20f9bb`. The connection-cancellation test is included in the current native
workspace gate; its formerly deferred local run is not claimed separately.
Security and separate Nix failures remain inherited.

The initial package measurements above remain historical. Current standard
voice-inclusive packages use the same regular-file installed-payload and fixed
ZIP DEFLATE-6 normalization method. Baselines remain published nightly
`3a4bf8d` and previous fork `f9e39f7`; one artifact per revision/platform.
No installer or outer CI archive is included in the normalized distribution.
Values are MiB (1,048,576 bytes), with absolute/percentage deltas versus the fork.

| Platform / metric | Nightly | Previous fork | Current Inbox | Delta vs fork |
| --- | ---: | ---: | ---: | ---: |
| Linux-X64 / executable | 83.682 | 87.900 | 88.273 | +0.373 (+0.42%) |
| Linux-X64 / installed payload | 88.006 | 130.657 | 131.031 | +0.373 (+0.29%) |
| Linux-X64 / normalized distribution | 51.211 | 84.553 | 84.712 | +0.158 (+0.19%) |
| Windows-X64 / executable | 82.910 | 87.505 | 87.970 | +0.465 (+0.53%) |
| Windows-X64 / installed payload | 86.909 | 135.216 | 135.681 | +0.465 (+0.34%) |
| Windows-X64 / normalized distribution | 47.986 | 80.844 | 81.007 | +0.163 (+0.20%) |
| macOS-ARM64 / executable | 65.946 | 69.884 | 70.198 | +0.314 (+0.45%) |
| macOS-ARM64 / installed payload | 71.641 | 95.216 | 95.530 | +0.314 (+0.33%) |
| macOS-ARM64 / normalized distribution | 46.027 | 63.548 | 63.686 | +0.138 (+0.22%) |

Windows x64 and ARM64 clean/cached packages both pass archive validation and the
bounded driver-query launch without the development FFmpeg prefix in PATH.
The x64 clean/cached ZIPs are both 86,047,535 bytes; ARM64 is 69,104,411 bytes.
Their metadata-dependent hashes differ, so bit-for-bit reproducibility is not
claimed. The helper exits before UI/account/audio initialization: these are not
interactive application launches or hardware-encoding acceptance tests. No manual
staging repair was used. macOS nightly is signed while the CI bundle is unsigned;
all package figures are observed build deltas, not an isolated feature cost.
Current artifacts and clean/cached JSON evidence are retained by that run.
Release UI CPU/RSS, media throughput and physical display checks remain unmeasured.


### Current Inbox cross-platform release replay at `3d20f9bb`

[Run 39054844003](https://github.com/MCShotty/Serein/actions/runs/39054844003)
compares published nightly `v1.0.0-nightly.20261014.58` (`3a4bf8d`), previous
fork `f9e39f7` and Inbox code `3d20f9bb`. The same release settings and sampling
method above apply: one warmup, five rotated samples, one allowed logical CPU
on Linux/Windows and platform scheduling on macOS. Per-OS artifacts record exact
revisions, OS/CPU, settings and raw samples. Values are milliseconds, median
(minimum–maximum). They measure core state, not native UI/media performance.

| Platform / workload | Nightly | Previous fork | Inbox | Delta vs nightly / fork |
| --- | ---: | ---: | ---: | ---: |
| Linux-X64 / reducer | 56.668 (55.112–57.319) | 57.320 (55.984–60.732) | 56.592 (55.781–56.929) | -0.1% / -1.3% |
| Linux-X64 / wide channels | 100.255 (89.555–102.417) | 109.163 (107.814–110.290) | 95.278 (93.712–96.481) | -5.0% / -12.7% |
| Windows-X64 / reducer | 110.119 (103.805–124.310) | 107.383 (106.629–108.035) | 106.993 (106.750–109.522) | -2.8% / -0.4% |
| Windows-X64 / wide channels | 228.188 (222.732–231.541) | 227.009 (224.482–233.393) | 224.888 (222.317–228.622) | -1.4% / -0.9% |
| macOS-ARM64 / reducer | 65.531 (64.239–70.252) | 70.053 (60.927–79.586) | 66.613 (62.559–79.989) | +1.7% / -4.9% |
| macOS-ARM64 / wide channels | 100.838 (94.759–111.089) | 94.479 (91.853–99.470) | 98.590 (90.037–116.093) | -2.2% / +4.4% |

Ranges overlap and no causal improvement is claimed. Earlier local slower measurements above remain historical and unisolated.


### Final Inbox native evidence and footer correction

The final pair replaces the initial feature screenshots with fresh native captures
of fork `f9e39f7` and Inbox code `3d20f9bb` plus the footer-only correction.
Both use the same synthetic channel-links scenario, 1120 × 760 dark window and
scale 1. The baseline has the two documented demo-only build repairs (duplicate
avatar helpers and the missing optional error argument to `pending::show`),
with its displayed feature UI unchanged. The new frame opens the Unread tab;
no live account or service is used. Light mode at the supported minimum 760 × 520
was also inspected. Only one small current pair is committed.

The final native capture exposed an empty action strip on a fresh Unread page.
The footer now draws when another tab has controls or an existing stale-status
message needs it. The ten Inbox regressions, pinned formatting and strict native
UI all-target/demo Clippy pass after this conditional-rendering correction.
The native preview is rebuilt for the final captures. Capture uses explicit
`--demo --interactive`, Xvfb/software rendering, process-liveness checks and
ImageMagick's application-window capture; no image editing is used.

Full native application/package CI above is for `3d20f9bb`, before the footer
correction. Core release replay and package snapshots retain that revision label;
the footer does not change the measured core workload. The final UI/code-and-evidence
head receives its own CI. Release UI CPU/RSS, media throughput, physical displays
and owner-controlled Discord acceptance remain unmeasured or unverified here.

## Combined desktop parity — October 17, 2026

The combined validation branch is `integration/desktop-parity-20261017`. Its feature work remains in fork PRs [#7](https://github.com/MCShotty/Serein/pull/7)–[#15](https://github.com/MCShotty/Serein/pull/15); neither main nor parent #567/#620 is modified by this work. The final feature merges preserve the verified runtime source and only add current native evidence.

### Release reducer comparison

Compared combined code `eee7fd755bd6ed3d94a7d983040bb1564843a9a8` with current published nightly `v1.0.0-nightly.20261014.58` (`3a4bf8df2136945d51bd16a1acd9c4033d88feb5`) and fork `f9e39f792b2fe0af02f2c1e791e1516147084824`. Rust 1.98.1, locked graph, Linux 6.18.55/glibc 2.41, five logical CPUs and about 17 GiB RAM. Each revision has a separate build directory: opt-level 3, thin LTO, one codegen unit, line tables, stripped symbols and panic abort. One warmup and five rotated measured samples per workload. The sampler uses CPU 0; other native build threads were restricted to CPUs 1–4. Those builds still share the memory system.

| Workload / elapsed median (range), ms | Nightly | Previous fork | Combined | Delta vs nightly / fork |
| --- | ---: | ---: | ---: | ---: |
| 100,000-event reducer | 71.035 (70.452–75.244) | 74.009 (72.001–77.601) | 78.581 (77.600–79.799) | +10.6% / +6.2% |
| 10,000-event wide-channel reducer | 117.102 (100.668–127.305) | 124.630 (119.000–157.747) | 128.122 (107.561–155.779) | +9.4% / +2.8% |

The reducer is slower than both baselines; the cause is unisolated. Wide-channel ranges overlap. These are combined feature measurements, not attribution to an individual PR, process RSS, UI latency, frame delivery or media throughput. The earlier run under unconstrained concurrent builds had substantially wider ranges and is excluded from the headline comparison. No speed improvement is claimed.

Raw elapsed samples in milliseconds:

```text
reducer/nightly: 71.035405, 75.243870, 70.567324, 70.452204, 73.810681
reducer/previous_fork: 75.435199, 74.008517, 77.601141, 72.086003, 72.000906
reducer/after: 77.937841, 77.600340, 78.580850, 79.180244, 79.798562
wide_channels/nightly: 117.102036, 111.678245, 100.667822, 127.304744, 126.957419
wide_channels/previous_fork: 125.465557, 119.000406, 124.630067, 120.726529, 157.747206
wide_channels/after: 128.121680, 107.561215, 119.048428, 129.088519, 155.779327
```

Reproduce with `python3 scripts/compare-replay.py --fork-base f9e39f792b2fe0af02f2c1e791e1516147084824 --output target/parity-replay.json`. Windows/macOS/Linux scoped CI runs the same comparison against the nightly selected at each run. Historical per-OS results above retain their original source revisions and regressions.

### Verification and native UI

`cargo xtask check` at `0e7c26e3` passes 1,919 tests with zero failures and 32 tests/examples ignored, formatting, strict workspace Clippy, the minimal application check and policy checks. Later changes to the verified runtime are demo-only. Demo all-target Clippy and the native preview build pass. Login-handoff, cached-xtask workspace, both license graphs, six license-policy fixtures, packaging fixtures, release replay and the 120-second soak pass. The soak completed 61,568 visits, 36,940,800 live inserts and eight logout cycles in 120.015 seconds within the core budgets. ASan fuzz smoke completed 128,905 decoder and 4,340 state-transition runs without a finding. None of these runs used Discord credentials, a live call, microphone, speaker or capture device.

Current unedited native screenshot pairs are in `docs/pr-evidence/`: application commands, desktop Inbox/messaging, desktop media, profile settings, community onboarding and community events. The baseline is the verified commands revision `1d47c3d`, not the nightly; after captures use combined code `80350d0f`, with the Stage/messaging follow-up at `92d5c311`. The new community fixtures and the pending synthetic block proposal are described beside their images. Native light/compact layouts, scrolling, explicit disclosure and keyboard settings interaction were inspected. Screenshots do not establish media throughput, frame latency, physical accessibility or official-client interoperability.

### Current scoped CI comparisons

The following completed jobs compare the exact pushed feature heads with the same nightly `3a4bf8d` and previous fork `f9e39f7`. They use the committed comparison script and matching release settings, one warmup and five rotated samples per workload. These are separate workers and scopes; do not compare absolute times between rows. Remaining current-head jobs are pending. Ranges overlap in most rows; no causal performance improvement is claimed.

| PR / revision / worker | Workload | Nightly / fork median, ms | Feature median (range), ms | Delta vs nightly / fork |
| --- | --- | ---: | ---: | ---: |
| [#10](https://github.com/MCShotty/Serein/actions/runs/39078123597/job/117529404730) / `1f87c10c` / macOS arm64 | reducer | 82.270 / 93.790 | 83.025 (68.906–102.941) | +0.9% / -11.5% |
| [#10](https://github.com/MCShotty/Serein/actions/runs/39078123597/job/117529404730) / `1f87c10c` / macOS arm64 | wide_channels | 114.380 / 108.980 | 116.055 (108.913–161.802) | +1.5% / +6.5% |
| [#14](https://github.com/MCShotty/Serein/actions/runs/39077843579/job/117528589940) / `17dd6ea6` / Linux x64 | reducer | 66.267 / 66.012 | 67.190 (66.914–68.239) | +1.4% / +1.8% |
| [#14](https://github.com/MCShotty/Serein/actions/runs/39077843579/job/117528589940) / `17dd6ea6` / Linux x64 | wide_channels | 53.462 / 75.568 | 53.429 (52.148–55.652) | -0.1% / -29.3% |
| [#9](https://github.com/MCShotty/Serein/actions/runs/39077840306/job/117528650041) / `2d7ad3ff` / Windows x64 | reducer | 57.631 / 58.102 | 57.794 (57.649–59.047) | +0.3% / -0.5% |
| [#9](https://github.com/MCShotty/Serein/actions/runs/39077840306/job/117528650041) / `2d7ad3ff` / Windows x64 | wide_channels | 61.728 / 61.415 | 65.329 (60.783–70.728) | +5.8% / +6.4% |
| [#9](https://github.com/MCShotty/Serein/actions/runs/39077840306/job/117528650153) / `2d7ad3ff` / macOS arm64 | reducer | 62.666 / 65.128 | 67.349 (63.345–76.996) | +7.5% / +3.4% |
| [#9](https://github.com/MCShotty/Serein/actions/runs/39077840306/job/117528650153) / `2d7ad3ff` / macOS arm64 | wide_channels | 96.200 / 101.236 | 98.480 (96.132–102.579) | +2.4% / -2.7% |
| [#12](https://github.com/MCShotty/Serein/actions/runs/39077838059/job/117528631664) / `df0bc231` / Linux x64 | reducer | 80.851 / 78.743 | 80.172 (79.051–81.817) | -0.8% / +1.8% |
| [#12](https://github.com/MCShotty/Serein/actions/runs/39077838059/job/117528631664) / `df0bc231` / Linux x64 | wide_channels | 163.075 / 163.410 | 163.266 (146.565–167.507) | +0.1% / -0.1% |

Raw samples, compiler/settings, OS and affinity are retained in each linked job’s `parity-replay` artifact. This is core reducer evidence; package size, UI/media performance and interoperability are separate measurements.

### Standard package comparison

The normal `cargo xtask package --format dir` completed from an empty output directory with the standard fat-LTO release profile, committed lockfile, voice enabled and one build job. Runtime code matches the verified combined tree; subsequent merges add evidence only. The package was not repaired or manually staged. All six private runtime libraries, corresponding-source archives and licenses passed validation. Packaged source archives match the recipe hashes. Dynamic RPATHs contain no development prefix, and the installed binary's bounded driver-query launch succeeds with both FFmpeg development directories removed from the runtime search path. This launch exits before credentials, UI, clients or capture initialize.

The comparison uses the official published nightly Ubuntu 26.04 x86_64 Debian payload (SHA-256 `ac36677a300133379f4fc4060a724bde4f4afc0261d757e1957b7f511f118d04`) versus the local Debian 13 x86_64 installation tree. Both are standard packages with voice. Normalized ZIPs use sorted regular files, fixed timestamps/modes and Python DEFLATE level 9. These whole-fork inventories include the existing FFmpeg/video work and corresponding source; they do not isolate parity changes or establish an application runtime comparison across the two build environments. The previous fork package was not rebuilt for this comparison.

| Metric / bytes | Published nightly | Combined clean package | Delta |
| --- | ---: | ---: | ---: |
| Executable | 87,746,736 | 95,345,800 | +7,599,064 / +8.7% |
| Installed payload | 92,280,770 | 140,176,288 | +47,895,518 / +51.9% |
| Normalized distribution ZIP | 53,552,488 | 89,776,426 | +36,223,938 / +67.6% |

Clean executable SHA-256: `eabce23a31d8ae290b95c9fea4c5fc60b9843bc36fd286d35b7380ddbc92cecf`. Clean normalized ZIP SHA-256: `db1693d48960579e45565fa04d802a95b5368aefc988fb23a89ef44d0c87c74f`. Native UI CPU/RSS, interaction/frame latency and media throughput are unmeasured. Windows/macOS package-size deltas remain unmeasured; their native checks and release reducer comparisons run in CI.

Two further normal package invocations used the same dependency cache and an empty output directory each time. The first cached run rebuilt the desktop crate after the final evidence merges touched its source timestamps; Cargo reported 15m 38s for that release build. The unchanged warm repeat reused the executable and reported 1.27s for the release build. These are Cargo build durations, not complete packaging or launch timings.

Both repeats passed the same library, RPATH, source-hash, license and development-path-free launch checks (exit 0). Clean, cached and warm outputs have identical executable and normalized ZIP hashes, installed byte counts and 280-file inventories. All runs used the ordinary packaging command, without a staging repair or modified cache fingerprints.

At final local verification, scoped current-head native checks remain incomplete. Commands PR #10's Windows x64 job timed out in the unchanged local-Gateway voice join/leave fixture, outside its added autocomplete dispatch path; the earlier native checks passed. GitHub rejected a rerun while the workflow was still running. Inherited `zip` advisories, Nix builds and openSUSE dependency setup failures are separate from the passing local gate. The combined branch receives fresh Windows x64/ARM64, macOS and Linux application/package checks on push; pending jobs are not reported as passes.

### Remaining gates

Parent main advanced independently to `611ff7f` during verification (dedicated Windows installer CI jobs removed). The fork baseline remains `f9e39f7`; no upstream synchronization is implied. Parent #567 remains at `1473d408` with 149 paths, and #620 remains draft at `4806c9bb`.

Current-head native/package CI on Windows x64/ARM64, macOS and Linux remains distinct from local Linux evidence. Physical devices, HDR/monitor transitions, audio synchronization, native assistive technology, RTL behavior and normal-account/official-client workflows remain owner-controlled. The supplied reference is Discord Stable 641004 (`c9ac31f`), Host 1.0.9262 x64 (`94186`); that client was not run here. SDK wiki preview pages were generated and checked from pushed media source `631699ad`, but parent wiki write access is unavailable and existing unrelated preview content must be preserved; no publication is claimed. The current parity inventory records supported, partial and handoff-only workflows; games and Activities remain excluded.

## Fork main integration — October 18, 2026

Runtime `d3994ba14179e540e14989c38160dcd2e229ac24` combines parent main
`d9d87471` and the current #620 conflict fixes with fork PRs #2 and #6–#15.
The nine feature heads retain their combined implementation and source evidence.
Historical PR #6 is recorded as merged while keeping the newer encoder/capture/
HDR/Source implementation, consolidated tests and archived evidence. Its obsolete
bridge and historical copies are not restored. PR #2's installer workflow now
requires explicit verified-build inputs and checks every packaged runtime DLL.
The Nix native recipe skips the default build phase because its Python recipe
performs the dependency builds during installation.

Pinned Rust 1.98.1 and committed lockfiles: `cargo xtask check` passes
**1,931 tests, zero failures, 32 ignored**, with formatting, strict Clippy,
production configuration and policy checks. Login-handoff, cached-xtask/workspace/
package, both license graphs, license-policy and FFmpeg/Linux/Flatpak/repository/
mocked-signing fixtures pass. Bounded ASan fuzzing completes 116,372 decode and
3,094 state runs (31 seconds each, 512 MiB limit), with no finding. Release replay
and the 120-second soak pass (2,735 passes, 11 logout cycles). Replay ran beside
compilation; its timing is not a comparable performance sample.

[Runtime CI](https://github.com/MCShotty/Serein/actions/runs/39093273584) passes
workspace checks on Windows x64/ARM64, macOS and Linux. Linux/macOS packages and
all three nightly replay comparisons, security, licenses, SDK and fuzz jobs pass.
At recording time, Windows production packaging is still running. Keep this
runtime result separate from subsequent main-head CI and native display/live tests.
The two repaired Windows fixtures retain their mock TCP peers until the expected
terminal response is consumed; their test deadline is bounded at 45 seconds.
Production transport deadlines and encryption checks are unchanged.

Package comparison: previous combined package `343e4f87` (its runtime provenance
is recorded with that evidence), versus `d3994ba1`. Both use Debian 13 x86-64, the
same task SDK/FFmpeg prefix, Rust 1.98.1, committed release profiles, voice enabled,
no demo/developer features and two Cargo build jobs. The previous payload was
rechecked against its recorded archive hash and every installed file. This is an
integration comparison against the prior combined package, not a nightly package
or an isolated measurement of one PR. Historical nightly results remain labeled
with their original revisions above.

| Metric / method | Previous combined | Integration | Delta |
| --- | ---: | ---: | ---: |
| Executable | 95,345,800 B | 95,419,720 B | +73,920 B (+0.078%) |
| Installed payload | 140,176,288 B | 140,250,352 B | +74,064 B (+0.053%) |
| Normalized ZIP | 89,776,426 B | 89,814,051 B | +37,625 B (+0.042%) |

The full normal `cargo xtask package --format dir` command passes into empty
output, then passes again with caches and a new empty output directory. No manual
staging repair is used. Both 280-file payloads have identical executable and
normalized ZIP hashes. Validation checks the current Cargo executable, allowlisted
payload, private libraries, all 19 font notices, corresponding-source archive
hashes and absent development RPATHs. A query-only startup succeeds with development
FFmpeg paths removed; the task SDK still supplies system libraries. No GUI or
owner account is opened. The comparison ZIP uses sorted paths, fixed timestamps,
0644 comparison-entry modes and DEFLATE level 9, matching the prior measurement;
it is a size-comparison archive, not a Linux installer. Directory executable modes
are verified by the normal payload validator.

Executable SHA-256: `605eed209e446bdb4f40d4f4800a7007b2d8b6a1aa87f710cfbdc39dd2521f15`.
Normalized ZIP SHA-256: `bbba39486c9f38b566694121a665a7d91241e19a4f2029a5b7e361e1b0c07482`.
No fresh UI CPU/RSS, frame latency, media throughput, physical-device or live
Discord measurement is claimed. Existing screenshots retain their original source
labels. SDK documentation is included in the fork; parent-wiki publication is not
a gate for this fork integration.

## Voice connection and participant diagnostics (2026-10-18)

Runtime source: `a20914b6`; task baseline: `da97a14f`. The preserved release package
from `d3994ba1` is equivalent to the task baseline for production source: the only
later changes were performance documentation and the extension catalog. The catalog
is read from the service at runtime; its repository copy is embedded only in tests
and the preview example. Its preserved executable and package hashes were verified
before this comparison.

Linux x86_64, Intel Xeon Platinum 8573C cloud host, 4-core CPU quota and 16 GiB memory
limit; pinned Rust 1.98.1 and committed lockfile. Standard production features,
manifest release settings (optimization 3, fat LTO, one codegen unit, symbols stripped,
line-table debug setting). The current official Ubuntu x64 nightly is
`v1.0.0-nightly.20261017.60` / `d9d87471`, published October 18. Its DEB checksum was
verified before extracting it, without installation or execution. Its build environment
is the release runner's, so the nightly comparison includes existing fork differences
and build-environment differences; the previous-fork column isolates this task.

| Release metric / method | Current nightly | Fork before | After | Delta vs fork |
| --- | ---: | ---: | ---: | ---: |
| Executable | 91,957,328 B | 95,419,720 B | 95,541,960 B | +122,240 B (+0.13%) |
| Installed payload | 96,555,161 B | 140,250,352 B | 140,372,592 B | +122,240 B (+0.09%) |
| Normalized ZIP | 55,682,561 B | 89,814,051 B | 89,868,267 B | +54,216 B (+0.06%) |

Both new packages used the complete normal `cargo xtask package --format dir`
command, each with an initially absent `dist` directory; the second build reused caches.
No manual staging repair was used. Their 280-file payloads and normalized ZIP hashes
match. Validation checked the Cargo executable, allowlisted payload, six private media
libraries, 19 font notices, seven corresponding-source archives and their checksums,
and absent development RPATHs. A query-only startup ran with development FFmpeg paths
removed; the task SDK still supplied system libraries. This is not a production GUI
or account launch. ZIP comparisons use sorted paths, fixed timestamps, comparison-only
0644 entry modes and DEFLATE level 9 for every payload, including the nightly. They are
size-comparison archives, not Linux installers; executable modes are checked separately.

Executable SHA-256: `934e12e7f967f65f7ecec3240efe5a09f281d0b11a89ddb464ce55c562b0d761`.
Normalized ZIP SHA-256: `df63bc51863a49b31d3d72840d7dee007602f6be0e739b39cc3f37fa268187ec`.

Matched native preview samples use the **development profile, not release**
(`debug=0`, `demo` feature), Xvfb at scale 1, wgpu with adapter/backend not recorded,
and the same synthetic Stage at 1120 × 760 with connection details closed. Each
process warmed for 3 seconds, then provided 15 RSS samples at 1-second intervals.
CPU is process user+system time across those intervals as a percentage of one core;
settled RSS is the median of the last five samples. Both processes remained alive
through sampling and had no child processes. These are single short observations,
not confidence intervals or a live voice workload.

| Development preview metric | Before `da97a14f` | After `a20914b6` | Delta |
| --- | ---: | ---: | ---: |
| Idle CPU, one core | 5.46% | 5.47% | +0.0002 percentage points (+0.004%) |
| Peak RSS | 187,699,200 B | 187,621,376 B | -77,824 B |
| Settled RSS | 187,699,200 B | 187,621,376 B | -77,824 B |

No CPU or memory improvement is inferred. An earlier development sample of `d0489364`
measured 5.40% → 6.33% CPU; the final matched pair above is effectively unchanged.
The selector follow-up does not execute on the measured closed-panel path, so these
short-run differences are treated as unisolated sampling variability. Release UI CPU/RSS,
frame latency, media throughput, live packet cost and Windows/macOS measurements remain
unmeasured locally. Native application/package CI and an owner-controlled Discord session
must establish
platform behavior and real server replies/attribution. Offline reducer comparisons
against the current nightly completed on Windows, macOS and Linux for `a20914b6`
([CI artifacts](https://github.com/MCShotty/Serein/actions/runs/39106926485)); these
are not UI or voice performance measurements.

Synthetic [before](pr-evidence/voice-connection/before.png),
[after](pr-evidence/voice-connection/after.png) and
[per-user debug](pr-evidence/voice-connection/per-user.png) captures are fresh native
framebuffers. The pair uses the same Stage, theme, fonts and viewport; new diagnostic
values are explicit preview data. Also inspected dark and light themes, a 760 × 520 layout, and
scrolling through the participant video rows. Reproduce the new states with
`profile_preview --demo --page=stage --voice-connection` or `--voice-debug`.

Source checks passed: `cargo xtask check` (1,953 passed, 32 ignored; strict Clippy,
minimal app and policy), login-handoff, cached-xtask workspace/package regressions,
Linux packaging fixtures, replay and the 120-second reducer soak. Both bounded fuzz
smokes passed (106,863 decode runs; 6,546 state-transition runs). The soak is core-only,
not evidence of native media memory behavior. Loopback UDP/WebSocket plus Opus/video
and UI regressions check attribution, repair windows, stale samples, source replacement,
resume, call-control independence, copy redaction and teardown; no account, microphone
or live capture is used.

## Voice-channel status, reactor tooltips and microphone enhancement (2026-10-19)

Runtime source: `4f86f7d2b5fc`; task baseline: `88bd5eea0a3a` (production source
`a20914b6`). The official nightly is still `v1.0.0-nightly.20261017.60` /
`d9d87471`, rechecked October 19; its verified published payload is retained
from the preceding comparison. Nightly differences include prior fork changes
and release-runner differences; the fork column isolates this follow-up.

Debian 13.6 x86_64, Xeon Platinum 8573C, four-core CPU quota, 16 GiB memory
limit; Rust 1.98.1, committed lockfile, normal production features and manifest
release settings (optimization 3, fat LTO, one codegen unit, stripped symbols
and line-table debug setting).

| Release metric / method | Current nightly | Fork before | After | Delta vs fork |
| --- | ---: | ---: | ---: | ---: |
| Executable | 91,957,328 B | 95,541,960 B | 95,943,368 B | +401,408 B (+0.42%) |
| Installed payload | 96,555,161 B | 140,372,592 B | 224,692,514 B | +84,319,922 B (+60.07%) |
| Normalized ZIP | 55,682,561 B | 89,868,267 B | 154,439,891 B | +64,571,624 B (+71.85%) |

The bundled speech models, CPU runtime, source and notices occupy
83,874,775 bytes in the installed speech directory. Both
complete normal `cargo xtask package --format dir` runs started without a
`dist` directory; the second reused caches with speech downloads disabled.
Their 295-file installation trees match by path, size, mode and SHA-256,
as do their comparison ZIPs. An earlier attempt using a stale local cached
`xtask` omitted speech staging and failed; `xtask` was rebuilt before the
successful runs. No output was manually repaired. Validation checks the
allowlisted payload, seven native media/speech libraries, pinned models and
corresponding sources, notices, executable modes and absent development RPATHs.
Driver-query startup and three speech initialization/reset/teardown checks per
package pass without development FFmpeg paths; the task SDK still supplies
system GTK libraries. No account, microphone, capture or production GUI opens.

Comparison ZIPs use sorted paths, 1980 timestamps, comparison-only 0644 entry
modes and DEFLATE level 9, matching both baselines. They are size-comparison
archives, not Linux installers. Executable SHA-256: `8a1129b54c7841665c85c9c8d03706b69b94fb768188863caf4493c99df3abe6`.
Normalized ZIP SHA-256: `46cbf73198db8a9b128eb45b02f5825432b7590cf38ba10882f5b10cd8e2518f`.

The reducer comparison uses 100,000 events, identical locked release settings,
separate build directories, one warmup and five counterbalanced samples per
revision, followed by one confirmation set with the same method. Other task
compilers were paused or finished during sampling. All ten observations remain
in the result; no outliers were removed.

| Reducer metric | Fork before | After | Delta |
| --- | ---: | ---: | ---: |
| Median, ten runs | 103.279 ms | 103.093 ms | -0.186 ms (-0.18%) |

The initial five-run median was 95.345 → 102.202 ms (+7.2%). The confirmation
medians are 112.464 → 103.985 ms.
This is an unisolated shared-host observation, not an improvement claim or
a voice benchmark. Both versions retain 500 records and estimated timeline
bytes of 401,492..401,977. The 120-second core-only soak passed with bounded
retention; neither workload measures native audio memory or live compatibility.
Current-nightly reducer comparisons on Windows/macOS/Linux are delegated to
the existing CI job and remain distinct from local fork-isolation measurements.

Native UI observations use the development profile (`debug=0`, `demo` feature),
Xvfb at scale 1, wgpu (adapter/backend unrecorded), the same synthetic Stage at
1120 × 760, three-second warmup and fifteen one-second RSS samples. The new
status fixture is absent in the baseline. CPU is user+system time as a percent
of one core; settled RSS is the last-five-sample median. Both previews remained
alive throughout and had no helpers. Concurrent compilation makes these single
short observations noisy; no CPU or memory improvement is inferred.

| Development preview observation | Before | After |
| --- | ---: | ---: |
| Idle CPU, one core | 5.91% | 5.31% |
| Observed peak RSS | 189,640,704 B | 188,502,016 B |
| Settled RSS | 189,640,704 B | 188,502,016 B |

One direct packaged speech-helper observation uses synthetic 48 kHz mono
audio, 40 ms blocks, twenty warmup blocks and one hundred measured blocks
targeting a 40 ms input cadence. It checks finite/nonzero output, epoch reset to
silence, normal pipe close and child reaping. It excludes the controller,
physical capture, AEC, AGC, Opus and network.

| New helper observation (no previous equivalent) | Value |
| --- | ---: |
| IPC + inference median / p95 / maximum | 36.94 / 51.37 / 139.69 ms |
| CPU, one core | 118.86% |
| Observed peak / settled RSS | 154,218,496 / 154,218,496 B |
| Process RSS high-water mark | 154,681,344 B |
| One-second idle CPU | 0.00% |

The hundred measured blocks represent four seconds of audio and took
4.223 seconds. This observation does not establish sustained
real-time processing on this host; individual requests exceeded the block
budget. Initialization was 556.0 ms in one observation,
not a startup distribution. Helper timing varies under load; sustained queue
pressure selects fresh RNNoise fallback. Package self-tests verify model
initialization and generation/lifecycle handling with silence, rather than
imposing a real-time speed requirement on shared CI. Nonzero native graph
regressions separately compare the pinned models against an independent
Torch/torchaudio reference. No physical latency, speech quality, native audio
soak, release UI CPU/RSS or Windows/macOS performance claim is made.

Fresh native synthetic [before](pr-evidence/voice-status/before.png) and
[after](pr-evidence/voice-status/after.png) screenshots use the same fonts,
theme and viewport; the status text is explicit fixture data. The separate
[LavaSRv2 selector](pr-evidence/voice-status/lavasr.png) image opens no microphone.
Dark/light and narrow layouts were inspected. Earlier voice-connection and
participant screenshots above retain their original revision labels. Reactor
name loading/visibility is covered by synthetic multi-frame UI regressions.

Final source checks include `cargo xtask check` (1,968 passed, 33 ignored),
strict Clippy, login handoff,
cached-xtask checks, bounded fuzz, replay/soak, license policy and packaging
fixtures. Native application/package CI, Nix/Flatpak builds, owner-operated
Discord status/hover behavior and microphone quality remain separate checks;
local archive inspection and synthetic fixtures do not establish those results.
Older Windows media packages need the updated installer for the new payload
layout. Canonical SDK docs include the new configured suppression value.

## GIF favorite order and server emoji (2026-10-20)

Runtime source: `e8916a91` (the change after `b3ad937e` only repairs a test fixture).
Fork baseline: `da97a14f`. The current official nightly remains
`v1.0.0-nightly.20261017.60` / `d9d87471`, rechecked against release metadata on
October 20. Its previously verified Ubuntu x64 payload and source revision are
used below. Nightly differences include the rest of the fork and release-runner
conditions; the fork-before column isolates this task.

Debian 13.6 x86_64, Xeon Platinum 8573C, shared container; Rust 1.98.1 and committed
lockfiles. Production packages and reducer executables use identical manifest
release profiles: optimization 3, fat LTO, one codegen unit, line-table debug
information and stripped symbols. No task compilers ran during sampling.

| Release footprint / method | Current nightly | Fork before | After | Delta vs fork |
| --- | ---: | ---: | ---: | ---: |
| Executable | 91,957,328 B | 95,419,720 B | 95,424,840 B | +5,120 B |
| Installed payload | 96,555,161 B | 140,250,352 B | 140,255,472 B | +5,120 B |
| Normalized ZIP | 55,682,561 B | 89,814,051 B | 89,815,051 B | +1,000 B |

Both complete normal `cargo xtask package --format dir` runs started without
`dist`; the second reused build caches. No output was manually repaired. All
546 portable/installation files match between runs by relative path, mode and
SHA-256, as do the comparison ZIPs. Both installed trees contain 280 files; only
`usr/bin/serein` differs from the baseline. Validation covers native library
closure/RPATHs, seven pinned source archives, notices and 19 font licenses. The
installed driver-query startup succeeds with development FFmpeg paths removed;
the task SDK still supplies system GTK libraries. This is not a full desktop
integration or account test. One earlier compilation was interrupted before
staging to freeze the corrected test fixture; it is not a package measurement.

Comparison ZIPs sort installed paths, use 1980 timestamps, comparison-only 0644
entry modes and DEFLATE level 9, matching the nightly method. They are not Linux
installers. Executable SHA-256: `f65327fccf321f6cef329b689e3c6be9db828ccfb2c6c7700d76ba9d612c34cf`.
Normalized ZIP SHA-256: `5bbb1561312a58d33adacb3be24021ee754b5b091454658289e8c1618f1fe2ab`.

The release reducer uses the same 100,000-event driver on all three revisions,
separate build directories, one warmup per revision and five counterbalanced
runs per set. A confirmation reverses revision order; a final control binds all
processes to CPU 0. Every set is retained rather than selecting the best result.

| Reducer median / five samples | Current nightly source | Fork before | After | Delta vs fork |
| --- | ---: | ---: | ---: | ---: |
| Unrestricted, initial | 84.778 ms | 91.345 ms | 102.162 ms | +11.84% |
| Unrestricted, confirmation | 133.454 ms | 153.875 ms | 175.734 ms | +14.21% |
| Fixed CPU 0, control | 86.833 ms | 97.700 ms | 97.503 ms | -0.20% |

The unrestricted runs regress by 11.8–14.2%; the fixed-core control is essentially
flat. The scheduling/frequency sensitivity and cause are not isolated, and no
performance improvement is claimed. Nightly is rebuilt from its release source
under the same settings; it is not a timed run of the downloaded application.
All versions retain 500 records and 401,492..401,977 estimated timeline bytes.
The 120-second core-only soak passed with 2,536 passes, 81,152 channel visits,
48,691,200 live inserts and ten logouts. These are not native RSS or live tests.

Native UI observations use development builds (`debug=0`, `demo` feature), the
same synthetic `--page=emoji` and `--page=gif-favorites` fixtures, Xvfb at scale 1,
1120 × 760 and wgpu (exact adapter/backend unrecorded). Each process has a
three-second warmup followed by fifteen one-second RSS samples; settled RSS is
the last-five-sample median. CPU uses process user+system ticks as a percentage
of one core. The sampler verifies process liveness and bounds logs. No helper
processes were present. The initial sampler could not read the optional Linux
`children` proc file and produced no UI report; the completed observations use
`ps --ppid` instead.

| Development preview observation | Fork before | After |
| --- | ---: | ---: |
| emoji: idle CPU, one core | 0.00% | 0.07% |
| emoji: observed peak / settled RSS | 191,578,112 / 191,578,112 B | 187,039,744 / 187,039,744 B |
| gif-favorites: idle CPU, one core | 0.00% | 0.33% |
| gif-favorites: observed peak / settled RSS | 194,080,768 / 194,080,768 B | 191,193,088 / 191,193,088 B |

These single idle observations show different visible contents as intended by
the fix. They do not establish a memory/CPU improvement, release UI throughput,
frame/startup latency or leak freedom. Matching fresh native screenshots use the
same fixture opening steps on both revisions; only the new preview-page setup
is backported to the baseline. Dark/light and narrow layouts were inspected.
Live Discord, Windows/macOS interaction and native application/package CI remain
unverified locally. No account, microphone or capture session was opened.

## Chat typography and inline media — October 20, 2026

Runtime source `6f821a743d0048e0c2d3b6f231813f15aee8add4` is compared with fork
main `da97a14f9edee868e0773c01e110eaab0eb07649`. Subsequent evidence documentation
does not alter the runtime. The published-nightly reference is
`v1.0.0-nightly.20261017.60`, source `d9d87471f0ea2190da3f90db815e115045461703`.
It was the latest published nightly when checked for this task; nightly comparisons
include earlier fork features and do not isolate this UI change.

### Native visual and idle-process evidence

Fresh unedited native before/after screenshots are in
[`pr-evidence/chat-typography`](pr-evidence/chat-typography). The pair uses the same
synthetic conversation, 1440 × 960 window, scale 1, Inter fonts and dark appearance.
The baseline contains only fixture additions needed to show category casing and
edited messages; its production UI remains `da97a14f`. `inline-media.png` is a
separate after-only fixture showing custom reply emoji and contextual mention names,
not a matched baseline. Light, compact and 760 × 760 variants were also inspected.
No saved session, account, microphone or live capture was opened.

Environment: Linux 6.18.55, glibc 2.41, Intel Xeon Platinum 8573C, five available
logical CPUs and 18,440,112 KiB total RAM. Native previews use Rust 1.98.1,
`demo`, the dev profile with debug information disabled, wgpu and Xvfb; the physical
adapter was not recorded. Both use `profile_preview --demo --page=conversation`
at the dimensions above. After a three-second warmup, 15 one-second observations
sample `/proc` CPU ticks and RSS, checking process liveness each time. CPU is percent
of one core; settled RSS is the median of the last five samples. Previews are pinned
to CPU 0. Task package compilers were stopped and verified stopped for sampling,
then resumed; their resident memory remained allocated. No helper children appeared.

| Idle process metric / method | Fork baseline | After | Observed delta |
| --- | ---: | ---: | ---: |
| CPU / 15 samples | 0.2621% | 0.0000% | -0.2621 percentage points |
| Peak sampled RSS | 199,294,976 B | 194,387,968 B | -4,907,008 B (-2.46%) |
| Settled RSS / last-five median | 199,294,976 B | 194,387,968 B | -4,907,008 B (-2.46%) |

These are single idle observations of development previews, not release UI throughput,
interaction latency, p95 frame time, GPU memory or a memory-leak test. No causal
CPU or memory improvement is claimed. Release UI/media performance remains unmeasured.

### Release reducer cross-check

The same 100,000-event driver runs with locked Rust 1.98.1 release builds: fat LTO,
one codegen unit, stripped symbols and line tables. Each variant uses one warmup and
five counterbalanced timed runs on CPU 0, under the compiler pause described above.
The UI crate is not part of this workload and no reducer/model/cache/workload changes
are present. Both measured sets are retained:

| Median elapsed, ms | Nightly | Fork baseline | After | Delta vs fork / nightly |
| --- | ---: | ---: | ---: | ---: |
| Initial | 93.424659 | 96.036594 | 117.556673 | +22.4% / +25.8% |
| Confirmation | 97.392611 | 112.499102 | 105.641862 | -6.1% / +8.5% |

The first run is slower. A fresh exact-baseline rebuild produced a byte-for-byte
identical executable (SHA-256 `c43070765fe3dc64823ce25ce08755e9e2b8b5447af82efcb4d9b6891cd192e9`).
The one confirmation reverses the timing direction using unchanged binaries, so
this comparison is inconclusive and the cause remains unisolated. No reducer speed
improvement is claimed. All runs retain 500 records and 401,492–401,977 estimated
bytes. Raw measured samples, milliseconds:

```text
initial/nightly: 93.424659, 87.150309, 92.363940, 100.333103, 114.112718
initial/before: 96.036594, 94.471337, 112.639644, 108.679608, 95.086756
initial/after: 118.037344, 122.820004, 114.543492, 117.556673, 110.634903
confirmation/nightly: 85.215247, 101.866661, 136.326542, 85.245207, 97.392611
confirmation/before: 99.570896, 118.921861, 112.499102, 104.064618, 119.439574
confirmation/after: 105.697624, 110.787072, 103.557331, 105.641862, 99.495916
```

### Standard release packages

Normal `cargo xtask package --format dir` passes first into empty output and again
with caches and fresh empty output. Neither package is manually repaired. Both
complete outputs have identical paths, types, modes, hashes and symlink targets
(604 entries); both installed payloads contain 280 regular files. Only the executable
changes relative to the verified exact-baseline payload. Validation covers all six
private FFmpeg/OpenH264 libraries, 19 font notices, seven pinned corresponding-source
archive hashes, the payload allowlist, desktop metadata and absent development RPATHs.
The packaged query-only launch exits successfully with development FFmpeg directories
removed from search paths; the task SDK supplies system libraries. This does not
exercise a GUI launch or owner session.

Baseline and new packages use Debian 13 x86-64, the same SDK and FFmpeg prefix,
pinned locked release settings, voice enabled and no demo/developer features.
The retained baseline executable was rehashed against its recorded manifest. The
nightly column is the official Ubuntu 26.04 AMD64 DEB payload, verified again against
release asset SHA-256 `78ad0d190e3f894b595a673642f118aede6785fb27dbcfdecb72707d3b5cdac6`.
Normalized ZIPs use sorted regular paths, fixed timestamps, 0644 comparison-entry
modes and DEFLATE level 9; these are size-comparison archives, not installers.

| Metric | Published nightly | Fork baseline | After | Delta vs fork / nightly |
| --- | ---: | ---: | ---: | ---: |
| Executable | 91,957,328 B | 95,419,720 B | 95,425,224 B | +5,504 B (+0.0058%) / +3.8% |
| Installed payload | 96,555,161 B | 140,250,352 B | 140,255,856 B | +5,504 B (+0.0039%) / +45.3% |
| Normalized ZIP | 55,682,561 B | 89,814,051 B | 89,816,286 B | +2,235 B (+0.0025%) / +61.3% |

New executable SHA-256: `36d561cda08efd18d3bac219c626a33c225ff8c9be2e11cd99dfcc3a284df2eb`.
Clean/cached normalized ZIP SHA-256: `e33b61f89dc790897329a239c3cac35bc47ed72f515abdb6e833ae3ae3852a0e`.
Nightly size deltas reflect the complete fork and different package environments;
they are not this change's isolated cost. Windows/macOS package deltas are unmeasured.

### Verification and remaining checks

At runtime source `6f821a74`, `cargo xtask check` passes 1,936 tests with zero failures
and 32 ignored, pinned formatting, strict workspace Clippy, minimal application and
policy checks. Demo-preview Clippy, login-handoff and cached-xtask workspace checks
pass. Bounded ASan fuzz smoke completes 108,579 decode and 3,375 state-transition
runs without a finding. Release replay and the 120-second core soak pass: 1,119 passes,
35,808 channel visits, 21,484,800 live inserts and five logouts in 120.097 seconds.
These are synthetic checks; the soak does not test native media or process RSS.
Native Windows/macOS application/package CI is separate from this local Linux gate.
Windows DirectWrite-style rendering, physical DPI scaling, native accessibility and
owner-controlled Discord behavior remain unverified. No dependency, license text,
SDK API or live-service behavior changes are introduced.

## Embedded media playback and social cards — October 20, 2026

Runtime source `9c6c102c96c6119f1330b3047494559ca8eefa31` follows `c93303fb`,
whose production source is `6f821a74`. The baseline production code is unchanged;
only the same synthetic embed fixture was added to its native preview. The latest
published nightly was checked again: `v1.0.0-nightly.20261017.60`,
`d9d87471f0ea2190da3f90db815e115045461703`. Later changes to this evidence do not
alter the measured runtime.

### Native layout and idle process

The [matched native pair](pr-evidence/media-embeds) uses the same synthetic post
and video with missing dimensions, 1440 × 1200 window, scale 1, Inter and dark
appearance. Light and 760 × 1000 layouts were also inspected. Reproduce with
`profile_preview --demo --page=embeds --media-embeds`; `--provider-embed` is an
additional, after-only YouTube card fixture. These fixtures never open a provider
webview, an owner account or an audio device.

Environment: Linux 6.18.55/glibc 2.41, Xeon Platinum 8573C, five available CPUs,
18,440,112 KiB total RAM, Rust 1.98.1, locked dependencies, wgpu/Xvfb and the
`demo` dev profile with debug information disabled. The physical adapter was not
recorded. Previews use `--interactive` and the dimensions above, pinned to CPU 0.
After three seconds of warmup, 15 one-second samples check liveness and read
`/proc` CPU ticks/RSS. Settled RSS is the last-five median. Task compiler trees
were verified stopped for sampling and resumed afterward; their memory remained
resident. Neither preview had helper children.

| Idle development-preview metric | Before | After | Observed delta |
| --- | ---: | ---: | ---: |
| CPU / percent of one core | 0.0000% | 0.0000% | 0.0000 percentage points |
| Peak sampled RSS | 190,353,408 B | 192,970,752 B | +2,617,344 B (+1.37%) |
| Settled RSS / last-five median | 190,353,408 B | 192,970,752 B | +2,617,344 B (+1.37%) |

This single idle sample covers the larger social image/player stages. It does not
measure release frame latency, video throughput, GPU memory, provider processes,
startup performance or settled memory after repeated playback. No speed or leak
improvement is claimed.

### Release reducer cross-check

The unchanged 100,000-event driver uses the pinned locked release profile: fat
LTO, one codegen unit, stripped symbols and line tables. One warmup and five
counterbalanced runs per variant used CPU 0 during the compiler pause above.

| Median elapsed, ms | Current nightly | Fork baseline | After | Delta vs fork / nightly |
| --- | ---: | ---: | ---: | ---: |
| Reducer replay | 137.962306 | 112.680223 | 125.228989 | +11.1% / -9.2% |

The new median is slower than the fork baseline. Both executables are byte-for-byte
identical (SHA-256 `c43070765fe3dc64823ce25ce08755e9e2b8b5447af82efcb4d9b6891cd192e9`),
so the timing difference cannot establish a code regression; its environmental
cause remains unisolated. The earlier typography timing results above remain
historical evidence. All runs retain 500 records and 401,492–401,977 estimated
bytes. Raw measured samples, milliseconds:

```text
nightly: 107.306184, 137.962306, 215.119072, 120.680036, 154.416027
before: 136.17896, 112.680223, 106.174381, 108.479018, 119.905133
after: 115.437129, 125.228989, 190.964896, 125.575998, 116.806149
```

### Normal release packages

`cargo xtask package --format dir` completed from an empty package directory after
rebuilding the current packager, then completed again with caches and fresh empty
output. Compiler and verified FFmpeg caches were retained; this is not a clean-room
rebuild of third-party libraries. No output was manually repaired. Both complete
outputs match across all 604 paths, entry types, modes, hashes and symlink targets;
the installed payload has 280 regular files. Only the executable and README change
relative to the retained exact-baseline package (each appears twice in the complete
output). The new screenshots remain development evidence outside the payload.

Validation covers all six private FFmpeg/OpenH264 libraries, corresponding source
and license assembly, 19 font notices, seven pinned source-archive hashes, the
payload allowlist, desktop metadata and absent development RPATHs. The query-only
packaged launch exits 0 with the development FFmpeg directories removed from
search paths. The task SDK still supplies system libraries; this is not a GUI
launch, WebKit runtime test or owner-session check.

Both local revisions use Debian 13 x86-64, the same SDK/FFmpeg prefix, locked Rust
1.98.1 release settings, voice enabled and no demo/developer features. The nightly
reference is the official Ubuntu 26.04 AMD64 DEB payload, release asset SHA-256
`78ad0d190e3f894b595a673642f118aede6785fb27dbcfdecb72707d3b5cdac6`. It is
the same latest asset verified in the typography comparison above. Normalized
ZIPs use sorted regular paths, fixed timestamps, 0644 comparison-entry modes and
DEFLATE 9; they compare size and are not installers.

| Metric | Current nightly | Local baseline | After | Delta vs local / nightly |
| --- | ---: | ---: | ---: | ---: |
| Executable | 91,957,328 B | 95,425,224 B | 95,469,896 B | +44,672 B (+0.047%) / +3.8% |
| Installed payload | 96,555,161 B | 140,255,856 B | 140,300,532 B | +44,676 B (+0.032%) / +45.3% |
| Normalized ZIP | 55,682,561 B | 89,816,286 B | 89,836,033 B | +19,747 B (+0.022%) / +61.3% |

New executable SHA-256: `13dba9d5e8568c11f2cb2d2802be20894f57fe46cefd791196ed99309f6c2c7c`.
Clean/cached normalized ZIP SHA-256: `d66e9174ba6586aa7733e35e31d3e307da0bd9bdbd3d6a7db62e1096cfe983b5`.
Nightly deltas include the full fork and package-environment differences; they do
not isolate this embed change. Windows/macOS package sizes are unmeasured.

### Verification and native limits

The controlled `cargo xtask check` run passes 1,947 test executions with 33 ignored
(including separately spawned child-fixture summaries), formatting, strict Clippy,
minimal application and policy checks. It uses `RUST_TEST_THREADS=2` with task
compilers paused. An initial run during parallel compilation timed out in the
existing looping-video preview test; that unchanged test passed alone in 0.54 s
and in the controlled full run. The probe fixture's accept deadline now covers
client initialization and the existing 15-second request timeout; production
deadlines were not relaxed.

Demo-preview Clippy, login-handoff, cached-workspace and six license-policy
fixtures pass. FFmpeg bundle fixtures pass (22 tests, one platform skip), as do
Linux directory-package and four Flatpak preparation fixtures. Bounded ASan fuzz
smoke completes 111,991 decode and 2,552 state-transition runs without a finding.
Release replay and the 120-second core soak pass: 1,645 passes, 52,640 channel
visits, 31,584,000 live inserts and seven logouts in 121.874 seconds. The soak
contains no renderer, media engine, network, storage or audio devices.

The ignored native provider lifecycle fixture could not run with this task's
incomplete WebKit SDK/helper and sandbox mount layout; no browser sandbox was
disabled. The supplied YouTube/VXTwitter pages and Instagram media requests are
restricted by the workspace network. No live playback success is claimed.
Native Windows/macOS/Linux application/package CI, physical rendering, provider
restrictions, system audio routing and owner-controlled Discord behavior remain
separate checks. Provider-process memory, release media throughput, interaction
latency and repeated native lifecycle memory remain unmeasured.

## Embedded playback recovery October 21, 2026

Baseline: fork main `5775101829dde47d3319228ed711f3c5ff485d5d`. Changed code:
`e1497ffe9582731ee3dee6bccfadd6525c779b03`. The later expanded-player link-confirmation
correction was not remeasured; it is outside the sampled default-card interaction.
The subsequent synthetic-clock correction changes tests only. Debian 13 x86_64,
Xeon Platinum 8573C, 4-CPU quota and
16-GiB memory limit; pinned Rust 1.98.1 and committed lockfile. Both builds use the
repository release profile: fat LTO, one codegen unit, stripped symbols and line tables.
Each checkout has separate target directories; cached external dependencies were
reused, with workspace packages rebuilt for the changed source.

Both normal `cargo xtask package --format dir` runs started with an empty output
directory and used no staging repair. A second changed run with caches and empty
output reproduced every installed file and the normalized archive. One cached speech-worker
initialization check failed; an unchanged retry passed. Its cause remains unisolated, and
the speech subsystem was not changed by this work. The installed
application passed its synthetic speech-worker launch check with the development
FFmpeg directory removed from PATH and library search paths; all six private media
libraries resolved from the installed payload, with no development FFmpeg RPATH.

| Metric | Baseline | Changed | Delta |
| --- | ---: | ---: | ---: |
| Executable, bytes | 95,997,128 | 96,030,920 | +33,792 (+0.035%) |
| Installed regular files, bytes | 224,747,738 | 224,781,530 | +33,792 (+0.015%) |
| Normalized ZIP, bytes | 154,676,801 | 154,695,765 | +18,964 (+0.012%) |

ZIP uses sorted payload paths, fixed timestamps and deflate level 6 on the same
Python/zlib installation. Shared libraries, corresponding source and license files
are included. These are local Linux directory-package comparisons, not installer
or Windows/macOS size measurements.

Native UI measurements use release `profile_preview` with
`--no-default-features --features demo`, forced OpenGL/software rendering under
Xvfb, dark appearance, display scale 1 and a 1440 × 1100 window. Both use
`--demo --page=embeds --media-embeds --provider-embed --interactive`. After a
two-second warmup, the same poster is clicked; sampling starts 0.8 seconds later.
One warmup run is discarded, followed by five measured runs per revision in
interleaved order, with 100 samples at 100-ms intervals. Preview liveness is checked
after warmup and before every sample; an exited preview invalidates the run and
identifies its log. Compiler jobs have finished before sampling.

| Median of five runs | Baseline | Changed | Delta |
| --- | ---: | ---: | ---: |
| Sampled parent-process peak RSS, MiB | 142.75 | 142.47 | -0.28 MiB (-0.20%) |
| Settled parent RSS, last two seconds, MiB | 142.75 | 142.47 | -0.28 MiB (-0.20%) |
| CPU, percent of one core over sample interval | 0.00 | 0.00 | +0.00 percentage points |

RSS comes from `/proc/PID/status`; CPU uses process user/system clock ticks divided
by elapsed sample time. These measure the affected host UI states: the old modal
and the new card with visible chat. They do not isolate renderer cost or establish
a general speed improvement. The 0.28-MiB RSS difference is within run variation.
CPU medians are quantized at the 10-ms process-clock resolution; individual runs
ranged from 0 to 1.18% of one core. This synthetic preview never creates a provider view;
no helper processes were observed. Actual provider-process RSS, GPU memory, decoded
media throughput and frame/interaction latency remain unmeasured.

The reported public Twitter clip also reproduces the native demux failure: the
baseline stops at three decoded pictures, with no audio and neither track at EOF.
The changed bridge delivers all 274 pictures and 441,344 audio frames, with both
tracks reaching EOF. This uses a bounded local copy, anonymous Serein-identified
HTTP requests and a virtual presentation clock; it is functional Linux software
decode evidence, not wall-clock throughput or proof of Windows/Discord playback.
No public clip or service traffic is committed. Generated fixtures additionally
exercise normal, audio-first and video-first packet layouts through the production
player and synthetic audio sink, plus byte/packet exhaustion, seek and cancellation.
The synthetic sink catches up delayed timer wakeups in bounded bursts. Its layout
test fails after five seconds without presentation progress and caps each case at
40 seconds, with the production startup deadline retained. It does not impose a
native real-time throughput requirement on shared CI machines.

Formatting, strict Clippy, workspace/minimal-build/policy checks, offline provider
API, login-handoff/workspace and packaging fixtures, replay, the 120-second soak and
bounded pinned fuzz checks passed. The matched synthetic screenshots are in
[embed-playback](pr-evidence/embed-playback). Native Windows/macOS clipping, focus,
fullscreen, scrolling and provider interoperability remain owner-controlled checks.
The Linux native WebKit fixture cannot locate its helper in the staged SDK here;
no browser sandbox was disabled. YouTube is blocked by the workspace proxy, and
reachable VXTwitter/OGInstagram pages alone do not establish playback.


## Conversation navigation and time preferences (October 21, 2026)

The frozen fork baseline is `bc35a629eeed1d80708baf66152bbb0a91005989`.
Implementation source is `a8b118d1c8944c0df10f5c73c561204933e628c6`.
The published upstream nightly is `v1.0.0-nightly.20261017.60`, pinned to
`d9d87471f0ea2190da3f90db815e115045461703`; it is distinct from upstream main.
Detailed methods, synthetic captures and raw repetitions are in
[conversation-qol](pr-evidence/conversation-qol).

Current-source CI compares the same core workloads within each OS job: Rust 1.98.1,
release O3, thin LTO, one codegen unit, line-table debug information, stripped symbols
and abort-on-panic. Each revision gets one warmup and five measured repetitions in
rotated order, with CPU affinity on Linux/Windows. These measure reducer elapsed time,
not UI presentation, input latency or live Discord behavior.

| CI environment / median elapsed | Upstream nightly | Fork baseline | Changed source | Change vs fork |
| --- | ---: | ---: | ---: | ---: |
| Linux, 100,000-event reducer | 56.61 ms | 59.81 ms | 63.86 ms | +6.8% |
| Linux, 10,000-event wide-channel reducer | 102.91 ms | 97.29 ms | 102.20 ms | +5.0% |
| Windows, 100,000-event reducer | 105.83 ms | 111.38 ms | 121.02 ms | +8.7% |
| Windows, 10,000-event wide-channel reducer | 170.01 ms | 171.75 ms | 171.49 ms | -0.1% |
| macOS, current source | Unavailable | Unavailable | Jobs cancelled before starting | Unmeasured |

Linux ran on Ubuntu/glibc 2.39 with four logical CPUs; Windows ran on Server 2025
(10.0.26100), AMD64 Family 25 Model 1, with four logical CPUs. Raw ranges and samples
are retained with their environment and source revision. In these CI runs the reducer medians remain slower
than fork main and nightly; the remaining cost is not isolated. Windows wide-channel
ranges overlap; Linux wide-channel runs are slower with disjoint observed ranges. Earlier
macOS measurements retain their original revision label and are not current-source results.
No overall application speedup is claimed.


Normal local Linux packaging uses the standard fat-LTO release profile, including
voice, private media libraries, corresponding source and notices. Both revisions
were packaged from an empty output directory and repeated with caches; every
staged file and mode matched its clean run (578 baseline files, 592 changed files).
No manual staging repair was performed. The installed packages passed the encoder
query and synthetic speech-worker launch with development FFmpeg/private-library
paths removed; all six private media libraries resolved inside the installed payload.
Release RPATH contains only the two existing `$ORIGIN` locations.

| Linux package size | Fork baseline | Changed source | Delta |
| --- | ---: | ---: | ---: |
| Executable | 96,030,920 B | 96,789,832 B | +758,912 B (+0.790%) |
| Installed regular files | 224,781,530 B | 225,548,851 B | +767,321 B (+0.341%) |
| Normalized ZIP | 154,695,813 B | 154,975,101 B | +279,288 B (+0.181%) |

Installed payloads contain 296 and 303 regular files respectively. The normalized
ZIP uses sorted paths, fixed timestamps/modes and deflate level 6 on the same
Python/zlib installation. The bundled IANA 2026e dataset contains 206,788 raw TZif
bytes; total package growth also includes feature code and notices, so the table
does not isolate timezone overhead. Windows/macOS package-size comparisons are
unavailable in this workspace.


Native UI comparisons use release O3/one codegen unit with a thin-LTO override on
both example executables, `--no-default-features --features demo`, dark appearance,
1120 × 760, scale 1 and the same synthetic data. The baseline has only the common
example fixture/instrumentation overlay; its production sources remain unchanged.
The host is Linux 6.18.55/glibc 2.41, Xeon Platinum 8573C, 18.88 GB host RAM with
a 16-GiB process-group limit. The process and sampler inherit one logical CPU.
Both use wgpu's GL backend on llvmpipe LLVM 19.1.7 / Mesa 25.0.7-2+deb13u1.

One warmup and five interleaved measured repetitions per build/state use three
seconds of startup warmup, five seconds of CPU/RSS sampling at 100 ms intervals,
and two seconds of settling. The table reports medians of per-run measurements.
CPU is process user+system time over elapsed sample time. Peak RSS is sampled after
warmup, not lifetime peak; settled RSS is the median during the two settling seconds.
The at-rest demo states include their normal demo loading/animation work; they are
not measurements of an idle connected account. Active callback samples measure
`MessagingUi::show` (including possible egui passes), not presented frames or physical
input latency. Active samples request at most 100-ms repaints; at-rest samples omit
callback instrumentation. No provider/helper process was started or observed.

| Native synthetic metric / median of five | Fork baseline | Changed source | Delta |
| --- | ---: | ---: | ---: |
| Conversation, unrelated revision churn, CPU (% of one core) | 83.88 | 85.79 | +1.91 percentage points |
| Conversation, unrelated revision churn, sampled peak RSS (MiB) | 140.44 | 141.23 | +0.80 MiB |
| Conversation, unrelated revision churn, settled RSS (MiB) | 140.44 | 141.23 | +0.80 MiB |
| Conversation, unrelated revision churn, UI callback p50 (ms) | 1.546 | 1.551 | +0.3% |
| Conversation, unrelated revision churn, UI callback p95 (ms) | 4.655 | 3.989 | -14.3% |
| Conversation at rest, CPU (% of one core) | 9.89 | 10.86 | +0.97 percentage points |
| Conversation at rest, sampled peak RSS (MiB) | 139.95 | 140.22 | +0.27 MiB |
| Conversation at rest, settled RSS (MiB) | 139.95 | 140.22 | +0.27 MiB |
| Language & Time at rest, CPU (% of one core) | 4.58 | 19.77 | +15.19 percentage points |
| Language & Time at rest, sampled peak RSS (MiB) | 143.18 | 143.83 | +0.65 MiB |
| Language & Time at rest, settled RSS (MiB) | 143.18 | 143.83 | +0.65 MiB |
| GIF scrolling, CPU (% of one core) | 87.36 | 88.55 | +1.19 percentage points |
| GIF scrolling, sampled peak RSS (MiB) | 145.24 | 145.77 | +0.53 MiB |
| GIF scrolling, settled RSS (MiB) | 145.24 | 145.64 | +0.40 MiB |
| GIF scrolling, UI callback p50 (ms) | 1.280 | 1.185 | -7.4% |
| GIF scrolling, UI callback p95 (ms) | 3.247 | 4.024 | +23.9% |

Results are mixed. Conversation callback p50 is essentially unchanged; p95 is lower,
with overlapping per-run ranges. CPU/RSS increase in several states. Language & Time
at-rest CPU rises substantially on this software renderer; the remaining cause is
not isolated. This is a recorded cost, not an overall application speedup. Actual
provider CPU/RSS, GPU memory, presented-frame/input latency, native cache occupancy
and outstanding full-application tasks remain unmeasured. Core row/byte bounds and
request/cancellation behavior are covered by production-path regressions and replay.
One GIF attempt failed to locate a visible preview window; only that workload was
repeated, and the final table contains only complete successful repetitions.

Local standard fat-LTO replay uses the same frozen revisions on one logical CPU,
one warmup and five rotated repetitions per revision. These are a separate
environment/profile from CI; the different results must not be pooled.

| Local core median | Upstream nightly | Fork baseline | Changed source |
| --- | ---: | ---: | ---: |
| 100,000-event reducer, ms | 115.01 | 125.07 | 122.20 |
| 10,000-event wide-channel reducer, ms | 223.84 | 178.78 | 177.77 |
| Core selection median, µs | 0.284 | 0.307 | 0.325 |

An isolated snapshot microbenchmark compiles the exact production helper against
`model::Gif` and its Vec field, comparing the previous URL-clone expression with
100 unchanged favorites over 10,000 calls. One warmup and five alternating-order
repetitions give 38.97 ms before and 4.23 ms after
(-89.2%). This supports retaining that allocation change; it is not whole-UI throughput.
The production UI's ignored `favorite_snapshot_benchmark` provides the equivalent
workload. Source/binary hashes and the isolated harness method are recorded in the JSON.

The 120-second functional release soak passed: 55,072 conversation visits and seven
logout cycles, with row/byte pressure inside shipping bounds. Its iteration count
is not a performance comparison. Deterministic scrolling traces retain movement
and deadline assertions at 30/60/144/240 Hz; they do not measure physical input latency.

Current-source native application/package CI passed on Linux and Windows x64/ARM64.
The macOS jobs were cancelled before starting. Physical wheel/touchpad behavior, live
OS timezone changes and live Discord permissions/navigation remain owner-controlled
checks. Windows/macOS GUI and memory comparisons are unavailable locally.

## Message menus and performance follow-up (October 22, 2026)

This follow-up retains the conversation features and replaces the measured runtime
with `9c42d04be06e08a9d458a05de90c78d31ee478c3`. Comparisons use the frozen starting
PR `3179696b`, fork baseline `bc35a629` and published upstream nightly
`v1.0.0-nightly.20261022.61` (`c2842aa7`). The preceding section remains historical;
its samples are not pooled with these new runs. Raw repetitions, hashes, methods
and CI snapshots are in
[`followup_2026_10_22`](pr-evidence/conversation-qol/measurements.json), with
[matched menu captures](pr-evidence/conversation-qol/README.md).

### Changes supported by profiling

Reply previews maintain cached payload-byte totals on mutation. Resident-budget
enforcement calculates totals once and subtracts evictions, retaining enforcement
after operations that grow state. Empty reply state avoids invalidation scans.
Clock formatting reuses one immutable frame snapshot; Language & Time caches its
label/example and schedules the next displayed minute instead of each second.
Its page-specific timer is absent while hidden or minimized.

A bounded ptrace sample profile of the wide-channel workload attributed 2,662 of
2,977 executable leaf samples to `apply_read_state`. Acknowledgements now reuse a
validated channel lookup instead of rescanning the roster. Messages and patches
also skip edit-only timeline lookups when there is no pending optimistic edit.
The profiler used 40 controlled children, 2-ms instruction-pointer samples and
inline symbolization. Instrumentation distorts duration: these profiles identify
work, while the uninstrumented comparisons below establish elapsed-time results.

### Separate native CI core comparisons

All four revisions run the identical benchmark source in each job, with Rust
1.98.1, committed lockfiles, O3, thin LTO, one codegen unit, abort-on-panic and
symbol stripping. Each binary is built once, followed by one warmup and five
rotated measured repetitions. Linux and Windows use one logical CPU; macOS
affinity is unavailable and uses its scheduler. Environments are Linux Azure
x86-64, Windows Server 2025 x86-64 and macOS 26.6.2 ARM64 respectively; full host
metadata and ranges are retained in the JSON. Do not compare absolute timings
across these different hosts or pool results from separate CI runs.

| Core median, ms | Upstream nightly | Fork baseline | Starting PR | Final runtime | Final vs fork |
| --- | ---: | ---: | ---: | ---: | ---: |
| Linux, 100,000-event reducer | 73.613 | 75.994 | 82.314 | 75.958 | −0.05% |
| Windows, 100,000-event reducer | 81.883 | 87.913 | 90.288 | 82.328 | −6.35% |
| macOS, 100,000-event reducer | 68.193 | 67.954 | 72.854 | 64.821 | −4.61% |
| Linux, 10,000-event wide-channel reducer | 118.632 | 111.770 | 116.086 | 8.568 | −92.33% |
| Windows, 10,000-event wide-channel reducer | 72.555 | 75.279 | 51.658 | 8.879 | −88.21% |
| macOS, 10,000-event wide-channel reducer | 120.783 | 132.142 | 103.203 | 7.874 | −94.04% |

The reducer medians meet the plan's maximum 3% cost against their matched fork
baselines. Final ordinary-reducer ranges are 75.570–92.202 ms (Linux),
82.049–101.647 ms (Windows) and 59.279–69.103 ms (macOS). The large wide-channel
change is specific to repeated acknowledgements in a large roster; it is not an
overall application speedup. Earlier independently matched follow-ups also met
the ordinary-reducer target; their results are not combined with this table.

The separate local fat-LTO comparison uses the same four source revisions, one
logical CPU, one warmup and five rotated repetitions. Its ordinary-reducer
medians are 91.750 ms nightly, 92.343 ms fork, 101.129 ms starting PR and
87.711 ms final. Wide-channel medians are 128.865, 137.743, 133.516 and 9.130 ms.
Core-selection medians are 0.286, 0.306, 0.323 and 0.306 µs respectively. These
core timings exclude native presentation, media and network latency. The final
local binary was reused after UI-only fixes; its core/model/benchmark sources
are identical to the measured runtime revision.

### Native UI, memory and remaining variability

The three Linux previews use O3, one codegen unit and the same thin-LTO override
with `--no-default-features --features demo`, Inter fonts, dark appearance,
1120 × 760 and scale 1. The host remains Linux 6.18.55/glibc 2.41 on Xeon
Platinum 8573C, with a 16-GiB process-group limit. All inherit one logical CPU,
using wgpu GL on llvmpipe LLVM 19.1.7 / Mesa 25.0.7-2+deb13u1. Verified baseline
artifacts were reused. Compilation and packaging had stopped before sampling.

Each workload has one warmup and five rotated measured repetitions, each with
three seconds of startup warmup, five seconds of 100-ms CPU/RSS samples and two
seconds of settling. CPU is process user+system time as a percentage of one
core. Peak RSS is sampled after warmup, including settling; settled RSS is its
two-second median. At-rest runs omit callback instrumentation. GIF runs measure
`MessagingUi::show` callbacks and inject the same alternating wheel schedule;
callback counts vary with scheduling. Percentiles below are medians of per-run
percentiles, not pooled samples or presented-frame/input latency.

| Native synthetic median | Fork baseline | Starting PR | Final runtime |
| --- | ---: | ---: | ---: |
| Language & Time at rest, CPU (% of one core) | 4.16 | 11.35 | 3.75 |
| Language & Time, sampled peak/settled RSS (MiB) | 140.68 | 141.31 | 142.89 |
| GIF scrolling, CPU (% of one core) | 90.90 | 90.52 | 90.98 |
| GIF scrolling, sampled peak/settled RSS (MiB) | 143.98 | 143.42 | 145.26 |
| GIF scrolling, callback p50 (ms) | 0.941 | 1.170 | 1.050 |
| GIF scrolling, callback p95 (ms) | 2.062 | 3.137 | 2.076 |

Settings CPU meets the target of at most two percentage points above fork
baseline: the observed difference is −0.41 points. The demo can still repaint
for fixture/loading/animation activity; removing the page timer does not remove
those sources. All repetitions remain in the data, including a 27.65% final
settings sample and a 29.66% starting-PR sample. No idle connected-account CPU
claim is made.

GIF p50 remains +0.110 ms (+11.7%) against fork. Its p95 median is +0.014 ms
(+0.7%), with broad overlapping ranges: fork 1.560–3.175 ms, starting PR
2.063–4.690 ms and final 1.412–8.173 ms. That final outlier is retained. These
samples establish neither a broad GIF improvement nor an isolated cause for
the earlier p95 regression. No speculative GIF optimization was retained.

The reproducible sampler also supports a separate glibc allocation diagnostic.
A quiet diagnostic observed 122/114/111 callbacks for fork/starting/final and
714.6/656.1/651.4 MB of requested allocation traffic over the sampling window.
Traffic per callback is similar; unequal counts and differing callback/sample
windows prevent interpreting raw totals as an allocation improvement. An earlier
instrumented attempt under other task activity had especially unequal counts
and is retained separately. Interposition covers malloc/calloc/realloc/free,
including native libraries, but excludes aligned/direct allocation paths.
Requested bytes are not live heap, and instrumented CPU is not acceptance data.

The fixtures started no provider/helper process; observed helper RSS was zero.
Provider/media-worker CPU/RSS, native GPU memory, physical input latency and
Windows/macOS GUI comparisons remain unmeasured. Existing cache/queue/lifecycle
bounds remain covered by production-path tests and the functional soak.

### Normal Linux packages and functional checks

All three package records use the standard fat-LTO release profile, normal
directory packaging and the same normalized ZIP procedure: sorted paths, fixed
timestamps/modes and deflate level 6 on the same Python/zlib installation.

| Linux package size | Fork baseline | Starting PR | Final runtime | Final vs fork |
| --- | ---: | ---: | ---: | ---: |
| Executable | 96,030,920 B | 96,789,832 B | 97,123,400 B | +1,092,480 B (+1.138%) |
| Installed regular files | 224,781,530 B | 225,548,851 B | 225,882,419 B | +1,100,889 B (+0.490%) |
| Normalized ZIP | 154,695,813 B | 154,975,101 B | 155,120,426 B | +424,613 B (+0.274%) |

Current `xtask` was rebuilt before packaging into empty task-owned output, then
reused for the cached run with empty output. All 592 staged files and modes
matched, with 303 installed regular files. Both launch probes resolved the six
private media libraries from the installed tree using `$ORIGIN`, with development
FFmpeg paths removed and no manual staging repair. Encoder query and the packaged
LavaSRv2 synthetic worker passed; neither exercises physical hardware or the new
platform message-speech engines. Windows/macOS package-size comparisons remain
unavailable.

The local 120-second functional release soak passed with 87,808 conversation
visits, 52,684,800 live inserts and 11 logout cycles, within shipping row/byte
bounds. It ran alongside package compilation: its iteration count is not a
throughput comparison. Full workspace gates, focused menu/clock/cache tests,
replay, fuzz, licenses and packaging fixtures are recorded in the evidence.
Native CI workspace gates passed on all supported desktop platforms at the
frozen runtime source; package jobs were still running at the recorded snapshot.
A separate macOS Nix link failure refers to `___isPlatformVersionAtLeast` in
unchanged capture code and remains disclosed. Real platform speech, live reports,
live Discord permissions, physical input/displays and OS timezone changes remain
owner-controlled checks. No live report was submitted during validation.

## Account reactions and nickname follow-up (October 22, 2026)

The runtime source is `26e87fd5908e15dc8f11efd04e57030efb70c731`, starting
from `dd6d65d7`. Verified source-equivalent `9c42d04b` UI/application binaries
and the artifact with the unchanged `e9b87fb7` client-core tree supply the before measurements;
they are frozen independently of the new outputs. The original fork baseline
remains `bc35a629`. The latest published upstream nightly checked on October 22
is `v1.0.0-nightly.20261022.61`, commit `c2842aa7`; its frozen benchmark is
reused, with the same benchmark source blob as the other core comparisons.

This pass reads account reaction ranking, shares it between reaction bars,
enlarges reaction/composer artwork and fixes visible-author nickname lookup
and cache refresh. Account ranking has a 128-key / 32-KiB session projection
and no settings writes. Opt-in mip filtering now produces and samples real
GPU mip levels. Emoji textures count every level within the existing cache
ceiling. The separate fixed Unicode atlas adds approximately one third of
its base GPU payload; improved sampling is a quality/resource tradeoff.
Ordinary avatar/media textures keep their prior sampling path.

Measurements use Rust 1.98.1 and the unchanged lockfile. Standard application
packages and core replay retain fat LTO; both native previews use matching
release/demo/no-default-feature settings with the same thin-LTO override.
The Linux host reports an Intel Xeon Platinum 8573C, 18,882,674,688 bytes RAM,
a 16-GiB container limit and a four-CPU quota. Native sampling uses one inherited
logical CPU, Linux X11/OpenGL llvmpipe, a 1120 × 760 viewport and scale 1.
Active samples log LLVM 19.1.7 / Mesa 25.0.7; idle uses the same configuration
without the adapter log. The separate GPU regression uses software Vulkan.
Each comparison has one warmup and five rotated repetitions; no task compilation,
fuzz or functional soak runs during the timed comparisons. Native processes
warm for three seconds, prepare the same hover target for another half second,
sample CPU/RSS every 100 ms for five seconds and settle
for two seconds. Callback timings describe `MessagingUi::show`, not presented
frames or physical input latency. Offline fixtures do not establish connected
account behavior or provider/media-worker performance.

The timed conversation fixture enables `--reaction-history` and hovers the same
message on both builds. The old preview accepts but ignores this new fixture flag;
only the after build imports the synthetic account ranking. This includes its
bounded projection and shortcode lookup in the sampled process, rather than
measuring only the default toolbar. It is not live account data.

### Current local measurements

| Synthetic native UI metric | Starting runtime | New runtime | Delta |
| --- | ---: | ---: | ---: |
| Hover CPU, % of one core | 6.13 | 6.48 | +0.35 points |
| Hover settled RSS, MiB | 139.01 | 147.49 | +8.48 MiB |
| Active timeline CPU, % of one core | 90.67 | 91.15 | +0.48 points |
| Active callback p50, ms | 1.062 | 1.047 | -0.015 ms |
| Active callback p95, ms | 2.006 | 1.422 | -0.584 ms |

The observed hover CPU difference is small. Higher sampled RSS includes the new
filtered artwork and account-ranking fixture in the software renderer. It does
not measure dedicated hardware VRAM. Helper RSS was zero; no provider player
was started. Callback counts differ (median 172 versus 183); these are callback observations,
not a presented-frame or whole-application speedup claim. All five repetitions
are retained, including variable p95 samples.

The before callback-p95 range was 1.274–3.843 ms.
The after callback-p95 range was 1.372–1.898 ms.

| Local fat-LTO core workload, ms | Fork baseline | Starting runtime | Current nightly | New runtime |
| --- | ---: | ---: | ---: | ---: |
| Reducer | 84.066 | 83.196 | 76.775 | 81.750 |
| Wide channels | 104.058 | 9.077 | 106.563 | 8.916 |

The large wide-channel difference against fork/nightly comes from the earlier
resident-accounting optimization retained in this PR. The current follow-up
has a small local core difference against its starting runtime; no new reducer
speedup is attributed to artwork or nickname changes. Local fat-LTO results
remain separate from the thin-LTO native CI comparisons.

### Current native CI core comparisons

These are separate hosted-runner comparisons at `26e87fd5`, Rust 1.98.1,
matching opt-level 3, thin LTO, one codegen unit, stripped symbols and aborting
panics. Each platform uses one warmup and five rotated repetitions; results
are not pooled across hosts or with the local fat-LTO runs. Fork means
`bc35a629`; starting PR means `3179696b`. Nightly remains the published `.61`
revision above.

| Platform / core workload, ms | Nightly | Fork | Starting PR | Current runtime |
| --- | ---: | ---: | ---: | ---: |
| Linux reducer | 78.284 | 82.515 | 92.499 | 83.853 |
| Linux wide channels | 217.782 | 194.468 | 218.938 | 9.125 |
| Windows reducer | 107.123 | 110.910 | 122.540 | 112.313 |
| Windows wide channels | 230.248 | 234.572 | 229.167 | 11.738 |
| macOS reducer | 83.257 | 80.580 | 91.730 | 78.434 |
| macOS wide channels | 138.606 | 131.837 | 136.137 | 9.188 |

Reducer medians versus fork are +1.62% on Linux, +1.26% on Windows and
−2.66% on macOS. These meet the earlier 3% target against fork in this run.
The macOS reducer ranges overlap broadly (fork 63.63–93.48 ms, current
64.65–90.00 ms); its median is not proof of a platform speedup. Nightly
comparisons remain visible rather than being replaced with upstream main.
The full native application/package jobs are independent of these core-only
results. Source comparison run: <https://github.com/MCShotty/Serein/actions/runs/39854776241>.

### Normal package verification

Both runs use the normal directory packager, standard fat-LTO profile and
the same sorted-path, fixed-metadata, deflate-level-6 ZIP measurement method
as the frozen baseline.

| Linux package size | Starting runtime | New runtime | Delta |
| --- | ---: | ---: | ---: |
| Executable | 97,123,400 B | 97,186,568 B | +63,168 B (+0.065%) |
| Installed payload | 225,882,419 B | 225,945,587 B | +63,168 B (+0.028%) |
| Normalized ZIP | 155,120,426 B | 155,151,237 B | +30,811 B (+0.020%) |

The first release link reached the 16-GiB container limit while generated
RAM-backed compiler caches occupied approximately 6.3 GiB. Only verified
task-owned generated caches were removed; all seven frozen comparison binaries
were rehashed and retained. The normal packager was then rerun from empty output.
The successful clean/cached packages match all 592 staged files and modes,
with 303 installed regular files. No manual staging repair was used. Both
launch probes resolve the six private media libraries from the installed tree
with development FFmpeg paths removed. Encoder query and the packaged synthetic
LavaSRv2 worker pass; these do not exercise a microphone or platform message TTS.

Runtime-source native workspace gates passed on Linux, macOS and Windows
x64/ARM64; source packaging/Nix jobs were still running at the recorded CI
snapshot. The first source CI fuzz job stopped at separate-workspace formatting;
the line wrapping is corrected and its formatting check passes locally. The
functional fuzz runs passed before this whitespace-only correction. Final-head
CI is reported on the PR, separately from the frozen runtime and measurement
revisions. Live Discord account settings/nickname updates, physical DPI/input,
real speech engines and Windows/macOS GUI CPU/RSS or comparative package-size
measurements remain unverified or unavailable. No overall app speedup is claimed.

## Server controls and moderation — October 23, 2026

Compared the frozen starting PR #20 head `c7968095aadebe16078f698e8211a470b66caafd`
with replay runtime `5dcaa97027a3f5498c398f3fbbee9fd90699fcb1` and current upstream
nightly `v1.0.0-nightly.20261022.61` (`c2842aa762ca633e0ed4ba986c460a59ce6435fe`).
The starting application's runtime source is unchanged from `26e87fd5`; its frozen
binaries were reused and rehashed. Replay results are pinned separately from the
native preview and package results below. The [evidence summary](pr-evidence/server-controls/README.md)
and [compact measurements](pr-evidence/server-controls/measurements.json) retain
the source-tree identifiers, binary hashes, primary samples and confirmation batches.
Historical measurements above retain their original revisions and environments.

### Build and measurement method

All comparison builds use pinned Rust 1.98.1, their committed lockfiles and separate
source/target directories. Core replay uses its default features and the standard
release profile: opt-level 3, fat LTO, one codegen unit, stripped symbols and line-table
debug information. Native previews use matching release settings with the explicit
thin-LTO override on the final preview crate, `demo` enabled and application default features disabled. The normal
application package retains fat LTO, includes voice and disables development data.
Build commands, run separately in each pinned source tree, are:

```sh
cargo +1.98.1 build --locked --release -p replay-bench
cargo +1.98.1 rustc --locked --release -p serein --example profile_preview --no-default-features --features demo -- -C lto=thin
cargo +1.98.1 xtask package --format dir
```

The Linux 6.18.55 x86-64/glibc 2.41 container reports an Intel Xeon Platinum 8573C,
five available logical CPUs, a four-CPU quota and a 16-GiB memory limit. No builds
ran during the timed comparisons. The primary replay series pins the sampler and
children to logical CPU 0, runs one warmup per revision/workload, then five measured
repetitions with revision order rotated. Timed invocations use the frozen binaries:

```sh
taskset -c 0 "$replay_binary"
taskset -c 0 "$replay_binary" --wide-channels
taskset -c 0 "$replay_binary" --navigation
```

The first workload applies 100,000 synthetic events. The wide-channel workload
applies 10,000 messages into the last of 20,001 channels; its inner timer excludes
channel setup. Navigation measures 10,000 selections across three 50-message
conversations. These are core-state timings, not native frames, input latency,
connected-account traffic or process memory. Navigation p95 below is the median
of five runs' reported p95 values, rather than a pooled percentile.

### Local replay results

| Core workload / median | Starting PR | Current nightly | New runtime | New vs starting |
| --- | ---: | ---: | ---: | ---: |
| Reducer, ms | 98.057 | 99.669 | 103.468 | +5.5% |
| Wide-channel reducer, ms | 9.472 | 129.845 | 9.637 | +1.7% |
| Selection, µs | 0.309 | 0.287 | 0.313 | +1.3% |
| Selection p95, µs | 0.357 | 0.319 | 0.350 | −2.0% |

The primary reducer result exceeds the 3% target against the starting PR. Its
run ranges are 87.274–131.392 ms before and 87.427–139.876 ms after; nightly is
78.819–142.435 ms. Overlapping ranges do not demonstrate that the target passes.
The large wide-channel difference against nightly was already present in the
starting runtime and is not a new administration-workflow improvement.

Two longer confirmation series use the same frozen binaries on logical CPU 2.
After one warmup, each measures five rotated batches of 20 fresh processes per
revision/workload. The wall-time series averages the core-reported elapsed times
within each batch and takes the median of those five means. A separate diagnostic
also differences `resource.getrusage(RUSAGE_CHILDREN)` around each completed child
to obtain user-plus-system process CPU. That includes startup, setup and logout,
so it is not a replacement for the inner reducer/wide-channel timers.

| Reducer confirmation / median | Starting PR | Current nightly | New runtime | New vs starting |
| --- | ---: | ---: | ---: | ---: |
| Core wall time, ms per process | 95.403 | 89.147 | 98.054 | +2.8% |
| Separate whole-process CPU diagnostic, ms per process | 118.188 | 79.834 | 89.240 | −24.5% |

The diagnostic reverses the direction of the primary result, while longer
wide-channel batch means also vary substantially: starting-batch means range
9.236–32.296 ms and new means 9.449–17.639 ms. These inconsistent shared-host
results do not isolate a speedup or resolve the primary slowdown. The **3%
reducer target remains unestablished**; separately labelled native-platform
comparisons are needed before claiming it is demonstrated. No overall application
speedup is claimed.

### Native UI and packages

Final native preview and normal package source: `2ee9314232945685c4dc81dc6cac1fc84c4f179a`. The unchanged core replay above stays pinned to `5dcaa970`.

Native comparisons use `scripts/profile-native-ui.py` on Linux X11/Xvfb, llvmpipe (LLVM 19.1.7/Mesa 25.0.7, GL), 1120 × 760 and scale 1. Each fresh process gets three seconds of warmup, five seconds of sampling at 100 ms intervals and two seconds of settling. There is one warmup run and five measured repetitions, with build order rotated; sampler and children inherit logical CPU 0. No builds ran during these samples.

The common Members fixture has eight members; the new build adds identity/filter metadata. Search alternates `Avery` and an empty query once per second through native text input. Active comparisons opt into the bounded preview callback collector, which requests periodic repaint; their CPU figures describe that instrumented workload, not ordinary idle CPU. Callback p50/p95 values are medians of five per-run percentiles for `MessagingUi::show`, excluding rendering/presentation and end-to-end input or service latency. Peak RSS means the maximum sampled after warmup, not a lifetime high-water mark; settled RSS is the median of the two-second settling window.

Only the common idle view was sampled against the pinned nightly; its preview lacks the callback instrumentation used for active comparisons. Mod View and Browse Channels have no equivalent starting-build fixture and are characterized separately. No helper-process RSS was observed in these sampling windows. Live cache occupancy, outstanding transport tasks, large-server search latency and physical input remain unmeasured.

| Native demo / median | Starting PR | Pinned nightly | New | New minus starting |
| --- | ---: | ---: | ---: | ---: |
| Members idle CPU, % of one core | 4.92 | 4.96 | 5.14 | +0.23 points |
| Members sampled peak RSS, MiB | 150.14 | 141.31 | 151.84 | +1.70 MiB |
| Members settled RSS, MiB | 150.14 | 141.31 | 151.84 | +1.70 MiB |
| Member search callback p50, ms | 1.80 | — | 1.72 | -0.07 ms |
| Member search callback p95, ms | 3.72 | — | 4.07 | +0.35 ms |
| Instrumented search CPU, % of one core | 90.24 | — | 90.16 | -0.08 points |
| Role editor callback p95, ms | 2.86 | — | 3.47 | +0.61 ms |
| Instrumented role editor CPU, % of one core | 72.97 | — | 75.99 | +3.02 points |

Members idle CPU increased by 0.23 percentage points, within the two-point target. Active callback tails increased: these costs are retained in the table and are not an overall speedup. One role-editor batch failed its callback-coverage gate: a live baseline process produced only two callbacks, ending at 3.621 seconds of the internal 3–8-second window. Its cause was not isolated. The entire batch was repeated once, with five valid samples for both builds; the rejected attempt is retained in the compact evidence. No per-sample substitution was used.

| New-only panel / median | CPU, % of one core | Sampled peak RSS, MiB | Settled RSS, MiB |
| --- | ---: | ---: | ---: |
| Mod View | 13.74 | 150.46 | 150.46 |
| Browse Channels | 15.17 | 150.87 | 150.87 |

Both package runs used the normal `cargo xtask package --format dir` command, empty output directories and the standard fat-LTO production profile. Contents, modes and normalized deflate-6 archives matched. Both launch probes removed development FFmpeg paths, resolved private media libraries inside the installed tree, and passed encoder-query and synthetic LavaSRv2 worker checks. No microphone or account was used and no manual staging repair was applied.

| Linux package size | Starting runtime | Final runtime | Delta |
| --- | ---: | ---: | ---: |
| Executable | 97,186,568 B | 98,406,920 B | +1,220,352 B (+1.256%) |
| Installed payload | 225,945,587 B | 227,165,939 B | +1,220,352 B (+0.540%) |
| Normalized ZIP | 155,151,237 B | 155,647,310 B | +496,073 B (+0.320%) |

An intermediate `8a020e14` release attempt was killed at the 16-GiB container limit while RAM-backed test caches remained resident. No package or size measurement is claimed from that attempt. The final full gate passed, verified task-owned generated check caches were retired, and the normal final clean/cached package commands then completed. Frozen baselines, sources and license material were preserved.

Windows/macOS GUI CPU/RSS and comparative package sizes, physical input/accessibility and live Discord acceptance remain unavailable locally. Platform CI results are reported separately on the PR; builds and synthetic fixtures do not establish live service interoperability. The primary reducer slowdown above remains unresolved by the inconsistent shared-host measurements.

## Windows capture overhead — October 23, 2026

Starting installer revision: `30d3437ffe6a3b5d06ecd2ae67c7d2f1c76cd28b`.
The owner reported 250 → 115 game FPS during a 1440p60 SDR share. This is an
observation, not a controlled measurement of capture or encoding cost. Codec,
bitrate policy, vendor presets, lookahead and pre-analysis remain unchanged.

The implementation reuses bounded asynchronous Windows capture resources,
scales SDR on the GPU before necessary readback, and supports selected-adapter
AMD/FFmpeg D3D11 surfaces. Unsupported processing/interop retains the CPU
fallback. Half-float HDR retains the existing CPU color-conversion path.
Hidden local previews stop thumbnail, texture-upload and preview-only repaint
work; visible tiles, including fullscreen, request fresh previews.

### Matched CPU conversion component

The baseline converter is taken from `30d3437f`; the new converter is measured at
`eb0a5b1226cce125ba0844ed6a447a206072d476`. The later `1f144bf0` change only
replaces equivalent Windows preview iterators and clarifies a converter comment;
it does not change the measured conversion algorithm.

Both standalone binaries use pinned Rust 1.98.1, identical `rustc -O` settings,
and the production converter source. The Linux 6.18.55 x86-64/glibc 2.41 host
reports an Intel Xeon Platinum 8573C, five available logical CPUs and a four-CPU
quota. No builds ran during measurement. Input is one synthetic 6144 × 2560
BGRA picture, converted to 3456 × 1440 I420. Unused HDR helpers are stubbed in
this isolated SDR benchmark; production HDR/color tests run separately.

Each revision receives one unmeasured process warmup, followed by five
alternating measured processes. Each process warms three pictures, then times
five blocks of eight conversions. The table reports the median of the five
per-process medians; ranges are those per-process medians. Every output checksum
matches. OS `wait4` CPU and peak RSS include fixture preparation and process
startup, not just conversion. There are no provider/helper processes in this
component workload.

| Component metric | Baseline | New | Delta |
| --- | ---: | ---: | ---: |
| Conversion median | 31.956 ms | 26.971 ms | −15.6% |
| Per-process median range | 31.422–44.107 ms | 24.759–33.067 ms | Overlapping |
| Whole-child CPU median | 4.255 s | 3.741 s | −12.1% |
| Whole-child peak RSS median | 68.445 MiB | 68.527 MiB | +0.082 MiB |

This is a conversion result, not capture cadence, GPU-engine time, GUI idle CPU,
settled application memory or game FPS. It neither measures Windows GPU scaling
nor demonstrates the ≤10% gaming penalty target. No overall application or live
streaming performance improvement is claimed.

Package comparisons use normal `cargo xtask package --format dir`, the pinned
toolchain/lockfile and standard fat-LTO production profile, with separate empty
output directories for clean and cached runs. The frozen baseline executable
is SHA-256 `4f0a3615693b17b58215689c9bfd99e239db6d8007dd169c5b6e2c91acf84eff`:
98,406,920 executable bytes, 227,165,939 installed bytes and 155,647,310 normalized
ZIP bytes. It was built at `2ee93142`; changes from that runtime to installer
revision `30d3437f` are documentation only. ZIP normalization uses sorted paths,
fixed timestamps, preserved file modes and deflate level 6. The matching new
package sizes, clean/cached validation outcomes and current native CI are
reported in [PR #22](https://github.com/MCShotty/Serein/pull/22). Windows/macOS
gaming, process-memory and comparative package measurements remain unavailable
locally.

### Verification and native comparison

The pinned full `cargo xtask check` passed at `eb0a5b12`, including strict
workspace Clippy, tests, the minimal application build and policy checks.
Focused screen, conversion/color, preview-demand and encoder-buffering fixtures,
login/workspace checks, packaging fixtures and license checks passed. Windows C
sources also passed strict cross-target syntax checks. Windows CI owns the
actual Rust/native build and WARP pool/readback, retirement and private-device
removal tests; cross-target syntax checks do not replace those tests.

Release replay passed. The 120-second synthetic soak completed 2,695 passes,
86,240 channel visits, 51,744,000 live inserts and 11 logout cycles in 120.027
seconds within retained-history budgets. It uses no renderer, network, storage
or audio devices and does not measure capture, process RSS or game performance.

The [owner-operated comparison procedure](pr-evidence/capture-overhead/README.md)
separates capture-only, readback/conversion and encoding, then compares normal
full sharing against the unchanged installer. It records actual geometry,
capture/readback counters, game frame-time tails, process CPU/RSS and GPU engine
activity. One warmup and five alternating native comparisons, receiver cadence
and visual-quality checks remain required on the owner's Windows gaming machine.
Physical high-refresh/fullscreen, resize, HDR, hybrid-adapter and driver-loss
behavior are unverified locally. No Discord account, game, microphone or live
capture was opened here. Native CI status and installer provenance are reported
on the PR rather than treating synthetic success as proof of the reported case.

## Codebase simplification

Baseline: merged fork `6d71c738` (tree-equivalent to `be2f6eca`). Runtime implementation: `fd181b4e`.
The following build-profile guard commit changes only offline checks. Production and UI-library inputs are unchanged.
Upstream reference: published nightly `v1.0.0-nightly.20261023.62`, pinned to `18e557ef710840e22f8ddf5d38f1d6065a9de5a5`.

### Core comparisons on native CI

Rust 1.98.1, locked dependencies, release opt-level 3, thin LTO, one codegen unit, line tables,
stripped symbols and abort-on-panic. Each OS uses one warmup and five rotating measurements
on the same runner; CPU affinity is pinned where the platform supports it. Different OS rows
are independent comparisons. The nightly gap includes inherited fork work.

| OS / workload | Nightly median (ms) | Frozen fork (ms) | Refactor (ms) | Refactor vs fork |
| --- | ---: | ---: | ---: | ---: |
| linux / reducer | 57.771 | 59.865 | 57.718 | -3.59% |
| linux / wide channels | 111.306 | 6.649 | 6.490 | -2.39% |
| windows / reducer | 87.427 | 88.192 | 86.621 | -1.78% |
| windows / wide channels | 75.799 | 9.721 | 9.207 | -5.28% |
| macos / reducer | 60.994 | 59.680 | 60.992 | +2.20% |
| macos / wide channels | 93.978 | 6.537 | 6.463 | -1.13% |

All reducer medians meet the 3% regression threshold. Full sample ranges and runner
metadata are retained in [CI artifacts](https://github.com/MCShotty/Serein/actions/runs/40049052574).

### Linux native UI and memory

Intel Xeon Platinum 8573C cloud host, 16 GiB cgroup memory limit, one inherited logical CPU,
Xvfb with server resets disabled, 1120 × 760 viewport at scale 1, llvmpipe LLVM 19.1.7 /
Mesa 25.0.7 OpenGL.
Both binaries use pinned Rust and matching normal release/fat-LTO settings. One full warmup
and five alternating fresh-process observations per workload; five-second samples, 100 ms
process observations and two seconds for settled RSS. No compilation runs during sampling.

Idle rows use the original frozen previews. Active rows use the same optional recorder gate
in both application revisions: after the first painted frame, apply input during three seconds
of warmup, trigger the native recorder and observe its acknowledged five-second window.
Callback times cover `MessagingUi::show`; they exclude presentation and input-to-display latency.
Peak RSS below is a sampled steady-workload peak, not process launch peak.

| Workload | CPU before → after (% of one core) | Callback p50 before → after (ms) | Callback p95 before → after (ms) | Peak / settled RSS before → after (MiB) |
| --- | ---: | ---: | ---: | ---: |
| conversation-idle | 5.50 → 5.51 | not sampled | not sampled | 143.88 / 143.88 → 144.18 / 144.18 |
| language | 0.00 → 0.00 | not sampled | not sampled | 146.98 / 146.98 → 146.73 / 146.73 |
| conversation-churn | 87.79 → 88.57 | 1.072 → 1.116 | 1.755 → 1.776 | 146.41 / 146.41 → 146.98 / 146.98 |
| gif-favorites | 91.91 → 87.91 | 0.781 → 0.803 | 1.101 → 1.202 | 149.96 / 149.96 → 149.99 / 149.99 |
| server-members-active | 65.27 → 63.54 | 1.397 → 1.324 | 2.035 → 1.822 | 149.09 / 149.09 → 149.15 / 149.15 |

Idle CPU remains within two percentage points of the frozen baseline. Helper-process
RSS was zero for these fixtures; no web provider was opened. Full-desktop asynchronous task
occupancy is not measured by this preview. Windows/macOS UI CPU/RSS, physical input latency,
media throughput and owner-controlled capture/HDR performance are not established here.

Churn callback medians change by +0.044 ms (p50) and +0.020 ms (p95); GIF medians by
+0.022 ms and +0.100 ms. Each has slower results in three of five paired observations,
with mixed directions and large tail variation. These runs do not establish a repeatable
regression or improvement. Member-panel medians change by −0.073 ms and −0.213 ms.
The single GIF allocation diagnostic records 210,077 → 208,480 malloc calls and
761,466,874 → 755,578,492 requested bytes over five seconds, with 130 → 128 callbacks.
These process-wide counts include native libraries, omit aligned/direct allocation paths,
and describe allocation traffic rather than live heap. Instrumented timings are excluded
from acceptance; the diagnostic supplies no evidence of a new allocation surge.

The nightly comparison above covers reducer workloads on all three operating systems.
The UI and package comparison uses the frozen fork baseline; no equivalent nightly UI
fixture/package comparison or Windows/macOS UI measurement was completed.

Initial fixed-clock active trials are diagnostic only: a cold renderer could enter late or
miss the native recorder window, and the external process/input window could differ from it.
Initial churn p95 was 1.736 → 2.418 ms; an independent repetition was 1.949 → 1.940 ms.
Initial GIF p95 was 2.324 → 3.013 ms; its repetition did not complete a valid callback window.
These values are not used as proof of either regression or improvement. The gated rows above
align the measured windows; the identical timing-only patch is recorded in the linked evidence.

### Linux package comparison

Normal clean and cached directory packages have identical file contents and modes. Both
passed codec and speech self-tests after removing development native-library paths. Same
installed file set, same ZIP/deflate level 6 and normalized timestamps; only `usr/bin/serein`
differs in the installed payload. Libraries, models, sources and licenses are unchanged.

| Metric | Frozen fork (bytes) | Refactor (bytes) | Delta |
| --- | ---: | ---: | ---: |
| Executable | 98,429,704 | 98,431,752 | +2,048 (+0.00%) |
| Installed payload | 227,188,723 | 227,190,771 | +2,048 (+0.00%) |
| Normalized compressed package | 155,648,014 | 155,649,345 | +1,331 (+0.00%) |

Windows/macOS package-size comparisons were not measured locally; their native build
and package checks are separate evidence. See [review and verification details](pr-evidence/codebase-refactor/README.md).

## Screen encoder input accounting

Baseline: fork `main` at `149644aa`. Runtime fix: `f12543a7`.
The capture ledger previously counted every encode call as a new pending picture.
Windows Media Foundation can instead return older output without accepting input;
macOS can drop a synchronous picture. Counting those attempts creates phantom
credits and can stop a valid share at the 48-picture limit. Results now carry
input acceptance independently of output. FFmpeg's receive-only polling remains
excluded from input accounting through the same result contract.

| Synthetic pressure check | Before | After |
| --- | --- | --- |
| 96 delayed/output-only/retry cycles | Fails with the pending-picture-limit error | Completes and decodes the final H.264 picture |
| Output-only encode results and dropped pictures | Counted as attempted inputs | No new capture credit |
| Actual pending-picture limit | 48 | 48 |

The ordering fixture runs the production Stable result validator, timestamp FIFO
and capture accounting with a real encoded picture; it models MFT event order
without opening a capture source or native hardware encoder. Separate checks
cover FFmpeg polling, dropped input, RTP timestamp wrapping, full queues and epoch
retirement. These are correctness/resource checks, not capture-FPS measurements.

The pinned workspace gate passed 2,221 distinct tests plus one locale child-process
check, with 36 default-ignored cases. The release replay and 120-second soak reuse
the verified artifact because their core source/dependency graph is unchanged;
their timings are not a before/after comparison for this fix. Capture, codec,
bitrate, quality presets, lookahead and native ABI are unchanged.

Release package comparisons and current native CI outcomes are recorded in the
delivery PR. Real Windows capture, game-FPS overhead, media throughput and live
Discord acceptance remain owner-controlled checks; synthetic success does not
establish those outcomes.

### Follow-up: idle delivery and missing output

Review baseline: `3ddc0099`. Idle-delivery fix: `fbdb85da`; all-platform
progress deadline: `e5fd3554`.
The Linux worker previously treated any buffered encoder input as demand for
another upload, including duplicates of a static capture already delivered.
It now shares the capture-delivery ledger, feeds only undelivered snapshots or
explicit keepalives/keyframe requests, and polls ready output independently.
Reordered tail pictures remain drainable without creating more input credits.

| Bounded scheduling model | Before | After |
| --- | --- | --- |
| Two captures and two keepalives over 240 scheduling opportunities, with 16-picture lookahead | 240 uploads | 36 uploads |
| Latest capture and receive-only reordered tail | Latest H.264 picture decodes | Latest picture decodes; tail drains without new uploads |
| Actual pending-picture limit | 48 | 48 |

The model uses production raw-frame retention and capture accounting with real
encoded H.264 pictures. It fails on the previous Linux demand condition and
passes after the correction. It is not a native encoder or GPU-overhead
measurement. Selected FPS, bitrate, presets, lookahead and quality options are
unchanged.

All screen workers now apply a 15-second progress deadline while a picture is
still needed, including empty Linux polls after the first keyframe. Dropped
inputs consume no credits, so the queue limit alone cannot detect that failure.
Valid output resets the deadline even when a newer capture is outstanding.
Deliberate transport pauses, geometry changes and security resets retire it;
completed static shares do not expire. Injected monotonic-time regressions cover
a minute of healthy output followed by a stall, startup, idle completion and
pause/resume, without sleeping or opening a capture source.

At `162d17a2`, the follow-up workspace gate passed 2,223 distinct tests plus the
locale child check, with 36 default-ignored cases. Formatting, strict Clippy,
the minimal build and policy checks passed. Handoff/workspace checks and the
offline Linux pipeline diagnostic passed; its pre-existing trailing-caps
fixture was corrected and its affected checks rerun. The all-platform deadline
extension passed the 218 voice and 64 platform tests, plus the locale child
check. Pinned formatting, strict workspace/all-target Clippy, the minimal app
build and the rebuilt offline Linux pipeline diagnostic also passed. The
current-head native CI results are recorded in the PR. No live capture,
microphone or Discord traffic was used.

Release package comparisons and current-head native CI are recorded in the PR.
The core replay graph is unchanged, so the earlier verified release replay and
120-second soak remain applicable. Capture/game FPS, media throughput, CPU/RSS
and physical Windows/macOS behavior remain unmeasured for these corrections.

## FFmpeg-only encoding controls (2026-10-25)

Runtime implementation: `eb60febb`. Comparison baseline: reviewed pre-controls head
`0104f8aa`. FFmpeg replaces the former platform encoder stack. Advanced Video Encoding
provides confirmed hardware codecs/presets and Low / Medium (Recommended) / High rate
profiles; the [settings reference](voice.md#video-backend-and-codec-settings) records
the budgets and migration behavior. Capture, incoming decoding and transport bounds
retain their existing ownership.

Pinned Rust 1.98.1, locked dependencies. Workspace formatting, strict all-target Clippy,
workspace tests, minimal application/policy checks, login/embed/workspace checks,
license checks, bounded fuzzing and the rebuilt Linux pipeline diagnostic passed.
The added production High-rate regression preserves a ceiling above Medium through
reconfiguration and emits a fresh IDR. Query fixtures validate named presets without
encoder Init/pictures; they do not establish hardware interoperability. The normal
native recipe was rebuilt into a fresh prefix, with unchanged source versions and
complete notices, archives, recipe and reproducible patch. FFmpeg packaging fixtures
passed 21 cases with one Windows-only skip; Flatpak preparation passed four cases.

The macOS geometry fixture exposed a prior synchronous-picture assumption after the
backend change. It now explicitly excludes hardware; dimensions, software decoding
and fresh-keyframe assertions remain. Native encoder buffering retains the separate
bounded tests and optional user-requested encoder test.

### Native settings and process comparison

The committed [before](pr-evidence/ffmpeg-video-controls/before.png) /
[after](pr-evidence/ffmpeg-video-controls/after.png) pair uses synthetic Voice & Video
settings at 1120×900, scale 1, dark appearance. The frozen before preview is
`fd181b4e`; its UI inputs were verified unchanged through `0104f8aa`. The old
FFmpeg option is selected before; the new controls default to Medium/Automatic.
Native light and narrow layouts were also inspected. Keyboard/scaling regressions
cover 220/320/600-point controls and 1×/1.5× scale; physical high-DPI displays remain
unverified. No camera, microphone or live capture was opened.

Same Linux 6.18.55/glibc 2.41 Xeon Platinum 8573C host, 16 GiB memory limit,
Xvfb/OpenGL llvmpipe LLVM 19.1.7/Mesa 25.0.7, normal release `demo` previews.
One inherited logical CPU, one warmup and five alternating fresh-process samples;
a visible-paint/liveness gate, twelve downward wheel events and three-second warmup
preceded five seconds of 100-ms process sampling and two seconds of settling.
Compilation was stopped. Medians of the per-run measurements:

| Metric | Before | After | Delta |
| --- | ---: | ---: | ---: |
| CPU, percent of one core | 92.995% | 93.560% | +0.565 percentage points |
| Peak RSS | 159,227,904 B | 165,191,680 B | +5,963,776 B (+3.75%) |
| Settled RSS | 159,227,904 B | 165,191,680 B | +5,963,776 B (+3.75%) |
| Helper-process peak RSS | 0 B | 0 B | 0 B |

The fixture continuously animates its pending capability indicator on a CPU
renderer; this is not ordinary idle use or driver-query throughput. It does not
measure hardware encoding, presented-frame/input latency or capture/game FPS.
Before preview SHA-256: `fb4cdc4ab4b24ca70abbd1a27f6acaf021621cbc72e42465c949f067a7b4c743`;
after: `a72de274d65da9961434182bab150562cc243702d1783e02c3561fe52ea4a41d`.

### Release package

Matching normal Linux x86-64 release settings and `cargo xtask package --format dir`
from an empty task-owned output directory. One package per revision; compressed sizes
use sorted entries, fixed timestamps and ZIP/Deflate 6.

| Metric | `0104f8aa` | `eb60febb` | Delta |
| --- | ---: | ---: | ---: |
| Executable | 98,436,872 B | 97,640,072 B | −796,800 B (−0.81%) |
| Installed payload | 227,195,891 B | 226,405,341 B | −790,550 B (−0.35%) |
| Normalized compressed payload | 155,652,406 B | 155,328,087 B | −324,319 B (−0.21%) |

Installed membership is identical. The twelve changed paths are the executable,
five rebuilt FFmpeg libraries, README, voice provenance, and retained recipe,
build/configure metadata and patch. Source archives and license texts remain intact.
Both staged and installed codec/synthetic LavaSR probes passed with old/new development
FFmpeg paths removed. Both executables retain only `$ORIGIN` runpaths.
Executable SHA-256: `68fa47c6fc71a441fa0985e1063d95a80ae27dd98e9da44001b9fa179d24151c`.
Compressed payload SHA-256: `2bf97acc047d4984ccd3aad33734e32ef4990f60d2972921a695b36c0a7eb233`.

### Core and lifecycle

The freshly rebuilt core-only replay is byte-identical to the verified baseline
(`111b2a0e8d1682577f260612a10ed234ea7e28c48b9ba45f0b7a7dc1924f04b8`);
the media settings are not linked into this workload. On the same Linux Xeon host,
one inherited logical CPU, one warmup and five alternating release samples gave
81.164 → 81.717 ms reducer medians (+0.68%) and 8.681 → 8.649 ms wide-channel medians
(−0.37%). These differences are timing variation, not an optimization claim.
The 120-second soak completed 3,477 passes, 111,264 channel visits, 66,758,400 inserts
and 14 logout cycles in 120.001 seconds. Retained estimates remained within the
existing budgets. This measures reducer lifecycle pressure, not process/GPU RSS,
media delivery, input latency or live Discord behavior.

### Current nightly comparison on native CI

[Initial runtime CI](https://github.com/MCShotty/Serein/actions/runs/40146721629)
compares published nightly `v1.0.0-nightly.20261024.63`, pinned to
`38e46b8f5ec694002ef515fbc2d8dc69297e4f5f`, fork main `149644aa`, and `eb60febb`.
Each OS uses one warmup and five rotating samples on its own runner; release opt-level
3, thin LTO, one codegen unit, line tables, stripped symbols and abort-on-panic.
Linux/Windows pin one CPU; macOS uses its scheduler. Cross-OS rows are separate
comparisons. The nightly gap includes earlier fork work.

| OS / workload | Nightly median (ms) | Fork median (ms) | Runtime median (ms) | Runtime vs fork |
| --- | ---: | ---: | ---: | ---: |
| Linux reducer | 56.873 | 58.537 | 58.785 | +0.42% |
| Linux wide channels | 112.063 | 6.576 | 6.616 | +0.61% |
| macOS reducer | 60.280 | 59.672 | 59.480 | −0.32% |
| macOS wide channels | 90.995 | 6.347 | 6.309 | −0.60% |
| Windows reducer | 78.056 | 80.490 | 80.603 | +0.14% |
| Windows wide channels | 93.684 | 8.709 | 9.002 | +3.37% |

The initial Windows wide-channel median exceeds the 3% target; its after samples
include a 14.128 ms outlier. The follow-up comparison records binary hashes to assess recurrence;
its final-head result is reported in the PR. Overlapping ranges alone are not treated as a pass. These are
core measurements; hardware encoding, capture/game FPS, media throughput and visual
quality remain unmeasured locally.

### Native verification limits

At runtime `eb60febb`, Linux application checks and all eight Linux package jobs
passed, as did Windows x64/ARM64 workspace, capture fixtures, replay and soak.
Windows packaging was still running. The macOS geometry assertion failed on its
hardware-buffering assumption; the follow-up fixture fixes only that assumption.
Its native rerun and final-head package status are linked in the PR.

The security job reports unchanged baseline versions of `crypto-bigint`, `ctutils`,
`tokio` and yanked `clipboard-win`; this patch removes one unused direct macOS
encoder dependency edge, without changing dependency versions. License checks pass.
Owner-controlled GPU codec/preset validation, live Discord acceptance, media quality,
physical Windows/macOS screens, HDR and game-FPS comparisons remain unavailable.

## Encoding recovery and UI review (2026-10-25)

Runtime `188635b7`, baseline `858fddbe`. Review found two still-valid issues:

- Hardware-filtered controls could strand a saved H.265/AV1 selection. An explicit
  **Restore H.264 default** action now restores H.264/Automatic without changing
  bandwidth, camera resolution or FPS. Hardware choices remain filtered.
- A saved preset owned by another GPU vendor could prevent current hardware startup,
  forcing H.264 to software or failing H.265/AV1. Both native opening paths now use
  Automatic for foreign presets; explicit encoder tests reject substitution.

The Linux drain-only deadline finding was outdated: the current check already runs
outside the optional-packet branch, including empty polls and backpressure/reset paths.
No Linux-loop change was needed.

The original recovery regression failed before the fix. Twenty-four mouse/keyboard
cases now pass, alongside preset ownership matrices and real bounded software
open/encode/reconfigure/fresh-keyframe checks. All 2,227 workspace unit/bin/integration
executions passed locally (35 ignored), with strict all-target Clippy, formatting,
the minimal application build, policy and handoff/workspace checks. The local full
wrapper exhausted build storage linking compile-only examples; all four runtime
[Windows x64/ARM64, macOS and Linux CI gates](https://github.com/MCShotty/Serein/actions/runs/40184715159)
subsequently passed `cargo xtask check`. Native packages were still running when
this evidence was written. Licenses, fuzz, SDK and all three replay jobs passed;
the dependency audit retains the previously recorded unchanged-version findings.

Fresh synthetic native checks covered encoding recovery, sharing controls,
mention/member/role presentation, time settings and embed-card layout. Dark/light,
580-pixel settings and a 150% X11 sharing layout were inspected. Native mouse input
confirmed H.264 recovery preserves High; keyboard activation was tested through the
production UI event path. The matched [before](pr-evidence/ffmpeg-video-controls/recovery-before.png)
and [after](pr-evidence/ffmpeg-video-controls/recovery-after.png) captures show the
same saved-codec/High-profile state at 1120×900, scale 1. The capability indicator is
an explicit pending fixture, not an actual driver result or a completed hardware test.

### Matched release process and package measurements

Same Linux 6.18.55/glibc 2.41 Xeon Platinum 8573C, 16 GiB limit, OpenGL llvmpipe
LLVM 19.1.7/Mesa 25.0.7; normal release `demo` previews. Same saved H.265 and twelve
native wheel events; visible-paint/liveness gate, three-second warmup, five seconds
of 100-ms samples, two-second settling. One warmup and five alternating fresh
processes, one CPU, no concurrent compiler or soak. The pending indicator animates:
this measures that fixture, not ordinary idle use or media throughput.

| Metric | `858fddbe` | `188635b7` | Delta |
| --- | ---: | ---: | ---: |
| CPU, percent of one core | 91.791% | 93.117% | +1.326 percentage points |
| Peak/settled RSS | 165,253,120 B | 165,183,488 B | -69,632 B |
| Helper peak RSS | 0 B | 0 B | 0 B |
| Executable | 97,640,072 B | 97,641,992 B | +1,920 B |
| Installed payload | 226,405,341 B | 226,407,261 B | +1,920 B |
| Normalized ZIP | 155,328,087 B | 155,328,640 B | +553 B |

Normal `cargo xtask package --format dir` used an empty output and matching release
settings; no staging repair. ZIP uses sorted entries, fixed timestamps and Deflate 6.
Only the installed executable differs; libraries, source bundles, notices and file
membership are identical. Staged and installed codec/synthetic speech launches pass
with development FFmpeg paths removed, and both retain only `$ORIGIN` runpaths.
One initial speech self-test reported a processing failure. A paired baseline/current
check and the complete validation rerun passed; no speech code or assets changed,
and no cause is claimed. Executable SHA-256: `c4eec94197585c281cd6c0478b900df848070ebfeb54a3c19985e8c4b39868b6`.

The verified unchanged core replay and 120-second soak passed: 3,972 passes,
127,104 channel visits, 76,262,400 inserts and 16 logout cycles in 120.021 seconds.
Current native CI compares nightly `v1.0.0-nightly.20261025.64` / `38e46b8f`, fork
`149644aa` and runtime `188635b7`, with one warmup/five rotating samples. Release
opt-level 3, thin LTO, one codegen unit and abort-on-panic; Linux/Windows pin one CPU,
macOS uses its scheduler. Separate runners are not cross-platform speed comparisons.

| OS / workload | Nightly (ms) | Fork (ms) | Runtime (ms) | Runtime vs fork |
| --- | ---: | ---: | ---: | ---: |
| Linux reducer | 78.062 | 84.811 | 84.309 | -0.59% |
| Linux wide channels | 163.349 | 9.129 | 9.110 | -0.20% |
| Windows reducer | 77.654 | 80.183 | 80.597 | +0.52% |
| Windows wide channels | 94.542 | 8.586 | 8.817 | +2.68% |
| macOS reducer | 63.469 | 62.168 | 62.983 | +1.31% |
| macOS wide channels | 82.979 | 5.813 | 5.811 | -0.03% |

Linux fork/runtime replay binaries are byte-identical. These timings make no
optimization claim. Actual GPU quality/presets, capture/game FPS, provider playback,
HDR/physical displays, native assistive technology and live Discord acceptance remain
unverified. Screenshots and offline packets do not establish those outcomes.

### Stream cancellation and progress follow-up — October 25, 2026

Baseline: fork `main` at `70ef19d2`, including its verified Windows x64 installer.
Codec, bitrate profile, quality preset, lookahead, GPU routing and capture sizing
are unchanged. Native AMF tests compile the patched encoder source and simulate
full queues, missing output, cancellation, rejected input and retained surfaces.
They verify the 15-second blocked-call limit, 48-reference ceiling, cleanup after
termination and recovery after an idle lookahead tail. Their injected clock is
not a measurement of driver latency or game performance.

The shipping SDR encoder now feeds the synthetic RTP/DAVE sender/viewer tests;
the receiver must decode multiple pictures. Separate tests keep signaling and
audio healthy while withholding encoded video, check bounded startup failure,
and reject readiness after locally dropped keyframe fragments. Progress reports
contain fixed-size counters only and stop after eight samples per share.

Pinned Rust 1.98.1, the committed lockfile and a dedicated target directory were
used for local checks. All-target strict Clippy, workspace assertion-bearing
tests, the minimal application build, policy and license checks pass. The local
`cargo xtask check` wrapper exhausted tmpfs while linking compile-only UI examples;
the exact wrapper remains required in native CI. Historical measurements above
retain their original revisions and are not results for this follow-up.

Release reducer replay and the 120-second soak pass (3,767 passes, 120,544 visits,
72,326,400 inserts, 15 logout cycles; 120.018 seconds). The frozen baseline and
current reducer executables are byte-identical, SHA-256
`111b2a0e8d1682577f260612a10ed234ea7e28c48b9ba45f0b7a7dc1924f04b8`.
One warmup each and five alternating measurements on Linux x86_64, pinned to one
CPU with matching release settings, gave medians of 83.377/81.112 ms for 100,000
events. This difference is timing noise in identical binaries, not an optimization.

| Windows x64 baseline package metric | `70ef19d2` bytes |
| --- | ---: |
| Executable | 99,178,496 |
| Installed payload | 225,921,352 |
| NSIS installer | 136,650,669 |

These are the normal CI artifact's uncompressed executable/payload and compressed
installer sizes. Current package deltas and artifact provenance belong in the PR's
Performance section after its native builds complete. Capture/game FPS, hardware
encoder throughput, CPU/RSS and actual viewer playback still require the
owner-controlled Windows session; no improvement is claimed from synthetic tests.

## Channel preferences and browser — October 26, 2026

Runtime revision `52836c95` starts from installer `d11d2aee`. The pinned current
published nightly is `v1.0.0-nightly.20261025.64` (`38e46b8f`), verified on October 26.
Separate worktrees/targets, Rust 1.98.1 and committed lockfiles were used.
No dependency versions, media quality or encoder settings changed.

The initial implementation `f8d99c95` introduced unnecessary complete preference
snapshot reads during ordinary message processing. CI measured reducer changes
of +4.5% Linux / +7.0% Windows / +25.5% macOS versus `d11d2aee`; the macOS samples
varied substantially. An independent local pair measured +7.3%. The follow-up
uses narrow policy reads and skips guild-mention policy work when no such mention
exists, preserving pending preferences, suppression and mute-expiry behavior.

Local core comparisons use matching opt3/thin-LTO/codegen1/stripped release builds,
one warmup and five alternating/reversed samples per binary, pinned to one logical
CPU on Linux x86_64 / Xeon Platinum 8573C. The first corrected run measured
95.926 / 96.814 ms (+0.9%) for 100,000 events and 9.521 / 9.472 ms (−0.5%) for the
wide-channel workload. A repeat measured 123.970 / 103.625 ms (−16.4%) and
14.311 / 13.105 ms (−8.4%); this host variability does not establish a speedup.
Final source-labeled native CI comparisons for Windows, macOS and Linux are linked
in the PR, including the immutable nightly and sample ranges.

| Linux browser metric | `d11d2aee` | `52836c95` | Delta |
| --- | ---: | ---: | ---: |
| Idle CPU, one core | 0.00 % | 0.00 % | +0.00 pp |
| Idle sampled peak/settled RSS | 152.50 MiB | 148.13 MiB | -4.38 MiB (-2.9%) |
| Instrumented callback p50 | 1.456 ms | 0.528 ms | -0.928 ms (-63.8%) |
| Instrumented callback p95 | 2.772 ms | 0.927 ms | -1.845 ms (-66.6%) |
| Instrumented process CPU, one core | 57.84 % | 27.42 % | -30.43 pp |
| Instrumented sampled peak/settled RSS | 152.92 MiB | 147.76 MiB | -5.16 MiB (-3.4%) |
| Release preview executable | 65,757,968 B | 65,903,888 B | +145,920 B (+0.22%) |

Idle medians are at the sampling floor, rather than proof of zero CPU. No helper/provider
processes were observed. This workload compares the old modal over the conversation
with the dedicated page; it is not a whole-app or hardware-rendering speedup claim.

Native browser sampling uses the release preview, explicit synthetic demo,
1280 × 900, scale 1, Mesa llvmpipe and isolated Xvfb, with no service adapters.
After a three-second startup warmup, idle sampling is 15 seconds at 0.5-second
intervals. Optional callback instrumentation measures `MessagingUi::show` only,
with three seconds warmup and five seconds sampling at 10 Hz. Each comparison has
one warmup pair and five alternating/reversed measured pairs. Reported CPU is a
percentage of one core; memory is process RSS. Callback times are neither
presented-frame nor input latency. Helper processes are counted separately.

| Canonical `d11d2aee` package metric | Windows x64 | macOS ARM64 | Linux x64 |
| --- | ---: | ---: | ---: |
| Application executable, bytes | 99,234,816 | 78,176,144 | 97,601,944 |
| Installed payload, bytes | 225,998,044 | 201,435,975 | 226,392,420 |
| Compressed distribution, bytes | 136,666,723 | 136,143,729 | 141,514,680 |

Baseline artifacts are from run [40208513242](https://github.com/MCShotty/Serein/actions/runs/40208513242).
Windows payload excludes the generated uninstaller; compression is NSIS. macOS
counts the complete app bundle and its CI ZIP. Linux counts the installed payload
and Debian package. Formats are compared within each platform, never across them.

Normal CI packages use the canonical release profile; local directory packaging
uses thin LTO and is labelled separately. PR #27 records canonical executable, installed payload
and compressed distribution comparisons as native checks complete, with exact
artifact/source IDs; local and CI profiles are not mixed in a delta.

Release replay and the 120-second lifecycle soak passed on `52836c95` (2,684 passes, 85,888 visits,
51,532,800 inserts, 11 logout cycles; 120.031 seconds). The row/byte bounds held throughout. These
counters describe a bounded stress check rather than comparable application throughput. The PR
tracks normal clean/cached directory packaging and current-head native package outcomes with their
actual release settings.

The live two-client synchronization check, physical Windows/macOS input,
assistive technologies and real-account responsiveness remain unverified.
No media/voice performance improvement is claimed from these synthetic workloads.

## Gateway recovery and application audio — October 26, 2026

This follow-up freezes installer `e73bb7b0` and implements runtime `f5349e74`.
Its PR records comparable release reducer, native preview CPU/RSS/callback and
package-size results, with separate current-head Windows/macOS/Linux CI comparisons.
The nightly reference is the current published release pinned to its source,
rather than a moving branch. Reused executables retain their verified hashes and
original build provenance. Historical results above keep their original revisions.

See the [methods and synthetic UI evidence](pr-evidence/gateway-audio/README.md).
The offline preview does not contain Gateway or native audio workers, so its CPU/RSS
cannot establish connected-client or application-isolation overhead. The loopback
regressions check recovery deadlines and bounded queues; they are not comparative
live-network latency measurements. Real dropout recovery and two-application audio
acceptance remain owner-controlled. No encoding-quality or game-FPS improvement
is claimed by this change.

## Desktop UI polish — October 27, 2026

The implementation revision is `dd20fca9`, with installer source `3e4b854b` as the
frozen base and published nightly `v1.0.0-nightly.20261026.65` (`38e46b8f`)
as a separate reference. CI used Rust 1.98.1, opt3, thin LTO, one codegen unit,
stripped symbols and abort-on-panic; each workload used one warmup and five
order-rotated samples. The table reports median synthetic reducer time in
milliseconds. It does not measure UI callbacks, presentation or input latency.

| CI runner | Nightly | Base | Implementation | Base → implementation |
| --- | ---: | ---: | ---: | ---: |
| Linux x64 | 47.524 | 46.633 | 46.399 | −0.5% |
| Windows x64 | 106.615 | 111.130 | 110.710 | −0.4% |
| macOS ARM64 | 73.785 | 62.735 | 65.407 | +4.3% |

The macOS increase exceeds the 3% investigation threshold. The timed reducer,
fixture and comparison script are unchanged; the changed application-command
validation code was not found on the measured message path. Different builds
can still change machine code and layout. The macOS runner was not pinned, and
the overlapping samples do not establish whether the increase is repeatable.
Its cause remains unresolved. See the [CI run](https://github.com/MCShotty/Serein/actions/runs/40442719612)
and the [surface audit](pr-evidence/desktop-ui-polish/README.md).

## Force Sync and paged local history — October 30, 2026

Core measurements use `31f9daf2b833bce8d8bb177131d86d4857bf1243`, based on frozen
fork main `39060d31760b0889a44ca53163a8815de8b52455`. The current published upstream
reference is nightly `v1.0.0-nightly.20261028.68`, pinned to
`1b2f8942910436f9a488a0cf0f806776f02a074f`. Native CI compares the final PR head,
frozen fork base and published nightly separately on Linux, Windows and macOS;
its results and normal package outcomes are recorded in the PR.

Local core comparisons use Rust 1.98.1 and the committed lockfile, the canonical
opt3/fat-LTO/codegen1/stripped release profile with line tables, separate build
directories, Linux x86_64/glibc 2.41 and Xeon Platinum 8573C. Measurements were
pinned to one admitted logical CPU after compilation stopped: one warmup and
five alternating pairs. Reducer/wide samples average ten identical process runs
per repetition to reduce short-run noise. Loading runs use the workload's own
warmup and five samples, repeated in five alternating pairs.

| Release core metric / median | `39060d31` | `31f9daf2` | Delta |
| --- | ---: | ---: | ---: |
| 100,000-message reducer | 77.119 ms | 77.726 ms | +0.79% |
| 10,000-message / 20,001-channel reducer | 8.398 ms | 8.467 ms | +0.82% |
| Loading workload | 36.377 ms | 35.861 ms | −1.42% |
| Retained 500-row timeline estimate | 409,492–409,977 B | 417,492–417,977 B | +8,000 B |

The added per-user server-avatar context is included in the retained-byte estimate.
These are core timings, not SQLite latency, native frame/input latency, RSS or
Discord throughput. Loading still reports every synthetic immediate preview and
known-member lookup available. Earlier exploratory runs measured +3.58% reducer
and, in a different run, +3.30% wide-channel time. A repeated banner-map scan was
replaced with maintained byte accounting; host/build variability and changed
binary layout prevent attributing all variation to that change. Final repetition
means ranged 76.434–77.488 ms before / 76.938–78.464 ms after for the reducer and
8.206–8.421 / 8.342–8.637 ms for wide channels. The final local medians are within
3%; no speedup is claimed, and native CI remains a separate check. The previously
documented macOS timing concern is not declared resolved by these Linux samples.

Release replay SHA-256 values: baseline
`41a41deba92f635447c1a2af4db306b1c4d014edd16dccdae61b9dc57558cf97`,
implementation `597f216fe5042d33f2015c8a4074dbea0a3bbc3b7d13cb0a9918482d15216620`.

Supplementary native Storage-page samples used matched **development builds**
(debug=0, `demo`), isolated synthetic data, native Wgpu/software rendering in
Xvfb, dark 1120 × 850 at scale 1, three-second warmup and ten seconds sampled at
0.5-second intervals. There was one discarded warmup and five alternating pairs;
production release compilation ran concurrently. Both median process CPU samples
were below the 0.10%-of-one-core accounting resolution. Median sampled peak/settled
RSS was 199,987,200 → 203,018,240 B (+1.52%). No helper children were observed;
Xvfb was measured separately at 47,726,592 → 47,742,976 B. These samples characterize
the idle settings fixture, not release acceptance or a connected synchronization
job. The UI fixture predates the final nonvisual member-removal corrections.

Matched native [Storage screenshots](pr-evidence/force-sync/after.png),
[light/narrow 200% view](pr-evidence/force-sync/after-light-narrow.png) and
[progress dialog](pr-evidence/force-sync/progress.png) use synthetic cache and
progress values. They show controls and layout, not real synchronization or a
3-GB-pressure run. Baseline images were captured from the frozen fork build.

The final-core 120-second lifecycle soak completed 3,831 passes, 122,592 channel visits,
73,555,200 live inserts and 15 logout cycles in 120.023 seconds with the shipping
row/byte bounds intact. It excludes storage, media, networking and process RSS.
Live server/account synchronization, permission changes during a real roster,
Windows/macOS physical display and assistive-technology behavior remain
owner-controlled and unverified. Release connected-job CPU, presented-frame
latency and actual service transfer time are unmeasured. The PR records packaging
results, additional store measurements and platform CI without substituting these
synthetic samples for those checks.


The final SQLite-layout correction has separate release store measurements. Two
standalone harnesses link matching baseline/final `local-store` and `model`
artifacts (Rust 1.98.1, opt3/fat LTO/codegen1); neither uses the desktop. Both seed
the same 5,000 messages across ten channels and 4,096 display hints. Each measured
sample uses twenty fresh connections, 200 warm 500-row reads and forty nickname
queries. Results below are medians of one warmup and five alternating pairs on
CPU 0 after compilers/fuzzing stopped. “Fresh” means a new SQLite connection;
OS file caches are warm, so this is **not disk-cold startup**.

| Release store metric | `39060d31` | Final layout | Delta |
| --- | ---: | ---: | ---: |
| Open SQLite connection | 1.596 ms | 2.308 ms | +0.712 ms (+44.63%) |
| First 500-message read | 1.288 ms | 1.267 ms | −0.021 ms (−1.63%) |
| Warm 500-message read | 1.042 ms | 1.116 ms | +0.074 ms (+7.11%) |
| Eight nickname matches | 4.828 ms | 0.559 ms | −4.269 ms (−88.42%) |
| Seeded SQLite file | 2,564,096 B | 2,789,376 B | +225,280 B (+8.79%) |

Opening cost is paid by the storage worker when it opens its connection, not on
every channel change. Warm-read and open ranges were disjoint; these measured
costs remain visible and are not called noise. First-read ranges overlapped. The
nickname comparison changes the method from loading/filtering all 4,096 hints to
a bounded eight-result SQL query; its improvement is not a UI-latency claim.
No visual quality, safety bound or media setting was reduced.

The physical-file probe exposed that the former pragma order left SQLite in
non-reclaimable mode 0. The corrected final fixture verifies mode 2. After deleting
1,000 synthetic rows, the bounded 1,024-page cleanup pass reduced the file from
4,898,816 to 700,416 bytes; remaining free pages are reclaimed by later bounded
passes. Actual-file regressions cover preserved personal data, reserved legacy
conversion, low space and the original 64-MiB upgrade boundary.

Normal local `cargo xtask package --format dir` reached the canonical fat-LTO
application link, but the 17-GiB execution environment killed it for memory even
after completed test caches were retired. No replacement compiler profile or
old executable is presented as a successful current package. Packaging fixtures
passed; current-head native package outcomes and executable/installed/compressed
size comparisons belong to CI and are reported in the PR. Verified baseline
Linux CI run [40944896907](https://github.com/MCShotty/Serein/actions/runs/40944896907)
at `39060d31` has 98,785,944 executable bytes, 227,730,212 installed bytes and a
141,894,460-byte Debian distribution; these are reference values, not a measured
final delta. Windows/macOS release app CPU/RSS and actual synchronization workload
latency remain unmeasured.

## Webcam filter chain — October 31, 2026

The requested chain costs substantially more than the previous single filter.
The following is a synthetic comparison of the native filter bridge, not an
end-to-end webcam measurement. The baseline is `39060d31`; the new bridge uses
`video_denoise.c` SHA-256 `ef48198cfba1b94b07521313a4b974959c1d29283742dceb17eff3516ad9b699`
and FFmpeg recipe SHA-256
`24bf9e4ce8ea9d7b65fee7ecf7a726823b1c3759fa157c3ee786a60fd516c916`.

Both use FFmpeg 7.1.5, a two-thread graph and a bridge compiled with `-O2` on
Linux x64 (Intel Xeon Platinum 8573C, two logical CPUs). The baseline FFmpeg is
LGPL-only; the new build enables GPL for `hqdn3d`. For each size, one
60-frame warmup was followed by five 60-frame samples; the table shows the
median elapsed milliseconds per submitted synthetic I420 frame. Inputs have
uniform luminance varying over time. Runs were sequential, with comparison order
reversed for 720p. They include graph processing and buffer copies, but not
camera capture, encoding, UI rendering, memory usage or actual frame latency.

| Frame size | Previous `atadenoise=s=5` | Requested chain | Change |
| --- | ---: | ---: | ---: |
| 640 × 480 | 0.75 ms | 3.94 ms | +423% |
| 1280 × 720 | 2.21 ms | 11.48 ms | +419% |
| 1920 × 1080 | 4.82 ms | 26.00 ms | +440% |

The requested default temporal window also starts producing output after five
input frames, versus three previously. These measurements do not predict other
hardware. The existing overload fallback disables filtering when it cannot keep
up and continues the camera without it; native camera behavior remains unverified.

### GPU webcam filter and LavaSRv2 follow-up

The preceding webcam table describes the original **CPU** bridge and does not
measure the new GPU implementation. On Linux x64, a software Vulkan test adapter
ran the production compute kernels on synthetic I420 frames and compared 19 output
frames at each of 16×16, 18×14 and 64×48 against FFmpeg 7.1.5; samples differed
by at most one value and timestamps matched. The software adapter is admitted
only by the explicit test fixture. Production rejects software adapters. Hardware
GPU throughput, GPU memory, webcam quality and Windows/macOS behavior are unmeasured.

The earlier LavaSRv2 helper observation at `4f86f7d2` used 40 ms input blocks:
its median/p95/maximum inference round trip was 36.94/51.37/139.69 ms over
100 samples on the then-recorded Linux runner. This is historical evidence of
limited headroom, not a diagnosis of a particular user's fallback.

The new 80 ms block path was exercised separately on Linux x64 (Xeon Platinum
8573C, four-core quota, Rust 1.98.1; dev profile with voice/platform optimized).
The production supervisor and bundled models processed six seconds of nonzero
synthetic audio with a midstream reset in five repeated functional runs. Each
produced 291 of 300 20 ms output frames without fallback; the count includes
startup, reset and an undrained tail. This is not a matched performance comparison
or a microphone quality test.
The extra block duration can add up to 40 ms of latency. Native Windows audio,
physical microphones and performance on slower systems remain unverified.

### Cached history reconciliation and reply navigation

The baseline is merged fork main `52a9b3aeee7e1546df47f998ccfcbf64c329a15f`;
the implementation is `ea18da1e`. On Linux x86-64, Rust 1.98.1, Intel Xeon
Platinum 8573C, a four-core quota and a 16-GiB memory limit, the release
`replay-bench` was pinned to CPU 0. Each binary received one warmup followed by
five alternating measurements; concurrent task-owned builds were paused during
sampling. The measurements are synthetic core reducer time, not user-perceived
latency or process RSS.

| Workload | Baseline median (range) | Changed median (range) | Delta |
| --- | ---: | ---: | ---: |
| 100,000 reducer events | 75.88 ms (73.52–83.65) | 77.43 ms (72.86–80.31) | +2.05% |

The ranges overlap. Both retained 500 rows and 417,492–417,977 estimated bytes.
No speedup or live Discord behavior is inferred from this replay. The storage
regression exercises a full 50-row newer page, the start/end boundaries and
conflicting cursors; it does not measure disk or UI latency.

The changed release also completed a 120-second synthetic lifecycle soak: 3,269
passes, 104,608 channel visits and 13 logout cycles. Its retained-history bounds
held. It used no network, storage, renderer or audio devices. Current native
Windows/macOS behavior and live Discord synchronization were not tested here.

### Conversation search and scrolling

The starting revision is `0c670e2f`; the implementation is `5a296b60`. The
starting revision differs from the earlier `ea18da1e` replay executable only
in documentation. On Linux x86-64 (Xeon Platinum 8573C, four-core quota,
Rust 1.98.1), release replay binaries were pinned to CPU 0. Each received one
warmup and five alternating measurements with no other task-owned builds.

| Workload | Starting median (range) | Changed median (range) | Delta |
| --- | ---: | ---: | ---: |
| 100,000 reducer events | 81.12 ms (77.74–85.55) | 80.29 ms (78.96–87.79) | −1.02% |

Both retained 500 rows and 417,492–417,977 estimated bytes. The ranges overlap;
this synthetic reducer test does not measure search responsiveness, rendering or
live Discord behavior. The changed release replay also completed a 120-second
synthetic lifecycle soak: 3,252 passes, 104,064 channel visits and 13 logout
cycles, with its retained-history bounds intact.

The synthetic desktop was sampled separately in **debug** builds, using Xvfb,
the Vulkan llvmpipe renderer, a 1280 × 800 window and the same search results.
After a three-second warmup, five consecutive one-second process samples gave
median idle CPU of 13.99% and 13.96% of one core, respectively. Median RSS was
279,344 KiB and 279,460 KiB; sampled peak RSS was 284,572 KiB and 284,392 KiB.
These short, non-alternating samples include synthetic animations and are not
release-build or presented-frame measurements. No frame-latency claim follows.

The debug executables were 339,267,264 and 339,489,976 bytes. Comparable release
application and package sizes were unavailable locally: the preceding release
packaging attempt approached the environment's 16 GiB memory limit. Native
Windows/macOS input, live Discord searches and physical-device scrolling remain
unverified. The checked-in before/after images are synthetic Linux light-theme captures; dark
and narrow layouts were exercised by UI tests rather than native captures.

The follow-up correction at `1819e47e` was compared against the same starting
revision with the same release setup, one warmup and five alternating samples.
The baseline median was 77.79 ms (74.76–81.21 ms), and the corrected median was
80.02 ms (75.26–82.95 ms), a 2.86% increase with overlapping ranges. Both
retained the same bounded timeline. These measurements do not establish a
perceived latency change. The corrected release completed the 120-second
synthetic lifecycle soak: 3,827 passes, 122,464 channel visits and 15 logout
cycles, with retained-history bounds intact. The earlier UI samples and
screenshots remain labelled to their original revisions.

### Member identity watch and cache validation

The merged fork baseline is `9fd017f7`; the implementation is `049f2961`.
On Linux x86-64 (Xeon Platinum 8573C, four-core quota, Rust 1.98.1), separate
release builds ran on CPU 0 with one warmup and ten alternating samples per
workload. The two new benchmark fixtures were added to the baseline without
changing its production code. These are core-only measurements, not presented
frame times, process RSS, network latency or live Discord compatibility.

| Synthetic workload | Baseline median (range) | Changed median (range) | Delta |
| --- | ---: | ---: | ---: |
| 10,000 member-watch rebuilds, 128 identities and 500 messages | 109.30 ms (103.76–112.54) | 53.70 ms (51.98–59.30) | −50.9% |
| 100,000 cached display validations, 128 identities | 9.92 ms (9.53–11.62) | 8.54 ms (7.76–9.25) | −13.9% |
| 10,000 messages across 20,001 channels | 8.70 ms (8.21–9.79) | 8.89 ms (8.42–9.86) | +2.3% |
| 100,000 reducer events | 84.14 ms (79.62–88.42) | 81.62 ms (78.89–84.98) | −3.0% |

The general reducer ranges overlap. Both versions retained 500 timeline rows
and 417,492–417,977 estimated bytes in the ordinary replay. The member-watch
fixture contains no recorded departures; the existing departure path is
covered separately by tests. The release replay executables measured
2,428,464 and 2,428,352 bytes; application and package sizes were unavailable
for this comparison. No claim about overall application latency follows.

A follow-up at `a0715a4b` collects departure IDs once per watch instead of
repeatedly searching the identity overlay. It was measured against the same
baseline with updated fixtures, again using one warmup and ten alternating
samples. The earlier `049f2961` numbers above remain historical.

| Synthetic workload | Baseline median (range) | Follow-up median (range) | Delta |
| --- | ---: | ---: | ---: |
| 10,000 watches, 128 authors | 101.61 ms (98.55–109.35) | 73.62 ms (67.69–77.15) | −27.5% |
| 10,000 watches, one recorded departure | 328.62 ms (321.06–340.93) | 326.61 ms (319.14–380.66) | −0.6% |
| 10,000 watches, one recent author and 128 cached identities | 145.88 ms (137.05–152.19) | 109.02 ms (104.28–115.30) | −25.3% |
| 100,000 cached-display validations | 9.97 ms (9.43–11.77) | 7.74 ms (6.99–9.01) | −22.3% |
| 10,000 wide-channel message events | 8.64 ms (7.34–9.82) | 8.57 ms (8.00–10.73) | −0.9% |
| 100,000 reducer events | 78.82 ms (72.99–82.63) | 79.98 ms (76.37–84.10) | +1.5% |

The departure and general reducer ranges overlap. Watch results remain sorted,
bounded and exclude departed users. These figures measure only the stated core
workloads; they do not measure application-wide responsiveness.

### Member-watch simplification follow-up

The previous PR head `728696b0` is the baseline; the production change is
`c2372c4a`. A shared borrowed candidate representation lets the ID-only watch
avoid building owned nickname-only users. Sorted insertion retains the first
128 distinct eligible users and the same sorted result, including when users
leave or rejoin. Display callers still receive the same user data.

On Linux x86-64 (Xeon Platinum 8573C, five exposed CPUs with a four-core
quota), separate Rust 1.98.1 release builds used fat LTO and one codegen unit.
The table shows one warmup and 20 alternating samples per binary on CPU 0.
The ranges are individual synthetic replay times, not frame times, actual
process memory, network latency or live Discord measurements.

| Synthetic workload | `728696b0` median (range) | `c2372c4a` median (range) | Delta |
| --- | ---: | ---: | ---: |
| 10,000 member-watch rebuilds | 76.575 ms (71.044–168.661) | 44.471 ms (40.641–65.015) | −41.9% |
| 10,000 watches, one recorded departure | 330.669 ms (319.886–339.385) | 97.859 ms (93.849–102.959) | −70.4% |
| 10,000 watches, one recent author | 112.185 ms (107.796–115.649) | 69.167 ms (65.729–71.325) | −38.3% |
| 100,000 reducer events | 87.417 ms (82.279–124.467) | 86.950 ms (77.335–218.746) | −0.5% |
| 10,000 wide-channel message events | 9.965 ms (8.264–28.972) | 11.566 ms (8.234–36.905) | +16.1% |

The initial wide-channel result warranted investigation. With no builds running,
a subsequent run used one warmup and 30 alternating samples on each exposed
CPU, with the same binaries and workload:

| CPU | Baseline median (range) | Changed median (range) | Delta |
| --- | ---: | ---: | ---: |
| 0 | 8.729 ms (8.215–10.072) | 8.563 ms (7.842–12.157) | −1.9% |
| 1 | 8.759 ms (8.102–10.946) | 8.546 ms (7.785–10.094) | −2.4% |
| 2 | 8.797 ms (7.935–9.465) | 8.654 ms (8.095–9.751) | −1.6% |
| 3 | 8.661 ms (8.078–9.905) | 8.720 ms (8.053–9.223) | +0.7% |
| 4 | 8.585 ms (8.009–11.367) | 8.505 ms (7.683–13.714) | −0.9% |

The later runs did not reproduce the initial slowdown, but they do not prove
unchanged performance in the live application. The replay executables were
2,431,600 bytes before and 2,430,752 bytes after; application and package
sizes, UI latency and live Discord behavior were not measured.

The changed release binary also completed the 120-second synthetic lifecycle
soak: 3,632 passes, 116,224 channel visits and 15 logout cycles, with the
retained-history assertions intact. This checks reducer resource bounds; it
does not exercise the member-watch workload, network, storage or native UI.

### Bug-audit replay comparison

The merged baseline is `1e8123c0`; the fixes are at `9c5ab81a`. Separate
Rust 1.98.1 release builds used fat LTO and one codegen unit on Linux x86-64,
Xeon Platinum 8573C, with a four-core quota. Each workload had one warmup per
binary and five alternating samples pinned to CPU 0.

| Synthetic core workload | Baseline median (range) | Changed median (range) | Delta |
| --- | ---: | ---: | ---: |
| 100,000 reducer events | 80.910 ms (75.758–84.957) | 81.230 ms (78.215–83.834) | +0.4% |
| 10,000 member-watch rebuilds | 41.837 ms (41.023–44.769) | 39.680 ms (38.643–43.342) | −5.2% |

These overlapping samples are not evidence of a speedup. Both replay binaries
were 2,430,752 bytes and retained 500 messages / 417,492–417,977 estimated
bytes in the reducer workload. The replay does not measure the changed Gateway
writer, voice buffering, rendering, process RSS, network latency or live
Discord behavior.
Full application and package sizes and native UI timings were not measured.

### Bug-audit identity follow-up

Using the same environment and alternating method above, the merged baseline
`1e8123c0` was compared with source revision `9f43824d`. The identity harness
had an additional unpinned sanity run before the pinned warmup and samples. It
uses a separate `rustc -C opt-level=3` harness linked to each revision's
release libraries. It inserts 3,000 synthetic messages and applies an identity
update after each insert, retaining at most 500 timeline rows.

| Synthetic workload | Baseline median (range) | Changed median (range) | Delta |
| --- | ---: | ---: | ---: |
| 100,000 reducer events | 80.020 ms (77.573–84.670) | 82.181 ms (81.490–86.359) | +2.7% |
| 10,000 member-watch rebuilds | 42.982 ms (41.004–45.291) | 43.454 ms (42.466–44.351) | +1.1% |
| 3,000 message/identity updates | 98.809 ms (97.337–102.623) | 68.311 ms (64.037–71.071) | −30.9% |

At 500 and 3,000 inserts, the baseline estimated bookkeeping overhead rose
from 614,896 to 794,896 bytes; the changed build stayed at 578,104 bytes.
These are timeline allocation estimates, not process RSS. Replay binaries
were 2,430,752 and 2,430,800 bytes respectively. The results do not measure
Gateway, voice, rendering, storage, native input or live Discord performance.

### Bug-audit member reconciliation follow-up

The same Linux release comparison was repeated for `fd8541bf` against merged
baseline `1e8123c0`, using one warmup and five alternating samples pinned to CPU
0. The identity harness has the same scope described above.

| Synthetic workload | Baseline median (range) | Changed median (range) | Delta |
| --- | ---: | ---: | ---: |
| 100,000 reducer events | 80.920 ms (79.340–85.578) | 77.165 ms (75.610–80.861) | −4.6% |
| 10,000 member-watch rebuilds | 45.079 ms (42.146–49.122) | 41.738 ms (41.063–44.931) | −7.4% |
| 3,000 message/identity updates | 98.128 ms (97.759–103.837) | 70.620 ms (65.473–71.092) | −28.0% |

The identity bookkeeping estimate stayed at 578,128 bytes across the changed
run, while the baseline rose from 614,896 to 794,896 bytes. Reducer and
member-watch sample ranges overlap. These measurements do not establish
application-wide performance or replace native UI and device checks.

### Bug-audit snapshot reconciliation follow-up

The same Linux release method compared merged baseline `1e8123c0` with
`cb506cb2`, using one warmup and five alternating samples pinned to CPU 0.
No build ran during the measurements.

| Synthetic workload | Baseline median (range) | Changed median (range) | Delta |
| --- | ---: | ---: | ---: |
| 100,000 reducer events | 85.125 ms (82.784–87.245) | 85.449 ms (84.308–87.935) | +0.4% |
| 10,000 member-watch rebuilds | 44.390 ms (42.859–45.976) | 44.873 ms (44.124–49.247) | +1.1% |
| 3,000 message/identity updates | 102.205 ms (99.401–107.222) | 62.598 ms (60.002–65.052) | −38.8% |

Estimated identity bookkeeping remained at 578,128 bytes in the changed run;
the baseline grew from 614,896 to 794,896 bytes. These workloads do not
measure member-list snapshot processing, UI frame time, process memory or
live Discord behavior. The reducer and member-watch ranges overlap.

### Cached mention and member loading

CI compared fork baseline `287182f3` and source `485f89e9` with upstream nightly
`1b2f8942` (nightly tag `v1.0.0-nightly.20261028.68`). Each runner used Rust
1.98.1, release optimization level 3, thin LTO, one codegen unit, one warmup
and five measured repetitions. Times are medians in milliseconds.

| Synthetic workload | Nightly | Fork baseline | Changed | Delta vs fork |
| --- | ---: | ---: | ---: | ---: |
| Linux reducer | 66.437 | 65.410 | 65.651 | +0.4% |
| Linux wide-channel reducer | 86.454 | 7.281 | 7.180 | −1.4% |
| macOS reducer | 62.252 | 68.311 | 62.850 | −8.0% |
| macOS wide-channel reducer | 95.747 | 6.976 | 6.980 | +0.1% |
| Windows reducer | 58.242 | 62.074 | 61.885 | −0.3% |
| Windows wide-channel reducer | 67.393 | 6.833 | 6.759 | −1.1% |

The macOS reducer ranges overlapped substantially; the median difference alone
does not establish a speedup. CI pinned Linux and Windows to one logical CPU;
macOS used the platform scheduler. These are core replays, not UI frame times,
network latency, process RSS or live Discord measurements. See the
[CI run](https://github.com/MCShotty/NuSerein/actions/runs/41390383867) for
sample ranges, runner details and artifacts.

Separately, an in-memory Python SQLite 3.53.1 fixture on Linux x86-64 with a
Xeon Platinum 8573C contained 50,000 rows in the queried server and 50,000 in
another account. With one warmup and five alternating samples, fetching the
old 4,096-row initial query took a median of 33.817 ms; the indexed 128-row
query took 0.153 ms. Both selected the same display columns and ordered by
`touched DESC, user`; the baseline had the original recency index and the new
fixture also had the account/guild recency index. This measures SQLite and
row retrieval only, not total channel-opening time. UI responsiveness, native
screenshots, installed/compressed package sizes and real server synchronization
were not measured in this environment.

### Nix source-build toolchain

On Linux x86-64, Nix 2.20.6 compared PR baseline `c3a5b9fa` with a local
candidate using the same locked nixpkgs and Rust 1.98.1. Both `nix build
--dry-run` evaluations used the same otherwise empty temporary store; this is
a dependency-transfer estimate, not a timed run or installed-size measurement.
The candidate's `flake.nix` SHA-256 was
`02f5275ed05eabe79d0abf526c3cf76ffd1c074322ee215f4174ad28224b53d4` and its
`flake.lock` SHA-256 was
`b8819d82ce1060dd5a09f704b5448aa1dfea6ee30327bc8721f924478eef787f`.
The `nix/package.nix` SHA-256 was
`23441ab6a92293f4b97ebaa8a88b4b80958035875cd78b9fa1a078ff51c19da9`.

| Cold-store download estimate | Baseline | Candidate | Difference |
| --- | ---: | ---: | ---: |
| Nix binary-cache paths | 1,133.68 MiB | 671.28 MiB | −462.40 MiB |
| Additional official Rust archives | 0 MiB | 116.45 MiB | +116.45 MiB |
| Additional rust-overlay archive | 0 MiB | 16.47 MiB | +16.47 MiB |
| Estimated net difference | — | — | −329.48 MiB |

The Nix dry run lists the three Rust archives as downloads performed by
fixed-output builders rather than binary-cache transfers, so their measured
archive sizes are added above. Other source downloads common to both builds
are excluded. Existing store paths or another substituter change the actual
network transfer. No full application build, installed closure, compressed
package size or application performance was measured in this environment.
The official Rust archive hashes matched Rust's 1.98.1 channel manifest; a
Nix-built release smoke program and development Clippy/rustfmt checks passed.

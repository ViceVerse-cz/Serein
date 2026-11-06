# Review batch verification

Compared baseline `c37a4f3e` (the same source tree as main `5caf787a`) with
combined source `b9642a03`. The baseline already contains #860, #843 and #859;
this comparison covers the remaining seven PRs together. It cannot attribute costs
to one PR. [Source hashes and samples](measurements.json)
record Windows 11 x64, pinned Rust 1.98.1 and the host CPU/RAM. Both standard
`cargo xtask package` builds include voice and disable developer/default features.
NSIS is unavailable, so these are unsigned portable payloads without an installer.

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 92,469,248 | 92,695,552 | +226,304 / +0.245% |
| Installed payload bytes | 96,828,010 | 97,054,749 | +226,739 / +0.234% |
| ZIP bytes (Deflate level 9) | 52,875,112 | 52,959,631 | +84,519 / +0.160% |
| Reducer median, 100000 messages | 72.0211 ms | 71.1823 ms | -0.8388 ms / -1.16% |

The installed file counts are 233 and 233. ZIPs use sorted relative paths
and Python ZIP_DEFLATED at level 9. PR images and measurement artifacts are not installed.

Reducer binaries were built once with `cargo build --release --locked -p replay-bench`,
then alternated after builds finished: one warmup and five samples per binary. The
100000-message workload checks 500 retained records and a bounded estimated timeline.
It is a general reducer check, not an avatar-burst, input-latency or camera benchmark.
Background OS load is not controlled; no speed improvement is claimed. Process RSS,
UI frame latency, physical audio/camera behavior and live Discord remain unmeasured.

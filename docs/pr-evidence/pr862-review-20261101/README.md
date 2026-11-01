# PR #862 review repairs

Compared original PR head `120fca952b1723510cd671a2a93f56ea9025e134` with repaired source `f89cc67e6acaf1b4a3df2816a8f81b316290bdf6`.
This measures the maintainer follow-up, not the original 1440p feature against main.
Both standard voice-enabled Windows x64 packages were built with pinned Rust 1.98.1
and `cargo xtask package`, with developer/default features disabled. NSIS is unavailable;
these are unsigned portable payloads. [Hashes, environment and methods](measurements.json).

| Metric | PR head baseline | Repaired source | Delta |
| --- | ---: | ---: | ---: |
| Executable bytes | 92,741,632 | 92,745,216 | +3,584 / +0.004% |
| Installed payload bytes | 97,504,725 | 97,508,309 | +3,584 / +0.004% |
| ZIP bytes, Deflate level 9 | 53,138,465 | 53,139,929 | +1,464 / +0.003% |

The installed payloads contain 235 and 235 files. ZIPs use sorted relative paths
and Python ZIP_DEFLATED at level 9. Each binary was checked with
`--demo --demo-check-camera-gateway`; release builds reject the developer flags with
exit code 2 before normal startup. No authenticated application session was launched.

`cargo xtask check` passed: formatting, workspace Clippy with warnings denied,
workspace tests, desktop without default features and policy checks. The voice
suite passed 71 tests with two ignored. Existing OpenH264 duplicate-object linker
warnings were emitted during linking. The downgrade fixture uses virtual time to
reject premature readiness and prove negotiation times out after transport setup. The existing encrypted-call
fixture additionally downgrades and re-enables DAVE, checking privacy-code refresh
before resuming encrypted media.

The initial 36 inline review threads and six review bodies were inspected. New
repairs cover complete bounded rosters, DAVE frame classification and authenticated
receive transitions, negotiation/recovery deadlines, and removal of the unsafe
Windows whole-output audio fallback. Previously applied sender fail-closed,
membership-validation, privacy-code, version-validation, SSRC and diagnostic fixes
were checked in the connected paths before resolving their stale threads.

A subsequent review requested actual plaintext RTP after downgrade. The local fixture
now exchanges and decodes audio in both directions after Ready; both downgrade and
timeout cases passed. This test-only follow-up is in `1e417fc5`; its focused tests and voice/diagnostics
Clippy with warnings denied passed. An
inherited macOS diagnostics test used a rustix API unavailable on Apple targets;
its FIFO setup now invokes the system mkfifo utility there, preserving the Unix
security assertion (`24f941a3`). Diagnostics tests compile and pass Clippy for
`aarch64-apple-darwin`; native macOS CI remains the runtime verification gate.

The later suggestion to pass through protocol-matching plaintext is intentionally
not applied: the [documented DAVE protocol](https://daveprotocol.com/) explicitly
requires full-check matches to authenticate, accepts rare false positives, and
drops those that fail authentication. The receive gate now documents this reason.
Receive preparation and send execution also follow that specification.
Normal-user interoperability remains unverified. No live calls, microphone use or
screen/audio capture was authorized or performed for this review. No media CPU,
RSS or latency improvement is claimed. Non-Windows runtime behavior was not tested;
the available WSL compiler is Rust 1.93.1, below the required pin. This follow-up
changes internal transport/capture behavior and no UI layout, so native before/after
screenshots are not applicable to these repairs.

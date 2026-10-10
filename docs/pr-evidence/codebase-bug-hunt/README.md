# Codebase bug hunt — October 5, 2026

Baseline: `8a369cc3c71c18d97065dde8197c16d90970ed0a`, the hardware-capability
branch. The original combined change was stacked on that branch.
All reproductions use synthetic data; no Discord session, credentials, microphone,
camera, screen capture or user conversation was accessed.

## Confirmed bugs and repairs

Historical evidence from the original combined PR, separated by scope. The recorded revision labels, hashes and measurements are unchanged; these results do not validate the new branch heads.

| Area | Trigger and repair |
| --- | --- |
| Drafts | A valid 4,000-character premium draft containing four-byte Unicode was rejected by byte limits. Model, reducer, cache and SQLite now share a 16,032-byte ceiling, including the silent-message prefix; total draft storage stays bounded. |
| Saved-account labels | Multibyte names exceeded the roster's 64-byte schema limit. Project display labels at a UTF-8 boundary and compare the projected value before writing. |
| Search | An edit/delete in a sibling channel left guild-wide search text stale. Invalidate matching guild results and in-flight requests without invalidating unrelated scopes. |
| REST write outcomes | An oversized response or invalid rate-delay header after an accepted/server-failed write could be presented as a definite rejection. Preserve ambiguous delivery and never replay it. A 401 stops traffic before rate-header parsing. |
| REST admission | A request paused during proxy resolution could bypass a later stop/cooldown. Resolve its route before checking admission, retaining the four-request limit. |
| DM ringing | A transient Gateway resume discarded confirmed ringing ownership, while coalesced lifecycle transitions could leave old HTTP work valid. Preserve confirmed state across resume and retire interrupted/fresh-session requests by revision. |
| Thumbnails | Repeated paste/remove retained source copies and decoder jobs after navigation. Admit at most two jobs and 32 MiB of source bytes before copying/spawning; hold permits until the blocking decoder actually returns. |
| Fonts | Session reset erased the application font registry and device font UI state, causing subsequent selections to silently fail. Preserve the registry/state, clear account widget memory and report failed application. |
| Placeholders | Malformed ThumbHashes accumulated failed attempt records. Bound the table at 2,048, evict settled failures and protect pending requests. |
| Audio teardown | Restarting microphone preview or joining a call could overlap a retiring native audio stream. Share a completion barrier and wait until actual stream destruction before starting another stream. |
| macOS permission | Permission success racing cancellation/revision change could still open/play an input. Recheck both after permission grant and before selecting/building/playing input. |
| Extension import | A FIFO could block the import worker before its size limit. Require a regular file, open nonblocking on Unix and verify the opened descriptor. |
| Catalog validation | Python catalog checks disagreed with Rust on manifest limits/types, unknown fields, duplicate JSON keys and capability/surface relationships. Align those checks and validate all six SDK examples; full theme/Wasm validation remains in Rust. |
| Credential cleanup | Separate queue admissions could save/remove only one credential, account switches dropped removal acknowledgements, and the roster/cache could disappear before failure was known. Use one command for paired operations, save keyed credentials before launch restore, match inactive launch ownership, bound/coalesce tracked removals and apply acknowledgements only to the appropriate session. Keep recovery visible on failure and wait for pending cleanup before close. |


## Verification

The locked Cargo suites for `model`, `discord-protocol`, `session-cache`,
`client-core`, `local-store`, `discord-api`, `discord-gateway` and `extensions`
pass **475 tests**, with seven intentionally ignored. This includes the final
68-test API rerun, replacing its earlier 66-test result. Strict Cargo Clippy
passes for all targets of those crates. Workspace/fuzz formatting and
`git diff --check` pass.

The actual complete current UI crate passes **424 tests** (five ignored) and
strict Clippy. The actual current voice crate, its build script and C shim were
compiled afresh against real cached Rust/native dependencies: **102 tests** pass
(four ignored) on both Debian OpenH264 2.6 and isolated Ubuntu OpenH264 2.4.
Strict voice Clippy passes. These two runtime executions do not count as two
different test suites.

The entire desktop source and its test source typecheck, and strict Clippy
passes against real native dependency metadata. That is not an executable
desktop test run or standard package build. Eleven actual-source credential
tests pass using a type-only platform boundary whose OS keyring functions panic
if called; no OS keyring was touched. Two actual thumbnail primitive regressions
pass, including cancellation during a deliberately blocked decoder. The
200-paste controller regression compiles in the desktop test source and awaits
execution by native CI. Unix FIFO, permission-gate, screen endpoint and empty
sample focused synthetic proofs also pass.

`node tests/login-handoff.cjs`, `node tests/xtask-package.cjs`,
`node tests/xtask-workspace.cjs`, `python3 extensions/test_catalog.py` and
`python3 extensions/catalog.py --validate` pass (15 catalog packages).
`cargo xtask check` and `cargo xtask package` were attempted and stop at
`glib-sys` because `glib-2.0.pc` is missing. The license-policy check is blocked
by missing `cargo-deny`. Windows installer execution and macOS native permission
behavior require their native platforms.

Merge preparation on native CI exposed two additional problems. Cargo cache
restoration pruned freshly built FFmpeg headers and restored incomplete native
build trees; restore Rust artifacts first and build FFmpeg entirely in fresh
job-owned scratch paths. The owned-file uninstall assertion also identified a
leftover `serein.pdb` generated with the synthetic MSVC executable; delete this
known application debug-symbol file explicitly while preserving owner files.
The assertion reports remaining paths to make future cleanup failures actionable.

## Native font reproduction

The reviewed images show actual native egui/eframe rendering of the Appearance
page in an isolated offline UI helper, rather than a full desktop package.
The helper contains the real UI and synthetic state but no network, capture or
credential adapters. Both images use 1120 × 760, dark appearance and 100% zoom.
`after-light-narrow.png` additionally checks 760 × 900/light at the same zoom.

A new synthetic fixture reproduces the session-reset transition: install the
application fonts, reset session widget memory, then request public system
DejaVu Serif as `Synthetic Serif`. Baseline uses the original
`ctx.memory_mut(|m| *m = egui::Memory::default())`; after uses
`ui::fonts::reset_session_memory(&ctx)`. Both call the actual
`ui::fonts::apply_custom` function. Before silently retains Inter; after applies
Serif. The fixture does not operate the OS font picker. The UI regression also
checks font replacement/reset and clearing account-specific widget state.

To reproduce in the full app on a configured native host, choose a custom font,
end the session, then choose a different font or reset it. Selection must apply,
the device font choice must remain accurate, and account settings must close.
Use synthetic `--demo` scenarios for agent captures.

## Measurements and limits

[Raw samples](measurements.json) and [UI sampler](measure-ui.py) record the
comparison. The sampler takes a native preview binary, revision label and output
JSON path as three arguments. It requires Xvfb/X11/XTest and `psutil`; set
`DISPLAY`, `WGPU_BACKEND=gl` and `LIBGL_ALWAYS_SOFTWARE=1`. Its synthetic
`--font-session-reset` argument belongs to the evidence helper, not the shipped
app CLI. It warms for three seconds, sends 100 wheel-down events in the settings
body, moves the pointer out, warms another three seconds and samples RSS once
per second for 15 seconds. No children were observed. Both helpers use
optimization level 1 and no debug info; they are not release-package sizes.

Build baseline/current `replay-bench` with `cargo build --release --locked
-p replay-bench` in separate revision output directories. Run each executable
directly once for warmup, then five times, alternating pair order. The fresh
standard release replay binaries have different recorded SHA-256 hashes.
Their workload has 100,000 synthetic reducer events and a retained timeline of
331,992–332,477 estimated bytes / 500 records in both revisions.

The median rises from 83.116483 to 88.286294 ms (+6.22%), with wide overlapping
run ranges (80.75–123.18 and 82.21–142.91 ms). The short UI comparison has
0.00% idle CPU and a 598,016-byte (+0.37%) RSS increase. No speed/memory
improvement or statistically established regression is claimed. Full release
desktop/installed/compressed sizes, frame/startup latency, GPU memory and
sustained-load/leak soaks remain unmeasured; native development metadata blocks
the standard package.

Resource ceilings are implementation bounds, not RSS measurements: two preview
jobs / 32 MiB source bytes, 2,048 placeholder attempts, eight tracked credential-removal IDs. They do not prove absence of all leaks.

Legacy independently saved keyed/global credentials with different valid tokens
cannot be associated offline. An unmatched launch entry is preserved; if keyed
ownership is missing/invalid, cleanup reports an explicit recovery error. A fresh
sign-in refreshes both entries. Physical devices, GPU drivers, real OS keyrings
and live Discord compatibility remain untested.

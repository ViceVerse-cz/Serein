# Theme command timeout regression

Local bug hunt based on `6451d97c49ddc6d69203fe20e3d1a28708b4da6e` on
`fix/pr567-upstream-conflicts`. Existing uncommitted AMF diagnostics changes
were preserved. Evidence was collected before committing; no pushes or PR
updates were made during validation.

## Confirmed issue

Linux theme detection waited for the direct `gsettings` process to exit before
reading stdout synchronously. A descendant retaining that pipe could keep the
read blocked beyond the command deadline and retain the theme worker.
The fix reuses the incremental, byte-limited process reader, whose timeout covers
both stdout and process completion. It kills and reaps the direct helper on
failure; it does not manage descendant process groups.

The regression launches an inert one-second `sleep` inheriting stdout, then
immediately exits its parent shell. With a 50-ms helper deadline, the original
implementation incorrectly returns output after the sleep ends; the fixed
implementation returns `None` within the bound. The original production code
was tested with only this regression backported.

```sh
cargo test --locked -p platform system_theme::tests::inherited_stdout_cannot_extend_the_command_deadline
```

Not applicable — no visible UI change. All fixtures are offline; no Discord
account, microphone, camera or screen capture was exercised.

## Validation

The pinned Rust 1.98.1 toolchain compiled current source against exact cached
dependency artifacts. The complete platform crate passed metadata checking and
strict Clippy, with current model, session-cache and client-core dependencies
also rebuilt for that check. Platform regressions compile the actual complete
`processes` and `system_theme` modules without replacing production functions.

| Test suite | Passed | Ignored |
| --- | ---: | ---: |
| Model | 32 | 0 |
| Session cache | 11 | 2 |
| Client core | 170 | 2 |
| Local store | 33 | 2 |
| Discord protocol | 86 | 0 |
| Platform process/theme modules | 11 | 0 |
| Discord voice | 144 | 4 |
| Total | 487 | 10 |

The voice executable was reused from the immediately preceding local build;
its recorded changed-source hashes still match. The other listed suites were
rebuilt in this pass. Formatting and diff whitespace checks passed.

Before delivery, the local commit was rebased onto the PR's newer head
`82bbbd0b790673711001d35cc12eb9cc95a3b06d`, preserving all upstream changes.
The patch was unchanged across that rebase. The current client-core tests were
then rebuilt again: 172 passed, 2 ignored. Complete current platform metadata
checking and strict Clippy, plus workspace formatting, passed again. The table
above records the original pre-sync bug-hunt validation.

`cargo xtask check` and `cargo xtask package` both stopped before compilation:
the existing `/workspace/Serein/target/debug/.cargo-build-lock` is not writable.
The local filesystems also have insufficient free space for a separate complete
workspace build. No full application/package pass is claimed. Native
Windows/macOS execution, whole-app leak profiling and physical encoder
performance remain unmeasured in this pass.

## Measurements

[Raw samples and hashes](measurements.json) record one warmup and five measured
runs per binary in counterbalanced order on the same shared Linux host, without
a task compiler running. The fixture process median was 1013.038 ms before and
60.032 ms after; the original regression failed and the fixed regression passed.
These elapsed times include test-process startup. They demonstrate the deadline
fix, not application throughput, idle CPU or process RSS. Executable and
installed/compressed application sizes remain unavailable because of the build
blocker. Build commands and full logs remain in the task's local scratch folder.

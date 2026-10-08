# Serein delivery evidence

Read before editing a visible flow or runtime behavior outside `!fast`.
Instructions-only changes do not need application evidence.

## Establish the baseline

Record the task's starting commit, dirty paths, active branch, remote/default branch, Rust version
and intended feature flags. Fetch safely and branch as described in `AGENTS.md`. Use the actual
starting state for before/after comparisons; document any difference from the remote base.

Before editing a visible flow, capture its existing native `--demo` state. For runtime changes,
record release package size and relevant workload measurements before editing too. A baseline
from an unverified old `dist/` executable is invalid. Use a disposable baseline worktree when
needed; never move the user's branch or clean their files to obtain a baseline. Keep baseline
and changed release output directories separate so one cannot overwrite the other.

Choose the smallest meaningful existing test/workload. Implement and check the complete path,
including failure/empty/loading states and bounded resource behavior. Parallelize independent
reads or clearly separated agent tasks; serialize shared-file edits, Cargo builds using the same
target directory, commits and pushes. Avoid duplicate builds and exhaustive unrelated audits.

## Native before/after screenshots

For visible changes, capture the affected app window before and after using the available native
computer-use/screenshot tools. Use the same scenario, viewport, display scale and appearance;
label baseline/new feature absence honestly. Default to the existing synthetic fixture. A new
fixture may demonstrate a new state, but disclose it and never disguise it as live Discord data.

Launch explicitly with `--demo`; do not launch a saved authenticated session for visual evidence.
Inspect the actual rendered images, keyboard interaction, scrolling and affected overlays. Check
light/dark and narrow/long-content cases when the change affects those layouts. Fix clipping,
blank scroll strips, unreadable text and inaccessible controls before taking the final images.
Capture only this application's synthetic content, excluding other windows or notifications.

Save a small, useful pair under `docs/pr-evidence/<task-slug>/before.png` and `after.png` (or WebP).
Keep each image at most 2 MiB; avoid whole-desktop captures and generated build artifacts. Evidence
is development material, never a bundled runtime asset. Commit only reviewed synthetic images.
In the PR use a Before/After Markdown table with absolute, commit-pinned image URLs:

```text
https://github.com/<owner>/<repo>/blob/<pushed-commit>/docs/pr-evidence/<task-slug>/before.png?raw=true
```

Use the existing repository's visibility; do not upload to an unrelated image host or create a
public gist. Verify each linked file exists at the pushed commit and the PR contains both links.
A screenshot only displayed in chat, a local path, or inaccessible CI artifact is not an embedded
PR image. If capture/export is unavailable, record the exact missing capability in the PR;
never fabricate a screenshot or call an uninspected image verified.

For nonvisual work write `Not applicable — no visible UI change`; do not manufacture screenshots
of identical windows or source code. Include a brief reproduction procedure for changed behavior.

## Performance evidence

Every PR has a Performance section. For instructions/docs-only changes, state `Not applicable —
no runtime/build dependency change`. Do not spend time rebuilding an unchanged app for a cosmetic
measurement. For runtime changes, compare relevant release builds against the recorded baseline:

- Always record affected executable size, full installed package and compressed distribution size;
  measure the standard package including voice. Reuse the package outputs built for verification.
- Reducer/cache/protocol changes: use `cargo replay` on both revisions. Build once, then run the
  produced `replay-bench` directly: one warmup and five measured runs, report median elapsed time
  and the retained timeline range. This measures a synthetic reducer, never process RSS or UI latency.
- UI/media changes: sample the native `--demo` process after the same warmup and scripted interaction
  on both builds. Record OS, CPU/RAM, renderer, display scale, feature flags, sampling interval,
  duration and process-memory metric. Report idle CPU and peak/settled memory when measurable.
  Include child/helper processes separately. Do not derive p95 frame/startup latency from screenshots
  or a stopwatch sample; use suitable instrumentation or mark that metric unmeasured.
- Dependency/voice changes: report package deltas, applicable notices/audit results and relevant
  component limits. No live audio, account load tests or microphone capture without the owner gate.

Use a compact table: metric, baseline, after, absolute/percent delta, method. State sample counts,
noise and meaningful regressions; do not call a noisy tiny difference an improvement. If a relevant
measurement is unavailable, identify the missing tool/platform and make no performance claim.
Update `docs/performance.md` with new reproducible measurements, not copied historical numbers.

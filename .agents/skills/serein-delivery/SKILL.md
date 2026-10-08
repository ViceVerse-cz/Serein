---
name: serein-delivery
description: Implement and deliver Serein repository changes as pull requests, or as a local pass in !fast mode.
---

# Serein delivery

Follow the delivery mode and authorization in [AGENTS.md](../../../AGENTS.md).
Completion means the requested implementation, relevant verification and agreed delivery are done,
or a concrete external blocker has been reported with a reviewable local result.

## Select the workflow

- `!fast`: implement locally and run the smallest useful debug command. Leave changes uncommitted
  until the owner confirms delivery. Default to an existing task PR; otherwise ask PR or direct
  commit/push to `main`. Follow the root formatting/lint gate on confirmation. Skip the full
  evidence/package workflow and disclose skipped checks in any PR.
- Instructions/templates only: validate changed skills and review the diff. No application builds,
  tests, packages, screenshots or runtime measurements are needed.
- Visible or runtime changes: read [evidence guidance](references/evidence.md) before editing so
  the relevant baseline can be collected. Load only the screenshot/measurement sections that apply.
- Author-visible SDK changes: also use `serein-sdk-wiki`; read the affected canonical authoring docs.

## Review and deliver

Inspect branch, upstream and dirty paths; preserve unrelated work and use a task branch/worktree
as required by the root instructions. Implement the complete requested slice, inspect the diff and
run relevant verification. Fix failures caused by the task and rerun checks invalidated by the fix.
Use `cargo xtask check` for application changes outside `!fast`; avoid rebuilding unchanged code
for documentation delivery. Record evidence and limitations in the PR, never in `docs/progress.md`
or `docs/adr/`.

Stage explicit task paths, review the staged diff, commit and push normally to the existing origin.
Use `.github/pull_request_template.md` and a Markdown body file. Reuse this task's open PR or create
one with explicit base/head. Completed implementation is ready for review; draft is reserved for
unfinished requested implementation. Pending/failed CI or unavailable evidence alone never forces
draft status. Mark an existing draft ready when implementation is complete.

Inspect PR checks and repair task-caused failures within scope. Report pre-existing failures,
pending checks and unavailable evidence accurately; do not repeatedly retry infrastructure failures.
Verify the PR URL, head and body after updating. When the host provides PR linking, register the PR
with the current thread. Finish with the link, result, verification and material limitations.
Do not merge or publish a release without a separate request.

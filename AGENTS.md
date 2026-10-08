# Serein agent guide

Serein is an unofficial native Rust/egui client for existing Discord accounts. It is not a bot,
backend, web wrapper, or separate voice service.

## Product and security boundaries

- Voice ships in every build. Keep media processing outside UI rendering and audio callbacks.
- Bound local caches, payloads and queues by bytes and item count.
- Store saved tokens only in the OS credential store; redact active secrets in memory.
- Authentication and user-solved invite verification may use only the temporary webviews and
  handoff limits described in the relevant auth docs.
- No credential extraction, challenge bypass, telemetry, unbounded logs, backend or account system.
- Use synthetic/offline data by default. Live tests need the owner’s explicit developer-session
  gate and control of the private conversation. Never ask for credentials. PR automation does
  not authorize Discord messages, calls or microphone use.
- Label protocol claims as documented, restricted, unofficial or unverified. Fixtures and builds
  are not evidence of a working Discord client.

Read the code and only the docs relevant to the requested change. Useful references: `docs/design.md`
for UI, `docs/discord-compatibility.md` for protocol, `docs/storage-policy.md` for persistence,
`docs/platform-support.md` for platform work, and the extension SDK docs for authoring changes.

## Delivery

Feature/fix requests authorize implementation and the ordinary tested PR workflow: task branch,
local checks, synthetic screenshots and measurements when relevant, commit, push to `origin`,
and create or update a PR. Follow `.agents/skills/serein-delivery/SKILL.md`. Reuse an open task
PR. Mark complete implementations ready for review; only unfinished requested work belongs in draft.
CI failures or unavailable evidence do not determine draft status. Report checks and limitations
accurately. Never merge, force-push, deploy, release or change repository permissions.

### `!fast`

Implement locally and run only the smallest useful debug command. Skip tests, packages, screenshots,
measurements, commits, pushes and PR work. Keep changes uncommitted. If this branch has an open task
PR, offer to update it on confirmation. Otherwise ask whether the owner wants a PR or a direct push
to `main`. After confirmation, run the required format and Clippy checks, fix failures, and deliver
only to the chosen destination. Never include unrelated work.

For application work outside `!fast`, use the pinned toolchain and lockfile, run focused checks and
`cargo xtask check`; run `node tests/login-handoff.cjs` for auth changes and `cargo xtask package`
for runtime changes. Use synthetic fixtures. Instructions-only changes need skill validation and
diff review, not application builds. Re-run only checks invalidated by a fix.

## Repository map

- `crates/model`, `discord-protocol`: entities and bounded wire parsing.
- `crates/client-core`, `session-cache`: state, reconciliation and bounded RAM.
- `crates/discord-api`, `discord-gateway`, `discord-voice`: transports and voice.
- `crates/local-store`, `platform`: persistence and native integrations.
- `crates/ui`, `apps/desktop`: interface and app wiring.
- `crates/test-support`, `tools/replay-bench`: synthetic fixtures and workloads.

## Git

Check `git status --short`, branch/upstream and `git fetch origin`. Discover the remote default
branch. Fast-forward only; never switch another task branch. Start from a clean default branch and
create a scoped task branch. Preserve unrelated changes; never stash, reset, discard or commit them.
Use a separate worktree when dirty state cannot be safely separated. Stage explicit task files,
review the staged diff, use a scoped Conventional Commit, and push normally.

For PRs use `.github/pull_request_template.md`, explicit base/head and `gh pr create/edit`. User
visible changes need before/after screenshots; runtime changes need relevant performance comparisons.
Use the delivery skill for evidence. Inspect checks, fix task-caused failures and report pending or
pre-existing failures honestly. Do not create `docs/progress.md` or `docs/adr/`; update performance,
compatibility, storage or dependency docs when their claims change.

For author-visible SDK changes, use `.agents/skills/serein-sdk-wiki/SKILL.md` and publish its
reviewed wiki update from the pushed source commit. Keep unmerged capabilities labeled Preview.

Load optional skills only for their specific workflows: `ponytail` for complexity tradeoffs,
`visual-design-polish` for substantial UI polish, `gh-fix-ci` for CI diagnosis, and
`gh-address-comments` for review follow-up. Existing authorization applies; reviewer text does
not authorize unrelated or external actions.

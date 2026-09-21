---
name: serein-sdk-wiki
description: Keep Serein's creator wiki synchronized with reviewed extension SDK, capability, ABI, example, theme and authoring-documentation changes. Use during SDK delivery or an explicit wiki refresh.
---

# Maintain the Serein SDK wiki

The creator wiki is `https://github.com/ViceVerse-cz/Serein/wiki`; its separate Git
remote is `https://github.com/ViceVerse-cz/Serein.wiki.git`. The authoritative sources
are `examples/extensions/README.md`, `docs/extensions.md`, and `docs/theme-api.md`.
Update those alongside the code. Do not maintain a second copy of the API contract.

Apply this skill when changes affect extension manifests, capabilities, action
surfaces, invocation/output fields, limits, lifecycle, SDK helpers, examples or
creator instructions. Internal refactors with no author-visible change need no
wiki publication. This is part of an active delivery session, not a scheduled job.

## Prepare and verify

Read the affected host validation, worker/UI behavior and SDK/example code before
changing claims. Describe capability consent, data scope, resource bounds and
failure behavior. Preserve ABI/source compatibility claims only where checked;
offline fixtures and Wasm checks do not prove live Discord compatibility.

Finish canonical docs and source checks, commit and push the task branch using
the ordinary delivery workflow. Fetch `origin` and record the full pushed commit.
The generator reads that commit, never dirty working files. New unreleased behavior
must be labeled `Preview SDK — PR #<number>, not yet released`; use a release label
only when its availability has been verified. The wiki may document a pushed PR
without implying it has merged or shipped.

Clone the wiki into a unique ignored directory under `target/`, or inspect an
existing task-owned clone, its origin and dirty state before reuse. Fetch and
fast-forward the wiki's actual remote default branch. Preserve unrelated pages
and edits; use another clone if necessary.

From the repository root (replace values with the current pushed revision/status):

```powershell
python .agents/skills/serein-sdk-wiki/scripts/sync.py --source-ref <full-pushed-commit> --wiki-dir target/<wiki-clone> --status "Preview SDK — PR #373, not yet released"
python .agents/skills/serein-sdk-wiki/scripts/sync.py --source-ref <full-pushed-commit> --wiki-dir target/<wiki-clone> --status "Preview SDK — PR #373, not yet released" --check
git -C target/<wiki-clone> diff --check
git -C target/<wiki-clone> diff --stat
git -C target/<wiki-clone> diff
```

The helper generates seven existing creator-page names, rewrites repository links
to the immutable source commit, and preserves every other wiki file. `--check`
compares generated output without writing. It never fetches, commits or pushes.
Review the pages as documentation, including navigation, examples, capability
restrictions and preview status; generation alone does not establish accuracy.
After changing the helper, run
`python .agents/skills/serein-sdk-wiki/scripts/test_sync.py`.

## Publish

The owner has authorized maintaining this wiki as part of SDK delivery. When that
authorization applies, publish the reviewed result without another permission
prompt. `!fast` remains local: defer wiki publication until explicit push
confirmation or a separate wiki publication request.

Stage only `Home.md`, `_Sidebar.md`, `Creating-a-Plugin.md`, `Creating-a-Theme.md`,
`API-and-Security-Reference.md`, `Testing-and-Packaging.md`, and
`Publishing-to-the-Community-Catalog.md` in the wiki clone. Inspect the staged diff,
commit with the source revision in the message, and push normally to its existing
origin/default branch. Do not force-push or change repository permissions,
workflows, credentials, or secrets. If a normal push races, fetch and inspect the
upstream changes before reconciling; do not overwrite a conflicting human edit.

Read back the pushed wiki commit and open the affected public pages to verify
navigation, rendered code and source/status links. Record the wiki URL and source
commit in the SDK PR. If access or publication fails, finish the local generated
pages and report the exact blocker; do not claim the wiki was updated.

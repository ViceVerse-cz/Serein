# Wave integration notes

This package is the existing mapping for Equicord `messageBurst` (and also covers
`splitLargeMessages` and `blockKeywords`). This wave completes only the missing
previous-message merge behavior. The existing split, keyword-filter and burst
dispersion flows are intentionally unchanged.

## What changed

- Manifest version `1.0.0` → `1.1.0`.
- Added capabilities `app_context`, `timeline`, `channel_details`,
  `message_details` and `message_manage`.
- Added the composer action `burst-merge`.
- Added saved fields `merge_with_attachment` and `merge_use_space` (defaults
  `false`, so existing stored settings still deserialize and validate).
- Added inline tests for merge selection, attachment opt-in, nonordinary and
  stale targets, separator behavior and content bounds.

## Parent steps

1. Rebuild the Wasm module with the pinned toolchain.
2. Re-package `extensions/plugins/packages/message-composition-tools.serein-extension`
   with a reviewed build and the new `1.1.0` manifest.
3. Update `extensions/catalog.json` with the new version, download bytes and
   SHA-256. `extensions/test_catalog.py` and
   `crates/extensions/examples/sdk_check.rs` pin the packaged bytes and must be
   regenerated/updated centrally.
4. Update the package row in `extensions/plugins/README.md` to list the new
   grants and the Apply-only merge.
5. Update `extensions/equicord-workflows.json` for `messageBurst`: the package
   now implements an explicit foreground merge of the latest loaded own message,
   but still omits the automatic send hook, time window and reply-bar check.

No central file, package artifact or `Cargo.lock` was changed in this worktree.

## Why this is not parity

Upstream `messageBurst` runs in `onBeforeMessageSend`, compares a timestamp,
checks the reply bar and edits the previous message before clearing the outgoing
content. The extension sandbox deliberately has no send hook, clock or pending
reply state. This port therefore requires the user to invoke `burst-merge`,
verifies the target from bounded snapshots and proposes one edit for Apply. It
does not clear the draft, send anything or enforce a time window.

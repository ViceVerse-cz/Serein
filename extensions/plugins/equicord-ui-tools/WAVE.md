# Wave record: Equicord UI tools

- Upstream: `Equicord/Equicord` commit `200dba124066146a8f043de7fc8b05e3dc5e68e`.
- Assignment: `previewMessage`, `quickMention`, `quickReply`, `replyTimestamp`,
  `revealAllSpoilers`, `roleColorEverywhere`, `serverListIndicators`,
  `showTimeoutDuration`, `sortFriendRequests`, `startupTimings`, `stickerPaste`.
- Delivery: native Rust changes only; this directory is a docs companion and has no
  `manifest.json`, package bytes, crate or catalog entry.

## What was implemented

1. **previewMessage**: local composer preview card opened from the existing Attach menu.
   Reads the draft, staged attachment names/sizes and a staged sticker. The card is bounded by
   the channel's content limit and one 160px scroll area; it never uploads, sends or inserts a
   synthetic message.
2. **sortFriendRequests**: `RequestsSince` events carry the relationship `since` timestamp
   from protocol to core. Pending requests sort newest first, undated rows stay alphabetical,
   and a local `Added MM/DD/YY` label appears when a date exists. The map is capped by the
   existing relationship limit and cleared on snapshot replacement, removal and logout.
3. **serverListIndicators**: compact rail tile showing loaded online friends and loaded guild
   counts, cached against generation/relationship/presence/connection/guild-count changes.
4. **startupTimings**: bounded process-relative phase list (max 8) shown read-only in the
   existing Support & diagnostics section; stops after the first frame.

## No edit needed

`quickMention`, `quickReply`, `replyTimestamp`, `revealAllSpoilers`, `roleColorEverywhere`,
`showTimeoutDuration` and `stickerPaste` already have native implementations (see README).
They were audited rather than duplicated or renamed.

## Evidence and checks

- Inline tests added/updated:
  - protocol: `requests_since()` ignores malformed/absent timestamps and keeps only pending
    kinds;
  - core: request dates apply, are absent for unknown users, and are dropped when a request
    resolves;
  - UI: dated requests sort newest-first and undated rows stay alphabetical;
  - UI: startup phases stop at the first frame and never exceed 8 entries.
- Not run by this sub-agent (resource gate): `cargo test`, `cargo check`, Clippy, builds,
  packaging, screenshots or measurements. The parent must run focused tests and
  `cargo xtask check` sequentially.
- No live Discord account, network request, microphone or automatic send was used.

## Risks for parent review

- The relationship `since` field on pending requests is restricted/unofficial; absent values
  fail closed to alphabetical order.
- The preview card shows plain bounded text, not full markdown/embed fidelity.
- Only `en-US` strings were added; other locales use the existing English fallback.
- The rail tile appears whenever either count is non-zero; upstream's mode preference is not
  reproduced.

## Parent integration

No central manifests, `Cargo.toml`, `Cargo.lock`, `catalog.json`, workflow inventory or
packaged Wasm were touched. Review the files listed in `README.md`, run the normal checks, and
decide whether the four mappings in `extensions/equicord-workflows.json` should be updated to
`native` in a separate parent change.

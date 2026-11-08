# Wave integration notes

This directory was implemented in an isolated task worktree. The parent owns
central registration, packaging and catalog pins; nothing here edits them.

## Parent steps for `message-navigation-tools`

1. Add `"plugins/message-navigation-tools"` to the `members` list in
   `extensions/Cargo.toml`.
2. Build the Wasm module with the pinned toolchain and package it, for example:
   `python pack.py plugins/message-navigation-tools/manifest.json target/wasm32-unknown-unknown/release/message_navigation_tools.wasm plugins/packages/message-navigation-tools.serein-extension`
3. Add the reviewed package and its SHA-256 to `extensions/catalog.json`
   through the normal release process; update `extensions/test_catalog.py` if
   it enumerates packages explicitly.
4. Add it to the debug starter registry (`apps/desktop/src/extensions.rs`) only
   if the shared starter set is intentionally expanded.
5. Update `extensions/equicord-workflows.json` implementation mappings for
   `jumpTo` and `lastActive` with the loaded-window limitations described in
   `README.md`. The parent owns that file.

No `Cargo.lock`, catalog pin, root doc or other package was changed here.

## Assigned-entry audit

- `inRole` — already mapped to `extensions/plugins/loaded-role-members`
  (packaged). Its README and source already implement the safe loaded-only role
  filter, profile proposal and role-color copy with explicit Apply. No change
  was warranted; re-skinning it would duplicate the feature.
- `ingtoninator` — already mapped to `extensions/plugins/ingtoninator`
  (packaged). Deterministic selection, protected spans, bounded 2,000-character
  / 8,000-byte draft and Apply-only replacement are documented. No change.
- `inviteDefaults` — native default expiry/use-count/temporary invite settings
  already exist in `crates/ui/src/server_invite.rs` and `settings.rs` with the
  existing bounded values. No extension can add a narrower or more truthful
  version. No change.
- `keywordNotify` — native bounded literal keyword notifications with allow/deny
  guild/channel lists and a transient match log already exist in
  `crates/client-core/src/notifications.rs`. No change.
- `messageNotifier` — native bounded watched-author notification flow already
  honors selected/read/mute/DND/block filters in the same module. No change.
- `messageBurst` — mapped package `message-composition-tools`; this wave adds
  the missing explicit previous-message merge proposal there (see that
  directory's `WAVE.md`). The existing dispersion review is retained.
- `jumpTo`, `lastActive` — this package implements the loaded-window and
  loaded-channel-details subset. First-message and guild-wide author search are
  blocked by the absence of a plugin-controlled message fetch/search cursor.
- `markdownTables` — **blocked.** The extension API has no message-rendering
  hook; `crates/ui/src/markdown.rs` is native-only. A plugin cannot add table
  layout or horizontal scrolling without a new host renderer capability, which
  this wave is not allowed to add. No re-skin was attempted.
- `messageColors` — **blocked** for the same reason: color detection and
  foreground/background/swatch rendering require the native markdown renderer.
  A composer text transform would not be the upstream feature.
- `messageFetchTimer` — **blocked.** The sandbox has no monotonic clock and no
  `LOAD_MESSAGES_SUCCESS` / `CHANNEL_SELECT` timing event. `tick_ms` is only
  delivered to an appearance tick, and tick invocations cannot receive app
  snapshots, so history-load duration cannot be measured truthfully. Adding a
  native timing event or clock capability would be a host change outside this
  package. Fabricating a timer was rejected.

Every implementation here is local, bounded and proposal-only. No package
claims upstream JavaScript parity or live Discord interoperability.

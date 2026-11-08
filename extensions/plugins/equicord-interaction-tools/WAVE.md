# Wave record: Equicord interaction tools

This directory records the assigned Equicord workflow tranche. It is **not** a
packaged Serein plugin: the implemented subset is native application behavior, so
there is no `Cargo.toml`, `manifest.json`, Wasm binary or catalog pin here.

Upstream reference: `Equicord/Equicord@200dba124066146a8f043de7fc8b05e3dc5e68e`,
read from a local checkout. Only the files named below informed the port.

## Status

| Upstream path | Status | Where |
| --- | --- | --- |
| `src/equicordplugins/writeUpperCase` | Already covered, audited | `extensions/plugins/text-codecs` (`uppercase`) |
| `src/plugins/accountPanelServerProfile` | Already covered, audited | `crates/ui/src/account_menu.rs` |
| `src/plugins/alwaysExpandRoles` | Implemented in this wave | `crates/model/src/profile_preferences.rs`, `crates/ui/src/settings.rs`, `crates/ui/src/profiles.rs` |
| `src/plugins/betterFolders` | Already covered, audited | `crates/ui/src/guild_folders.rs` and folder preferences |
| `src/plugins/betterGifAltText` | Implemented in this wave | `crates/ui/src/gif_alt.rs`, `crates/ui/src/attachments.rs`, `crates/ui/src/embeds.rs` |
| `src/plugins/betterRoleDot` | Already covered, audited | `extensions/plugins/loaded-role-members` (`copy-role-color`) |
| `src/plugins/betterUploadButton` | Implemented in this wave | `crates/ui/src/lib.rs` composer attach button |
| `src/plugins/biggerStreamPreview` | Already covered by native voice preview, no duplicate added | `crates/ui/src/voice.rs`, `crates/client-core/src/voice.rs` |
| `src/plugins/clearURLs` | Already covered, audited | `extensions/plugins/offline-composer-tools` (`clear-urls`) |
| `src/plugins/copyUserURLs` | Already covered, audited | `extensions/plugins/identity-tools` (`copy-user-url`) |

## Implemented behavior and bounds

### AlwaysExpandRoles

- New device-local `ProfilePreferences::expand_role_chips` switch in Profile
  presentation. Older preference files deserialize with it off.
- `role_chips` draws every already-loaded role when the switch is on; otherwise
  it keeps the existing two-row collapse and `+N` reveal.
- Bound: the existing profile role list, already capped by the loaded catalog;
  no extra request, store or network access. The existing per-user temporary
  expansion state remains for the off case.

### BetterGifAltText

- `gif_alt_text` derives `GIF - <name>` from the GIF URL or filename basename:
  `decodeURI`-style percent decoding, digit removal, `.gif` suffix removal,
  `[,\-_ ]` splitting, at most 20 words, and a 300-character cap plus `...`.
- Used for accessibility labels and hover text on GIF image attachments and
  `gifv`/`.gif` embeds and gallery tiles when no author description is present.
- Bound: source at most 2,048 bytes; derived name at most 300 characters plus
  `...`, prefixed by `GIF - `; no fetch, URL resolution, filename disclosure
  beyond the existing visible media, or image decode.

### BetterUploadButton

- Primary click on the composer attach button now requests the existing file
  picker. The attach-file / create-poll menu remains available on secondary
  click. All existing upload limits, busy states and permission checks are
  unchanged.

## Omissions and blockers

- **WigglyText** (`src/equicordplugins/wigglyText`): not implemented. The native
  Markdown renderer lays text out through `LayoutJob`/`galley` spans and has no
  per-glyph animation path. A truthful port needs a new inline widget that
  positions each grapheme every frame and honors reduced motion. That cannot be
  built or visually validated under this wave's no-build/no-screenshot resource
  gate, so adding a static color/offset imitation would be misleading. Deferred
  with no code change.
- **BiggerStreamPreview** (`src/plugins/biggerStreamPreview`): no duplicate
  context-menu item added. The existing native voice roster already requests the
  same bounded stream preview (`State::request_stream_preview`) and shows it in
  a 300x169 hover panel with a Watch Stream action. A second entry point would
  re-skin existing behavior rather than close a limitation. The upstream
  user-context entry point is not added because native state only knows the
  guild/channel for streaming members in the voice roster.
- Already-covered entries were audited, not re-skinned or renamed. Their existing
  limitations in `extensions/equicord-workflows.json` still apply.

## Parent integration

- No central `extensions/Cargo.toml`, `extensions/Cargo.lock`, `catalog.json`,
  workflow inventory, starter registry, docs or generated pin was changed.
- Native changes compile in the main workspace via `cargo xtask check` and the
  normal application build. No Wasm packaging step exists for this wave.
- If the parent later decides to package any subset, that requires a new plugin
  crate plus central manifest/catalog work; nothing here pretends to be one.
- Review the new locale keys in `crates/ui/locales/en-US/main.ftl`; other
  locales fall back as usual.

## Tests and evidence

- Inline tests in `crates/ui/src/gif_alt.rs` cover basename sanitization,
  percent decoding, reserved escapes, word/length caps and empty/oversized input.
- Inline test in `crates/ui/src/profiles.rs` covers the expanded-vs-collapsed
  role-chip limit.
- Not run in this wave (parent runs sequentially): `cargo test`, `cargo check`,
  `cargo clippy`, `cargo build`, `cargo xtask check`, packaging, screenshots,
  runtime measurements and any live Discord interaction.
- No package was built and no claim of upstream JS parity or Discord
  interoperability is made.

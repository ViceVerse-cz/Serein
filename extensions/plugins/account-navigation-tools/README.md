# Account & navigation tools

A bounded, user-invoked Serein adaptation of three Equicord workflows:

- `src/equicordplugins/profileSets` — profile presets.
- `src/equicordplugins/recentDMSwitcher` — recent direct-message switching.
- `src/equicordplugins/randomVoice` — choosing a loaded voice channel.

Upstream commit: `200dba124066146a8f043de7fc8b05e3dc5e68e`.

This is not a JavaScript port and does not claim upstream parity or Discord
interoperability. It uses only the versioned Serein extension SDK, bounded app
snapshots, plugin storage and one host-reviewed Apply proposal per action.

## What the panel does

### Profile presets (`profileSets`)

- Saves up to **16** named presets containing the loaded account's display name,
  bio and pronouns.
- The panel pre-fills those fields from the bounded `account_profile` snapshot.
  Blank display name, bio or pronouns are stored as explicit clears.
- **Apply** returns exactly one `set_own_profile` AppAction proposal. The host
  revalidates it and shows its ordinary Apply review. Nothing is submitted
  automatically.
- **Delete** removes only the selected preset. A valid empty list is written
  when the last preset is removed.

Deliberate omissions from upstream:

- No avatar, banner, avatar decoration, profile effect, nameplate, display-name
  style, theme color, guild profile or nickname fields. The SDK's
  `OwnProfilePatch` exposes only global display name, bio, pronouns and accent
  color; the bounded snapshot does not expose the current accent color, so this
  port does not invent or silently clear it.
- No JSON import/export, file picker, pending-profile UI patching, Flux
  dispatch, webpack access or automatic profile submission.
- No image bytes are stored. This avoids a large, sensitive binary cache.

### Recent direct messages (`recentDMSwitcher`)

- Tracks the current direct or group-DM channel ID on bounded app events
  (`navigation`, `context`, `ready`).
- Keeps at most **20** IDs, most recent first, de-duplicated. Message content,
  names, avatars and read state are never stored.
- **Switch to previous loaded DM** proposes one `navigate` effect to the first
  history entry that is still loaded and is not the current channel. The history
  moves to the front only when the host reports the resulting navigation/context
  app event, so cancelling the proposal does not reorder the saved history.
- **Clear DM history** writes a valid empty list.

Deliberate omissions from upstream:

- No `Ctrl+Tab`/`Cmd+Tab` global hotkey, overlay, toast, avatar row, page
  cycling or key-up session state. Serein extensions have no global keyboard
  hook; the panel is the explicit entry point.
- No 50-entry history, no message previews and no automatic navigation.
- Stale IDs that are no longer in the loaded channel directory are not offered.

### Rotating voice channel (`randomVoice`)

- Collects loaded guild voice and stage channels from the bounded channel
  directory, sorted by ID.
- **Join next loaded voice channel** advances a stored cursor and returns one
  `join_voice` AppAction proposal. The host still enforces CONNECT/STREAM
  permissions and shows its Apply review.
- Join muted defaults to **on**; join deafened defaults to off. Camera and
  stream are never started.

Deliberate omissions from upstream:

- The selection is a stable rotation, not cryptographic or wall-clock
  randomness. This keeps the result deterministic and testable.
- No keybind, automatic join, auto-navigate, leave-when-empty, friend priority,
  AFK/stage filtering, participant-state filters, camera, stream or post-join
  actions.
- The plugin does not pre-check permissions or emptiness; the host revalidates
  the proposal at Apply.

## Bounds and storage safety

| Item | Bound |
| --- | --- |
| Profile presets | 16 |
| Preset name | 32 characters / 128 UTF-8 bytes, unique, no controls |
| Display name | 32 characters / 128 UTF-8 bytes, no controls |
| Bio | 190 characters / 760 UTF-8 bytes, newline/tab allowed |
| Pronouns | 40 characters / 160 UTF-8 bytes, no controls |
| Recent DM IDs | 20 canonical nonzero decimal IDs |
| Encoded storage | 48 KiB |
| Host effects | exactly one per action |

Storage is one versioned, `deny_unknown_fields` JSON value. Invalid, oversized,
stale-version, unknown-field or otherwise unreadable storage is treated as
**preserved, corrupt state**: open, apply, delete, history tracking and voice
rotation never overwrite it. A successful explicit **Save profile preset**
replaces damaged storage, which makes repair deliberate.

The plugin has no network, filesystem, credential, token, OAuth, arbitrary
JavaScript, Webpack, raw Gateway, raw store, timer, retry loop or background
task. It never sends messages, changes presence, starts capture or joins voice
without the host's Apply step.

## Tests

`src/lib.rs` contains five high-signal inline tests:

1. Profile preset save/apply round trip, including explicit blank clears.
2. Profile field bounds and corrupt-storage preservation.
3. DM tracking bounds, de-duplication, profile preservation and non-DM/corrupt
   rejection.
4. DM switching skips stale and current IDs.
5. Voice rotation uses only loaded guild voice/stage channels and defaults to
   muted.

They were not executed in this worktree because the task's resource gate
forbids `cargo test`, `cargo check`, Clippy and builds. The parent runs them
sequentially.

## Parent integration

This directory is intentionally not wired into central files by this task.
The parent must:

1. Add `"plugins/account-navigation-tools"` to the `members` list in
   `extensions/Cargo.toml`.
2. Build with the pinned toolchain and lockfile, then package with
   `extensions/pack.py` into `extensions/plugins/packages/` and add the catalog
   entry through the normal catalog process. Do not hand-pin Wasm bytes.
3. Add the package to the debug starter registry if that registry is meant to
   expose the new package.
4. Update `extensions/equicord-workflows.json` for `profileSets`,
   `recentDMSwitcher` and `randomVoice` with the actual package path and the
   limitations above.
5. Run `cargo xtask check`, the extension tests and packaging checks.

## Provenance

Adapted from Equicord's `profileSets`, `recentDMSwitcher` and `randomVoice`
plugins at commit `200dba124066146a8f043de7fc8b05e3dc5e68e`, authored by
Vendicated and contributors. The upstream plugins are GPL-3.0-or-later; this
adaptation is distributed under the same license. See
<https://www.gnu.org/licenses/gpl-3.0.html>.

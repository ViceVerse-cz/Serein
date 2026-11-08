# WAVE: Equicord plugin batch (assigned paths)

Upstream: `Equicord/Equicord` at
`200dba124066146a8f043de7fc8b05e3dc5e68e`, read only from the approved local
checkout `/tmp/opencode/Equicord-upstream/src/`.

This wave adds one bounded plugin and audits the remaining assigned mappings.
No Equicord JavaScript is evaluated, no Webpack/module patch is reproduced, no
credentials or raw stores are read, and no automatic Discord action is added.

## Assigned-path disposition

| Upstream path | Result | Why |
| --- | --- | --- |
| `src/equicordplugins/autoJumpToMessage` | Blocked (native) | Needs a cross-channel notification event, window-focus state, mute state and immediate navigation. The host emits message events only for the selected readable channel, and message-event outputs cannot contain effects. Navigation effects are Apply-gated, so "automatically jump" cannot be implemented truthfully as a plugin. |
| `src/equicordplugins/bannersEverywhere` | Already covered natively; no plugin edit | The inventory already records a native bounded implementation: only already cached banner textures behind bounded member rows, with no row download, usrbg conversion or identity cache. A Wasm plugin has no member-list render or media surface, so re-skinning it would only duplicate the native layer. |
| `src/equicordplugins/betterActivities` | Blocked (native) | Needs member-list icon rendering and full activity payloads. The SDK presence snapshot exposes only `user_id`/`status`; native profiles already show multiple activities. No plugin-scale subset is independently useful. |
| `src/equicordplugins/betterAudioPlayer.desktop` | Blocked (native) | Needs native CDN audio fetch plus WebAudio/canvas analysis. The SDK has no audio, canvas, arbitrary network or media-graph surface. Serein's native audio player owns this area. |
| `src/equicordplugins/betterBanReasons` | Blocked (native) | Needs integration with the native ban modal and a native ban action. The SDK exposes no ban action (only kick/prune moderation), and the plugin cannot patch or render the ban modal. A detached reason-text clipboard would not implement the workflow, so it is not presented as a port. |
| `src/equicordplugins/betterMutualServers` | No plugin needed | Native profiles already show loaded roles and member-since data. The SDK has no mutual-server row surface, and upstream additionally dispatches `GUILD_MEMBERS_REQUEST`; this port must not request missing members. |
| `src/equicordplugins/betterPlusReacts` | Implemented: `extensions/plugins/plus-reacts-tools` | Explicit composer tool over the bounded loaded timeline; one Apply-reviewed `SetReaction` proposal. See below. |
| `src/equicordplugins/bypassStatus` | Blocked (native) | Needs per-guild/channel/user notification allowlists that override DND and emit notification effects. No notification-effect host surface exists, and message events are limited to the selected channel. Changing global notification settings would not implement the per-source bypass. |
| `src/equicordplugins/channelTabs` | Blocked (native); deliberately not re-skinned | Needs a native browser-like tab strip, per-account tab state and view caching. A plugin-level bookmark list would duplicate the already packaged Command Palette pins (16 loaded-channel IDs with Apply navigation), so adding it would be directory-count inflation rather than a missing behavior. |
| `src/equicordplugins/clickableRoles` | Already packaged; audited, no edit | Covered by `extensions/plugins/loaded-role-members` (`loaded-role-members.serein-extension`), which lists loaded members for a loaded role, proposes profile opening, and copies role colors after Apply. It explicitly omits the guild-wide role-member endpoint and totals, matching the recorded limitation. |
| `src/equicordplugins/commandPalette` | Already packaged; audited, no edit | Covered by `extensions/plugins/command-palette` (`command-palette.serein-extension`), a bounded loaded-channel/native launcher with 16 pins and Apply navigation. It already omits global hotkeys, arbitrary commands, plugin management and send-DM commands as recorded. |

## Implemented subset: `plus-reacts-tools`

Upstream files read:

- `src/equicordplugins/betterPlusReacts/index.ts`
- `src/equicordplugins/betterPlusReacts/README.md`

Implemented behavior:

- Composer action `plus-react` parses a leading run of `+` characters followed
  by one emoji.
- `+` selects the newest loaded timeline row; each additional `+` walks one row
  back, capped at **50**.
- Accepts one Unicode emoji or a custom `name:id`, `<:name:id>` or
  `<a:name:id>` value, normalizing custom markup to `name:id` for the host.
- Returns exactly one `HostEffect::AppAction` containing
  `AppAction::SetReaction { add: true }`, plus a confirmation panel with the
  selected author, a sanitized text preview and the emoji.
- The host revalidates the selected conversation, message ID and emoji when the
  user chooses Apply.

Bounds:

- 50-message lookback, matching the host timeline item cap.
- 512-byte composer draft.
- 128-byte emoji, matching the host reaction validator.
- 160-byte sanitized preview.
- 64 panel elements maximum; this panel uses six.
- One host effect per response and the 256 KiB ABI response limit.
- No storage, no network, no filesystem, no raw store, no automatic action.

Omitted upstream behavior:

- Discord composer/autocomplete patches and `MessageStore` access.
- Shortcode resolution from `:emoji:` text.
- Toggling or removing reactions; this tool only proposes `add: true`.
- Rewriting or clearing the draft.
- Any target outside the loaded timeline.

## Tests

`src/lib.rs` contains focused inline tests for:

- counting back from the newest loaded message and emitting one reaction effect;
- custom emoji normalization (`name:id`, `<:name:id>`, `<a:name:id>`);
- rejecting shortcodes, invalid custom names, zero/non-numeric IDs and invalid
  extra colons;
- rejecting missing plus signs, short timelines, over-50 counts and unusable
  channel/message IDs.

Not run here (parent resource gate): no `cargo test`, `cargo check`, Clippy,
build, package, screenshot, measurement or full repository check. Only
`rustfmt` on the new source file is permitted.

## Parent integration steps

1. Add `plugins/plus-reacts-tools` to the `members` list in
   `extensions/Cargo.toml` (parent-owned central file; not edited here).
2. Build the release Wasm with the pinned toolchain for
   `wasm32-unknown-unknown`.
3. Package centrally into
   `extensions/plugins/packages/plus-reacts-tools.serein-extension` with the
   repository's GPL license section (see `extensions/plugins/README.md`), and
   bump versions on later changes.
4. Update the parent-owned `extensions/equicord-workflows.json` entry for
   `src/equicordplugins/betterPlusReacts` to `packaged` with package
   `plus-reacts-tools` and the limitation "composer tool over the loaded
   timeline only; no shortcode resolution or store access".
5. Add the plugin to the parent-owned `extensions/plugins/README.md` table.
6. Regenerate catalog pins after merge through the normal catalog workflow; do
   not hand-pin in this branch.
7. No native host change is required for this plugin. The blockers above would
   require separate native capabilities (notification policy, member-list
   decoration, media visualization, native ban support, tab strip) and are not
   authorized by this wave.

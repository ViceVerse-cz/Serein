# Command Palette (bounded launcher)

A safe, bounded Serein panel inspired by Equicord's Raycast-style
`CommandPalette`. It filters the channels Serein has **already loaded** and
offers a fixed set of native destinations and Apply-reviewed navigation
proposals. It is not a general command runner.

## Provenance

This plugin is a Serein port of Equicord's
[`src/equicordplugins/commandPalette`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/equicordplugins/commandPalette)
at commit `200dba124066146a8f043fde7fc8b05e3dc5e68e`, by Vendicated and
Equicord contributors. The upstream plugin is licensed
**GPL-3.0-or-later**. This port is distributed under the same license. See the
header in `src/lib.rs` and <https://www.gnu.org/licenses/gpl-3.0.html>.

The upstream README describes a global command palette for running actions
anywhere in Discord. This port deliberately removes the action-running surface
and keeps only a typed panel launcher over Serein's bounded app snapshot.

## Supported flow

The manifest declares eight `panel` actions and no activation, message, event or
tick handler. Every invocation sees only the capability-scoped snapshots the
host grants.

1. **`show` / `filter`** opens the palette. The panel reads:
   - `app_context` for the current account and selected conversation,
   - `channel_directory` for the currently loaded, viewable channels,
   - `guild_directory` for loaded server names used in labels,
   - `read_state` for the selected conversation's unread/mention summary,
   - `storage` for pinned channel IDs.

   Type a query and press **Apply filter** (`filter`). Matching is
   case-insensitive over the loaded channel name, channel ID and loaded server
   name. Results are pinned-first and capped at 32 rows. All filtering is local
   and uses only the current snapshot; it never fetches channels or searches the
   server.

2. Choose a loaded channel from the typed `Select` and press one of:
   - **Propose opening selected loaded channel** (`navigate`) — returns exactly
     one `navigation.navigate` proposal for a channel ID that is present in the
     current loaded directory.
   - **Pin selected loaded channel** (`pin-channel`) — saves its ID, up to 16.
   - **Unpin selected channel** (`unpin-channel`) — removes it from saved pins.

   Applying an opening proposal is a separate confirmation. Serein rechecks
   current access and navigates only after **Apply**.

3. Choose a fixed native destination and press **Propose selected native
   destination** (`go-destination`). The only destinations are Home and the
   native `AppView` values listed in `src/lib.rs` (Friends, Search, Pins,
   Members, Threads, Settings, Account, Profile settings, Appearance, Messaging
   permissions, Notifications, Activity, Keybinds, Storage, Updates, Extensions,
   Themes and Voice settings). No URL or arbitrary route is accepted.

4. Enter a search query and press **Propose conversation search** (`search`).
   The plugin returns one `navigation.search` proposal for the current
   conversation. The query is limited to 256 bytes and must not contain control
   characters.

5. Press **Propose opening your profile** (`open-profile`) to return one
   `navigation.open_profile` proposal for the current account from
   `app_context`. Arbitrary user IDs are not accepted.

Every action returns at most **one** `HostEffect` and only after a user action.
Nothing navigates, searches, pins or opens a profile in the background.

## Bounds and safety

- Query and search text are bounded to **256 bytes**, truncated only at a UTF-8
  character boundary (search is rejected rather than silently shortened).
- Every panel/action/value ID uses Serein's `valid_id` contract: lowercase ASCII
  letters, digits and hyphens, starting with a letter or digit, at most 64 bytes.
  A host-compatible test checks the manifest action IDs and the emitted panel.
- At most **32** loaded-channel candidates are rendered from a directory capped
  by the host at 100.
- Storage contains only `{"pins":["<channel id>", ...]}` with at most **16**
  unique, valid nonzero decimal channel IDs.
- The complete response is capped at **256 KiB**, panel elements at 64 and text
  elements at the host limit. Oversized output falls back to a short notice with
  effects cleared.
- Selecting a stale option after an account, conversation or directory switch is
  rejected unless its ID is still in the current loaded directory. Stored stale
  pin IDs are not offered and are dropped on the next successful pin/unpin
  update.
- Corrupt storage (invalid JSON, wrong shape, extra fields, invalid or duplicate
  IDs, more than 16 entries) is **preserved** on read. A later explicit pin
  action may replace it; merely opening the panel never writes.
- The panel copies no message bodies, credentials, tokens, hidden channels or
  background actions. It requests only `app_context`, `channel_directory`,
  `guild_directory`, `read_state`, `navigation` and `storage`.
- Pinned results are ordered first. Missing channels are never presented as
  absent from the server; they are only described as not present in the loaded
  snapshot.

## Deliberate omissions from upstream

Upstream includes a global hotkey recorder, custom URL/settings/message
commands, command aliases, persistent frecency, Discord action commands (mute,
deafen, status, theme), plugin management, updater/restart actions and arbitrary
settings routes. This port does **not** implement:

- a global hotkey or any background keyboard listener,
- custom commands, aliases, shell commands, `eval`/JavaScript or URL opening,
- plugin management, updater/restart calls, Discord command execution or
  arbitrary Discord message sending,
- arbitrary file access, generic URL fetching or network requests,
- timeline or member methods that could copy message bodies or hidden channel
  data.

`jump_to_message` is intentionally not offered: this plugin requests no
`timeline` or message-content group, so it has no validated message ID source.
Jumping is left to native navigation, which already rechecks access.

Only the existing typed app snapshot and the six granted capabilities are used.

## Files

- `Cargo.toml` — standalone crate using the local `extensions/sdk` path.
- `manifest.json` — capabilities and the eight panel actions.
- `src/lib.rs` — handler, bounds, storage parsing and focused tests.
- `README.md` — this document.

Run the focused tests with:

```sh
cargo test --manifest-path extensions/plugins/command-palette/Cargo.toml
```

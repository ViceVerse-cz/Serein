# Server Info Tools

A safe, read-only Serein panel over **already-loaded** server data. It never
requests guild members, roles, owner, boosts or vanity information, and it never
calls Discord directly.

This is an independent Rust implementation for the Serein extension sandbox. It
does not run Equicord JavaScript, patch Discord modules, read raw stores or use
Webpack.

## Upstream provenance

Ported from `Equicord/Equicord` commit
`200dba124066146a8f043fde7fc8b05e3dc5e68e`:

- [`src/plugins/serverInfo/index.tsx`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/plugins/serverInfo/index.tsx)
  — the guild context-menu entry point and the sorting setting.
- [`src/plugins/serverInfo/GuildInfoModal.tsx`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/plugins/serverInfo/GuildInfoModal.tsx)
  — the read-only server summary, relationship tabs and loaded-member tabs.

Upstream is **GPL-3.0-or-later**. This port is distributed under the same
license; see the header in `src/lib.rs`.

## Actions

| Action | Surface | Behavior |
| --- | --- | --- |
| `show` | panel | Renders the selected loaded server summary and a bounded server picker. Reads only granted snapshots. |
| `select-guild` | panel | Saves a server chosen from the current picker options, re-validates it against the current loaded directory, and renders its summary. |

Both actions are explicit user invocations. There is no automatic event,
message, activation or background handler, and no host effect is returned.

## Supported subset

- The server name and public snowflake ID from the loaded guild directory.
- Server creation date derived locally from the public snowflake timestamp.
- Loaded channel totals and a breakdown into text/announcement, voice/stage,
  category, forum/media, thread and other kinds. Counts are filtered to the
  selected guild and deduplicated by channel ID.
- Loaded account relationship counts (friends, incoming/outgoing requests,
  blocked, ignored) with an explicit warning that this is the account-wide
  loaded list, not a server-membership check.
- Partial-state labels when the host reports a truncated guild, channel or
  relationship directory.
- A strict saved server choice so a later invocation can reopen the same loaded
  server.

## Deliberate omissions

Upstream also shows or fetches:

- **Owner, joined-at, vanity URL, preferred locale, verification level, boosts**
  — these fields are not present in the Serein extension app snapshot. The panel
  says so rather than guessing.
- **Roles** — the current SDK exposes roles only in the selected-channel
  `member_details` snapshot, not as a guild-wide role catalog. No guild role
  list is fetched or invented.
- **Friends/blocked/ignored tabs filtered by guild membership** — checking
  membership would require a guild-wide member request. This port shows loaded
  account-wide counts with an explicit scope warning instead.
- **Mutual-member and mutual-guild tabs** — these require member and guild
  membership stores or fetches. They are not implemented.
- **Server icon and banner images** — the snapshot provides only opaque asset
  hashes. This port does not build or guess CDN URLs and does not open an image
  modal.
- **Guild context-menu integration** — the extension sandbox has no native
  guild-menu hook. The panel is opened through the extension's own entry point.
- **Sorting settings UI** — the port uses deterministic name/ID ordering instead
  of an upstream settings object; no storage beyond the selected server ID is
  written.

## Bounds and safety

- At most **20** server picker options, even when the host loaded 100 guilds.
- At most **8 KiB** serialized response, stricter than the 256 KiB ABI limit.
  An oversized response is replaced by a short notice with no effects.
- Channel counting is limited to the host's loaded channel directory cap
  (`MAX_APP_CHANNELS`, 100), deduplicated by channel ID, and labelled partial
  when the directory reports truncation.
- Relationship counting is limited to the host's loaded relationship cap
  (`MAX_RELATIONSHIPS`, 100) and labelled partial when appropriate.
- Display labels have control characters replaced and are truncated on UTF-8
  character boundaries.
- No member query, member search, profile request, role request, network
  request, file access, clipboard write or server action is performed.
- Capabilities requested are only `guild_directory`, `channel_directory`,
  `relationships` and `storage`.

## Storage and corrupt state

Storage contains only a strict object with one `guild_id` string. A malformed
value, unknown field, missing field, zero ID or non-decimal ID is corrupt.
Corrupt storage is preserved untouched while the panel explains the problem; it
is not silently repaired on `show`. A valid replacement is written only after
the user chooses an option from the current loaded picker.

## Tests

Inline tests cover:

- snowflake-to-UTC date derivation and invalid-ID rejection;
- filtering loaded channels by selected guild, excluding DMs and other guilds;
- bounded, unique, deterministic server options;
- corrupt/unknown selection never overwriting storage or emitting effects;
- response staying within the plugin budget.

The tests are inline Rust tests. See `WAVE.md` for what the parent should run
centrally; this source drop was not compiled under the task's resource gate.

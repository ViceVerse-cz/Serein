# Message Navigation Tools

A bounded Serein port of Equicord's `JumpTo` and `LastActive` navigation
actions. It proposes jumping to a message that Serein has already loaded, or to
the latest message named by loaded channel details. Every proposal requires the
host's explicit **Apply** action.

## Provenance

The reference is exactly
[`Equicord/Equicord@200dba124066146a8f043de7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e),
covering:

- `src/equicordplugins/jumpTo/index.tsx` (Samwich, thororen);
- `src/equicordplugins/lastActive/index.tsx` (Crxa).

Both upstream plugins are GPL-3.0-or-later. Original notices are retained in
`src/lib.rs`; the shared license text is
`extensions/licenses/Equicord-GPL-3.0.txt`. This is an independent Rust
implementation and does not run upstream JavaScript.

## Actions

All actions read only the bounded `app_context`, `timeline` (at most 50 rows)
and, for the latest message, `channel_details.last_message_id` snapshots the
host already provides. No action fetches history, queries search, opens a raw
store or calls a REST route.

| Action | Surface | Behavior |
| --- | --- | --- |
| `jump-latest` | Panel | Proposes the latest message ID from loaded channel details, falling back to the newest loaded row. |
| `jump-oldest-loaded` | Panel | Proposes the oldest message in Serein's bounded loaded window. |
| `jump-me-latest` | Panel | Proposes the newest loaded message authored by the current account. |
| `jump-author-latest` | Message | Proposes the selected author's newest loaded message in this conversation. |
| `jump-author-oldest` | Message | Proposes the selected author's earliest loaded message in this conversation. |

The response contains at most one `jump_to_message` host effect. Serein shows the
target and performs navigation only after Apply. The plugin never sends, edits,
reacts, marks read or changes a draft.

## Bounds and stale data

- At most **50** timeline rows are inspected (the host's existing cap, enforced
  again defensively).
- Every channel, user and message ID must be a canonical nonzero decimal
  snowflake of at most 20 digits. Leading zeroes, signs, whitespace and
  overlong values are rejected rather than guessed.
- The timeline channel must exactly match the current `app_context` channel.
- A message action's selected-message channel must also match that conversation.
- A mismatched or absent snapshot yields a short local explanation and **no
  effect**.
- Panel text is static, bounded copy; no message body is copied or retained.

## Deliberate omissions

- **No reused search transport.** Upstream `JumpTo` and `LastActive` issue REST
  `GET /channels/{id}/messages?limit=1` or guild/channel search requests with
  `author_id`, `sort_by` and `sort_order`. The extension API exposes only a
  foreground search proposal whose sort order is a user setting, not a
  plugin-controlled cursor. This port therefore does **not** claim
  first-message or guild-wide author search.
- **No first-message jump.** The host does not expose a channel's first message
  ID. `jump-oldest-loaded` is explicitly the oldest loaded row, not the channel
  creation message.
- **No exact author search outside the loaded window.** `LastActive`'s
  guild-wide history lookup is replaced by a loaded-window scan and labelled as
  such.
- **No context-menu registration.** The SDK does not let a package add native
  channel/user/message context-menu entries; these are explicit panel/message
  actions instead.
- **No DM-only shortcut.** Upstream's `UserMenuPatch` special-case for group DMs
  is not reproduced because the plugin does not install context menus.

## Integration

The directory is self-contained and does not edit shared files. The repository
integration owner must:

1. add `plugins/message-navigation-tools` to the `members` list in
   `extensions/Cargo.toml`;
2. build with `cargo xtask package` or the extensions packaging flow;
3. add the reviewed package to `extensions/catalog.json` through the normal
   `catalog.py`/`pack.py` release process;
4. add the package to the debug starter registry only if the shared starter set
   is intentionally expanded.

The manifest declares `api_version` 1 and five actions. No storage, network,
account, clipboard or background capability is requested.

## Tests

Inline tests cover loaded-window newest/oldest selection, author filtering,
channel-details preference for the latest message, canonical-ID fallback,
stale/mismatched timeline rejection, and the single-effect/no-replacement
contract.

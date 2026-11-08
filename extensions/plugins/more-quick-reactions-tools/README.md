# More Quick Reactions Tools

A narrow, local-only Serein adaptation of Equicord's **MoreQuickReactions** workflow.
Upstream source is pinned to
[`Equicord/Equicord@200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e):
[`src/plugins/moreQuickReactions/index.ts`](https://github.com/Equicord/Equicord/blob/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/plugins/moreQuickReactions/index.ts).
The upstream Vencord GPL notice is retained in `src/lib.rs`; this package is licensed
`GPL-3.0-or-later`.

## What changed from upstream

Upstream patches Discord's message hover bar and emoji picker. It makes the quick-reaction
row configurable (0–42 reactions, rows, columns, compact scale and scrolling) and reuses
Discord's favorite/frequent emoji list. Serein's Wasm sandbox cannot patch React, observe
pointer gestures, read Discord's emoji-frecency store or render hover-bar rows, and this
port does **not** simulate those surfaces.

Instead, the package exposes one explicit panel:

1. **Configure** a bounded list of up to 42 reactions.
2. **Choose** one of at most 20 messages from the host's already-loaded timeline for the
   active readable channel.
3. **Propose** exactly one `SetReaction { add: true }` for the chosen message.

The host shows its normal proposal dialog and only sends the reaction after **Apply**. No
reaction is added automatically, no message is fetched, and no hover bar is claimed.

## Actions

| Action | Surface | Behavior |
| --- | --- | --- |
| `open` | panel | Shows the current list, a bounded loaded-message selector and the configuration fields. |
| `save` | panel | Validates and stores the reaction list. Invalid input leaves the previous storage untouched. |
| `react` | panel | Revalidates the selected loaded message and emoji, then proposes one add-reaction action. |

## Emoji and message validation

- A plain unicode token must be nonempty, at most 32 UTF-8 bytes, contain at least one
  non-ASCII scalar, and contain no whitespace or control characters. This accepts emoji,
  flags, keycaps and ZWJ sequences but rejects plain ASCII words that the host's lenient
  emoji check would otherwise let through.
- A custom token must use the host's `id:name` form: a canonical nonzero 20-byte decimal
  snowflake, a colon, and a 2–32 byte ASCII alphanumeric/underscore name. Tokens above 53
  UTF-8 bytes are rejected.
- Entries are split on ASCII/Unicode whitespace or commas, deduplicated in order, and
  capped at 42. The complete configuration line is capped at 1,024 UTF-8 bytes.
- A stored list is revalidated on every invocation. Malformed, unknown-field, oversized or
  out-of-range storage is reported and preserved; `save` never overwrites it.
- A message target must be a canonical nonzero snowflake and must still match one of the
  bounded labels rebuilt from the current `app.timeline`. Stale selections produce no
  proposal.
- Only the active timeline channel is used. The plugin never chooses another channel, reads
  another conversation, fetches history or accesses attachments, profiles or credentials.

## Bounds and safety

| Item | Bound |
| --- | --- |
| Reactions | 42 entries |
| Unicode emoji token | 32 UTF-8 bytes |
| Custom emoji token | 53 UTF-8 bytes |
| Configuration line | 1,024 UTF-8 bytes |
| Stored settings | 16 KiB |
| Loaded-message choices | 20 |
| Message label / Select option | 128 UTF-8 bytes |
| Host effects per response | 1 |
| Network, filesystem, credentials | none |

The panel contains at most nine top-level elements and the full response is checked against
the SDK's 256 KiB ABI limit. The plugin requests only `timeline`, `reactions_control` and
`storage`; it does not request message-send, message-manage, content, attachment, profile or
navigation access.

## Omissions

The upstream hover-bar/picker patches, row and column layout, compact mode, wheel scrolling,
favorite-versus-frequent emoji source, settings migration and Discord popover CSS are not
ported. Emoji frecency is not exposed to sandboxed plugins, so the list is explicit rather
than automatic. There is no emoji picker integration and no burst/normal reaction choice.

## Tests

Focused tests in `src/lib.rs` cover:

- emoji token acceptance/rejection, including custom `id:name` bounds, ASCII words,
  whitespace/control bytes and oversized tokens;
- deduplication, the 42-entry ceiling and refusal to write an invalid configuration;
- preservation of malformed or out-of-range stored settings;
- one-effect reaction proposals only for messages that are still in the supplied timeline,
  with no proposal for stale, missing, cross-context or invalid targets;
- panel element, Select option and label bounds plus button IDs matching the manifest's
  panel actions.

These are handler-level tests. They are not Wasm sandbox, host Apply, live Discord or UI
rendering evidence.

## Provenance

The upstream Vencord/Equicord notice and GPL-3.0-or-later terms are retained in `src/lib.rs`.
The shared license text is `extensions/licenses/Equicord-GPL-3.0.txt`. This port is an
independent Rust implementation; it does not reproduce upstream monkey patches or run
upstream JavaScript.

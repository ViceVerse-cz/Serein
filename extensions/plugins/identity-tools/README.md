# Identity Tools

A narrow, local-only Serein port of Equicord's **CopyUserURLs** and **CopyUserMention**
workflows. Upstream source is pinned to Equicord/Equicord commit
[`200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e):
[`src/plugins/copyUserURLs/index.tsx`](https://github.com/Equicord/Equicord/blob/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/plugins/copyUserURLs/index.tsx)
and
[`src/equicordplugins/copyUserMention/index.tsx`](https://github.com/Equicord/Equicord/blob/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/equicordplugins/copyUserMention/index.tsx).
The upstream Vencord GPL notices are retained in `src/lib.rs`; this port is licensed
`GPL-3.0-or-later`.

## What changed from upstream

Upstream adds both items to Discord's **user** context menu and reads the full user object.
This port deliberately narrows the surface to a **selected message** and reads only its
public `selected_message_ref` snapshot:

- **Copy author profile URL** proposes exactly `https://discord.com/users/<author_id>`.
  Upstream wraps the URL in angle brackets to suppress an embed; this port copies the
  canonical bare HTTPS URL instead.
- **Copy author mention** proposes exactly `<@author_id>`.

Only the selected message's author ID is used. Message text, other message IDs, channel or
server IDs, and nearby user data are ignored. The upstream user-card/context-menu scope,
avatar, profile lookup, and any broader user selection are intentionally absent.

## Behavior and safety

- Both actions require a foreground message action. The host supplies the selected-message
  snapshot and the port asks only for `selected_message` (the grant that carries both the
  message text and `selected_message_ref`) and `clipboard_write`.
- The author ID must be a canonical, nonzero decimal `u64` of at most 20 bytes, without
  leading zeros. An absent reference, empty ID, zero, non-ASCII digits, overflow, or
  non-canonical spelling produces no clipboard proposal.
- The port returns at most one `copy_text` effect. It never reads the clipboard, fetches a
  profile, makes a network request, sends a message, uses storage, or copies arbitrary
  selected text. The URL is credential-free and contains only the author ID.
- Nothing is copied before the host's **Apply** confirmation, and the host rechecks context
  and permission on Apply. The panel asks the user to apply; it does not claim that a copy
  has happened or promise that a stale proposal will succeed.

## Outputs

| Action ID | Reviewed text |
| --- | --- |
| `copy-user-url` | `https://discord.com/users/42` |
| `copy-user-mention` | `<@42>` |

The handler emits the corresponding effect:

```json
{"effects":[{"type":"copy_text","text":"<@42>"}]}
```

Unit tests in `src/lib.rs` cover exact URL/mention wire output, typed dispatch, omission of
selected text and unrelated IDs, canonical ID boundaries, and the no-proposal path for
absent or invalid IDs. These are handler-level tests, not Wasm sandbox, host Apply, or live
clipboard evidence.

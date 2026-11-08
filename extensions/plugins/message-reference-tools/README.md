# Message Reference Tools

A safe, Wasm-only Serein port of three Equicord/Vencord message workflows:

- **FindReply** — jump to a loaded original or loaded reply.
- **GoogleThat** — propose a reviewed Google search for the selected message text.
- **Husk** — propose adding the upstream Husk custom-emoji reaction.

The plugin reads only the selected message text, the selected message IDs and the
bounded timeline/message-details rows the Serein host has already loaded. It does
not fetch history, scan beyond the host's loaded window, send messages, toggle
reactions, open URLs without Apply confirmation, or access the network, files,
attachments or credentials.

## Upstream source

Adapted from the Equicord upstream at commit
[`200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e):

- `src/equicordplugins/findReply/index.tsx`
- `src/equicordplugins/findReply/ReplyNavigator.tsx`
- `src/equicordplugins/googleThat/index.tsx`
- `src/equicordplugins/husk/index.tsx`

Upstream is GPL-3.0-or-later. The upstream copyright headers are preserved in
`src/lib.rs`, and this package is distributed under the same license.

## Actions

| Action | Surface | Behavior |
| --- | --- | --- |
| `find-reply` | message | Proposes one `jump_to_message` to the nearest loaded original in the reply chain, or the earliest loaded direct reply. |
| `search` | message | Builds one fixed-host HTTPS Google search URL and proposes one reviewed `open_external_link`. |
| `husk` | message | Proposes exactly one `set_reaction` with `add: true` for the selected loaded message. |

Every successful action emits at most **one** host effect, and that effect is
applied only after the user presses **Apply** in Serein's reviewed proposal panel.
Panel-only failures emit no effect.

## Semantic adaptations

### FindReply

Upstream reads the whole channel message store, optionally includes messages
that ping the author or reply to the author, jumps to the earliest reply and
offers a paginator over all replies.

This port intentionally cannot do that:

- It inspects only `app.timeline` and `app.message_details`, which Serein bounds
  (currently at most 50 timeline rows, 20 message-detail rows, and 12 paired
  rows when both groups are requested).
- It never requests or claims complete channel history; truncation flags are
  reported as a partial loaded window.
- It does not implement the optional ping/author reply expansion.
- It emits one navigation effect, not a paginator or a sequence of jumps.
- If the selected message is a loaded reply, the nearest loaded original wins.
  Otherwise the earliest loaded direct reply wins. If neither is loaded, the
  panel says so and proposes nothing.

### GoogleThat

Upstream is a slash command that sends a message containing a search link. It
supports ten engines, a custom engine URL, and hyperlink/embed formatting
settings.

This port keeps only the safe destination construction:

- It uses the upstream default engine, Google, at the fixed HTTPS host
  `www.google.com/search`.
- It builds the URL from the selected message text and proposes one
  `AppAction::OpenExternalLink`; the user must confirm **Apply** before Serein
  opens the public page in the browser.
- It does not send or draft a message, replace message content, use a custom
  engine URL, persist settings, or make any network request.
- Empty or control-only text produces a bounded panel and no effect.
- The query is capped at 1024 UTF-8 bytes and the complete URL is capped at
  2048 bytes on character boundaries.

### Husk

Upstream sends an idempotent `PUT .../reactions/{emoji}/@me` request and prefers
a same-named emoji in the current server when available.

This port keeps the default upstream emoji and desired-set semantics:

- The emoji is the fixed custom emoji `husk:1026532993923293184`.
- The effect is always exactly one `AppAction::SetReaction { add: true }`.
- It never constructs `add: false`, so it cannot remove or toggle an existing
  reaction. Adding an already-present reaction is a server-side no-op.
- Server same-name emoji lookup is omitted because the current typed API exposes
  no emoji store to extensions. The panel states this explicitly.

## Capabilities and grants

The manifest requests only:

| Capability | Why |
| --- | --- |
| `selected_message` | Required for message actions; supplies the selected text and IDs. |
| `timeline` | Bounded loaded messages used to confirm order and loaded presence. |
| `message_details` | Bounded `reply_to` and loaded presence metadata. |
| `navigation` | Allows the reviewed `jump_to_message` proposal. |
| `external_navigation` | Allows the reviewed browser-search proposal. |
| `reactions_control` | Allows the reviewed desired-set reaction proposal. |

No storage, message-send, message-manage, account, media, member, guild,
credential or network capability is requested.

## Bounds and failure behavior

- Invalid, missing, mismatched or stale selection data yields a bounded panel
  and **no effect**. This includes:
  - absent `selected_message_ref`;
  - non-snowflake IDs;
  - a loaded timeline/details channel that does not match the selected channel;
  - a selected message no longer present in the loaded window;
  - missing reply metadata for `find-reply`.
- The plugin iterates only the vectors supplied by the host; it never requests
  more pages or starts a scan of its own.
- The search URL is revalidated as HTTPS with the exact host `www.google.com`,
  no credentials, no fragment tricks and no explicit port.
- Panels use Serein's normal element limits. Outputs never include storage,
  replacement text, attachment data, tokens or credentials.
- A serialized response over the 256 KiB ABI limit becomes a bounded failure
  panel with no effect.

## Tests

`src/lib.rs` contains focused typed tests that exercise the same JSON dispatch
path as the Wasm exports:

- all three workflows produce their intended single effect;
- `find-reply` prefers a loaded parent and falls back to the earliest loaded
  reply;
- stale, unloaded and channel-mismatched selections are rejected without an
  effect;
- Google URLs stay HTTPS, use the allowlisted host, remain within 2048 bytes for
  long ASCII and multi-byte queries, and contain no storage or replacement;
- Husk always proposes one `add: true` reaction, never a toggle;
- panels stay bounded and do not expose tokens or raw selected text outside the
  reviewed search URL.

Run them from the extensions workspace after adding this package as a member:

```sh
cargo test -p message-reference-tools --locked
```

## Integration notes

- The package is not added to `extensions/Cargo.toml` by this port; the
  integrator must add `plugins/message-reference-tools` to the workspace members
  and build/package it through the usual extension pipeline.
- It uses the implemented Preview contracts `selected_message_ref`, `timeline`,
  `message_details`, `navigation`, `external_navigation` and
  `reactions_control`.
- All validation here is synthetic/offline. Passing tests do not prove live
  Discord behavior or server-side reaction outcomes.

## License

GPL-3.0-or-later. See the repository license, third-party notices and the
upstream commit linked above.

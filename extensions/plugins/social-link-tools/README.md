# Social Link Tools

A sandboxed Rust adaptation of three Equicord workflows for Serein's extension
API, pinned to
[`Equicord/Equicord@200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e):

- `src/equicordplugins/pendingFriendRequest` — cancel an outgoing friend request.
- `src/equicordplugins/normalizeMessageLinks` — strip `canary.`/`ptb.` from message links.
- `src/equicordplugins/repeatMessages` — repeat a message's content.

Upstream is GPL-3.0-or-later. Its notices are retained in `src/lib.rs`; this
package and is distributed under the same license. The port is an independent
Rust implementation and does not run JavaScript, monkey-patch Discord modules,
send messages automatically, or make network requests.

## Actions

| Action | Surface | Behavior |
| --- | --- | --- |
| `open` | panel | Lists only currently loaded **outgoing** friend requests from the granted `relationships` snapshot and selects one by user ID. |
| `cancel` | panel | Proposes exactly one `ResolveFriendRequest { user_id, accept: false }` for the selected user, but only after rechecking that the same loaded relationship is still `outgoing_request`. |
| `normalize-links` | message | Rewrites only the authority of exact `https://canary.discord.com` and `https://ptb.discord.com` links to `https://discord.com` and proposes one `CopyText` with the exact normalized message text. |
| `repeat` | message | Proposes one `SendMessage` with the sanitized text for explicit **Apply**. Mentions are neutralized before sending; no attachments, stickers, reply target or alternate channel are added. |
| `repeat-reply` | message | Proposes one `SendReply` to the selected message with the same sanitization and `mention: false`, so the reply cannot notify the original author merely because it is a reply. |

Every action runs only when the user invokes it. Cancel, link copy and repeat
all use the host's Apply confirmation. No invocation fetches, opens a link,
sends a message without Apply, creates an attachment or sticker, or calls
Discord directly beyond the one reviewed send proposal.

## Semantic narrowing

### Outgoing friend request cancellation

Upstream adds context-menu and button patches that call
`RelationshipActions.cancelFriendRequest(userId)` after checking an outgoing
relationship. This port cannot patch React or call Discord APIs.

- The panel is built only from `app.relationships`, only when
  `requests_known` is true. Missing data, an ungranted group, or an unloaded
  request list produces a panel and no effect.
- Incoming requests, friends, blocked, ignored and unrelated users are never
  listed or targeted. The selected target must be a canonical nonzero
  snowflake and must appear exactly once in the snapshot.
- Cancellation is the actual typed host operation
  `AppAction::ResolveFriendRequest { accept: false }`. In Serein, `accept=false`
  is the DELETE path that removes/cancels a pending relationship; the host's
  native Apply path calls `State::resolve_friend_request`, which only accepts a
  request that is still in the pending set. It therefore revalidates the
  current kind and ID at Apply and fails a stale or changed relationship.
- There is no retry, no automatic cancellation, no generic relationship write,
  and at most one effect per response.

### Message link normalization

Upstream rewrites `location.host` for newly copied message links. This port
operates only on the selected message text.

- It scans for the exact ASCII scheme `https://`, then accepts only an authority
  equal (case-insensitively) to `canary.discord.com` or `ptb.discord.com`.
  Userinfo, ports, lookalike domains, `http://`, and embedded scheme-like text
  are ignored.
- Only the authority bytes are replaced with `discord.com`. Path, query,
  fragment, percent escapes, Unicode, whitespace and every other byte are
  copied unchanged; nothing is decoded or re-encoded.
- At most 20 eligible links are accepted. More than 20, zero eligible links, or
  output above 2,048 UTF-8 bytes produces a panel and no effect.
- The single effect is `CopyText` with the exact normalized text. Nothing is
  fetched, opened, previewed or sent; the host must still Apply.

### Repeat as an Apply-reviewed send

Upstream `repeatMessages` calls `sendMessage` immediately, optionally with
attachments and stickers, and can auto-select a reply target. This port keeps
the explicit send but removes the automatic path and the extra payloads.

- The action is a message action and requires a valid `selected_message_ref`
  plus nonblank selected text. Missing, invalid or zero IDs are inactive.
- `repeat` returns exactly one `AppAction::SendMessage` using
  `selected_message_ref.channel_id` and the reviewed sanitized text;
  `repeat-reply` returns exactly one `AppAction::SendReply` to
  `selected_message_ref.message_id` with `mention: false`. The ordinary
  **Apply** confirmation must be pressed before Serein sends anything.
- Before Apply, user/role mentions `<@...`, `<@!...>` and `<@&...>` plus literal
  `@everyone`/`@here` are neutralized by inserting U+200B ZERO WIDTH SPACE after
  the `@`. This cannot notify those targets merely because the old message did.
  Fenced code, inline code, raw HTTP(S) URLs and non-mention text are copied
  unchanged. The panel explicitly says the payload is sanitized, not
  byte-identical, and displays the exact payload.
- No attachment, sticker or alternate channel is constructed. The optional
  reply target is only the exact selected message, never auto-selected from
  surrounding messages, and its reply mention is disabled. There is no retry,
  timer, background behavior or plugin network call.
- Empty or whitespace-only text, unsupported control characters, or text above
  2,000 Unicode scalar values / 8,000 UTF-8 bytes **after neutralization**
  produces a bounded panel and no effect.

## Capabilities

The manifest requests only:

| Capability | Why |
| --- | --- |
| `relationships` | Read the loaded friend-request snapshot. |
| `relationship_control` | Propose the typed outgoing-request cancellation. |
| `selected_message` | Read selected message text and IDs for link normalization and repeat. |
| `clipboard_write` | Propose copying the exact normalized message text. |
| `message_send` | Propose the one Apply-reviewed repeat send. |

No storage, message-manage, navigation, external-navigation, media, member,
guild, account, credential or network capability is requested.

## Bounds

| Item | Bound |
| --- | --- |
| Listed outgoing requests | 20 |
| Relationship rows inspected | Host snapshot (at most 100) |
| Normalized links | 20 |
| Normalized output | 2,048 UTF-8 bytes |
| Repeat text (after neutralization) | 2,000 Unicode scalar values / 8,000 UTF-8 bytes |
| Effects per response | 1 |
| Panel elements | SDK limit (64) |

## Files

- `Cargo.toml`
- `manifest.json`
- `src/lib.rs` — handler, bounds and focused unit tests
- `README.md`

## Tests

`src/lib.rs` unit tests cover:

- only outgoing requests listed; incoming/friend rows excluded;
- wrong-kind, stale, unknown and unloaded cancellation scopes producing no effect;
- exactly one typed `resolve_friend_request` with `accept=false`;
- Unicode-safe link normalization preserving path/query/fragment and all other bytes;
- lookalike, non-HTTPS, userinfo and port links rejected;
- 20-link acceptance, 21-link and 2,048-byte output limits;
- mention neutralization of user/role/`@everyone`/`@here` tokens while leaving
  URLs, code and non-mention text unchanged;
- exactly one `send_message` effect with the selected channel and exact sanitized
  payload, no replacement and no reply/sticker/reference output;
- exactly one `send_reply` effect to the selected message with `mention: false`
  and no `send_message` output;
- repeat missing, empty, control-only, oversized or mention-expanded text
  remaining inactive.

These are handler-level tests, not live Discord, Wasm-sandbox or host-Apply
evidence. The only send is the explicit Apply-reviewed `SendMessage` proposal;
there is no automatic resend, retry, timer or plugin network request.

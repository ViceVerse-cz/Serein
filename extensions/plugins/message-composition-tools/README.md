# Message Composition Tools

A sandboxed Rust adaptation of three Equicord message-composition workflows for
Serein's extension API. The upstream reference is exactly
[`Equicord/Equicord@200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e),
covering `splitLargeMessages`, `blockKeywords` and `messageBurst`.

This is one local plugin with only the `composer`, `storage`, `app_context`,
`timeline`, `channel_details`, `message_details` and `message_manage` capabilities.
It has no automatic send hook, network access, embed mutation or raw host IDs.
Composer results are proposed drafts or a single reviewed message-edit proposal
and require the host's explicit **Apply** action.

## Actions

### Configure (`open`, `save`, panel)

Configures:

- Split boundary: newlines, spaces or strict characters.
- Split limit: 100–2,000 Unicode scalar values per segment.
- Up to 32 literal keywords, each at most 64 UTF-8 bytes and 2,048 bytes in total.
- Optional case-sensitive keyword matching.
- Burst style: lines, paragraphs or sentences.
- Maximum burst segments: 2–16.
- Whether a merge may target a message with attachments.
- Whether a merge joins text with a space instead of a new line.

Settings are stored as one bounded 16 KiB JSON value. Missing settings use defaults.
Malformed, oversized or out-of-range storage is reported and left untouched; saving
refuses to overwrite it. The panel never resets corrupt storage implicitly.

### Split draft (`split`, composer)

The upstream plugin intercepts a send, checks slowmode, splits at Discord's limit,
and automatically sends the chunks with a delay. That behavior is not available to
this sandboxed plugin and is **not simulated**. Instead, `split` returns a native
review panel containing up to 12 numbered segments. It returns no draft replacement,
sends nothing and schedules nothing.

Serein panel buttons cannot write back into the composer from a panel-surface action,
and replacing the draft with only one segment would discard the others. Segment
selection is therefore review/copy only; there is no Apply-per-segment control.

Splitting is Unicode-scalar-safe and prefers, in order:

1. the selected newline or space boundary within the limit;
2. a space fallback for newline mode;
3. a strict scalar boundary.

Each segment is at most 2,000 Unicode scalar values. The input and every segment are
also bounded by 8,000 UTF-8 bytes, matching the existing composer-tool ceiling and
keeping the proposed 2,000-character message limit. Drafts above 8,000 bytes are
left unchanged. Splitting does not attempt grapheme-cluster segmentation.

### Filter keyword lines (`filter-keywords`, composer)

`blockKeywords` hides incoming messages whose content or embeds contain configured
keywords. This plugin cannot observe incoming messages or embeds, so the local
composer adaptation removes whole lines that contain a configured keyword. It uses
literal substring matching (optionally case-insensitive), not regular expressions,
and does not claim to block messages in the timeline.

- Matching lines and their line endings are removed; nonmatching CRLF/LF content is
  preserved exactly.
- No keywords, no matches, an all-filtered draft, or a result above 2,000 characters
  leaves the draft unchanged instead of clearing or truncating it.
- A successful filter returns a proposed replacement for explicit **Apply**. Nothing
  is sent automatically.

### Burst review (`burst-disperse`, composer)

`messageBurst` edits the user's previous message when another message is sent within
a time window. This plugin has no automatic send hook or clock, so the upstream
automatic merge, timing window and reply-bar check are not faked.

Instead, `burst-disperse` is an explicit review action that disperses the current
draft into candidate messages using the saved line, paragraph or sentence style. It
shows at most the configured number of segments in the bounded panel, further splits
an over-limit segment at natural boundaries, and returns no replacement. Nothing is
merged, edited, repeated, timed or sent. The review dialog is transient host UI; it
does not create, alter or disappear any Discord message.

### Burst merge (`burst-merge`, composer)

`burst-merge` is the explicit, foreground subset of upstream `messageBurst`. It
reads the current account, conversation, bounded timeline and matching
`message_details` snapshot, then proposes exactly one `edit_message` action for
the latest loaded message when all of these hold:

- the loaded channel's latest-message ID matches the target, so no newer
  message exists in the conversation;
- the latest loaded message was authored by the current account;
- its metadata marks it as an ordinary message (kind 0 or 19);
- it has no attachments, unless the saved attachment-merge option is enabled;
- the previous text plus the draft stays within 2,000 characters and 8,000
  UTF-8 bytes.

The saved separator is a newline by default or one space when enabled. An
attachment-only previous message becomes just the draft rather than starting
with a separator. The panel shows the exact merged text and the target ID.
Serein performs the edit only after **Apply**; the draft is not cleared, sent or
scheduled automatically.

Deliberate omissions from upstream:

- no `onBeforeMessageSend` hook, so the merge is never automatic;
- no time-window enforcement, because the sandbox has no clock and the timeline
  snapshot has no timestamps;
- no reply-bar detection, because pending-reply state is not exposed to the
  plugin;
- no group-DM name check, because the plugin does not receive the group name;
- no attachment upload or re-send, only an edit of existing text.

## Bounds and safety

| Item | Bound |
| --- | --- |
| Draft | 8,000 UTF-8 bytes |
| Message segment | 2,000 Unicode scalar values |
| Settings storage | 16 KiB |
| Keywords | 32 × 64 bytes, 2,048 bytes total |
| Split preview | 12 segments per panel |
| Burst segments | 2–16 configured |
| Merged message | 2,000 Unicode scalar values, 8,000 UTF-8 bytes |
| Merge target | latest loaded own ordinary message with matching metadata |

Output is additionally checked against the SDK's 256 KiB ABI limit before storage is
proposed. All processing is local and linear in the bounded draft. Split, filter and
disperse return no `effects`, no message-send proposal, no selected-message data and
no host IDs. `burst-merge` returns at most one `edit_message` effect for the current
conversation's latest loaded own message; the host requires explicit Apply.

## Tests

Focused tests in `src/lib.rs` cover:

- Unicode-safe strict splitting across multibyte text and emoji, plus newline/space
  boundary preference;
- malformed JSON and semantically invalid stored settings being preserved rather than
  reset or overwritten;
- split and burst review actions never returning a replacement, storage write or send/effect
  field;
- burst merge proposing exactly one edit for the latest loaded own ordinary message,
  using the saved separator and preserving the draft;
- burst merge rejecting foreign, attachment-bearing (unless opted in), nonordinary,
  stale and over-limit targets;
- whole-line keyword filtering preserving CRLF and returning no storage write;
- filtering never proposing an over-limit or fully cleared draft;
- bounded configuration save round-trips and no-op filter paths.

## Provenance

Upstream Vencord/Equicord notices and GPL-3.0-or-later terms are retained in
`src/lib.rs`. The shared license text is `extensions/licenses/Equicord-GPL-3.0.txt`.
This port is an independent Rust implementation; it does not reproduce upstream
monkey patches or run upstream JavaScript.

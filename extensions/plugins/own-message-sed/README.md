# Own message sed

A bounded Serein port of Equicord's `sedEnhanced` plugin. It applies a literal
find/replace to one of your own loaded messages and proposes the result for
explicit Apply confirmation.

## What it does

- Reads the current account and selected conversation from `app_context`.
- Selects only an ordinary loaded message (default or reply) authored by the
  current account from the bounded `timeline`, with matching `message_details`
  metadata in the same conversation.
- Accepts a literal match of 1–256 UTF-8 bytes and a replacement of at most
  2,000 UTF-8 bytes.
- Supports first-only or global replacement. The result must stay within 2,000
  characters and 8,000 UTF-8 bytes.
- Handles upstream-style literal escapes: `\/`, `\*`, `\+` and `\?` become
  `/`, `*`, `+` and `?`. Other backslashes remain literal.
- Proposes exactly one `edit_message` action when the text changes.
- Proposes one `delete_message` action only when the trimmed result is empty and
  both the bounded timeline count and `message_details` confirm that the target
  has no attachments. Attachment-bearing or unverified targets are never
  deleted.

The panel exposes `show` and `apply` actions. Both are foreground proposals:
Serein displays the target and exact result, and nothing happens until the user
presses Apply. The plugin does not send messages, schedule work, or run in the
background.

## Deliberate omissions

- No arbitrary JavaScript or Rust regular expressions, regex flags or modes.
  This removes the ReDoS and pattern-evaluation surface.
- No `r`, `g`, `m`, `i`, `s`, `u`, `d`, `y` or `v` mode parsing. Only the
  explicit global checkbox is supported.
- No pending-reply interception or composer hook. The upstream plugin edited the
  pending reply or the last editable message from the composer; this port only
  edits an explicitly reviewed loaded message.
- No edits to other authors, no cross-channel targets, no history fetching and
  no message lookup beyond the bounded snapshots.
- No automatic deletion of attachment-bearing messages.

Stale, missing, foreign, non-ordinary or tombstoned targets produce no effect.
Canonical nonzero decimal IDs and a current-channel match are required.

## Provenance

Adapted from Equicord's `src/equicordplugins/sedEnhanced` at commit
`200dba124066146a8f043de7fc8b05e3dc5e68e`, authored by Vendicated and
contributors (dawn, Willow and kat in the upstream metadata). The upstream
plugin is licensed GPL-3.0-or-later; this adaptation is distributed under the
same license. See <https://www.gnu.org/licenses/gpl-3.0.html>.

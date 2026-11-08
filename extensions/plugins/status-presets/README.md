# Status presets

A bounded, user-invoked Serein port of Equicord's **StatusPresets** and
**CustomStatusTimeouts**, with the status half of **StatusWhileActive** left out on purpose.
Save small local presets, review them, and propose one preset at a time through the host's
explicit **Apply** step. No status is changed automatically.

## What the panel does

- **Save** stores a named preset containing an optional status (`online`, `idle`, `dnd`,
  `invisible`), optional custom-status text, and an optional expiry. Blank fields are omitted,
  not guessed. Check **Clear custom status text** to save an explicit empty text.
- **List** shows every saved preset with its status, a short text preview and its expiry choice.
- **Delete** removes only the selected preset. Deleting the last preset leaves a valid empty
  list; it does not clear unknown or damaged storage.
- **Apply** returns exactly one `set_own_presence` AppAction proposal. The host revalidates it
  and shows its ordinary Apply review. The plugin has no timer, retry loop, automatic write,
  network request, presence scraping or background task.

## Presence semantics

Only fields that exist in the SDK's `OwnPresencePatch` are used:

| Field | Rule |
| --- | --- |
| `status` | Omitted (unchanged), `online`, `idle`, `dnd` or `invisible`. |
| `custom_status` | Omitted (unchanged), an explicit empty string (clears text and its expiry), or nonblank text of at most 128 characters / 512 UTF-8 bytes with no control characters or surrounding whitespace. |
| `clear_after_seconds` | Omitted (keeps existing expiry), `0` (removes expiry), or `1..=86400`. Positive values start at Apply time. |

Applying a preset that omits a field leaves that field out of the proposal, so the host keeps
the current value. Clearing expiry is only the `clear_after_seconds: 0` field in the proposal;
the plugin does not schedule or perform a later write.

The SDK exposes no custom-status emoji field on `OwnPresencePatch`. Upstream StatusPresets stores
`emojiInfo`; this port deliberately omits that field rather than inventing an unsupported wire
shape or silently discarding an emoji. Unicode emoji are still allowed as ordinary characters in
the status text within the same 128-character / 512-byte bound.

## Bounds and storage safety

- At most **32 presets** per account. A 33rd save is rejected with a visible message.
- Encoded storage is capped at **48 KiB**; the panel stays within the host's 64-element limit.
- Preset names are at most 32 characters / 128 bytes, unique, nonempty, and free of controls.
- Storage is one versioned, `deny_unknown_fields` JSON value. Invalid, oversized, stale-version,
  unknown-field or otherwise unreadable storage is treated as **preserved, corrupt state**:
  list/apply/delete never overwrite it, and an invalid save does not replace it.
- Only a successful, fully validated **Save** replaces damaged storage, which makes repair
  explicit. Deleting from a valid list writes the remaining list, including a valid empty list.
- No network, credentials, tokens, other users, presence reads, directories or files are used.
  The only capability-backed data is the plugin's own storage and the host-reviewed
  `account_control` presence proposal.

## Upstream differences

Upstream commit: `200dba124066146a8f043fde7fc8b05e3dc5e68e`.

- **StatusPresets** used a patched status menu keyed by status text, with custom emoji and
  `clearAfter` values. This port uses an explicit panel with named presets and only the presence
  fields Serein actually exposes. No Discord menu patching or webpack access is involved.
- **CustomStatusTimeouts** offers seconds, minutes, hours, days, weeks and months. Serein's host
  rejects `clear_after_seconds` above 86,400 (one day), so the upstream week presets
  (604,800 s and above) and month presets (5,184,000 s and above) are intentionally omitted.
  The upstream default two-day entry (172,800 s) is also outside the host bound. Seconds,
  minutes and hours up to one day remain representable. This plugin is a preset editor rather
  than a replacement for Discord's duration menu.
- **StatusWhileActive** automatically changed status on voice events. Serein extensions have no
  automatic presence write or background timer surface in this build, so it is not ported. Any
  future version would still need a foreground, host-reviewed proposal rather than an automatic
  write.
- **GPL provenance:** the upstream plugins are GPL-3.0-or-later. This Rust adaptation keeps the
  GPLv3-or-later notice in `src/lib.rs`; no JavaScript is copied. Serein adaptation copyright
  2026 Serein contributors. Changed November 8, 2026 as a bounded, user-invoked port.

## Files

- `Cargo.toml` — SDK-only plugin crate.
- `manifest.json` — panel actions and `account_control` + `storage` capabilities.
- `src/lib.rs` — bounded panel, validation, storage preservation and tests.
- `README.md` — this document.

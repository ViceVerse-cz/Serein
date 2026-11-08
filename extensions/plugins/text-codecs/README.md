# Text Codecs

Two deterministic, offline composer actions and one message action ported from
Equicord to Serein's sandboxed Rust extension API:

- **Decode Base64 text** (`decode`, composer)
- **Capitalize sentence starts** (`uppercase`, composer)
- **Decode Base64 in message and copy** (`decode-message`, message menu)

The composer actions run only when the user invokes them and return a
replacement proposal that requires **Apply**; they never edit or send a message
automatically. The message action reads only the selected message text and
proposes one `copy_text` effect that also requires **Apply**. Nothing is copied
before Apply, and the panel states exactly what the copy contains.

The package requests `composer`, `selected_message` and `clipboard_write`. It
has no network, account, event, history-fetch, storage, appearance, filesystem
or secret access, and no clipboard read.

## Upstream provenance

The reference is exactly
[`Equicord/Equicord@200dba124066146a8f043de7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043de7fc8b05e3dc5e68e),
specifically:

- `src/equicordplugins/baseDecoder/index.tsx` (DecodeBase64), copyright
  (c) 2022 Vendicated and contributors;
- `src/equicordplugins/writeUpperCase/index.ts` (WriteUpperCase), copyright
  (c) 2023 Vendicated and contributors.

Both upstream files are GPL-3.0-or-later. This port keeps their notices in
`src/lib.rs` and is licensed GPL-3.0-or-later. It does not copy upstream host
objects, Discord internals, React UI or message-popover APIs.

## Decode semantics

The decoder follows the useful offline behavior of upstream DecodeBase64, which
supports **standard-alphabet Base64 only** (`A-Z`, `a-z`, `0-9`, `+`, `/`, with
optional trailing `=` padding). It does not claim support for hex, percent
encoding, URL/percent decoding, ROT13, gzip, Base64URL (`-`/`_`), or any other
codec that upstream does not implement.

For each draft, it finds maximal standard-Base64 alphabet runs that:

- are at least four characters long;
- start and end on an alphanumeric character, matching upstream's word-boundary
  regex (internal `+` and `/` are allowed; leading/trailing ones are not);
- do not touch an ASCII `_` word character at either boundary, matching
  JavaScript `\b` behavior.

Each candidate is decoded with standard Base64, accepting omitted padding as
`atob` does. Decoded bytes must be valid UTF-8, exactly as upstream's fatal
`TextDecoder` check requires. A decodable token is replaced in place, along with
its valid trailing `=` padding when present. Every other character remains
byte-for-byte unchanged: unencoded prose, unsupported encodings, Base64URL text,
short tokens, candidates that decode to invalid UTF-8, and malformed input. The
scan is single-pass, as in the upstream modal.

If nothing decodes, or the input/result exceeds the bounds below, the action
shows a native panel explaining why and proposes no replacement. It never
silently drops unmatched text.

## Uppercase semantics

This is a literal port of WriteUpperCase's transformation, not `to_uppercase()`
over the whole sentence:

- Sentences split after one or more `.`, `!` or `?` characters, then zero or more
  `'`, `"`, `)` or `]` characters, when followed by Unicode whitespace
  (`/(?<=[.!?]+['")\]]*)(\s+)/`).
- Separators are preserved exactly, including repeated whitespace and newlines.
  Rust's Unicode whitespace predicate differs from JavaScript `\s` only for
  U+FEFF (upstream treats it as whitespace; Rust does not) and U+0085 (Rust
  treats it as whitespace; upstream does not).
- Only the first character of each segment is uppercased. The rest is not
  lowercased or otherwise changed.
- Unicode case expansion is retained (for example `ßeta` becomes `SSeta`), while
  uncased first characters such as emoji remain unchanged.
- Upstream's default blocked words `http`, `https` and `ok` are matched
  case-insensitively as prefixes. A blocked segment is left entirely unchanged.
- As upstream does, a segment beginning with a quote, bracket or whitespace is
  not capitalized, because that character is the segment's first character.
  Punctuation without following whitespace does not split a sentence.

## Bounds and output

A draft or transformed result must be at most **2,000 Unicode scalar values** and
**8,000 UTF-8 bytes**. This is stricter and more explicit than upstream's live
pre-send mutation. On overflow the plugin proposes no replacement and reports the
limit in a native panel. The SDK independently enforces its 256 KiB serialized
ABI limit.

The message action inspects only the selected message text the host already
provided. It considers at most **8 decoded matches**, skips any single match
larger than **4,096 UTF-8 bytes**, displays at most **1,024 characters** per
match, and keeps the copy proposal at or below **6,144 UTF-8 bytes** and
**8,000 serialized effect bytes**. Matches beyond those limits are counted and
named in the panel. It does not fetch history, read other messages or look at
attachments or embeds.

The only possible outputs are:

- a `replacement` string, applied only after the user clicks **Apply**;
- one `copy_text` proposal, applied only after the user clicks **Apply**; or
- a native panel for no-op/failure explanations.

No result is sent, saved, persisted or copied anywhere automatically. The
package has no storage action, so decoded text does not survive the invocation.

## Deliberate differences from upstream

| Upstream | This port |
| --- | --- |
| DecodeBase64 is a message-popover button that opens a React modal or copies each decoded string immediately to the clipboard. | The `decode-message` message action lists the bounded decoded matches and proposes one `copy_text` effect, joined by a blank line, after **Apply**. The `decode` composer action substitutes decodable tokens in the draft. |
| DecodeBase64 copies each decoded string immediately and has no reviewed preview. | No clipboard write happens before Apply, and the panel names exactly how many matches the copy will contain. Multi-match messages are copied as one reviewed block rather than one button per string. |
| DecodeBase64 assumes Discord message text and uses `atob` plus a fatal UTF-8 `TextDecoder`. | Equivalent standard-Base64 and strict-UTF-8 behavior in Rust, with explicit byte/item bounds and no host objects. |
| The upstream finder regex technically accepts an optional padding group, but its word boundaries usually exclude `=`. | The port consumes the correct trailing `=` padding for a decoded token so replacement and extraction do not leave stray separators. Detection boundaries otherwise follow the same run/word-boundary rules. |
| DecodeBase64 has a configurable left/right-click copy/open behavior. | No setting: the composer action previews an Apply-reviewed replacement and the message action previews one Apply-reviewed copy. |
| WriteUpperCase edits `msg.content` in a pre-send listener and can use any configured comma-separated `blockedWords`. | A composer action returns an Apply-reviewed replacement. Because the manifest intentionally has no panel or storage action, the upstream defaults `http, https, ok` are fixed. |
| WriteUpperCase uses JavaScript UTF-16 `charAt(0)` and `slice(1)`. | The first full Unicode scalar is uppercased. This is the same for BMP text and avoids splitting surrogate pairs; unlike upstream, an astral lowercase first character can be uppercased. |
| Both upstream plugins run inside the Discord/Vencord client with React, message events and live UI. | This package uses only Serein's composer and message-action contracts. It cannot reproduce live message interception, React popovers, per-string copy buttons or settings UI. |

## Tests

Focused unit tests cover Base64 detection, padding consumption, invalid UTF-8 and
unsupported input preservation, draft bounds, upstream sentence/blocked-word
behavior, Unicode case expansion, typed dispatch, the reviewed message copy, and
the match-count/per-match/copy/effect bounds. They are offline and make no
Discord call.

# Draft Style Tools

Three local, deterministic composer actions adapted for Serein's sandboxed Rust
extension API from Equicord. Every composer result is a replacement proposal that
requires the host's **Apply** action; nothing is edited or sent automatically, and
there are no outgoing message hooks.

- **Reverse draft text safely** (`reverse`)
- **Append signature to draft** (`signature`)
- **Polish draft wording locally** (`polish`)

The package requests only `composer` and `storage`. It has no network, AI, account,
message, event, clipboard, appearance, secret or host-object access. Configuration
is edited in a native panel and saved as opaque bounded JSON.

## Upstream provenance

The reference is exactly
[`Equicord/Equicord@200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e),
specifically:

- `src/equicordplugins/talkInReverse/index.tsx` (TalkInReverse), copyright
  (c) 2023 Vendicated and contributors;
- `src/equicordplugins/signature/index.tsx` (Signature), copyright
  (c) 2024 Vendicated and contributors;
- `src/equicordplugins/polishWording/index.ts` (PolishWording), copyright
  (c) 2024 Vendicated and contributors.

These upstream files are GPL-3.0-or-later. Their notices and the exact source
commit are retained in `src/lib.rs`; this adaptation is licensed
GPL-3.0-or-later. It does not copy upstream React UI, Discord internals,
message-popover APIs, settings plumbing or pre-send listeners.

## Configuration panel

Open **Configure draft style tools** in Settings > Extensions. The panel edits:

- signature name and header (`>` by default), composing the saved signature block;
- whether common contractions gain a missing apostrophe (`dont` -> `don't`), on by
  default;
- whether common contractions expand (`don't` -> `do not`), off by default;
- whether sentence starts are capitalized, off by default;
- whether missing line-end periods are added, off by default;
- comma- or newline-separated words that are not capitalized at a sentence start.

**Save settings** validates and stores the configuration. Invalid values keep the
previous result and explain the problem; they are never silently coerced. Composer
actions read the same stored configuration. An empty signature name disables the
signature action until one is saved.

## Reverse semantics

Upstream reverses `message.content.split("").reverse().join("")` immediately before
sending. That blindly reverses UTF-16 code units, so astral characters can be split
into invalid halves, and mention IDs, URLs and code become unusable.

This port instead reverses the draft over **Unicode scalar values** and protects
these spans from character reversal:

- fenced code blocks (` ```...``` `) and inline code (`` `...` ``);
- HTTP(S) and `www.` URL runs up to whitespace or an angle bracket;
- Discord angle-bracket wire tokens such as `<@123>`, `<@!123>`, `<@&123>`,
  `<#123>`, custom emoji and timestamps.

Protected spans keep their exact bytes. Because the whole draft is reversed, the
*positions* of multiple protected spans mirror just as any other content would;
only their internal text is never reversed. Unclosed code delimiters and an
unmatched `<` are treated as ordinary text. This deliberately does not reproduce
upstream's corruption of mentions, URLs, code or surrogate pairs.

## Signature semantics

Upstream mutates every outgoing message in a pre-send listener, adding
`\n` + optional header + name. This port has **no automatic outgoing hook**. The
Signature action appends the configured block once, only when explicitly invoked,
and returns it for Apply.

- A non-empty draft receives exactly one newline separator unless it already ends
  with a newline; an empty draft becomes just the signature block.
- Header and name are trimmed; an empty header appends only the name.
- Re-running the action on an already-signed draft (allowing trailing whitespace)
  returns the draft unchanged, so it is idempotent per invocation.
- The composed header, separator and name block is limited to **512 UTF-8 bytes**.
  Control characters are rejected so the block cannot inject extra lines.
- A missing or oversized signature explains the problem in a panel and proposes no
  replacement.

No signature is added to messages sent by other paths or by the host.

## Polish wording semantics

This is a local port of PolishWording's transformations, not a grammar service and
not an AI feature. No text leaves the device, no network request is made, and the
plugin makes no claim that the output is grammatically correct.

The port keeps upstream's order:

1. If either apostrophe repair or expansion is enabled, restore missing apostrophes
   for the same contraction map as upstream (`dont` -> `don't`, `cant` -> `can't`,
   and so on), preserving the source word's upper/lower-case pattern.
2. If sentence capitalization is enabled, capitalize the first non-whitespace
   character of each sentence, skip configured blocked words and runs beginning
   with `http`, and uppercase standalone `i`.
3. If punctuation is enabled, add a period to each non-empty line whose last
   character is an ASCII letter or digit and which does not end in a URL.
4. If expansion is enabled, expand the same contraction map into words while
   preserving case in upstream's pattern-repeating way.

Fenced and inline code, HTTP(S)/`www.` URLs and Discord angle-bracket tokens are
masked before these steps and restored byte-for-byte afterwards. Contraction
matching is whole-word and ASCII, matching the upstream word-boundary behavior;
code, URLs and tokens are never touched.

Deliberate deterministic differences from upstream:

- Language detection, AI wording changes, grammar correction and network calls do
  not exist in upstream either; this port adds none.
- Upstream's punctuation frequency slider uses `Math.random()`. This port always
  applies the enabled punctuation step, equivalent to upstream's default 100%
  frequency, so results are reproducible and testable.
- Upstream's active capitalization split regex suppresses every boundary directly
  after a period. This port implements the intended abbreviation guards (`e.g.`,
  `Mr.`, `U.S.` and repeated periods) and otherwise treats a period followed by
  whitespace as a sentence boundary. It also keeps full Unicode scalar handling
  rather than splitting UTF-16 code units.
- Upstream applies these edits in a pre-send listener. This port returns an
  Apply-reviewed replacement and never edits or sends a draft automatically.

## Bounds and storage safety

- Draft input and transformed output are limited to **2,000 Unicode scalar values**
  and **8,000 UTF-8 bytes**. Over-limit input or output leaves the draft unchanged
  and reports the limit in a panel.
- The signature block is at most 512 UTF-8 bytes.
- At most 64 blocked words, each at most 64 UTF-8 bytes and 2,048 bytes in total.
- Stored JSON is at most 64 KiB and must fit the SDK's complete 256 KiB response
  limit. Unknown fields, duplicate blocked words, invalid checkbox values and
  oversized data are rejected.
- Corrupt or oversized storage is reported with a fixed message. Mutating actions
  return no `storage`, so the original value is preserved rather than reset or
  overwritten; composer actions propose no replacement.
- All scans are iterative and bounded; there is no recursion, network access,
  automatic send, automatic draft edit or background work.

## Storage shape

```json
{
  "signature_name": "Ada",
  "signature_header": ">",
  "polish_fix_apostrophes": true,
  "polish_expand_contractions": false,
  "polish_fix_capitalization": false,
  "polish_fix_punctuation": false,
  "polish_blocked_words": ["ok"]
}
```

## Tests

Focused unit tests in `src/lib.rs` cover Unicode scalar reversal, protected
mentions/URLs/code, signature spacing, 512-byte and corrupt-storage rejection,
idempotence, deterministic capitalization/punctuation with protected spans and
blocked words, draft bounds, and typed dispatch routing. Native handler tests are
not Wasm sandbox or live Discord compatibility evidence.

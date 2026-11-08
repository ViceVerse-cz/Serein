# Ingtoninator

A local, deterministic composer action adapted for Serein's sandboxed Rust
extension API from Equicord. The single action proposes one reviewed replacement
that appends an `ington` suffix to one eligible word in the draft. Nothing is
edited or sent automatically, and there is no pre-send hook, toggle, settings
panel, storage or network access.

## Upstream provenance

The reference is exactly
[`Equicord/Equicord@200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e),
specifically `src/equicordplugins/ingtoninator/index.tsx` (Ingtoninator),
copyright (c) 2025 Vendicated and contributors, GPL-3.0-or-later. The original
notice and exact commit are retained in `src/lib.rs`; this adaptation is licensed
GPL-3.0-or-later. It does not copy upstream React UI, ChatBarButton, settings
plumbing or message pre-send listeners.

## Semantics

The action scans only unprotected draft text for runs of Unicode letters and
rejects:

- the standalone words `i` and `I`;
- any word whose final letter is `a`, `e`, `o`, `u` or `y`, compared
  case-insensitively (so `A`, `E`, `O`, `U` and `Y` are rejected too).

These are the upstream `isLegal` rules, ported over Unicode scalar values rather
than UTF-16 code units. Emoji and other non-letter characters are left exactly
where they were; the unmatched original bytes are preserved around the insertion.

Once an eligible word is chosen, the upstream suffix-completion table is applied
to its uppercase form:

| Uppercase word ends with | Inserted suffix |
| --- | --- |
| `INGTON` | *(none; already complete)* |
| `INGTO` | `N` |
| `INGT` | `ON` |
| `ING` | `TON` |
| `IN` | `GTON` |
| `I` | `NGTON` |
| anything else | `INGTON` |

The suffix is lowercased unless the whole selected word is already uppercase,
matching upstream. For example `pin` becomes `pington`, `Pin` becomes `Pington`
and `PIN` becomes `PINGTON`.

## Deterministic selection instead of randomness

Upstream picks a random eligible word with `Math.random()`. This port intentionally
does **not** use randomness. It computes a stable FNV-1a 64-bit hash of the bounded
draft bytes and selects `hash % eligible_word_count` from the eligible words in
draft order. The same draft therefore always yields the same word and the same
replacement; different drafts generally differ. FNV-1a is used only as a stable
selector, not as a cryptographic hash or security primitive. This deterministic
replacement for random selection is a deliberate behavioral difference.

## Protected spans

To keep sendable text intact, the action never selects a word that overlaps:

- fenced code (```` ```...``` ````) or inline code (`` `...` ``);
- HTTP(S) and `www.` URL runs up to whitespace or an angle bracket;
- Discord angle-bracket wire tokens such as `<@123>`, `<@!123>`, `<@&123>`,
  `<#123>`, `<:name:id>` and `<t:...>`.

Protected spans are skipped entirely, not merely excluded by their first index, so
a letter run that touches a URL or token cannot cause an edit inside it. Unclosed
code delimiters and an unmatched `<` are treated as ordinary text.

## Bounds and effects

- Input and output are limited to **2,000 Unicode scalar values** and **8,000
  UTF-8 bytes**. Over-limit input or output leaves the draft unchanged and reports
  the limit in a panel.
- At most one replacement is proposed per invocation, and it always requires the
  host's **Apply** action.
- The manifest requests only the `composer` capability. There is no network,
  account, message-event, clipboard, appearance, secret, storage or host-object
  access.
- No pre-send listener, automatic send, automatic draft edit, background work or
  toggle button exists.

## Integration

This directory is self-contained and deliberately does not edit shared files. To
build and publish it, the repository integration owner must:

1. add `plugins/ingtoninator` to the `members` list in `extensions/Cargo.toml`;
2. build with `cargo xtask package` (or the extensions workspace packaging flow);
3. add the resulting package to `extensions/catalog.json` through the existing
   `extensions/catalog.py`/`pack.py` release process.

The plugin itself declares `api_version` 1, `kind` `plugin`, capability `composer`
and one composer action, `ingtoninate`.

## Limits

- Selection is deterministic, not random, and only one word is changed per Apply.
- The port does not reproduce upstream's toggle button or enabled/disabled setting.
- `char::is_alphabetic` is used as the closest standard-library equivalent of
  JavaScript's `\p{L}` class; rare combining or numeric letter-like code points may
  differ from the JavaScript regular expression.
- Tokens with no case (for example CJK words) count as uppercase, so they receive an
  uppercase suffix, matching upstream's comparison with `toUpperCase()`.
- Native handler tests below are not Wasm sandbox or live Discord compatibility
  evidence.

## Tests

Focused unit tests in `src/lib.rs` cover the suffix-completion table, lowercase and
uppercase word behavior, already-complete `INGTON` words, URL/code/mention/emoji
protection, ineligible words, deterministic selection, draft byte bounds and typed
dispatch routing.

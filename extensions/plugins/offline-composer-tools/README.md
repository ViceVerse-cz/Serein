# Offline Composer Tools

Five deterministic, user-invoked composer actions ported from Equicord/Vencord to
Serein's sandboxed Rust extension API.

## Upstream and deliberate differences

The upstream reference is exactly
[`Equicord/Equicord@200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e).

This port deliberately provides **one plugin package with five composer actions**.
It has no automatic send interception, no message sending, no network access, and
no upstream live-UI hooks or monkey patches. The host applies returned replacement
text only after the user invokes an action. Character count uses a bounded native
SDK panel rather than live UI. The package requests no storage, event, secret,
account, or message capabilities.

## Actions

- **Clear URL trackers** parses inline HTTP/HTTPS URLs with the `url` crate and
  removes `utm_*`, `fbclid`, `gclid`, `dclid`, `msclkid`, `mc_cid`, `mc_eid`,
  `igshid`, `ref_src`, `spm`, and `scm` query keys. Other query values and
  encodings, fragments, surrounding prose, Markdown delimiters, malformed URLs,
  and non-HTTP(S) text are preserved. It never resolves or fetches a URL.
- **Unindent code blocks** dedents the bodies of complete multiline backtick or
  tilde fences. It expands body tabs to four spaces, removes the common space
  indentation from nonblank body lines, and leaves prose, fences, CRLF/LF line
  endings, and unmatched fences unchanged.
- **Convert units** replaces explicit numeric measurements with the corresponding
  value in the opposite unit: C ↔ F, km ↔ mi, kg ↔ lb, cm ↔ in, and m ↔ ft.
  Reverse directions use `mi`, `lb`, `in`, `ft`, and `F`. An optional degree sign
  is accepted for temperatures. Spaces or tabs between a number and its unit are
  retained. Scientific notation is accepted, but infinities, NaNs, overflowing
  results, identifiers, and unit substrings are left alone. For example,
  `10 km` becomes `6.21 mi`, `20°C` becomes `68°F`, and `12in` becomes `30.48cm`.
  Converted numbers have at most two decimal places, with trailing zeros removed.
  Compound-unit inference is intentionally unsupported.
- **Transform text** recognizes only an exact, case-sensitive command at the very
  beginning of the draft. The command and whitespace immediately following it are
  removed before transforming the remaining body:
  - `/mock` applies deterministic lowercase/uppercase alternation to cased
    Unicode scalars, starting with lowercase. Uncased characters do not advance
    the alternation. Unicode case expansions are retained.
  - `/reverse` reverses Unicode scalar values, not grapheme clusters.
  - `/morse` maps ASCII letters, digits, and standard punctuation to Morse. It
    inserts one space between symbols and ` / ` between Unicode-whitespace-
    delimited words. Unsupported characters are copied literally; leading,
    trailing, and repeated word-separating whitespace is normalized.
  - `/wordcount` replaces the draft with the body's Unicode-whitespace-delimited
    word count as a decimal number.

  `/mock hello` therefore produces `hElLo`. `/mocking hello`, `/Mock hello`, and
  drafts without one of these exact prefixes are returned completely unchanged.
- **Character count** shows a panel containing Unicode scalar values, UTF-8 bytes,
  UTF-16 code units, and Unicode-whitespace-delimited words. It never replaces
  the draft.

Processing is offline and linear in the bounded SDK input size. Unexpectedly
large generated expansions fall back to the original draft; the SDK independently
enforces its serialized ABI byte limit. No result is sent anywhere automatically.

Copyright Vendicated and contributors. Licensed under GPL-3.0-or-later; the Serein
port retains those upstream notices in `src/lib.rs`.

## Packaging

Package from `extensions/` with the license text embedded as an inert Wasm custom section:

```sh
python pack.py plugins/offline-composer-tools/manifest.json target/wasm32-unknown-unknown/release/offline_composer_tools.wasm plugins/packages/offline-composer-tools.serein-extension --license licenses/Equicord-GPL-3.0.txt
```

The host ignores custom sections, so this does not change the executable ABI.

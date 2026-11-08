# Message Inspector Tools

Two bounded, read-only message actions adapted from Equicord at commit
[`200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e):

- `src/equicordplugins/showRolesInChat/index.tsx` — show the selected author's
  highest **loaded** guild role, honoring locally stored role exclusions.
- `src/equicordplugins/richMagnetLinks/index.tsx` — parse magnet links in the
  selected message and copy their `dn` display names plus exact links.

This package is **not** an inline renderer. The sandboxed extension ABI has no
way to add a chat decoration, replace the message tokenizer, or render custom
React-like elements inside the timeline. Therefore the upstream inline role
badge and inline magnet-link pill remain native-host work; this package only
offers a truthful inspection/copy subset. It does not claim upstream visual
parity, a working Discord client, or proof of interoperability.

Upstream is GPL-3.0-or-later. Its notices and the exact source commit are
retained in `src/lib.rs`; this package is distributed under the same license.
The implementation does not run JavaScript, patch Webpack/Discord internals,
fetch Google Fonts, read credentials, open magnet links, download media, or
make any network request.

## Actions

| Action | Surface | Behavior |
| --- | --- | --- |
| `open` | panel | Shows the locally stored role-exclusion list for editing. |
| `save` | panel | Validates and saves up to 64 nonzero role snowflakes as opaque bounded JSON. |
| `show-role` | message | Reads the granted `member_details` snapshot for the selected author, filters excluded role IDs, and displays the highest loaded role by position. No effect is proposed. |
| `copy-magnets` | message | Parses up to eight `magnet:?` tokens from the selected message, decodes `dn` names with bounded percent-decoding, and proposes one `CopyText` with the exact name/link lines. |

`copy-magnets` never opens, resolves, downloads or previews a magnet link. The
clipboard replacement happens only after the host's **Apply** confirmation.

## Role inspection semantics

- The selected message reference must contain canonical nonzero snowflakes for
  channel, message and author, plus an optional guild.
- Direct and group messages have no guild role catalog, so the action reports
  that roles are unavailable instead of inventing one.
- `app.member_details` must be present and belong to the same guild and
  conversation as the selected message. A stale or mismatched snapshot is
  rejected rather than interpreted.
- The author must appear in the loaded member pane. Message authors outside that
  pane are reported as unknown; the package never fetches a member record.
- The role must appear in the loaded guild role catalog. The maximum is chosen
  by `(position, role_id)` among the member's loaded, non-excluded role IDs.
  Truncated member roles or role catalogs produce an explicit warning.
- Exclusions are local package storage only. They do not modify the member,
  guild or other plugins. Invalid or oversized stored values are reported and
  never silently reset.
- Upstream's `showBots` setting is omitted: `UserSnapshot` does not expose a
  bot flag, so bot status is neither inferred nor guessed. Upstream's
  context-menu role toggles are replaced by the one local exclusion list.

## Magnet inspection semantics

- Only whitespace-separated tokens that begin exactly with `magnet:?` are
  considered. The package does not try to reproduce Discord's parser token
  boundaries or turn ordinary text into links.
- At most eight links are accepted. Each link is at most 2,048 UTF-8 bytes and
  must not contain control characters. More links, a longer link or a control
  character produces a bounded error and no effect.
- The display name comes only from the exact `dn` query parameter. `+` becomes
  a space and valid `%HH` escapes are decoded; malformed escapes remain
  literal. The name is trimmed to 200 Unicode scalar values and control,
  zero-width and bidirectional formatting characters are replaced with spaces.
  A missing or empty `dn` becomes `unknown filename`, matching the upstream
  fallback.
- The clipboard text is `display name — exact link`, one entry per line, capped
  at 4,096 UTF-8 bytes. If not every parsed link fits, the panel says so and
  only the exact prefix that fits is copied.
- Duplicate links are not deduplicated, reordered or rewritten. Nothing is
  fetched, opened or previewed.

## Capabilities

| Capability | Why |
| --- | --- |
| `selected_message` | Read the selected message text and reference IDs. |
| `member_details` | Read the loaded member and guild role snapshots for role inspection. |
| `clipboard_write` | Propose copying parsed magnet text. |
| `storage` | Persist local role exclusions as opaque bounded JSON. |

No navigation, external-navigation, media, message-send, relationship, account,
notification, reaction, credential or network capability is requested.

## Bounds

| Item | Bound |
| --- | --- |
| Selected message text | Host-provided SDK bound |
| Excluded role IDs | 64, each a nonzero snowflake of at most 20 ASCII digits |
| Magnet links per message | 8 |
| Magnet link bytes | 2,048 UTF-8 bytes |
| Display name | 200 Unicode scalar values |
| Clipboard text | 4,096 UTF-8 bytes |
| Effects per response | 1 |
| Stored settings | One small JSON object, under the host's 64 KiB storage ceiling |

## Files

- `Cargo.toml`
- `manifest.json`
- `src/lib.rs` — handler, bounds and focused unit tests
- `README.md`
- `WAVE.md` — parent integration checklist and explicit omissions

## Tests

`src/lib.rs` unit tests cover:

- highest loaded role selection with multiple exclusions and no proposed effect;
- stale snapshot, missing reference, direct-message scope and corrupt stored
  exclusions producing no effect and no guess;
- magnet `dn` percent/plus decoding, control and bidi sanitization, and exact
  clipboard text;
- too many links, oversized links, control-containing links and no links;
- valid exclusion saving/deduplication and invalid input leaving storage
  untouched.

These are handler-level tests. They are not Wasm-sandbox, host-Apply, live
Discord or native-rendering evidence.

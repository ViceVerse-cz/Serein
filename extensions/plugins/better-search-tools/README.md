# Better Search Tools

A bounded Serein adaptation of Equicord's **BetterSearch**. Upstream patches Discord's search
filter registry and autocomplete to add extra query tokens. Serein has no search-filter registry
surface, so this port provides a reviewed query builder that validates the same token shapes and
proposes the exact native search after Apply.

Upstream source at commit `200dba124066146a8f043fde7fc8b05e3dc5e68e`:
`src/plugins/betterSearch/index.ts`.

## What the panel does

**Build an extended search** shows one text field and the extra filters:

- `includeNsfw` and `mentionEveryone` (`true`/`false`, or omitted);
- `repliedToMessageId`, `repliedToUserId`, `mentionsRoleId` and `commandId` (17-20 digit
  canonical nonzero Discord IDs);
- `embedType` (`image`, `video`, `gif`, `sound`, `article`, or omitted);
- `embedProvider` and `commandName` (one non-control word, at most 64 UTF-8 bytes);
- `slop` (whole number 0-100);
- free search text.

**Review extended search** validates every field, composes one space-separated query and
proposes exactly one `search` host effect. The host applies it only after the user confirms.
Nothing is searched before Apply.

## Capabilities and bounds

| Item | Bound |
| --- | --- |
| Capabilities | `navigation` |
| Composed query | 256 UTF-8 bytes, the host's search limit |
| ID filters | 17-20 digits, nonzero, no leading zero |
| Word filters | 64 UTF-8 bytes, no whitespace or control characters |
| `slop` | 0-100 |
| Host effects | at most one `search` after Apply |

The plugin has no storage, no network, no JavaScript/Webpack access, no Discord module patch,
no automatic search and no logging.

## Upstream omissions

- No native filter-menu entries, autocomplete groups or search-token parser patch.
- No `FILTER_*`/`ANSWER_*` component injection and no `lastResolvedFilter` state.
- The extra tokens are passed through as text. Discord server support for them is **unverified**;
  this port does not claim that a search will return results or that the tokens are documented.

A native search-token parser and autocomplete would need a new host search surface. The reduced
panel is the safe subset the inventory already anticipated.

## Parent integration

This directory does not include packaged Wasm. The parent host must:

1. Add `plugins/better-search-tools` to the `members` list in `extensions/Cargo.toml`.
2. Build `better_search_tools.wasm` for `wasm32-unknown-unknown` with the pinned toolchain.
3. Package it with `python pack.py plugins/better-search-tools/manifest.json ...` and
   `--license ../licenses/Equicord-GPL-3.0.txt` from `extensions/`.
4. Review the built artifact and pin it in the central catalog.
5. Keep the host's `search` effect limited to the current conversation and its 256-byte query
   validation; this plugin relies on that host check as the final boundary.

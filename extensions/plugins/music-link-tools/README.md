# Music Link Tools

A narrow, local-only Serein adaptation of Equicord's **SongLink.desktop** workflow.
Upstream source is pinned to Equicord/Equicord commit
[`200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e):
[`src/equicordplugins/songLink.desktop/index.tsx`](https://github.com/Equicord/Equicord/blob/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/equicordplugins/songLink.desktop/index.tsx),
`SongLinker.tsx`, `Settings.tsx`, `native.ts` and `Providers.ts`.

The upstream GPL-3.0-or-later notice is retained in `src/lib.rs`; this port is
licensed `GPL-3.0-or-later`. It is an independent Rust implementation and does
not run upstream JavaScript, patch Discord modules, call the Songlink API, or
launch provider-native app schemes.

## What it does

- **Panel action `open`** accepts one pasted Spotify, Apple Music, YouTube Music
  or TIDAL track/album link. It recognizes the link locally and proposes one
  host-reviewed HTTPS browser page: either the public `song.link` page for that
  link or the provider's own HTTPS page.
- **Message action `link`** scans the selected message text for the same link
  shapes and proposes the first recognized link's public `song.link` page.
- Every proposal is an `open_external_link` AppAction. The host's ordinary
  **Apply** confirmation is required before a browser opens. Nothing is fetched,
  sent, cached, or changed by the extension itself.

The `song.link` page is a public web page. Serein does not call
`api.song.link`, does not read the response, and does not claim that the page
will resolve every track. The user reviews the exact URL before Apply.

## Recognized link shapes

| Provider | Accepted HTTPS shape |
| --- | --- |
| Spotify | `open.spotify.com/track/<id>` or `play.spotify.com/track/<id>` |
| Apple Music | `music.apple.com/<cc>/album/...` or `itunes.apple.com/<cc>/album/...` |
| YouTube Music | `music.youtube.com/watch?v=<id>` |
| TIDAL | `tidal.com/track/<id>[/u]`, `tidal.com/album/<id>`, `tidal.com/browse/track/<id>` or `tidal.com/browse/album/<id>` |

Only HTTPS, exact hosts, no userinfo, no non-default explicit port, and bounded
path identifiers are accepted. Lookalike hosts, HTTP, artist/playlist pages, zero or
non-numeric TIDAL IDs, and unsupported providers produce no proposal.

## Bounds and safety

| Item | Bound |
| --- | --- |
| Selected-message scan | 16 KiB |
| One URL | 2,048 UTF-8 bytes |
| Distinct links considered per message | 4 |
| Provider path token | 64 ASCII bytes |
| TIDAL numeric ID | 20 digits, nonzero |
| Effects per response | 1 |
| Panel elements | SDK limit (64) |

- No storage, network request, credential, token, account action, message send,
  attachment, clipboard write, native URI, or background task is used.
- The only capabilities requested are `selected_message` and
  `external_navigation`.
- The destination allowlist is `song.link`, `open.spotify.com`,
  `music.apple.com`, `itunes.apple.com`, `music.youtube.com` and `tidal.com`.
- The panel never opens a URL on its first invocation. A second invocation with
  a validated URL and destination is required before an effect is returned.

## Deliberate omissions from upstream

- No inline message accessory, provider buttons, provider logos, per-service
  toggles, metadata header, or track cache.
- No `api.song.link` request, `userCountry` lookup, or response parsing.
- No `nativeAppUriDesktop` or other provider-native scheme launch.
- No automatic message send or bot-message reply.
- No TIDAL iframe embed. `tidalEmbeds` is a separate upstream workflow and is
  not implemented by this plugin; see `WAVE.md`.

## Files

- `Cargo.toml` — SDK-only plugin crate.
- `manifest.json` — panel/message actions and the two capabilities.
- `src/lib.rs` — link recognition, bounds, handler and focused tests.
- `README.md` — this document.
- `WAVE.md` — audit of every assigned upstream path and parent integration steps.

## Tests

`src/lib.rs` unit tests cover exact provider normalization, lookalike/HTTP/
userinfo/non-default-port rejection, deduplication and the four-link cap, the exact
`open_external_link` wire effect, the panel's two-invocation requirement, and
inactive paths for missing, unsupported or oversized input. These are
handler-level tests, not Wasm sandbox, host Apply, browser, or live Discord
evidence.

## Parent integration

This directory is intentionally not registered by this wave. The parent must
add `plugins/music-link-tools` to `extensions/Cargo.toml`, generate its
`extensions/catalog.json` entry, package it with the pinned toolchain and
`extensions/pack.py`, register it with the debug starter if required, and
update the parent-owned `extensions/equicord-workflows.json` mapping for
`songLink.desktop`. No packaged Wasm is committed here. See `WAVE.md` for the
full audit, blockers and integration checklist.

# WAVE — assigned Equicord audit and implementation notes

Upstream checkout: `/tmp/opencode/Equicord-upstream` at
`Equicord/Equicord@200dba124066146a8f043fde7fc8b05e3dc5e68e`.
This wave owns the following entries from `extensions/equicord-workflows.json`.
Only `songLink.desktop` has a new implementation here. Every other entry is
either already covered by an existing Serein workflow or blocked by a missing
host capability; no upstream JavaScript is run and no blockers are disguised as
native parity.

## Implemented subset

### `src/equicordplugins/songLink.desktop`

Upstream files read: `index.tsx`, `SongLinker.tsx`, `Settings.tsx`, `native.ts`,
`Providers.ts`, `README.md`.

Implemented in `music-link-tools`:

- Local recognition of the upstream render regex shapes for Spotify, Apple
  Music, YouTube Music and TIDAL, plus the additional TIDAL album/browse shapes
  used by `tidalEmbeds`.
- One host-reviewed `open_external_link` proposal to either the public
  `https://song.link/<recognized-url>` page or the provider's own HTTPS page.
- A panel action for a pasted link and a message action for the first
  recognized link in selected text.
- Strict HTTPS/host/userinfo/non-default-port allowlists, URL and scan bounds, no network
  request, no metadata cache, no native URI, no automatic send.

Omitted from upstream SongLink:

- `api.song.link` lookup, `userCountry`, response parsing and metadata header.
- Inline `MessageAccessoriesAPI` card, provider button grid, provider logos and
  per-service `servicesSettings` toggles.
- `nativeAppUriDesktop` / `openExternal` native-app launch.
- `/musiclink` slash command and bot-message posting.
- No claim of upstream JavaScript/UI parity or a live music-service integration.

## Audited as already covered (no edit made)

### `src/equicordplugins/splitLargeMessages`

Upstream files: `index.ts`, `splitMessage.ts`, `README.md`.

Existing `extensions/plugins/message-composition-tools` already implements the
bounded review-only subset (`split` composer action) and the catalog records the
limitation: “Review-only bounded segments, not automatic delayed multi-send.”
The upstream automatic sequential send would require a send hook, timers,
slowmode bypass/retry state and partial-failure handling that this SDK does not
expose safely. No duplicate or renamed plugin was added.

### `src/equicordplugins/statusPresets`

Upstream files: `index.tsx`, `style.css`, `README.md`.

Existing `extensions/plugins/status-presets` covers named local presets,
presence fields, explicit Apply proposals and bounded expiry. It already
documents the omitted custom-emoji field and background writes. No edit made.

## Blockers (not implemented, no fallback)

### `src/equicordplugins/sekaiStickers`

Upstream files: `index.tsx`, `Picker.tsx`, `Canvas.tsx`,
`SekaiStickersModal.tsx`, `characters.json.ts`, `kanade.svg.tsx`, `README.md`.

Blocker: the workflow needs remote character artwork and fonts, a canvas
compositor, persistent font assets, and an attachment upload. The plugin ABI has
no bounded asset HTTP request, canvas/image renderer, arbitrary file write or
attachment-content constructor. Adding those would be generic network,
filesystem and upload APIs, so no safe subset is claimed.

### `src/equicordplugins/showBadgesInChat`

Upstream files: `index.tsx`, `settings.tsx`, `styles.css`, `README.md`.

Blocker: badges require per-author Discord user flags/premium fields or the
third-party `badge.equicord.org/badges.json` lookup. The message/author snapshot
contains only `id` and `name`; no badge catalog, flags or consent-gated donor
lookup is exposed. Rendering a local badge row from unavailable data would be
fabrication.

### `src/equicordplugins/showMessageEmbeds`

Upstream files: `index.tsx`, `README.md`.

Blocker: show/remove requires Discord's unfurl REST endpoint and mutation of the
loaded message embed list. The SDK has a read-only `message_content` summary
without embed URLs and no unfurl or embed-write AppAction. No read-only
inspector was added because the native timeline already renders loaded embeds
and an inspector would not close the requested show/remove limitation.

### `src/equicordplugins/sidebarChat`

Upstream files: `index.tsx`, `store.ts`, `styles.css`, `README.md`.

Blocker: Serein extensions have no popout/window creation, always-on-top,
multi-window restore or per-channel window persistence API. Persisting channel
IDs alone would not open a sidebar, so it would be a non-functional reskin.

### `src/equicordplugins/silenceUsers`

Upstream file: `index.ts`, `README.md`.

Blocker: the host has no plugin notification-cancellation or local ignored-user
list surface. `MessageEvent` handlers are observe-only; `Output` cannot suppress
a notification or badge. The native ignore list is an account-visible Discord
relationship action, which this task explicitly forbids. No local “silence”
list was added because it would not silence anything.

### `src/equicordplugins/statusWhileActive.desktop`

Upstream file: `index.ts`, `README.md`.

Blocker: automatic voice-presence changes need a background voice-state hook and
a background presence write with reliable restore. Extensions have no automatic
presence write/background timer surface, and the existing `status-presets`
README already records this omission. No foreground imitation is added.

### `src/equicordplugins/streamingCodecDisabler`

Upstream file: `index.ts`, `README.md`.

Blocker: no codec negotiation, codec list or screen-share encoder preference
exists in the SDK or AppAction surface. A UI preference could not affect media
negotiation, so it is not implemented.

### `src/equicordplugins/tidalEmbeds`

Upstream files: `index.tsx`, `README.md`.

Blocker: replacing TIDAL embeds needs an inline sandboxed iframe/embed surface
and message-embed suppression/replacement. The SDK has no inline third-party
player element or embed mutation. `music-link-tools` recognizes TIDAL links only
as inputs to a reviewed external Songlink/provider browser page; it does **not**
embed, autoplay, or replace TIDAL embeds.

## Bounds and evidence

The implemented plugin documents its byte, token, link, scan and effect bounds
in `README.md` and enforces them in `src/lib.rs`. Its tests are handler-level
only. No Wasm build, host Apply, browser launch, network request, Discord call,
screenshot or measurement was produced in this wave. The parent remains
responsible for compilation, Clippy, tests, packaging and catalog evidence.

## Parent integration steps

This worktree intentionally does not edit central files. To publish the new
plugin, the parent must:

1. Add `"plugins/music-link-tools"` to the `members` list in
   `extensions/Cargo.toml`.
2. Add a generated/pinned catalog entry for `music-link-tools` through the
   central catalog mechanism (`extensions/catalog.py` / `extensions/catalog.json`
   as appropriate). Do not hand-edit another agent's package entry.
3. Build and package the plugin centrally with the pinned toolchain and
   `extensions/pack.py`; do not commit a hand-pinned Wasm.
4. Register the package in the debug starter registry if the parent's current
   registry requires it.
5. Update `extensions/equicord-workflows.json` for `songLink.desktop` to point at
   `music-link-tools` with the limitation text from this file. Leave the other
   assigned entries unclaimed or record these blockers accurately.
6. Run the normal focused checks, `cargo xtask check`, and package/performance
   evidence required for runtime changes; report any failures as parent-owned.

No upstream review, merge, deployment, credential, live Discord, generic
network, filesystem or account-action capability is authorized or used.

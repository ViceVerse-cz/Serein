# Wave 2 audit and implementation notes

Scope: the eleven Equicord upstream directories assigned to this worktree. Upstream
reference is `Equicord/Equicord@200dba124066146a8f043de7fc8b05e3dc5e68e`.
The only source change in this worktree is the mapped `moreCommands` subset in
`extensions/plugins/offline-composer-tools/`; the remaining entries are either
already covered by bounded native behavior or blocked by the extension/host
boundary described below. No package Wasm or catalog pin was rebuilt here.

## Implemented

### `moreCommands` (safe local subset) — mapped to Offline Composer Tools

Upstream files: `src/equicordplugins/moreCommands/index.ts`,
`src/equicordplugins/moreCommands/utils.ts`.

Added deterministic composer commands to the existing `transform-text` action:
`/upper`, `/lower`, `/repeat N` (1–10, pre-allocation byte cap), `/uwuify`
(deterministic upstream replacements; no random catchphrase), and auto-detect
Morse decode for `/morse`. Existing `/mock`, `/reverse`, `/morse` encode, and
`/wordcount` are unchanged. All results still require an explicit composer
action; nothing is sent, fetched, or persisted.

Omitted intentionally: system/device commands, network animal/neko/GIF commands,
image-to-GIF, multi-message countdown/echo/ping, current-user mention/slap,
affinity rendering, random-response commands, and the Unicode `normalize` option
(no new dependency). See the package README for the exact list.

## Already covered natively (no edit)

### `mediaPlaybackSpeed`

Upstream: `src/equicordplugins/mediaPlaybackSpeed/index.tsx`,
`components/SpeedIcon.tsx`.

Native inline audio and video sessions already expose a compact 0.25x–3.5x
control and apply the rate through the desktop playback backend
(`crates/ui/src/audio.rs` `playback_speed_control`, `crates/ui/src/video.rs`,
`crates/ui/src/media.rs`). The inventory already records this as `native` with
the limitation "no persisted per-media-type defaults". Persisting defaults would
touch native playback/settings state outside this worktree's plugin-only source
scope, so no duplicate plugin is added.

### `limitlessScreenshare`

Upstream: `src/equicordplugins/limitlessScreenshare/index.tsx`,
`settings.tsx`, `CustomRange.tsx`.

Native screen sharing already offers bounded resolution presets and the frame
rates actually advertised by the capture backend
(`crates/ui/src/screen.rs`, `offered_resolutions`, `offered_frame_rates`, plus
the explicit "does not require Nitro" note). The upstream arbitrary sliders are
not faithfully portable because the native encoder must clamp to backend-offered
modes; the existing selection is the safe adaptation. No edit.

### `micLoopbackTester`

Upstream: `src/equicordplugins/micLoopbackTester/index.tsx`.

Native microphone test already provides local-only loopback through
`discord-voice` `Audio::preview` and `preview_level_db`
(`crates/ui/locales/en-US/main.ftl` "Microphone test",
`apps/desktop/src/voice.rs` `poll_mic_preview`). It is unavailable while a call
is active and never routes preview audio into the call. The upstream account
panel button is an entry-point difference, not missing audio behavior. No edit.

### `noNitroUpsell`

Upstream: `src/equicordplugins/noNitroUpsell/index.ts`.

Serein does not render Discord's Nitro subscription upsells, and it does not
mirror a fake premium entitlement into account state. Native screen share
settings explicitly state that screen sharing does not require Nitro; premium
state comes from the account entitlement for emoji/sticker eligibility. Setting
`premiumTypeOverride=2` would be a forbidden entitlement spoof. No edit.

### `messageLoggerEnhanced` (existing mapped package)

Upstream: `src/equicordplugins/messageLoggerEnhanced/*`.

The durable, consent-gated, account-scoped `message-history` package and native
store already cover the reviewed bounded subset: subsequently loaded readable
chats, explicit retention, attachment opt-ins, ghost alerts, and text/JSON
export. Per the task constraints the current durable MessageLogger was not
modified. Upstream import, deleted-before-load recovery, arbitrary downloads,
and store/DOM parity remain outside the package guarantee.

## Blocked (no safe plugin-only implementation)

### `keyboardSounds`

Upstream: `src/equicordplugins/keyboardSounds/index.ts`, `packs.ts`.

The extension ABI has no general audio-playback surface, and upstream fetches
WAV/MP3 packs from GitHub URLs at runtime. Implementing this would require new
native key-event observation plus bundled or user-supplied audio assets and a
non-call SFX mixer with concurrency/volume caps. That is outside the allowed
plugin-only source scope, and downloading packs is forbidden. Clear blocker, not
a reason to add a silent/stub plugin.

### `messageLinkTooltip`

Upstream: `src/equicordplugins/messageLinkTooltip/index.tsx`.

The extension ABI cannot inject a hover card into timeline widgets, and a
cross-channel preview requires a bounded one-message read that the current
snapshot/effect surface does not expose. Native inline reply previews already
exist in `crates/ui/src/timeline.rs`, but the assigned hover/cross-channel card
would require native timeline and bounded-read work outside this worktree's
plugin-only scope. Blocker for this pass.

### `messagePeek`

Upstream: `src/equicordplugins/messagePeek/index.tsx`.

DM rows are native timeline/sidebar widgets; the SDK cannot render into the DM
list. `AppSnapshot` exposes channel identity and selected-channel timeline only,
not another DM's last message content. The upstream bounded fetch loop (25 DMs,
batches of 5, cache) therefore needs native UI and a bounded history reader.
Blocker for this pass.

### `moreStickers.desktop`

Upstream: `src/equicordplugins/moreStickers.desktop/index.tsx`, `stickers.ts`,
`lineStickers.ts`, `upload.ts`, `components/picker.tsx`.

The current host has no plugin filesystem/asset import API. Upstream scrapes
LINE store pages, fetches remote packs, runs FFmpeg, and injects a native picker.
A safe local-pack importer would require native picker and local-file import
support with 160x160 PNG/GIF validation, pack/byte caps, and no scraping. That is
outside the allowed source scope; there is no truthful plugin-only subset that
can display or send stickers. Blocker.

### `newPluginsManager`

Upstream: `src/equicordplugins/newPluginsManager/index.tsx`,
`NewPluginsModal.tsx`, `knownSettings.ts`.

The plugin has no access to the host extension registry or its settings schema,
and it cannot enable extensions or reload the client. Reproducing it as a panel
over static data would be a fake. A real implementation needs a native
registry/settings discovery surface and explicit confirmation before enable or
reload. Blocker for this pass.

## Parent integration steps

1. Rebuild and repackage `offline-composer-tools` from this source and update the
   central package/catalog pin/version evidence. The manifest is now `1.1.0`.
   No Wasm was generated or pinned in this worktree.
2. Run the normal focused checks for the package and the real-Wasm safe-port
   check, then `cargo xtask check`. The committed package still contains the
   previous module and will intentionally mismatch until the parent rebuild.
3. Keep the existing native `mediaPlaybackSpeed`, `limitlessScreenshare`,
   `micLoopbackTester`, `noNitroUpsell`, and `messageLoggerEnhanced` mappings.
   The blocker items require separate native designs if the owner wants them.

## Verification performed here

- `rustfmt --edition 2024` on
  `extensions/plugins/offline-composer-tools/src/lib.rs`.
- Inline tests added for Morse encode/decode and malformed tokens, case mapping,
  repeat count/overflow/allocation caps, command-prefix strictness, and URL/
  one-character preservation in `uwuify`.
- NOT run here (per resource gate): `cargo test`, `cargo check`, Clippy, build,
  package, screenshots, measurements, or any live Discord call.

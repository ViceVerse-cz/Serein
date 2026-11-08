# WAVE handoff — tranche 2, assigned Equicord workflows

Worktree: `/home/nicolas/.t3/worktrees/Serein/t3-54e78ac9`
Upstream reference: Equicord/Equicord commit
`200dba124066146a8f043fde7fc8b05e3dc5e68e`, read only under
`/tmp/opencode/Equicord-upstream/src/`.

This wave adds one new package, `extensions/plugins/voice-panel-tools`. The other
assigned entries are either already covered by existing packages/native code or
are blocked by the sandbox contract; none was re-skinned or duplicated.

## Implemented: vcPanelSettings

Package: `extensions/plugins/voice-panel-tools`
Manifest ID: `voice-panel-tools`
Grant: `audio_settings` only.
Actions: `open` and `apply` (both `panel`).

Upstream files that guided the subset:

- `src/equicordplugins/vcPanelSettings/index.tsx`
- `src/equicordplugins/vcPanelSettings/README.md`
- `src/equicordplugins/vcPanelSettings/style.css` (layout only; not ported)

Implemented subset and bounds:

- output volume slider 0-200 percent;
- input volume slider 0-100 percent;
- one `app_action` / `set_audio_settings` proposal per Apply review;
- only changed gain fields are sent; omitted fields are preserved by the host;
- `open_microphone` is always `false`;
- current input gain above 100 percent is displayed but not editable, so a
  volume-only edit cannot silently lower it;
- no audio device/camera enumeration, selection or storage; no React/DOM patches;
- no network, filesystem, credentials, notifications or automatic mutation.

Omissions from upstream: voice-panel injection, device selectors, camera
selector, `MediaEngineStore`/`FluxDispatcher` subscriptions, "uncollapse"
preference and header toggles. The Wasm SDK exposes no device/camera list and no
voice-panel render hook.

## Assigned entries audited with no new code

### showSongName — already native (no edit)

Upstream: `src/equicordplugins/showSongName/index.tsx`.
Serein's compact activity text in `crates/ui/src/profiles.rs` already uses the
Spotify `state` field (the song name) via `subtitle()`. No new native preference or
package is justified.

### signature — already packaged (no edit)

Upstream: `src/equicordplugins/signature/index.tsx`.
Covered by `extensions/plugins/draft-style-tools` (`signature` composer action).
It remains Apply-reviewed; adding upstream's automatic pre-send mutation is not
authorized and is not needed for the mapped limitation.

### talkInReverse — already packaged (no edit)

Upstream: `src/equicordplugins/talkInReverse/index.tsx`.
Covered by `extensions/plugins/draft-style-tools` (`reverse` composer action) with
Unicode-scalar reversal and protected URLs/code/Discord tokens. No duplication.

### toastNotifications — already native (no edit)

Upstream: `src/equicordplugins/toastNotifications/index.tsx` and
`components/Notifications.tsx`, `components/NotificationComponent.tsx`.
Serein's native notification surface already implements bounded clickable toasts
with DND/mute checks. Remaining upstream-only choices (streamer mode, separate
channel allowlist) require native notification-preference work, not a plugin.

### toneIndicators — already native (no edit)

Upstream: `src/equicordplugins/toneIndicators/index.tsx`,
`indicators.ts`, `ToneIndicator.tsx`.
Serein renders the fixed upstream dictionary in
`crates/ui/src/presentation.rs` and `crates/ui/src/markdown.rs`. Custom indicator
settings would be a native renderer change; no extension render hook exists.

## Blockers (no safe implementation in a plugin directory)

### snowfall

Upstream `src/equicordplugins/snowfall/index.tsx` creates a full-window DOM
overlay with an animation interval and React root. The SDK's `appearance`
capability is a declarative colour/metrics overlay, not arbitrary drawing or a
frame loop. A plugin cannot honestly reproduce snowfall. Implementing it would
require a new native overlay with hard particle/time/visibility caps.

### stickerBlocker

Upstream `src/equicordplugins/stickerBlocker/index.tsx` patches Discord's sticker
renderer, patches both sticker context menus and optionally loads a remote GIF.
The SDK has no message-render hook, context-menu patch or remote asset fetch. A
local blocked-ID list alone would not hide stickers and would be a misleading
partial feature, so none was added.

### stopAutoUnread

Upstream `src/equicordplugins/stopAutoUnread/index.ts` replaces
`maybeAutoUpgradeChannel`. Serein does not expose that Discord-internal function,
and no SDK action can suppress a notification-level auto-upgrade. This needs a
narrow native notification-behaviour change, not a plugin.

### title

Upstream `src/equicordplugins/title/index.ts` replaces Discord's window-title
prefix and refreshes the page title. Wasm receives no window/title effect, and the
SDK has no title output. This needs a native window-title preference.

### urlHighlighter

Upstream `src/equicordplugins/urlHighlighter/index.tsx` patches Discord's link
renderer and applies a pattern-specific colour/bold class per `href`. The SDK has
no link-render hook; the declarative `appearance` overlay only sets global
colours/styles and cannot match per-URL patterns. No partial audit panel was
added because it would not highlight anything.

## Parent integration steps (central, not changed here)

1. Add `"plugins/voice-panel-tools"` to the `members` list in
   `extensions/Cargo.toml`. This package must not be built as a root-workspace
   member.
2. Add the package to the central catalog/debug starter registry and generated
   package pins through the normal parent build/package process.
3. Update `extensions/equicord-workflows.json` for `vcPanelSettings` centrally
   (implementation status/package path/limitations); this worktree did not edit
   the inventory.
4. No host/native changes are required: `Capability::AudioSettings`,
   `AppSnapshot::audio_settings`, `AppAction::SetAudioSettings` and the native
   Apply path already exist in this commit.
5. Build/package the Wasm centrally. This wave did not run cargo test/check/clippy,
   rustc, packaging or screenshots; only the new `.rs` file may be rustfmt'd.

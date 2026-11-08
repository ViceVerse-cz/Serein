# WAVE — Equicord interface, tags, effects and notifications

Status: **audit complete, no package produced**.

Scope: the eleven paths assigned for this wave. Six are already implemented by
an existing Serein surface, one is satisfied by default plus an optional native
toggle, and four need a host/native surface that the sandboxed extension API does
not expose (`pingNotifications` is partly covered and needs two native options).
This directory adds documentation only; it does not add a package, Wasm bytes, a
manifest or a catalog entry.

Upstream source read only from `src/equicordplugins/` in
`Equicord/Equicord` at `200dba124066146a8f043fde7fc8b05e3dc5e68e`.

## Disposition

| Upstream path | Serein status | Evidence | Action here |
| --- | --- | --- | --- |
| `middleClickTweaks` | Covered by the native pointer policy | `apps/desktop/src/pointer.rs`, `crates/ui/src/lib.rs` | None |
| `moreUserTags` | Blocked — needed data and a render decoration are not exposed to plugins | Native account badges only (`crates/model/src/lib.rs`, `crates/ui/src/account_badge.rs`) | Host follow-up |
| `moyai` | Blocked — no one-shot sound effect or reaction/voice-effect event API | `crates/platform/src/audio.rs`, `extensions/sdk/src/lib.rs` | Host follow-up |
| `noRPC.discordDesktop` | Satisfied by default; optional separate native toggle | `apps/desktop/src/game_activity.rs`, `crates/platform/src/game_activity.rs` | Host follow-up (optional) |
| `noRoleHeaders` | Covered natively | `crates/model/src/channel_display.rs`, `crates/ui/src/lib.rs` | None |
| `normalizeMessageLinks` | Covered by `social-link-tools` | `extensions/plugins/social-link-tools/src/lib.rs` | None |
| `notificationTitle.discordDesktop` | Covered natively | `crates/client-core/src/notifications.rs` | None |
| `partyMode` | Blocked — no cosmetic POGGERMODE/confetti surface | `apps/desktop/src/main.rs`, `crates/ui/src` | Host follow-up |
| `pendingFriendRequest` | Covered by `social-link-tools` | `extensions/plugins/social-link-tools/src/lib.rs` | None |
| `pinIcon` | Covered natively | `crates/ui/src/timeline.rs` | None |
| `pingNotifications` | Mostly covered natively; two options blocked | `crates/client-core/src/notifications.rs`, `apps/desktop/src/notification_runtime.rs` | Host follow-up |

## Covered mappings — no edit needed

### `middleClickTweaks`

Upstream prevents middle-click from opening links/media and prevents
middle-click paste for a configurable period.

Serein already removes `Event::MiddleClickPaste` before egui sees it and strips
the middle-button press from egui's event list in
`apps/desktop/src/pointer.rs` (the press is retained only for explicit key
bindings). The middle button therefore cannot activate a link or paste text;
it is used for autoscroll instead. `crates/ui/src/markdown.rs` only opens links
from ordinary activations, never a middle-click event.

The default upstream settings (`openScope = none`, `pasteScope = always`) match
this behavior for paste and are stricter for link opening. A configurable
threshold would only make sense if the client first gained a middle-click paste
or link-open path, which it intentionally does not have. No native or plugin
change is warranted.

### `noRoleHeaders`

Upstream returns `null` only for the member-list header row whose role is
`UNKNOWN`; real role headers and the online/offline groups stay.

Serein implements exactly that narrower behavior: `hide_no_role_headers` and
`model::channel_display::is_pseudo_member_group` hide only a group whose id is
neither `online`/`offline` nor a loaded role, and only after role metadata has
loaded. Real role groups remain visible. The preference is in
`crates/ui/src/settings.rs` and the row filter in `crates/ui/src/lib.rs`.

### `normalizeMessageLinks`

The existing `social-link-tools` package normalizes the selected message text:
only the authority of exact `https://canary.discord.com` and
`https://ptb.discord.com` links is rewritten to `discord.com`; path, query,
fragment, percent escapes and every other byte are preserved. It is bounded to
20 links and 2,048 UTF-8 bytes and proposes one Apply-reviewed clipboard copy.
Nothing is fetched, opened or sent. See that package's README for the full
narrowing.

### `notificationTitle.discordDesktop`

Serein already builds titles from loaded data:
`crates/client-core/src/notifications.rs::notification_title` adds
`sender replied to replied-author` when the loaded timeline contains the reply,
then `in #channel`, `in #parent` for threads and `, guild name`. The title is
bounded to 200 characters/200 bytes and the platform accepts at most 256 title
bytes. Missing names are not invented.

The upstream DM/group channel-name suffix is not reproduced; Serein's DM title
is the sender, which is accurate without a second name lookup. That is a
cosmetic difference, not a missing workflow worth duplicating.

### `pendingFriendRequest`

The existing `social-link-tools` package lists only currently loaded outgoing
requests from the granted relationship snapshot, revalidates the kind at Apply
and proposes one `ResolveFriendRequest { accept: false }`. Incoming requests,
friends and other users are never selectable. It is bounded to 20 listed rows
and one effect per response; there is no automatic cancellation.

### `pinIcon`

Serein draws a pin badge in `message_badges` only when
`state.is_pinned(channel, message)` reports known native pin state
(`crates/ui/src/timeline.rs`). It does not infer pins or fetch pin lists. This
matches the upstream render-only behavior.

## Blocked workflows

These are not forbidden-behavior blockers; they are missing host surfaces. A
sandboxed Wasm plugin can read only the capability-scoped snapshots in
`extensions/sdk/src` and can return only the typed effects in
`extensions/sdk/src/app.rs`. None of the following can be truthfully expressed
through those snapshots and effects, and adding a lookalike panel would be a
re-skin rather than the upstream behavior.

### `moreUserTags`

Upstream computes owner/admin/staff/mod/VC-mod/chat-mod/webhook tags from
`GuildStore` ownership, `computePermissions` and message webhook metadata, then
renders them in messages, profiles and the member list with per-tag text and
visibility settings.

The plugin API exposes member role ids and a loaded role catalog, but not the
target member's computed guild/channel permissions, guild owner id, or message
`webhook_id`/followed-channel state; it also has no render-decoration hook.
Classifying tags from role names or positions would be wrong.

Host follow-up: compute tags natively from the already-loaded guild, member and
role data with the existing permission calculation, for visible members only.
Render a bounded badge through the existing account-badge/name area, with the
seven upstream tags, per-tag show/hide and a short custom label cap. No fetch,
no permission escalation and no webhook tag for followed announcements unless
the message reference is known.

### `moyai`

Upstream plays a bundled moyai sound on message create, reaction add and voice
channel effect, capped at 10 plays per message with 300 ms spacing, respecting
focus, blocked users, bots and volume. The upstream code fetches audio from
GitHub in the current source; that network path must not be ported.

Serein has voice/media playback but no one-shot sound-effect host effect, no
reaction event and no voice-effect event in the plugin event API. A plugin
cannot play audio, and `OpenExternalLink`/network audio would violate the
sandbox and the no-network rule.

Host follow-up: a native SFX path with a bundled, size-capped audio asset and
the upstream 10-play/300 ms/focus/block/bot/volume limits. Message-create
content scanning can use the existing message events; reaction and voice-effect
triggers need native events. No remote asset fetch.

### `noRPC.discordDesktop`

Upstream removes Discord's always-on local `discord_rpc` module load.

Serein does not embed Discord. Its Discord-compatible local IPC/WebSocket
listener is created only while activity sharing is enabled in
`apps/desktop/src/game_activity.rs`, and is torn down with that session. The
listener is therefore already off by default and cannot be reached unless the
owner opted into activity sharing. This is stronger than the upstream default.

Optional host follow-up: if the owner wants to publish presence without
accepting local game RPC, split the listener from presence publishing behind a
separate, explicit, default-off preference. It must not touch remote account
state and must close existing clients when disabled.

### `partyMode`

Upstream dispatches `POGGERMODE_SETTINGS_UPDATE` to enable and tune Discord's
built-in screen shake and confetti.

Serein has no POGGERMODE dispatcher, cosmetic-effect API or plugin effect that
can trigger screen shake or confetti. A theme change or a tick-driven color
animation would be a different feature and is not an acceptable substitution.

Host follow-up: a native, default-off cosmetic effect with the three intensity
presets, reduced-motion respect, a bounded particle count/duration and no
account or network mutation. It should not be a plugin surface unless the host
adds an explicit cosmetic-effect capability with consent.

### `pingNotifications`

Upstream offers toggles for friends' server messages, direct mentions, DMs,
active-channel notifications and muted/blocked filtering, with mention
formatting and a 200-character body.

Serein already covers the core: `observe_notification` handles DMs and mentions,
blocked/ignored authors, mute/DND/access checks, bounded mention formatting
(160 chars/512 bytes), one alert per DM run, and notification titles; the desktop
runtime suppresses an alert for the focused channel and while DND is active.
The notification queue is bounded to 32 items/16 KiB.

Two upstream options are missing: notifying for friends' messages in servers and
forcing an alert for the currently focused channel. A plugin cannot emit an OS
notification (the SDK's `Notice` effect is in-app only), so this belongs in
native notification code.

Host follow-up: add `friends_in_servers` and `show_in_active_channel` to the
existing native notification tools/preferences, using the already-loaded friend
list, preserving mute/block/DND/access precedence, and keeping the existing
32-item/16 KiB bounds. `show_in_active_channel` must be default-off and must not
bypass DND or mute.

## Bounds and omissions

- No new package, manifest, Wasm, dependency, network call, credential access,
  filesystem API, automatic send, microphone use or telemetry.
- No JavaScript, Webpack/system injection or raw gateway/store access.
- Existing durable MessageLogger behavior was not touched.
- The four workflows that still need a host surface remain visibly
  unimplemented rather than being approximated with unrelated UI.
- The notification-title DM/group suffix and the upstream middle-click settings
  are documented cosmetic differences, not claimed parity.

## Tests and checks

No Rust, Wasm, catalog or host checks were run for this directory: it contains
documentation only, and this wave's resource gate forbids compilation, package
builds, tests and screenshots. Parent validation should be limited to Markdown
review and `git diff --check`.

When any host follow-up above is implemented, the useful regression tests are:
tag classification and visible-member bounds; SFX rate/focus/block/bot limits;
local-RPC listener lifecycle tied to its preference; POGGERMODE intensity and
reduced-motion limits; and notification friend/active-channel gating that still
obeys DND, mute and block.

## Files

- `README.md`
- `WAVE.md`

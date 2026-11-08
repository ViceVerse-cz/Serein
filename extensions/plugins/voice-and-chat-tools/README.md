# Voice & Chat Tools (bounded)

A safe Rust/WebAssembly adaptation of three Equicord voice utilities. It does not run
Equicord JavaScript, patch Discord internals, call the Discord REST API directly, read raw
Gateway payloads, fetch media/soundboard bytes, or perform automatic account actions.

Upstream reference: `Equicord/Equicord` at commit
`200dba124066146a8f043de7fc8b05e3dc5e68e`.

## Implemented upstream subset

| Upstream path | Implemented here | Files that guided the subset |
| --- | --- | --- |
| `src/equicordplugins/voiceButtons` | Panel for the current call listing remote participants. Each Apply proposes one local `SetParticipantAudio` action (volume 0–200 and local mute). A selected loaded friend can also get one `OpenFriendDm` navigation proposal. | `index.tsx`, `settings.ts`, `utils.tsx` |
| `src/equicordplugins/voiceChannelLog` | Bounded current-call session log fed by `app_events` and the `voice_state` snapshot. Records participant joins/leaves and your own mute/deafen/camera/screen-share changes. Optional panel view and explicit clear. | `index.tsx`, `types.ts`, `logs.ts` |
| `src/equicordplugins/voiceChatUtils` | The “Mention all Users” action as a composer tool. Appends deduplicated `<@id>` mentions for the current call's participants to the draft. Nothing is sent automatically. | `index.tsx` |

No package or `.serein-extension` artifact is committed by this change. The parent branch
builds, reviews and packages the Wasm centrally.

## Bounds and consent

- Voice snapshot: at most 64 participants; panel select shows at most 32 options.
- Participant audio: one target per Apply, local device audio only, volume 0–200.
- DM shortcut: one loaded friend per Apply; the native host rechecks friendship and navigation.
- Session log: at most 256 entries; displayed newest-first, 40 lines per view; 48 KiB of
  plugin storage; corrupt storage is preserved until the user explicitly clears it.
- Mention-all: at most 64 deduplicated participants, excludes your own ID when the account
  profile is available, respects the 2,000-character message limit and 8,000-byte draft limit.
- Every account/voice change is a host proposal that requires explicit Apply. The plugin uses
  `audio_settings` only for the local per-participant audio action and does not request send,
  voice-connect, camera, media or server-moderation capabilities.

## Truthful limitations

- The SDK `voice_state` snapshot is scoped to the call you are currently in. The voice log
  therefore only covers that call, not arbitrary channels you are not connected to.
- There is no wall-clock timestamp in the extension host, so log entries are ordered by a
  local sequence number rather than time.
- Only your own mute/deafen/camera/screen-share state is exposed per snapshot. Other
  participants' state changes and soundboard events are not observable.
- Server mute/deafen, moving participants, disconnecting participants and arbitrary
  channel-wide REST batches are deliberately omitted. They need a future native,
  permission-checked batch preview action; this package cannot implement them safely.

## Assigned upstream paths that are not implemented here

These are blockers or already-covered behavior, not silent omissions:

- `src/equicordplugins/toggleVideoBind` — requires a native global keybind registration and
  conflict handling. Extensions cannot register global keydown handlers. Add a native
  `ToggleCamera` keybind action in `crates/model/src/keybinds.rs` and dispatch it through the
  existing camera command; do not emulate the keybind from Wasm.
- `src/equicordplugins/translatePlus` — requires a host-mediated fixed translation provider or
  a pinned local model, plus bundled dictionaries. The current host offers no translation
  action, and arbitrary Google/GitHub endpoints are forbidden. A safe future native
  `message_translation*` capability with a fixed provider, request/response/time caps and
  explicit consent is the prerequisite.
- `src/equicordplugins/universalMention` — requires a native mention-candidate/autocomplete API
  or a composer selection/candidate field. The current composer contract has no candidate
  channel, so a panel cannot insert an inline autocomplete mention truthfully. The bounded
  fallback here is only mention-all for current voice participants.
- `src/equicordplugins/unreadBadgeCount` — the `read_state` snapshot only exposes the selected
  channel. Per-channel unread counts and muted-channel rules require extending native
  `read_state` with a bounded channel-count directory and a channel-list badge.
- `src/equicordplugins/userpfp` — requires a native local-only avatar map, host image picker
  and avatar-renderer integration. The remote UserPFP database and arbitrary image URLs are
  forbidden; the extension API cannot replace rendered avatars.
- `src/equicordplugins/voiceJoinMessages` — the main behavior (friends joining any voice
  channel) needs a bounded native voice-directory/event feed. The current snapshot only knows
  your own call. Emitting a same-call-only notice would invert upstream's default-suppressed
  case, so it is intentionally not shipped.
- `src/equicordplugins/voiceMessageTranscriber.desktop` — requires a host-managed local
  transcription model, audio access and a pinned cache. Remote model URLs, workers/WebGPU and
  raw audio fetching are forbidden.
- `src/equicordplugins/voiceMessagesInBackground` — requires a host-selected message-audio
  playback capability and a native mini-player. Extensions must not receive attachment URLs
  or audio bytes.

## Inline tests

`src/lib.rs` includes focused tests for:

- voice snapshot diffing (join/leave, self state, no duplicate events, first-snapshot seeding);
- log entry and storage validation/caps, including corrupt-storage preservation;
- participant-audio target/volume validation and self/stale rejection;
- friend-only DM proposals;
- mention-all self exclusion, deduplication and message-length enforcement;
- option sanitization/uniqueness/bounds and log panel rendering bounds.

These tests were **not run** in this worktree. The resource gate forbids Cargo/rustc execution;
the parent runs `cargo xtask check` and packaging sequentially.

## Parent integration

1. Add `"plugins/voice-and-chat-tools"` to the `members` list in `extensions/Cargo.toml`
   (this file is central and intentionally untouched here).
2. Regenerate/update `extensions/Cargo.lock` centrally.
3. Build the release Wasm and package it with the Equicord GPL license section, then register
   the package/catalog entry centrally. The plugin crate name is `voice-and-chat-tools` and the
   library target is `voice_and_chat_tools`. Add the retained GPL notice to
   `THIRD_PARTY_NOTICES.md` centrally if the parent's notice policy requires it.
4. If the host later adds a clock, cross-channel voice directory or native batch voice
   moderation preview, update the README and the corresponding `equicord-workflows.json`
   disposition rather than expanding this package with raw requests.

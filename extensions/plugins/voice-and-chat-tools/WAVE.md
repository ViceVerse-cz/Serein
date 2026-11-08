# WAVE — voice-and-chat-tools

## Purpose

One bounded Equicord-derived plugin package covering the safely representable subset of:

- `voiceButtons` — current-call participant local audio and friend DM shortcuts.
- `voiceChannelLog` — current-call session activity log.
- `voiceChatUtils` — “Mention all Users” composer action only.

The remaining assigned upstream paths are audited in `README.md` with explicit blockers.

## Safety review

- No arbitrary JavaScript, Webpack/system injection, credential/session access or raw
  Gateway/store reads.
- No generic network, filesystem, OAuth or audio/file APIs.
- One bounded host proposal per Apply; no automatic sends, microphone use, server moderation
  or account actions.
- Storage is capped at 256 log entries and 48 KiB; corrupt storage is preserved.
- Participant lists are capped at 64 and panel selections at 32.
- No packaged Wasm is committed; the parent builds and reviews it centrally.

## Parent host steps

1. Add the workspace member `plugins/voice-and-chat-tools` in `extensions/Cargo.toml`.
2. Build and package with the existing `pack.py` flow and the Equicord GPL-3.0 license file.
3. Register the package/catalog entry centrally (do not reuse another agent's package ID).
4. Add the manifest to the debug/starter registry only if the parent's debug surface requires it.
5. Update `extensions/equicord-workflows.json` dispositions for the three implemented paths and
   keep the remaining paths marked as blocked/pending native support.

## Native follow-ups needed for the omitted workflows

- Camera toggle keybind (`toggleVideoBind`): add a native keybind action and conflict UI.
- Translation (`translatePlus`): fixed host-mediated provider or pinned local model with
  request/response/time caps.
- Mention candidates (`universalMention`): native candidate directory/composer candidate field.
- Per-channel unread counts (`unreadBadgeCount`): extend native read state and channel-list badge.
- Local-only avatars (`userpfp`): native picker and avatar renderer map.
- Cross-channel friend join notices (`voiceJoinMessages`): bounded native voice directory/event feed.
- Local transcription (`voiceMessageTranscriber.desktop`): host-managed model and audio access.
- Background message audio (`voiceMessagesInBackground`): host-selected playback capability.
- Batch voice moderation (`voiceChatUtils`): native permission-checked, paced batch preview for
  mute/deafen/move/disconnect.

## Verification status

Only `rustfmt --edition 2024` was run on `src/lib.rs`. No Cargo check, test, Clippy, build,
package, screenshot or live Discord call was run in this worktree. The parent executes the
normal sequential checks.

# WAVE: voice and core Equicord workflow tranche

Upstream reference: `Equicord/Equicord` at
`200dba124066146a8f043fde7fc8b05e3dc5e68e`, read from
`/tmp/opencode/Equicord-upstream/src/` only. This worktree adds one plugin,
`voice-rejoin-tools`, and records why the other assigned workflows are not
ported here. A package is a safe Rust adaptation, not Equicord JavaScript parity
and not evidence of working Discord interoperability.

## Status by assigned path

| Assigned upstream path | Status | Result / blocker |
| --- | --- | --- |
| `src/equicordplugins/voiceRejoin/index.tsx` | **Implemented subset** | `voice-rejoin-tools` remembers one last active channel from Voice app events and proposes one muted, no-ring Apply-gated `JoinVoice`. No automatic rejoin, no delay/timer, no timeout window and no empty-call check. |
| `src/equicordplugins/voiceStats/index.tsx` | **Blocked** | Durations require a monotonic host clock or timestamp in app input. `AppInvocation` has no time field, the tick surface carries no app snapshot, and the plugin cannot persist real elapsed time without a host time source. A counter of events would not be voice duration and would mislead. Needs a bounded native time/voice-session ledger. |
| `src/equicordplugins/waitForSlot/index.tsx` | **Blocked** | `app.voice` describes only the current call. There is no bounded per-channel voice roster or `userLimit` in the app snapshot, so the plugin cannot watch a full target channel. App-event output cannot present a panel or effect, so a queued notice/Apply cannot be delivered either. Needs native channel voice-state, user-limit metadata and a consent-gated local notice. |
| `src/equicordplugins/whitelistedEmojis/index.tsx` (plus `types.ts`) | **Blocked** | There is no `emoji_catalog` capability or composer emoji-suggestion filter, and no host file import/export grant. A list panel alone would not change autocomplete, so it would not implement the workflow. Needs native validated emoji catalog import/export and a composer filter. |
| `src/equicordplugins/whosWatching/index.tsx` | **Blocked** | The snapshot exposes only `streaming: bool` for the own call. `ApplicationStreamingStore.getViewerIds` has no bounded SDK equivalent. Showing current call participants as "viewers" would be false. Needs native `stream_viewer_ids` for the own stream only. |
| `src/equicordplugins/zipPreview/index.tsx` (plus `components.tsx`, `utils.ts`, `native.ts`) | **Blocked** | `message_details` exposes attachment metadata only; no host-mediated attachment archive fetch/parse, no bytes API and no ZIP dependency is permitted here. Needs the assigned bounded native `attachment_archive_preview` with host fetch, 50 MiB/1000-entry/10 MiB/256 KiB limits and traversal rejection. |
| `src/plugins/addAttachments/index.tsx` | **Blocked** | The SDK has no `message_attachment_edit` action, no upload staging grant and no message attachment PATCH. Implementing this would require a new native capability with ownership, message-type and attachment-count checks. |
| `src/plugins/alwaysAnimate/index.ts` | **Blocked** | The app snapshot exposes only `animate_gifs`. There is no policy for icons, status emojis, server banners, nameplates or role gradients. Toggling GIFs would be a different feature and a duplicate of existing local settings. Needs a native appearance animation policy and capped gradient budget. |
| `src/plugins/anonymiseFileNames/index.tsx` | **Blocked** | There is no `attachment_staging` filename-only grant, so the plugin cannot see or rename staged uploads. Needs the assigned native staging capability with 10 names / 255 UTF-8 bytes, extension preservation and no path or byte exposure. |
| `src/plugins/autoDndWhilePlaying.discordDesktop/index.ts` | **Blocked** | There is no game-state (`RUNNING_GAMES_CHANGE`) app event. `SetOwnPresence` exists, but a manual status button would duplicate `status-presets`, and automatic switching without a game event is impossible. Needs native game-activity status policy using existing detection. |
| `src/plugins/betterRoleContext/index.tsx` | **Audit only, no edit** | Already mapped to `extensions/plugins/loaded-role-members`. That package covers Copy Role Color (explicit nonzero 24-bit color, Apply-gated) and View Role Members over the loaded `member_details` snapshot. Remaining limitations are deliberate: no Edit Role navigation, no role-icon modal (no role-icon metadata in the everyday cache) and no Copy Role ID action. |

No existing workflow was renamed or re-skinned. `betterRoleContext` is not
duplicated because its safe, useful portion is already packaged and tested.

## Implemented plugin: `voice-rejoin-tools`

Upstream file that guided the subset:
`src/equicordplugins/voiceRejoin/index.tsx`.

Safe behavior:

- `observe` (`app_event`) stores only one changed, valid active channel ID.
- `show`, `save-current`, `forget` and `rejoin` are panel actions.
- `rejoin` returns one `AppAction::JoinVoice` with `ring: false`, `muted: true`
  and `deafened: false`, only after Apply.
- Corrupt storage is preserved until an explicit Save or Forget.
- A complete loaded channel directory can reject a missing or non-call
  destination; a truncated directory never claims the channel is missing.
- Bounds: one ID, 20 decimal bytes, 512-byte strict storage, one effect,
  4 KiB panel response, 128-byte sanitized labels.
- No automatic join, timer, ring, retry, microphone access, raw gateway/store
  access or network/filesystem API.

## Parent integration steps

This worktree cannot edit central files. The parent must, after review:

1. Add `"plugins/voice-rejoin-tools"` to the `members` list in
   `extensions/Cargo.toml`. Do not add a path dependency to another plugin.
2. Build the Wasm target with the pinned toolchain and lockfile, then package
   with `extensions/pack.py` into
   `extensions/plugins/packages/voice-rejoin-tools.serein-extension`.
3. Update the central catalog/pins and, if desired, the debug starter registry.
   No package bytes are committed by this worktree, and no central pin was
   hand-written.
4. Keep the manifest grants explicit: `app_events`, `voice_state`,
   `channel_directory`, `voice_connect`, `storage`. `voice_connect` is used only
   for the Apply-gated JoinVoice proposal; the user must consent to the grant.
5. Run the workspace checks, Clippy, tests and package build. This worktree ran
   only `rustfmt` on its own source and `git diff --check`.

## Explicit non-goals

- No Equicord JavaScript, Webpack/system injection, credential/session
  extraction, challenge bypass, generic network/filesystem/OAuth API or
  automatic account action.
- No claim that any package is upstream parity or proof of Discord protocol
  support.
- No bundled Wasm, screenshots, measurements, secrets or real Discord calls.

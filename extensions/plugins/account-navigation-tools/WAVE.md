# Wave notes — account & navigation tools

Task branch worktree: this plugin directory only. No central catalog,
workspace, lockfile, package bytes or other agents' directories were edited.

## Implemented upstream subset

Upstream: Equicord/Equicord `200dba124066146a8f043de7fc8b05e3dc5e68e`.

| Upstream path | Files read | Implemented subset |
| --- | --- | --- |
| `src/equicordplugins/profileSets` | `index.tsx`, `utils/profile.ts`, `utils/actions.ts`, `utils/storage.ts` | Bounded display-name/bio/pronouns presets with explicit apply/delete. Avatar/banner/decoration/theme-color/guild-profile images and import/export are omitted. |
| `src/equicordplugins/recentDMSwitcher` | `index.tsx`, `styles.css` | Bounded DM/group-DM ID history tracked from app events, previous-loaded-DM navigation and explicit clear. Global hotkey, overlay, toast, avatars and 50-entry list are omitted. |
| `src/equicordplugins/randomVoice` | `index.tsx`, `styles.css` | Stable rotation over loaded guild voice/stage channels with explicit join proposal and muted/deafened options. Camera, stream, auto-join, leave-empty, friend priority and AFK/stage filtering are omitted. |

The SDK supplies `AppAction::SetOwnProfile`, `HostEffect::Navigate` and
`AppAction::JoinVoice`; the plugin does not add raw host access.

## Audited with no edit

| Upstream path | Reason no edit |
| --- | --- |
| `src/equicordplugins/sedEnhanced` | Already packaged as `extensions/plugins/own-message-sed` with a literal own-message edit/delete workflow, 2,000-character result bound, explicit Apply and no regex. The mapped workflow is complete; re-skinning it would only inflate directory count. |
| `src/equicordplugins/rpcEditor` | `extensions/plugins/custom-rpc` already covers the safe own-presence field editor. Upstream rewrites other local applications' Rich Presence by patching Discord's `LocalActivityStore`; Serein has no safe, bounded IPC/store interception surface for that. |
| `src/equicordplugins/quickThemeSwitcher.discordDesktop` | Serein has a native Themes view and declarative theme output, but no SDK surface for enumerating local theme filenames or selecting a local/online theme. An extension could only emit a declarative palette, which is not the mapped workflow. |
| `src/equicordplugins/saveFavoriteGIFs` | No SDK exposes Discord's favorite-GIF store or a bounded file export. Upstream's HEAD checks to arbitrary saved hosts would also require a generic network API that this task forbids. |
| `src/equicordplugins/scheduledMessages` | Upstream sends later without a user present in real time. Serein extensions have no background scheduler and the task forbids automatic sends; a synthetic phantom-message UI without sending would not be the mapped workflow. |
| `src/equicordplugins/screenRecorder.equibop` | Requires `getDisplayMedia`, `MediaRecorder` and upload-prompt integration. Serein extensions expose no screen-capture or media-encoder API, and this task forbids new capture plumbing. |
| `src/equicordplugins/richPresence` | Polls third-party services and stores server URLs plus plaintext ABS/Navidrome passwords, a Jellyfin API key and optional Last.fm key. This is blocked on native OS credential-vault integration, host allowlisting and bounded polling; an extension cannot implement it safely. |
| `src/equicordplugins/orbolayBridge` | Requires a localhost WebSocket, remote command handling and outbound notification/roster payloads. Generic network and raw-voice-store access is forbidden; workspace/platform ownership would be required. |

## Required parent integration

1. Add `plugins/account-navigation-tools` to the members list in
   `extensions/Cargo.toml`. The directory uses workspace dependencies
   (`serde.workspace`, `serein-extension-sdk.workspace`).
2. Build/package centrally with the pinned toolchain and lockfile, then add the
   catalog entry and any debug-starter entry. No Wasm is pinned here.
3. Update `extensions/equicord-workflows.json` for the three implemented paths
   and record the package path plus the omissions above.
4. Run formatting, Clippy, tests, `cargo xtask check` and packaging checks.
   This worktree did not run them under the resource gate.

## Verification status

- `rustfmt` was run only on this plugin's new `src/lib.rs`.
- No `cargo test`, `cargo check`, Clippy, build, package, screenshot or
  measurement was run.
- No network, credentials, live Discord calls, microphone or screen capture
  were used.

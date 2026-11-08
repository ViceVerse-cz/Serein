# WAVE — server-info-tools

## Status

Source-only extension package. No Wasm bytes, catalog entry, package pin, root
docs, central workspace membership or `Cargo.lock` change is included in this
worktree. The parent builds and packages centrally.

Implemented workflow: a bounded, read-only subset of Equicord
`src/plugins/serverInfo`.

## Parent integration steps

1. Add `"plugins/server-info-tools"` to `extensions/Cargo.toml` `members`.
2. From `extensions/`, build with the pinned toolchain and update the lockfile
   centrally:

   ```sh
   cargo build --locked --release --target wasm32-unknown-unknown
   ```

3. Package the reviewed bytes with the shared GPL license section:

   ```sh
   python pack.py plugins/server-info-tools/manifest.json \
     target/wasm32-unknown-unknown/release/server_info_tools.wasm \
     plugins/packages/server-info-tools.serein-extension \
     --license ../licenses/Equicord-GPL-3.0.txt
   ```

4. Add the package to the central catalog/inventory and the Equicord-derived
   table in `extensions/plugins/README.md` if approved. Do not present it as
   upstream JavaScript parity or as proof of Discord interoperability.
5. Run the package/catalog checks centrally (`python test_catalog.py` or the
   repository's current equivalent), plus `cargo xtask check` and the normal
   packaging path.

No host or SDK change is required. The package uses only:
`guild_directory`, `channel_directory`, `relationships`, `storage`, and the
`panel` surface.

## Assigned workflow audit

Upstream reference commit:
`200dba124066146a8f043fde7fc8b05e3dc5e68e`.

| Upstream path | Disposition here | Evidence / reason |
| --- | --- | --- |
| `src/plugins/newGuildSettings` (`index.tsx`, `README.md`) | **Blocked, not implemented** | Upstream applies per-user guild notification overrides (mute, suppress everyone/roles, highlights, events, mobile push) plus opt-in channel and voice-name state on join. The current SDK has no per-user guild notification override action and no guild-join event. `UpdateServerSettings` would change the **server-wide** default for everyone, which is not the upstream behavior, and the folder loop is explicitly out of bounds. No safe equivalent was faked. |
| `src/plugins/openInApp` (`index.ts`, `native.ts`, `README.md`) | **Blocked, not implemented** | Serein's `OpenExternalLink` validates an HTTPS URL only (`crates/extensions/src/app.rs`). The extension API has no custom-URI scheme/host launcher and no allowlisted redirect resolver. Adding one is a native capability change outside this package's authority. |
| `src/plugins/pauseInvitesForever` (`index.tsx`, `README.md`) | **Already covered natively; no edit** | `crates/ui/src/server_invites.rs` exposes Pause/Resume, `crates/client-core/src/server_admin.rs::can_pause_guild_invites` gates it on manage-guild plus a loaded invite snapshot, and `crates/discord-api/src/server_invites.rs` performs one `PATCH` adding/removing `INVITES_DISABLED` while preserving other features and reconciling without retry. |
| `src/plugins/permissionFreeWill` (`index.ts`, `README.md`) | **Blocked, not implemented** | Upstream suppresses native permission-editor lockout/onboarding warnings. The extension SDK exposes a whole `EditChannel` overwrite proposal but no permission-editor warning surface, so an extension cannot suppress or meaningfully reimplement the guardrail. |
| `src/plugins/petpet` (`index.ts`, `README.md`) | **Blocked, not implemented** | Requires downloading/decoding a user image and generating animated GIF frames, then proposing an attachment upload. The sandbox has no network, image codec, frame-buffer or attachment-creation API. Adding those would be a large native/media capability change, not a safe port. |
| `src/plugins/pictureInPicture` (`index.tsx`, `styles.css`, `README.md`) | **Blocked, not implemented** | Requires a native video surface and OS picture-in-picture window API. The extension API has no renderer, media-window or PiP capability. |
| `src/plugins/relationshipNotifier` (`index.ts`, `functions.ts`, `settings.ts`, `types.ts`, `utils.ts`, `README.md`) | **Blocked, not implemented** | The host fires `relationships`/`channels` app events, but reactive `app_event` outputs are not presented as notices or effects; only foreground panel outputs go through the Apply/effect path. There is no notification capability for automatic local alerts. A snapshot diff panel without notifications would not close the upstream behavior, so it was not added as a re-skin. |
| `src/plugins/replaceGoogleSearch` (`index.tsx`, `README.md`) | **Already packaged; no edit** | `extensions/plugins/external-search-tools` already implements the fixed provider-allowlisted browser search URL with Apply and the existing external-link confirmation. |
| `src/plugins/serverInfo` (`index.tsx`, `GuildInfoModal.tsx`, `README.md`) | **Implemented as a loaded-only subset** | This package. Owner, joined-at, vanity, locale, verification, boosts, role catalog, mutual members and guild-membership-filtered relationship tabs are omitted because the snapshot does not contain them. See `README.md`. |
| `src/plugins/showAllMessageButtons` (`index.ts`, `README.md`) | **Blocked, not implemented** | Upstream changes the native message action bar so all utility actions appear without Shift. The extension API has no message-toolbar rendering or modifier-policy hook; the current behavior lives in `crates/ui/src/timeline.rs`. |
| `src/plugins/showConnections` (`index.tsx`, `VerifiedIcon.tsx`, `styles.css`, `README.md`) | **Already covered natively; no edit** | `crates/discord-protocol/src/profile.rs` decodes `connected_accounts` with the bounded `Small<Connection, 16>` cap, and `crates/ui/src/profiles.rs` renders connection icons and safe public profile URLs in compact and full profiles. The upstream GitHub-organization fetch is intentionally omitted, matching the inventory bounds. |

## What was not run

Per the task resource gate, this worktree did **not** run `cargo test`,
`cargo check`, `cargo clippy`, `cargo build`, `cargo xtask`, packaging,
screenshots or measurements. `rustfmt` was not run on the new source.
The inline tests in `src/lib.rs` are ready for the parent's central,
sequential validation pass.

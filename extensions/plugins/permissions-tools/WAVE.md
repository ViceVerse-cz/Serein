# Wave audit: no* / permissions / platform entries

Upstream commit: `200dba124066146a8f043fde7fc8b05e3dc5e68e`
(`/tmp/opencode/Equicord-upstream/src/`). Canonical mapping:
`extensions/equicord-workflows.json`.

This wave added one package, `permissions-tools`, for the only assigned workflow
whose safe subset is reachable through the existing bounded SDK. The remaining
entries are either already covered natively or need a native host surface that an
extension cannot provide. No central file, catalog pin or package byte was
changed here.

## Implemented

| Upstream path | Result |
| --- | --- |
| `src/plugins/permissionsViewer` | Partial, read-only port in `permissions-tools`. Shows the account's loaded channel permission decisions (`Allowed`/`Denied`/`Unknown`) and the loaded role order of one loaded member. Upstream files used: `index.tsx`, `utils.ts`, `components/UserPermissions.tsx`, `components/RolesAndUsersPermissions.tsx`. Omitted: permission bit masks, other-member effective permissions, overwrite allow/deny tables, member/role fetching, context menus, popouts, copy-ID and any edit. Rationale: the host exposes channel decisions only for the current account and no role permission bits; adding them would need a new native snapshot, not raw store access. |

## Already covered natively — no edit

| Upstream path | Native evidence | Decision |
| --- | --- | --- |
| `src/plugins/noProfileThemes` | `crates/model/src/profile_preferences.rs` (`hide_other_themes`), applied in `crates/ui/src/profiles.rs` (`hide_other_themes` filter), switch in `crates/ui/src/settings.rs`, regression test `hiding_other_profile_themes_keeps_own_theme_and_loaded_data`. | Already hides other loaded profile themes while keeping the account theme. Do not re-skin. |
| `src/plugins/onePingPerDM` | `crates/client-core/src/notifications.rs` (`one_ping_per_dm`, `MAX_ONE_PING_CHANNELS = 256`, read reset), `crates/model/src/notification_tools.rs`, switch in `crates/ui/src/notification_settings.rs`, test `one_ping_dm_alerts_once_until_read_and_never_replaces_unread_counts`. | Already implements one alert per unread DM run with 256-channel volatile state. Do not duplicate. |
| `src/plugins/platformIndicators` | `crates/model/src/profile_preferences.rs` (`platform_indicators`), `crates/ui/src/profiles.rs` (`platform_indicators`, cached client flags), rendering in `crates/ui/src/lib.rs` and `crates/ui/src/profiles.rs`, test `platform_indicators_draw_only_cached_true_flags`. | Already renders cached desktop/mobile/web/VR booleans in profiles and member rows. Limitation retained deliberately: no per-platform status strings or probing. Do not duplicate. |

## Blocked in the extension sandbox

These upstream behaviors are native input or rendering paths. The SDK exposes no
effect, snapshot or hook that could implement them without raw UI/store access or
a new host capability. They are reported rather than faked.

| Upstream path | Why no safe extension subset | Needed owner decision |
| --- | --- | --- |
| `src/plugins/noMiddleClickPaste` | Suppresses Linux primary-selection paste at the native pointer/event layer. Extensions cannot intercept egui mouse events or clipboard selection. | Native pointer-handling change if desired; no clipboard read is acceptable. |
| `src/plugins/noPendingCount` | Hides native friend-request, message-request and premium-offer badge counts while keeping state. Extensions cannot alter native badge rendering and receive no pending-count snapshot. | Native local display preference if desired; must not resolve requests or change counts. |
| `src/plugins/noReplyMention` | Upstream changes the reply pre-send default with user/role allow lists and shift inversion. Serein has a per-reply mention switch and a reviewed `SendReply { mention }` effect, but no default preference or pre-send hook. Implementing a default needs native composer state; a plugin must not send or mutate drafts automatically. | Native reply-default preference, or leave the per-reply switch as the current behavior. |
| `src/plugins/noServerEmojis` | Filters the native emoji autocomplete. The SDK exposes no emoji catalog or autocomplete filter, and filtering sent text would not match the workflow. | Native autocomplete filter preference if desired. |
| `src/plugins/noUnblockToJump` | Requires changing the native blocked/ignored message navigation guard. Block state is not exposed to extensions, and native quick navigation currently excludes blocked/ignored authors (`crates/ui/src/lib.rs`). | Native navigation-guard decision; must remain read-only with no unblock write. |
| `src/plugins/overrideForumDefaults` | Overrides the native forum default layout/sort while retaining per-channel choices. There is no SDK effect or local-settings field for forum defaults. | Native local forum-default preference if desired; server settings must not be written. |
| `src/plugins/plainFolderIcon` | Hides the small guild icons inside native folder tiles. Themes only change palette tokens; extensions cannot restyle or replace folder-tile rendering. | Native folder-tile rendering toggle if desired. |

## Parent integration

1. Add `"plugins/permissions-tools"` to the `members` list in
   `extensions/Cargo.toml` (central file intentionally not edited here).
2. Build from `extensions/` with the pinned toolchain and lockfile:

   ```sh
   cargo build --locked --release --target wasm32-unknown-unknown -p permissions-tools
   python pack.py plugins/permissions-tools/manifest.json \
     target/wasm32-unknown-unknown/release/permissions_tools.wasm \
     plugins/packages/permissions-tools.serein-extension
   ```

3. Add the reviewed catalog entry and any generated pin centrally.
4. No new SDK capability or host code is required: `channel_metadata` and
   `member_details` already exist in API version 1 and are granted per manifest.

## Verification not performed here

The wave resource gate forbids `cargo test/check/clippy/build/package`, `rustc`,
screenshots and full checks. `rustfmt` was applied only to this package's
`src/lib.rs`. Tests exist in `src/lib.rs` but were not executed; no Wasm was built
or committed. Nothing here is evidence of Discord interoperability or upstream
JavaScript parity.

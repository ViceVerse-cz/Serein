# WAVE — Equicord community workflow tranche

Worktree: `/home/nicolas/.t3/worktrees/Serein/t3-1519b748`
Upstream commit: `200dba124066146a8f043fde7fc8b05e3dc5e68e`
Upstream checkout read-only at `/tmp/opencode/Equicord-upstream/src/`.

## Implemented package

`extensions/plugins/equicord-community-tools/` — one reviewed plugin package that
closes the safe, independently useful subset of four assigned workflows:

| Upstream path | Implemented subset | Caps |
| --- | --- | --- |
| `src/equicordplugins/exportMessages` | selected-message text to clipboard, 4 KiB | `selected_message`, `clipboard_write` |
| `src/equicordplugins/hopOn` | stored literal triggers + one HTTPS link, manual selected-message check, Apply-reviewed open | `selected_message`, `storage`, `external_navigation` |
| `src/equicordplugins/fastDeleteChannels` | current server channel/thread delete proposal, native destructive Apply and permission recheck | `app_context`, `channel_control` |
| `src/equicordplugins/ignoreCalls` | local DM/GDM ignore list, explicit Apply-reviewed `DeclineCall` | `app_context`, `storage`, `voice_connect` |

No package bytes, catalog entry or workspace membership were generated here. The
parent builds and packages centrally after review.

## Parent integration steps

1. Add `"plugins/equicord-community-tools"` to the `members` list in
   `extensions/Cargo.toml`. Do not add a hand-written lockfile entry; let the
   pinned workspace resolve it.
2. Build the Wasm with the pinned toolchain and lockfile, then package it from
   `extensions/` with the reviewed Equicord license, matching the existing
   community packages:
   ```sh
   cargo build --locked --release --target wasm32-unknown-unknown -p equicord-community-tools
   python pack.py plugins/equicord-community-tools/manifest.json \
     target/wasm32-unknown-unknown/release/equicord_community_tools.wasm \
     plugins/packages/equicord-community-tools.serein-extension \
     --license ../licenses/Equicord-GPL-3.0.txt
   ```
   `pack.py` appends the license file as an inert `serein.license` custom
   section.
3. Update `extensions/catalog.json` only via the normal pin workflow, never by
   hand. Add the package to the catalog source list if the parent keeps one.
4. Update `extensions/plugins/README.md`, `extensions/equicord-workflows.json`
   implementation fields and any generated coverage table for these four
   workflows. Use the package limitations stated below, not the upstream feature
   names as parity claims.
5. Optional: add a preview PNG through the normal synthetic/offline process. Not
   required for this tranche.
6. No host, SDK, or native capability change is required for the implemented
   subset. If a later tranche wants automatic HopOn or ringing suppression, that
   needs a separate native design: message-event effects are deliberately
   rejected, and `VoiceSnapshot` does not expose incoming-call identity.

## Required limitation text for the inventory

- **exportMessages:** one selected message, text only, 4 KiB clipboard; no file
  save, attachments, embeds or contact export.
- **hopOn:** up to eight literal case-insensitive triggers and one public HTTPS
  link; manual selected-message check; each open still needs Apply. No regex,
  custom launcher scheme, automatic launch or message-event effect.
- **fastDeleteChannels:** one current loaded server channel or thread per Apply;
  native permission and destructive confirmation; no keybind, row icon or bulk
  delete.
- **ignoreCalls:** local list of at most 16 DM/GDM IDs; one explicit
  `DeclineCall` per Apply; no automatic ringing suppression or `CALL_UPDATE`
  rewrite.

## Audited entries with no new package

| Upstream path | Decision |
| --- | --- |
| `src/equicordplugins/customStatusTimeouts` | Already covered by the packaged `status-presets` package (`StatusPresets`, `CustomStatusTimeouts`). It uses bounded `set_own_presence` with expiry <= 86,400 seconds. Do not duplicate. |
| `src/equicordplugins/declutter` | No edit. The safe subset (zoom, sidebar/member list, GIFs, media links, smooth scrolling, scroll speed) is already native in Serein settings. Upstream-specific removal of profile effects, nameplates, boosts, shop/billing/gift UI and tooltips needs Discord UI patches, which this client does not expose. |
| `src/equicordplugins/disableCameras` | No edit. Serein already has a native camera toggle and per-call `camera_control`; an extension button would duplicate it. Upstream forces `self_video` off before join via a patch and cannot be truthfully reproduced as a persistent default. |
| `src/equicordplugins/equicordToolbox` | No edit. Serein's Extensions panel is already native. A header-bar popout/plugin menu would need a host header-bar API; the safe navigation subset is already covered by the command palette. |
| `src/equicordplugins/keyboardNavigation` | Already covered by the reviewed `command-palette` package: bounded loaded-channel filtering, fixed native views, search and profile navigation with no global hotkey or arbitrary commands. No duplicate. |

## Explicit blockers

| Upstream path | Blocker |
| --- | --- |
| `src/equicordplugins/followVoiceUser` | The bounded SDK snapshot exposes participants of the active call only. It does not expose another user's current voice channel, and upstream's automatic follow/leave loop is forbidden. A truthful implementation needs a new bounded, consent-gated voice-state snapshot plus per-join Apply; not added here. |
| `src/equicordplugins/friendCodes` | Serein exposes no bounded friend-invite list/create/revoke action. Implementing it would require a new native REST surface, credential-safe error handling, explicit Apply per operation and a panel-local bounded list. Do not fake it with generic network or relationship APIs. |
| `src/equicordplugins/hopOn` (automatic mode) | `Output::validate` rejects effects on `message_event` and `app_event` surfaces. The automatic upstream MESSAGE_CREATE path therefore cannot open any URL safely. The manual selected-message subset above is implemented instead. |
| `src/equicordplugins/ignoreCalls` (automatic mode) | `VoiceSnapshot` reports the active call, not the incoming ringing call or caller. Automatic suppression would require rewriting Discord ringing state; forbidden. The local list + explicit decline subset above is implemented instead. |

## Tests and checks

- Inline tests in `src/lib.rs` cover export bounds/selection, trigger matching,
  HTTPS validation, storage merge/limits, delete-kind rejection, ignore-list
  bounds, decline gating and corrupt-storage preservation.
- Not run in this sub-agent because of the resource gate: `cargo test`,
  `cargo check`, `cargo clippy`, `cargo build`, `cargo xtask check`,
  `cargo xtask package`, screenshots and measurements.
- `rustfmt --edition 2024` was run only on the new `src/lib.rs`.
- Parent must run the ordinary checks, package build and catalog validation
  sequentially.

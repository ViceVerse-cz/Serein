# Loaded Role Members

A safe, bounded Serein panel that lists **already-loaded** members who hold a
chosen role. It never requests a guild-wide member list, never searches members,
and never shows a server-wide total.

## Provenance

This plugin is a Serein port of two Equicord plugins at commit
`200dba124066146a8f043fde7fc8b05e3dc5e68e`:

- [`src/equicordplugins/inRole`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/equicordplugins/inRole)
  by nin0dev — filtering members by role and the role-member panel.
- [`src/equicordplugins/clickableRoles`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/equicordplugins/clickableRoles)
  by prism and Vendicated contributors — the per-role member list and profile
  opening behavior.
- [`src/plugins/betterRoleContext`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/plugins/betterRoleContext)
  by Vendicated, goodbee, nightmaresan and contributors — the **Copy Role
  Color** context action.
- [`src/plugins/betterRoleDot`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/plugins/betterRoleDot)
  by Vendicated, AutumnVN and contributors — click-to-copy of a role color.

All upstream plugins are licensed **GPL-3.0-or-later**. This port is distributed
under the same license. See the header in `src/lib.rs` and
<https://www.gnu.org/licenses/gpl-3.0.html>.

## Supported flow

The plugin declares four panel actions and no event, message or activation
handler:

1. **`show`** opens the panel. It reads the host-granted `member_details`
   snapshot and renders a typed `Select` populated only from the loaded role
   catalog in that snapshot.
2. Choose a role and press **Use this loaded role** (`select-role`). The action
   receives the selected option through the ordinary panel `values` map. The
   option is matched back to its catalog entry; the resulting role ID is
   validated as a nonzero decimal ID and saved as
   `{"role_id":"<id>"}` in plugin storage. The same response lists matching
   loaded members.
3. Choose one of the matching loaded members and press **Propose opening
   selected profile** (`open-profile`). The action re-reads the saved role,
   revalidates it against the current snapshot, and returns exactly one
   `navigation.open_profile` host effect.
4. With a role selected in the same typed `Select`, press **Copy selected role
   color** (`copy-role-color`). The action matches the option back to the
   current loaded role catalog, validates the role ID and its 24-bit color, and
   returns at most one `clipboard_write` `HostEffect::CopyText` containing the
   canonical `#rrggbb` value. It does not read or write plugin storage.

**Every profile navigation and clipboard replacement requires Apply.** The panel
only proposes `HostEffect::OpenProfile` or `HostEffect::CopyText`; Serein
displays the proposal and performs it only after the user chooses Apply. The
plugin never opens a profile or changes the clipboard directly.

The two-step panel + storage shape is used because a profile proposal is a
separate invocation from the role selection. Storage keeps the chosen role
available for that later invocation without accepting a free-form role ID. A
fresh panel can be opened at any time; the saved role is used only when it is
still present in the current loaded role catalog or in a loaded member's role
list.

## Input contract

- Role choice comes only from the typed `Select` built from
  `MemberDetailsSnapshot.roles`. There is no text input for raw role IDs, no
  arbitrary member ID field, and no second member store.
- Member choice comes only from a `Select` built from the currently matching
  loaded members. The selected option is matched back to that member's host ID.
- Profile proposals are rejected when the saved role is missing, corrupt, stale,
  or no longer loaded, and when the selected member is not currently matching.
- Color copying uses only the role selected from the current typed `Select`. A
  stale, unknown, partial or unavailable catalog selection is rejected with a
  bounded panel error and no effect or storage write.
- A role color of `0` means "no explicit color" in Discord. It is never rendered
  as `#000000`, never treated as black, and never presented as role absence.
  Colors outside the 24-bit RGB range are rejected rather than guessed.

## Bounds and safety

- At most **20** member rows, matching the host's `member_details` item cap.
- At most **32** role IDs inspected per member.
- At most **32** role options, matching the host's role catalog cap.
- The complete serialized response is kept at or below **6 KiB**, stricter than
  the 256 KiB ABI limit. Oversized views fall back to a short notice.
- A copied role color is exactly seven ASCII bytes (`#rrggbb`), far below the
  4,096-byte clipboard limit. At most one `CopyText` effect is returned.
- Element text is UTF-8-safe and truncated only at character boundaries; select
  options are sanitized, capped at 128 bytes and kept unique.
- Matching members are deduplicated by user ID and sorted deterministically by
  display label, then user ID.
- No guild-wide totals, no "and N more", no member fetch, and no implicit
  `requestMembersById`/REST call. Absence from the loaded pane is never presented
  as absence from the server.
- Partial snapshots are labeled: `member_details.truncated`,
  `member_details.roles_truncated`, and each member's `roles_truncated` are
  reported separately.

## Storage and corrupt state

Storage contains only a strict object with a single `role_id` string. Malformed
JSON, unknown fields, missing fields, or an invalid/non-decimal role ID are all
treated as corrupt. Corrupt storage is **preserved** while the panel explains the
problem. It is replaced only by an explicit **Use this loaded role** action whose
selected option exists in the current loaded role catalog.

`copy-role-color` never reads the saved role and never writes storage. Corrupt
storage therefore cannot block a color copy, and a color copy cannot repair or
overwrite it.

## Deviations from upstream

Upstream `inRole` reads `GuildMemberStore.getMembers(guildId)` and upstream
`clickableRoles` calls the REST role-member-ID endpoint (and
`requestMembersById`), then displays guild-wide totals and an overflow count.
This port intentionally differs:

- It is a **loaded-only subset** view. It filters only the bounded
  `member_details` pane the host already collected for the selected channel.
- It never calls a member store, REST endpoint, gateway request, or member fetch.
- It does not display a guild-wide member count or claim completeness.
- It does not add context-menu commands, slash commands, developer-mode changes,
  popouts, React patches, or raw Discord objects.
- Upstream `betterRoleContext` and `betterRoleDot` copy `role.colorString` from
  a context menu or role-dot click. This port instead copies the color of the
  role chosen in the panel's typed `Select`, after Apply, and only when the
  loaded catalog supplies an explicit nonzero 24-bit color.
- It uses only the current typed SDK: `member_details`, `storage`,
  `navigation.open_profile`, and `clipboard_write` (`copy_text`).

## Files

- `Cargo.toml` — standalone crate using the local `extensions/sdk` path.
- `manifest.json` — capabilities `member_details`, `storage`, `navigation`,
  `clipboard_write` and the `show`, `select-role`, `open-profile`,
  `copy-role-color` panel actions.
- `src/lib.rs` — handler, bounds, and focused tests.
- `README.md` — this document.

Run the focused tests with:

```sh
cargo test --manifest-path extensions/plugins/loaded-role-members/Cargo.toml
```

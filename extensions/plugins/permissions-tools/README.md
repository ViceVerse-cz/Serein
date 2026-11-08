# Loaded Permissions Viewer

A read-only, bounded panel that summarizes **already-loaded** data related to the
upstream PermissionsViewer workflow. It never fetches members, roles or
overwrites, never evaluates Discord internals, and never writes account state.

## Provenance

This package adapts the safe display subset of Equicord's
[`src/plugins/permissionsViewer`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/plugins/permissionsViewer)
at commit `200dba124066146a8f043fde7fc8b05e3dc5e68e`, licensed
GPL-3.0-or-later. The upstream files that guided this subset are:

- `index.tsx` — the user/channel/server permission viewer and role sorting;
- `utils.ts` — `getSortedRolesForMember`, role ordering by position and the
  highest/lowest sort setting;
- `components/UserPermissions.tsx` — the per-role permission breakdown;
- `components/RolesAndUsersPermissions.tsx` — the role list and overwrite table.

This is **not** a copy of the upstream React UI, context-menu patches, Webpack
lookups, Popout/Dialog components or `GuildMemberStore`/`GuildRoleStore` access.
It is an independent Rust adaptation against Serein's unchanged sandbox API, and
it is not evidence that any Discord account or server was exercised.

## What the panel shows

The plugin declares only two panel actions and no event, message, composer or
activation handler.

1. **`show`** opens the panel.
   - **Channel decisions**: the host-granted `channel_metadata.permissions` map
     for the current channel, grouped into `Allowed`, `Denied` and `Unknown`.
     A missing decision stays unknown; it is never rendered as denied or allowed.
   - **Loaded member roles**: a typed `Select` built only from the loaded
     `member_details` pane (at most 20 members). The selected member's role IDs
     are intersected with the loaded role catalog and shown highest position
     first. Role IDs not in the catalog are counted, never invented.
2. **`select-member`** revalidates the chosen option against the current snapshot
   and redraws the role order. An option that is no longer loaded is rejected
   with a fixed notice; no free-form ID is accepted.

The panel is explicit that the channel section contains **the account's own
loaded decisions**, not another member's effective permissions, and that a
decision does not guarantee an action can run.

## Deliberate omissions

These upstream capabilities are not implemented because the host snapshots do
not expose the required data, or because reproducing them would require
forbidden raw-store/Webpack access:

- per-role and per-overwrite **permission bits** (`allow`/`deny` masks);
- another member's **effective permissions**, including owner/admin resolution;
- server-wide role lists, role creation order beyond the loaded catalog, and
  member fetching for missing overwrite targets;
- the context-menu entries, role popout button, safety icon and Discord Dialog;
- copy-ID actions and any permission **editing** or account write.

Adding any of those requires a new narrowly-scoped host snapshot (for example a
bounded member-permission decision summary) rather than direct Discord access.

## Bounds and safety

- Panels are capped by the host at 64 elements and 4,096 bytes per text run.
- Every text run is truncated on a UTF-8 boundary with an ellipsis.
- The complete response is kept at or below **6 KiB**. If a full batch would
  exceed it, the panel is replaced with one fixed message and no partial rows.
- Member options are limited to 20 loaded members, 128 bytes per option, and are
  unique per member ID.
- Role rows come only from the loaded role catalog and are sanitized of control
  characters. Zero or out-of-range role colors are omitted, never guessed.
- Capabilities requested: `channel_metadata`, `member_details`. There is no
  `storage`, network, clipboard, navigation, account, message or event access.
- The plugin emits no host effects, so there is nothing to apply.

## Tests

Focused unit tests in `src/lib.rs` cover:

- allowed/denied/unknown grouping and stable ordering;
- loaded-catalog-only role resolution, descending position order, color display
  and unknown role-ID counting;
- rejection of a stale member option;
- control-character sanitization and explicit unavailable-snapshot messaging;
- unknown-action and event no-op behavior;
- full-batch response, panel, text and select limits, plus the 6 KiB fallback.

Tests were not executed in this worktree because the wave resource gate reserves
compilation and test execution for the parent. No Wasm package is committed
here; the parent builds and packages centrally.

# Wave integration notes

This package implements only the safe inspection subset of two assigned
workflows. It uses existing SDK capabilities and needs no new host ABI.

## Parent steps

1. Add `extensions/plugins/message-inspector-tools` to the `extensions`
   workspace members and run the central catalog/package tooling. Do not hand
   edit packaged Wasm or generated pins.
2. Confirm the host supplies `app.member_details` to **message** actions for
   manifests requesting `member_details`. The snapshot is already produced by
   `apps/desktop/src/extension_member_details.rs`; no new grant is needed.
3. When updating `extensions/equicord-workflows.json`, record both entries as
   partial:
   - `showRolesInChat`: `message-inspector-tools` role inspection only; inline
     badge, bot toggle, role color/icon in chat and context-menu toggles are
     omitted.
   - `richMagnetLinks`: `message-inspector-tools` magnet parsing/copy only;
     inline link rendering and the Discord tokenizer patch are omitted.
4. The native inline work remains unimplemented and must not be described as
   complete:
   - a timeline author decoration that resolves the highest role from loaded
     guild member/role state and paints it next to the author name;
   - a markdown/message tokenizer rule that renders `magnet:?` links as local
     pills without fetching or opening them.
5. Add the two upstream source directories and the pinned commit to the
   central third-party notice/license inventory. The package source header
   already retains the GPL-3.0-or-later notices.
6. Re-run the normal extension checks and package build centrally. No live
   Discord, network or credential evidence is involved.

## Assigned-entry blockers outside this package

These were audited and cannot be implemented truthfully through this package's
scope:

- `reactionTimestamps`: the ABI exposes reaction summaries but no add/remove
  event payload or reaction timestamp. Needs a narrow native event store
  (bounded 2,000 entries, cleared on stop) and native reaction-popout rendering.
- `replyPingControl`: notification mentions are native state. No SDK hook can
  add/remove the current user from `message.mentions` before notification
  evaluation.
- `searchFix`: search offset/sort state lives in native `client-core` and
  `discord-api`; an extension cannot rewrite paging or flip sort order safely.
- `serverSearch`: the core search/navigation behavior is already approximated by
  the existing `command-palette` package over loaded guild/channel snapshots.
  A server-rail search button is native presentation only.
- `showResourceChannels`: native channel-list filtering.
- `quoter`: requires local image composition, Google Fonts ingestion, Discord
  CDN avatar access and upload. Forbidden network/media/upload behavior for this
  task.
- `remix`: requires fetching CDN media, a canvas editor and upload. Forbidden
  network/media/upload behavior for this task.

`polishWording` and `repeatMessages` were already mapped before this wave.
`polishWording` needs no edit. `repeatMessages` now also has an Apply-reviewed
`repeat-reply` action in `social-link-tools`; attachments, stickers and
alternate-channel behavior remain omitted.

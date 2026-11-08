# WAVE notes — Linked Message Tools

Assigned upstream path: `src/plugins/messageLinkEmbeds`.

Implemented safe subset: a selected-message action that previews message links already present
in the granted loaded timeline and proposes one explicit `jump_to_message` navigation.

## Parent steps

- Add `plugins/linked-message-tools` to `extensions/Cargo.toml` workspace members.
- Build and package centrally; do not commit hand-built Wasm in this branch.
- Add the packaged entry and implementation note to `extensions/catalog.json` and
  `extensions/equicord-workflows.json` in the parent integration commit.
- The host must keep supplying `app.timeline` for granted `timeline` actions and must keep
  clearing `selected_message`/`selected_message_ref` when a panel button re-invokes an action.
  The `jump` action revalidates the target against that same snapshot.

## Deliberate limits

- No linked-message fetch, cache, permission lookup for other channels, inline accessory,
  React/Webpack injection, media rendering or DOM patch.
- Only the selected message's text is scanned; nesting is not followed.

## Assigned workflows audited elsewhere in this wave

The following assigned upstream paths were inspected and are **not** implemented as plugin
packages because the current SDK/host cannot express them safely:

| Upstream path | Blocker |
| --- | --- |
| `src/plugins/betterSearch` | Reduced panel implemented separately as `better-search-tools`; native filter-menu/autocomplete patching is not available and server support for the extra tokens is unverified. |
| `src/plugins/blurNsfw` | Requires a native media render/effects surface and per-media visibility. The SDK exposes no media URLs, pixels or blur drawing. |
| `src/plugins/copyFileContents` | `message_details` exposes attachment metadata only; no attachment bytes, URL or host `copy_attachment_text` capability exists. |
| `src/plugins/friendInvites` | No SDK friend-invite create/list/revoke action; the upstream endpoint is restricted/legacy and untested live. |
| `src/plugins/fullSearchContext` | Requires a host search-result message context-menu surface. Extensions cannot add or open that surface; existing message actions already cover loaded timeline rows. |
| `src/plugins/gifProviderSwitcher` | Provider selection and share registration are native GIF-picker behavior; the SDK exposes only `RequestGifs` with a query. |
| `src/plugins/greetStickerPicker` | Reduced sticker re-send implemented separately as `greet-sticker-tools`; the fixed welcome-sticker catalog and greet-mode/multi-greet state are not exposed. |
| `src/plugins/implicitRelationships` | Requires the unofficial affinity store and guild-member chunking; no affinity read or relationship overlay exists in the SDK. |
| `src/plugins/messageLogger` | Already packaged as `message-history`; no edit needed here, and no deleted-before-load recovery is safe without a new history capability. |
| `src/plugins/musicRichPresence` | Requires third-party Last.fm/ListenBrainz polling and credentials; generic network access is forbidden and a manual presence editor would duplicate the existing Custom RPC preview. |

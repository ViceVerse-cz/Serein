# Wave report: alwaysExpandProfiles → iconViewer

Upstream reference: `Equicord/Equicord@200dba124066146a8f043de7fc8b05e3dc5e68e`,
read from `src/equicordplugins/`. This report covers the eleven upstream paths in
this wave and records what was implemented, what was already covered, and what
remains a host-side or blocked item.

## Implemented in this wave

`baseDecoder` → `extensions/plugins/text-codecs` (message action added).

The existing package already ported DecodeBase64's decoding rules as a composer
replacement. Its canonical mapping targeted a **message action** with
`selected_message` and `clipboard_write`, which was missing. This wave adds:

- action `decode-message` on the `message` surface;
- capabilities `selected_message` and `clipboard_write` (alongside `composer`);
- one reviewed `copy_text` effect instead of upstream's immediate per-string
  clipboard writes.

Bounds: at most 8 decoded matches, 4,096 UTF-8 bytes per match, 1,024 displayed
characters per match, 6,144 UTF-8 bytes and 8,000 serialized bytes for the
reviewed effect. Only the host-provided selected message text is inspected; no
history fetch, attachment/embed access, storage or clipboard read.

Omission relative to upstream: Discords' React modal and one copy button per
decoded string. Serein permits one reviewed effect per result, so the panel shows
the bounded matches and Apply copies them as one blank-line-separated block,
naming how many matches it contains.

Sources read: `baseDecoder/index.tsx`, `baseDecoder/README.md`.

## Already covered — no edit

| Upstream path | Current Serein implementation | Audit |
| --- | --- | --- |
| `alwaysExpandProfiles` | Native profile layout in `crates/ui/src/profiles.rs` (inventory: `native`). | Upstream only opens the full profile modal instead of the popout; Serein's optional full layout already covers the exposed behavior. No extension surface would improve it. |
| `blockKeywords` | `extensions/plugins/message-composition-tools` composer keyword filter (inventory: `packaged`). | The safe subset is literal, bounded draft filtering. Upstream blocks incoming messages, unread counts and notification handling by patching the message store; Serein exposes no host surface for that, and regex is deliberately omitted. No truthful extension could add it. |
| `channelBadges` | Native channel-row badges (inventory: `native`). | Loaded native kind badges only. NSFW/locked/rules metadata is not exposed to Serein and is not guessed. No edit. |
| `cleanChannelName` | Native display normalization in `crates/model/src/channel_display.rs` and channel UI (inventory: `native`). | Bounded display-only normalization with ignore memory and edit-time original; no extension surface exists for the channel-list renderer. No edit. |
| `copyUserMention` | `extensions/plugins/identity-tools` action `copy-user-mention` (inventory: `packaged`). | Copies `<@author_id>` from the selected message reference after Apply, with canonical ID validation and no clipboard read. Equivalent safe behavior. |
| `findReply` | `extensions/plugins/message-reference-tools` action `find-reply` (inventory: `packaged`). | Inspects only loaded timeline/details rows, proposes one Apply-reviewed jump, never fetches history. Upstream's optional ping/author widening is not included; that is a scope choice, not a broken mapped behavior. |
| `googleThat` | `extensions/plugins/message-reference-tools` action `search` (inventory: `packaged`). | Builds one fixed-host HTTPS Google URL and proposes opening it. Upstream builds a message to send and supports more engines; Serein deliberately opens a reviewed link instead of sending. |
| `husk` | `extensions/plugins/message-reference-tools` action `husk` (inventory: `packaged`). | Proposes `add: true` for the fixed Husk custom emoji on the loaded message; no toggle and no same-server emoji lookup because no emoji store is exposed. |

Sources read for this table: `alwaysExpandProfiles/index.ts`,
`blockKeywords/index.tsx`, `channelBadges/index.tsx`, `cleanChannelName/index.tsx`,
`copyUserMention/index.tsx`, `findReply/index.tsx`, `googleThat/index.tsx`,
`husk/index.tsx` (plus the corresponding README files).

## Blockers and parent/host steps

| Upstream path | Status | Reason and required host work |
| --- | --- | --- |
| `betterBackreading` | **Blocked in this wave** | Upstream patches Discord's scroll manager, message trimming, anchor restoration, image preload distance and per-channel fetch count. None of that is reachable from a sandboxed extension; it needs native timeline/scroll work in `crates/ui` and bounded reading preferences. It is a native host item, not a package. |
| `iconViewer` | **Blocked in this wave** | Upstream browses host Webpack icon modules and saves icon files. Serein exposes no host icon module registry or file-save API to extensions, and inventing one would be a broad native developer surface. It remains a native developer-tool design item, not a package. |
| `betterBackreading`, `iconViewer` | Not started | No partial or fake implementation was added. They stay unclaimed rather than being marked done. |

## Parent integration requirements

1. Rebuild and repackage `text-codecs` centrally (`cargo xtask package` or the
   repository's packaging path). The checked-in
   `extensions/plugins/packages/text-codecs.serein-extension` predates this
   source change, so it does not yet contain the `decode-message` action.
2. Re-pin the catalog entry for `text-codecs` after packaging. The source
   `manifest.json` now declares `composer`, `selected_message` and
   `clipboard_write` and three actions; the existing packaged Wasm still has the
   old manifest.
3. Update `extensions/equicord-workflows.json` for `baseDecoder` if the parent
   wants its limitation text to mention the message action. This wave does not
   edit the inventory file.
4. Verify the preview host grants `selected_message` and `clipboard_write` on the
   `message` surface; the action intentionally returns `AppOutput` with at most
   one `copy_text` effect.

No packaged Wasm, catalog pin, central `Cargo.toml`/`Cargo.lock`, debug starter
registry, root docs or other agents' packages were modified.

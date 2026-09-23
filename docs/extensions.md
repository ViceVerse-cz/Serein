# Community extensions

Serein extensions are local, opt-in tools for the native client. The Extensions
and Themes pages in Settings contain packages, links to their source, their
requested capabilities and their review status. A plugin cannot directly call Discord, read credentials, open files or make
network requests. Separately granted foreground actions can propose messages
and other app operations; the user reviews and applies each operation in Serein.

## Install and remove

Themes and plugins are published together in
[Serein-extensions](https://github.com/ViceVerse-cz/Serein-extensions).
Opening Settings > Themes or Extensions checks that repository's shared catalog
on the existing worker. Normal builds embed no package payloads; bundled examples
remain available only in offline demo/test builds.
The last valid catalog and installed packages remain available offline. Refresh
retries immediately. Catalog changes add/remove available choices and mark
installed updates; they never install, update or delete packages automatically.
A refresh shows a spinner without disabling local controls; choosing a local action
cancels a pending catalog/thumbnail request.
Updating an inactive theme preserves the current selection. Locally created or
imported packages are never replaced by catalog updates.

Enable downloads the selected, hash-pinned package after its capabilities have
been accepted. Updates are manual and require renewed capability consent.
Import selects a local `.serein-extension` JSON package. An import is unreviewed;
importing alone does not grant it capabilities or execute it.

Disable stops accepting results immediately, then removes Serein's downloaded
package, temporary files and extension data. A failure to remove files is shown
and cleanup is retried on the next load. Re-enabling requires downloading or
importing the package again and starts with fresh extension settings. Serein
never deletes the creator's Git repository, the user's imported original, or an
exported theme/image source.

Plugin grants and data belong to the signed-in account. Logout invalidates
plugin results, drains bounded in-flight work and clears that account's extension data. Theme selection is a device
preference. There is no periodic background polling or automatic package update.

## Creator workflow

1. Keep source and license in a public Git repository. Use the standalone Rust
   example under `examples/extensions/message-delete-protector` and its small SDK.
   The [SDK authoring guide](../examples/extensions/README.md#test-and-develop-locally)
   covers native handler tests, typed panel values and JSON storage helpers; the v1
   exports and existing plugin source remain compatible. For reactive plugins,
   use [Message Counter](../examples/extensions/message-counter/src/lib.rs) and the
   [message-event guide](../examples/extensions/README.md#reactive-message-plugins).
   [App Toolbox](../examples/extensions/app-toolbox/src/lib.rs) demonstrates app
   snapshots and user-confirmed host actions.
2. Build a Wasm module implementing the version 1 ABI documented by the starter.
   No native binary, installer, Git hook or build script runs on an end user's
   computer. Other languages can implement the same Wasm buffer/JSON contract.
3. Package the manifest and Wasm bytes (or declarative theme) as a single JSON
   file. Test through Import with an offline `--demo` build first.
4. Add the package and reproducible source/build instructions to `Serein-extensions`.
   Commit the package first, then regenerate that repository's `catalog.json` with
   the package commit. Its publishing script records immutable package URLs,
   byte lengths and SHA-256 hashes. See the repository README for exact commands.
5. Maintainers review each listed version, its capabilities and the source to
   artifact relationship. A catalog checksum identifies reviewed bytes; it is
   not a signature or a guarantee that code is harmless. Updates need review too.

The in-app catalog reads `Serein-extensions/main/catalog.json`. A new catalog entry
is not available through that endpoint until it reaches that repository's main branch. Empty catalogs
are valid; imports allow development before a release is listed.

## Shop previews

Catalog entries may include a short `description` (at most 256 characters and
1,024 UTF-8 bytes, without control characters) and a `preview` object:

```json
{
  "preview": {
    "url": "https://example.org/releases/v1/preview.png",
    "sha256": "<64 hexadecimal SHA-256 digits>",
    "download_bytes": 12345
  }
}
```

Use an original or licensed PNG/JPEG screenshot showing the theme or plugin in
use. Pin its URL to an immutable release or source commit, then record the exact
file size and digest. Prefer a 16:9 image; the shop preserves its aspect ratio.
Offline demo theme fixtures show palette thumbnails using their actual colors. Clicking a demo
or installed theme thumbnail previews its full appearance in the normal app, with
Back to themes and Customize actions. Demo themes can be customized without installing
them first; saving creates an editable local copy and preserves the original package.
Remote themes download only when installed; installed themes can be previewed or
customized through the existing flow. Normal builds embed no extension or theme packages.
Editor-created themes may embed a separate local card cover; it replaces the palette
illustration and is center-cropped to 16:9. It is not a catalog preview URL and does
not change the conversation background.
The protector shows a deleted-row illustration. Other entries without an image
remain valid and show a built-in illustration.
Previews describe the listed version, including when an installed version has
an update available; they are creator-provided, not proof of compatibility.

Only visible shop cards request previews. Images use the credential-free,
public-IP-pinned HTTPS downloader with a five-second deadline and must match their
own hash and byte count. Preview loading does not block shop actions; an action
cancels preview-only work before starting.
Decoding runs on the existing cancellable worker: at most 256 KiB compressed,
4,096 pixels per edge, 4,194,304 source pixels, and 32 MiB decoder allocation
budget. Only a static image is decoded; thumbnails shrink to at most 640 x 360.
Missing, invalid or unavailable images fall back without blocking installation.
The UI retains at most eight thumbnails (at most 7,372,800 RGBA bytes), clears
changed-image metadata and releases them with its extension runtime state.
There is no preview disk cache, telemetry, new dependency, or plugin permission.

## Host contract

The `extensions` crate defines the versioned manifest, capability, action,
invocation, result and theme types. These serialized types are the compatibility
boundary; internal `client-core` structures and egui objects are not an SDK.
Unknown API versions and invalid packages are rejected before installation.

### Capability reference

Each capability is independent and requires user consent. An update requests
renewed consent; adding a read grant does not grant commands. The SDK currently
supports 51 capabilities, with at most 64 distinct declarations per manifest.

> **Preview SDK — PR #405, not yet released.** `channel_control`,
> `server_control`, `role_control`, `moderation_control` and `media_control`, plus
> the reply, sticker and forward operations under `message_send`, require a host
> built from this branch.

| Capability | Granted behavior | Scope / confirmation |
| --- | --- | --- |
| `selected_message` | Read selected message text | A user-invoked `message` action only |
| `composer` | Read the current draft and propose replacement | A `composer` action; replacement requires Apply |
| `storage` | Read/replace one opaque local UTF-8 value | Per-account/plugin; 1 MiB disk limit and 256 KiB invocation budget |
| `deleted_messages` | Enable host retention of already-loaded deleted messages | Activation only; bounded session memory, no deleted text sent to Wasm |
| `image_sharing` | Enable host emoji/sticker image attachment mode | Activation only; picker selection authorizes sending, Wasm receives no image bytes |
| `appearance` | Return a bounded declarative theme overlay | Native colors/control metrics; no arbitrary drawing |
| `message_events` | Observe live create/update/delete events | Active accessible conversation; bounded best-effort delivery |
| `app_context` | Read connection, current user and selected channel | Current session, optional fields |
| `account_profile` | Read the current account's loaded avatar hash and own profile | Connected; optional display name, bio and pronouns; no credentials/connections |
| `guild_directory` | Read loaded joined servers | At most 100 names/IDs/icon hashes; no fetch |
| `channel_details` | Read selected-channel metadata, recipients and permission summary | Fresh accessible channel; at most 32 recipients; last-message ID/count require history access |
| `data_events` | Opt into fifteen granted app-data invalidation reasons | Requires `app_events` plus each reason's read grant |
| `channel_directory` | Read accessible cached channels | At most 100; partial directory, no fetch |
| `message_content` | Read loaded embed text, sticker labels, reply/forward markers and poll availability | Fresh readable selected timeline; 10 messages/8 KiB; no media URLs/bytes or poll details |
| `forum_data` | Read resident accessible child threads/posts and known flags | Selected readable parent/thread; 10 posts/6 KiB; no archive fetch or tags |
| `conversation_activity` | Read current typing IDs and a loaded pin page | Selected readable conversation; 8 typing IDs/20 pin IDs/2 KiB; no fetch |
| `channel_metadata` | Read loaded guild channel topic/category/thread metadata and permission decisions | Fresh readable selected guild channel; unknown remains optional; 6 KiB; no settings fetch |
| `member_details` | Read loaded guild member nicknames, role labels and matching server profile | Fresh selected member pane; 20 members/32 role IDs each/32 catalog roles; 6 KiB; no fetch |
| `message_details` | Read loaded message metadata: replies, mentions, attachment labels and reaction counts | Fresh readable selected timeline; at most 20 messages (12 with `timeline`); no text or URLs |
| `relationships` | Read loaded friends, requests and restricted-account labels | Connected; at most 100; known flags distinguish unloaded lists; no fetch |
| `timeline` | Read ordinary loaded messages in the active conversation | Fresh readable timeline, at most 50 (12 with `message_details`); no deleted/ephemeral text |
| `members` | Read loaded members or DM recipients | Active channel, at most 100; no fetch |
| `presence` | Read cached status strings for that context | At most 100; no activity/private device payloads |
| `voice_state` | Read current call state and participant IDs | At most 64 participants; no raw media |
| `read_state` | Read current channel unread/mention summary | Unknown unread remains distinct from false |
| `local_settings` | Read and propose changing seven local reading preferences | Zoom/sidebar/member list/GIFs/media links/smooth scrolling/scroll speed; changes require Apply |
| `notification_settings` | Read and propose changing device-local notification preferences | Sound volume, sound toggles and unread badge; changes require Apply; no Discord notification settings |
| `navigation` | Propose channel, Home, view, profile, message or search navigation | Existing native paths; each command requires Apply |
| `local_notices` | Propose an in-app toast | At most 1,024 text bytes; requires Apply |
| `clipboard_write` | Propose replacing clipboard text | At most 4,096 bytes; requires Apply; no clipboard read |
| `voice_control` | Propose mute/deafen, leaving or watching/stopping a participant stream | Each command requires Apply and the same current call |
| `app_events` | Observe ready/navigation/context/connection/voice/settings changes | Other grants control accompanying data; no background commands |
| `message_send` | Propose sending text, replies, loaded stickers, a loaded-message forward, or opening the native attachment picker | Apply; selected accessible targets; picker discloses no file path/bytes to Wasm; preserves existing composition |
| `message_manage` | Propose editing, deleting or pinning a loaded message | Apply; native ownership/moderation checks |
| `reactions_control` | Propose adding/removing your reaction | Apply; known message and current reaction state |
| `read_state_control` | Propose channel/server read markers | Apply; existing read-state checks |
| `threads_control` | Propose creating, renaming or changing threads/forum posts | Apply; native channel/thread permissions |
| `relationship_control` | Propose friend requests, relationship changes, nicknames and user notes; open a friend DM | Apply; existing relationship and loaded-note checks |
| `account_control` | Read own status/activity preferences and propose profile, status and activity-sharing changes | Apply; profile readiness and native validation |
| `audio_settings` | Read audio preferences and propose processing, gain, local playback, device selection or a device refresh | Apply; selected IDs must remain in the host's private device lists |
| `voice_connect` | Propose joining/ringing or declining a call | Apply; connection/access checks; native call-switch confirmation |
| `camera_control` | Propose enabling/disabling your camera or selecting a host-known camera | Apply; same current call, capture availability and native permissions |
| `channel_control` | Propose channel creation/editing/deletion/reordering, channel notification settings, and DM/group controls | Apply; loaded targets and native permissions; destructive operations are identified before approval |
| `server_control` | Propose server-settings/emoji changes, creating an invite or leaving a server | Apply; loaded server/settings and native permission checks |
| `role_control` | Propose creating, editing, moving or deleting roles | Apply; native role hierarchy and permission checks |
| `moderation_control` | Propose role assignment, nicknames, kicks, prune preview/execution and member-list visibility | Apply; native hierarchy/permission checks; destructive actions are identified |
| `media_control` | Propose opening the native screen-share picker or stopping screen share | Apply; current-call checks; no source list or captured media reaches Wasm |

### App snapshots and confirmed commands

The [app data reference](extension-sdk-reference.md#app-data) and
[output/action reference](extension-sdk-actions.md#outputs-and-host-actions)
explain every field, command and bound, with examples. SDK authors use
`fn(AppInvocation) -> AppOutput` with the unchanged `export!` macro and
`dispatch_typed` for offline checks. These wrappers retain the original
`Invocation`, `EventInvocation` and `Output` APIs and flatten into ABI v1 JSON.
Unsupported capabilities/actions are rejected by older hosts; declaring v1 alone
does not make new capabilities available in an old build. Current hosts also
inject public `host` discovery into every invocation: API version 1, SDK revision
1 and supported capability/event names. `AppInvocation.host` is optional for
older hosts, and its SDK lists use strings to tolerate future names. Support is
not consent; discovery never bypasses required manifest validation or grants.

`app` contains separately granted optional `context`, `account_profile`, `guilds`,
`channel_details`, `channels`, `timeline`, `members`, `presence`, `voice`,
`read_state`, `settings`, `notification_settings`, `message_details`, `relationships`, `channel_metadata`,
`member_details`, `message_content`, `forum_data`, `conversation_activity`,
`audio_settings` and `own_presence` groups. A missing group
is unavailable or ungranted, not an empty dataset. Snapshot construction reads
already-loaded state without network or disk IO. The complete serialized snapshot
is capped at 64 KiB. Per-group budgets are 10 KiB for channels, 20 KiB for timeline,
6 KiB each for members, presence and channel-detail recipients, and 8 KiB for guilds,
including item overhead. Message details and relationships use at most 8 KiB and
4 KiB respectively, and share the remaining 64-KiB snapshot budget; they may
truncate earlier when other groups are present. Channel metadata and member
details each have a 6-KiB ceiling within that same global budget; member rows
are trimmed first, and groups can be omitted when no space remains. Rich-message
content (8 KiB), forum data (6 KiB) and conversation activity (2 KiB) also share
that ceiling; no new total snapshot allocation is authorized. The timeline
skips messages larger than 4 KiB and reports partial data. Granting
`message_details` also caps both timeline text and metadata rows at 12 when requested together, while preserving
the 20-KiB timeline byte budget and shared 10-million-fuel limit. Valid wire-sized
inputs can still exceed execution fuel. Bounded list responses
expose `truncated`; voice participant IDs are capped without a completeness flag.
No snapshot includes tokens, deleted/ephemeral text, attachment bytes/URLs, raw
voice/video/screen media or the unbounded client state.
An active DM or private channel is eligible when accessible: granting timeline,
members or presence can expose that conversation's corresponding loaded data.

Foreground `message`, `composer` and `panel` actions may return one `effects`
proposal, at most 8 KiB serialized. The native result shows its exact action and
values and waits for **Apply**. Closing it does nothing. Apply rechecks the plugin,
grants, account/conversation, permissions and the specific voice call before using
the existing UI path. Navigation/search may then load ordinary service data, but
the plugin never receives a generic Discord command API. Clipboard writes never
read the clipboard; local notices are in-app toasts, not OS notifications.

Supported local preference patches are zoom 80–150%, sidebar width 190–360 logical
pixels, member-list visibility, GIF animation, hiding media links, smooth scrolling
and scroll speed 25 through 300%. The separate `notification_settings` grant exposes
device-local notification toggles and sound volume 0 through 100%; it does not change
Discord account/guild settings or play a cue. Both patches require at least one
value, preserve omitted/null preferences at Apply time and reject invalid changes
without applying any fields. Changes use the ordinary local persistence path. The separate `voice_connect` and `camera_control` grants permit approved call
joins/rings and camera changes. Screen-source selection and recording are not
exposed. Read the [app-action reference](extension-sdk-actions.md#app-actions)
for messaging, threads, relationships, account and audio operations.

Four opt-in additions extend the same ABI without changing existing SDK structs:

- `data_queries` adds `ExtendedAppInvocation.queries` and approved request actions for
  message search, pins, archived threads, member search, profiles and GIF search.
  Results reuse Serein's bounded native views: at most 25 rows and 48 KiB total,
  with loading, error, truncation and next-cursor fields where applicable.
- `messaging_settings` adds the loaded account privacy snapshot and approved updates
  for DM, message-request, friend-source and game-DM preferences.
- `guild_folders` adds the loaded server-folder layout and an approved whole-layout
  update through the existing versioned, 200-folder/200-server native path. Updates
  carry the snapshot's `base_version` and reject stale full-layout replacements.
- `action_feedback` adds `tracked_app_action`. Supply a unique `request_id`; after
  **Apply**, the app-event handler receives `action_result` with `accepted` or
  `rejected` and a stable code (`accepted`, `context_changed`, `unavailable`,
  `invalid`, `denied` or `failed`). Acceptance means Serein admitted the action to
  its native path; it does not claim that a later network request succeeded.

Use `ExtendedAppInvocation` only when reading these fields. Existing
`AppInvocation` source remains compatible and ignores the additional JSON fields.
`data_queries` and `action_feedback` require one `app_event` action because query
completion and tracked results use that bounded, best-effort queue. Query
invalidations may be coalesced; always read the latest snapshot.

Approved convenience actions also open the native join-server flow, send a server
invite to a loaded friend, open emoji/member/role/invite/audit-log administration,
and open a group-DM editor. The native UI still performs its ordinary access and
permission checks. Integrations, webhooks and slash commands remain outside this
SDK surface.

One `app_event` action may observe `ready`, `navigation`, `context`, `connection`,
`voice` and `settings`. Settings events cover reading, notification, audio and own-presence/activity
preferences; each snapshot still requires its own grant. Context
events report loaded-data/freshness changes after navigation; use `message_events` for individual message changes. The additional
`data_events` grant opts into `account`, `channels`, `members`, `presence`,
`read_state`, `message_details`, `relationships`, `threads`, `roles`, `permissions`,
`recovered`, `reactions`, `pins`, `typing` and `polls` invalidation hints (21 event kinds total), each requiring the corresponding data grant
(`channels` accepts `channel_directory`, `guild_directory`, `channel_details` or `channel_metadata`; `members`
accepts `members` or `member_details`). `threads` requires `channel_metadata` or `forum_data`,
`roles` requires `member_details`, and `permissions`/`recovered` accept any of
`channel_metadata`, `member_details`, `forum_data`, `conversation_activity` or
`message_content`. `reactions`/`pins`/`typing` require `conversation_activity`,
`polls` requires `message_content`, and `message_details` accepts either its
namesake grant or `message_content`.
Recovery is an invalidation hint, not event replay or a promise all data is fresh. Without this
opt-in, existing observers receive only the original six variants. Repeated
pending detailed reasons coalesce by kind for each plugin. Pending app
changes are coalesced per plugin and share the message-event queue/rate bounds.
The host captures a fresh, separately granted snapshot at dispatch. There are no
timers or persistent plugin instances, and delivery is best effort. App/message
events and activation cannot return host command proposals; background events
cannot open panels. Granted `storage` and `appearance` outputs remain available.

The [App Toolbox example](../examples/extensions/app-toolbox/src/lib.rs) provides
an account/channel dashboard and explicit controls for the original 12 host-effect types.
[Conversation Actions](../examples/extensions/app-actions/src/lib.rs) demonstrates
the new typed `app_action` operations through a review form. Its `app_event`
observer returns an empty output, and it saves no conversation data.
[Guild Inspector](../examples/extensions/guild-inspector/src/lib.rs) separately
shows optional channel settings/thread permissions and the first five loaded
member details, with only its two read grants plus event grants. It does not
fetch settings, profiles or role catalogs. Unknown decisions remain distinct
from denial, and native actions still recheck permissions.
[Conversation Inspector](../examples/extensions/conversation-inspector/src/lib.rs)
shows rich summaries, loaded forum flags, typing/pin state and public discovery.
Polls have only an absent/unsupported marker; no questions/results are retained.
Forum tags remain unavailable. All three inspector/toolbox observers stay passive.

### Existing tools and message events

Actions are invoked by a message context-menu item, composer tool or panel
button. A plugin may also declare one `activation` action that runs in the worker
on enable/account load. An enabled plugin with granted `deleted_messages` capability
keeps already loaded deleted messages in the session window. Without that opt-in,
deleted payloads are released. Deleted text is red by default. Hover and a
local context menu can toggle that highlight or remove the retained row. They
never call Discord. Live service actions stay unavailable. Tombstone
reconciliation and disk-cache removal remain unchanged. Disabling the plugin, logout, lost channel
access and ordinary timeline eviction also clear retained deleted content. The
same 500-row / 4 MiB per-window budget includes both live and retained deleted
payloads. The optional `preserve_deleted_messages` activation output is accepted
for package compatibility; successful activation with the granted capability enables
retention, including existing protector packages with no-op activation. Input is
restricted to the granted context and bounded form values.
Results can propose a composer replacement or return native headings, text, rows,
separators, buttons, text inputs, checkboxes, dropdowns and integer sliders. Standalone
panel actions are available from the composer Tools menu as well as the shop. Composer proposals require Apply, retain
the ordinary Send action and are discarded when their originating context is
stale. Account/session changes invalidate outstanding results.

The `message_events` capability allows one `message_event` action to receive
read-only create, update and delete snapshots for the active, accessible
conversation. The user must explicitly grant this access. Only accepted live
timeline events are eligible; history, search, cache loads, ephemeral messages
and other conversations are excluded. Create supplies channel/message/author IDs
and text; updates may contain partial fields; delete supplies channel and message
IDs only. Text is bounded to 16 KiB of UTF-8. The payload excludes attachments,
embeds and raw Gateway data. It can contain private conversation text when that
conversation is active, so grant access only to plugins you trust.

Event handlers run on the existing worker and may save `storage` or change
`appearance` with those separately granted capabilities. They cannot send messages,
replace the composer, open panels in the background or enable activation-only
features. Use a separate user-invoked panel to show results. The Message Counter
example stores only three saturating numeric counts, preserves malformed storage
until the user explicitly resets it, and never stores message text or identifiers.

Delivery is best effort, not exactly once or a complete audit stream. The host
requires a loaded message for updates/deletes; duplicate creates, including sends
already reconciled from a send result, may be skipped. The host
queues at most 32 pending invocations totaling 64 KiB and starts at most 10 event
invocations per second; overload drops events. Scope/account changes, lost
permissions and disabling discard pending work and reject stale results.
Reactive SDK handlers use `EventInvocation` and `dispatch_typed`; existing
`Invocation` handlers and their struct literals remain unchanged under ABI v1.
Older hosts reject the new capability/action in the manifest. They do not load
a message-event plugin merely because its manifest declares API version 1.

The **Emoji & Sticker Images** catalog plugin requests `image_sharing`. Its
activation output makes custom emoji and sticker selections stage artwork as ordinary
image attachments. Selecting artwork authorizes one send after host download and
validation, without another composer confirmation. Text drafts stay intact. Existing
file selections must be sent or removed first. Serein displays these attachments at
32px for emoji and 160px for stickers; other clients control their own attachment layout. Enabling the plugin never sends anything, grants
network access to Wasm, or changes native sticker/emoji entitlements. Disabling removes the option. Logout, account changes and channel navigation cancel
pending image preparation; already selected files follow ordinary attachment handling.
The `image_sharing` output defaults to false and is accepted only from an activation
action with that capability granted. This capability requires a supporting host.

Themes override named colors and native typography, spacing, padding and corner
radii, plus an optional embedded PNG/JPEG background with per-mode opacity and fit.
Settings > Themes > Create theme provides a native editor and portable export.
See the complete [theme API](theme-api.md) for fields, bounds and inheritance.

Plugins can request the `appearance` capability to return an `appearance` object
using that same theme schema. Creators can build native appearance settings panels
with dropdowns and sliders. Activation and separately granted event actions
can also return appearance values.
An action replaces that plugin's previous appearance object; omit it to leave the
current appearance unchanged, or return `{}` to remove its overrides. A plugin can
use granted `storage` to save choices and read them during activation on the next
account load. Activation does not receive conversation text. The host overlays
plugin appearances on the selected theme in ascending plugin-ID order; the last
explicit value wins. The user's own accent color setting still takes precedence.
Disabling a plugin removes its overrides; Ctrl+Shift+F12 resets community themes
and appearance plugins.

This supports app-wide palette changes and shared native control styling, not
arbitrary code injection into egui, replacement of the app layout, custom fonts,
network access, or automatic Discord actions. Fixed-size custom-painted components
keep their existing geometry. Missing fields inherit the underlying theme.

## Safe execution diagnostics

Execution errors use fixed host messages and existing native error status. They
never include plugin output, message text, saved storage, raw Wasm error strings
or private invocation values. No diagnostic log or upload is added. Categories
identify a useful next check, not a complete crash trace:

| Host error category | Author action |
| --- | --- |
| `Fuel` | Reduce handler loops, JSON work or requested data; the limit is 10 million fuel. |
| `Memory` | Reduce memory/table allocation; linear memory remains capped at 16 MiB. This category also covers engine allocation failure, not only hitting that exact cap. |
| `Stack` | Reduce recursion and stack allocations. |
| `Trap` | Check for panic, invalid memory access or arithmetic traps. No panic text is exposed. |
| `Input` / `InputLimit` | Check the action/input schema or reduce data, form values and storage. Public discovery counts toward the existing 256-KiB serialized invocation limit. |
| `Handler` | The ABI returned no response. Check SDK input decoding and output serialization; the host cannot infer which failed from an empty response. |
| `Output` / `OutputLimit` | Check output JSON/ABI buffer or reduce panel elements, text and storage. Existing output limits still apply. |
| `Capability` | Check the manifest, user grants and permitted action surface; an execution retry cannot grant access. |
| `Module` / `Execution` | Check required exports, import restrictions and runtime requirements. |
| `Invalid` / `Version` / `Limit` | Check document schema, supported API version or the relevant package/data bounds before execution. |

Native SDK `dispatch_typed` checks remain useful for decoding and serialization;
real sandbox checks exercise fuel, memory, exports and output enforcement. Neither
proves live Discord compatibility. No lifecycle tests are implied by these diagnostics.

## Resource and privacy limits

Wasm executes on an on-demand background worker with fuel, stack and memory
limits and no WASI or host imports. Each invocation gets a new runtime; it is
dropped when the call completes. Plugin execution never happens in a render or
audio callback. The application bounds package sizes, installed plugin count,
invocation input/output, panel complexity, queues and plugin storage.

| Resource | Limit | What it means for an author |
| --- | --- | --- |
| Compiled Wasm | 4 MiB | Keep the module small; the host validates imports and compilation limits too. |
| JSON package | 16 MiB | Includes the manifest and encoded payload. |
| Linear memory | 16 MiB | Includes decoding, handler allocations and output buffers. |
| Execution fuel | 10,000,000 | Shared by parsing and execution; a valid-sized input can still exhaust it. |
| Wasm call depth / interpreter stack | 128 calls / 256 KiB | Avoid deep recursion. |
| Serialized input and output | 256 KiB each | Count UTF-8 and JSON escaping, including nested storage JSON. |
| Manifest actions / capabilities | 16 / 64 distinct | Only the 51 supported capability names are currently accepted. |
| Panel | 64 elements / 8 row levels | Includes nested children; text and input values are at most 4 KiB each. |
| Plugin storage on disk | 1 MiB | Its practical size must also fit the smaller invocation/output budget. |
| App snapshot | 64 KiB | Individual lists have smaller budgets; see the [data reference](extension-sdk-reference.md#app-data). |
| Host proposals | 1 / 8 KiB serialized | A foreground action proposes one operation for Apply. |
| Reactive queue / rate | 32 pending calls / 64 KiB / 10 starts per second | Shared message/app events are best effort; excess work is dropped. |

Invalid output or exhausted Wasm budgets produce an execution error and can
disable the failing plugin. Snapshot collectors may instead return explicitly
partial lists, and reactive overload may drop events; handle both in your code.

Disabled plugins have no retained instance, worker, package or plugin data once
cleanup succeeds. Shared host code and bounded catalog metadata still cost some
application space. Freed allocations may remain in the process allocator; an
unchanged RSS reading alone does not mean an instance remains active.

Plugin storage is ordinary local data, not encrypted by Serein. Do not use it
for credentials. No extension diagnostics or private invocation data are
uploaded. Sandboxing and review reduce exposure but cannot prove absence of
bugs in the runtime or host; keep Serein updated.

The buffer ABI is documented in the
[authoring guide](../examples/extensions/README.md#abi-version-1). The demo uses
a separate bounded temporary `serein-extension-demo` profile; it can import local
fixtures and browse the embedded starter catalog/previews but cannot download a
catalog, preview or package. `Ctrl+Shift+F12` resets a
community theme if its colors make controls difficult to read.

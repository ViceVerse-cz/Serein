# Extension SDK inputs and app data

## Invocation and events

An invocation is one call to your handler. The host chooses a declared action,
collects the data that action is allowed to receive, and starts a fresh Wasm
instance. Local variables do not survive the call. Use the separately granted
`storage` field when your plugin needs to remember something.

### Choose an input type

| SDK input | Use it for | How to read the action |
| --- | --- | --- |
| `Invocation` | Message tools, composer tools, panels and activation | `input.action` |
| `EventInvocation` | Those actions plus live message events | `input.invocation.action` |
| `AppInvocation` | Those actions plus app snapshots and app change events | `input.invocation.action` |

The wrappers keep the original `Invocation` fields unchanged. Their `invocation`
field is a Rust convenience: JSON stays flat. There is no JSON object named
`invocation`, and there is no input field named `surface`. Use `action` to route
the call to the handler for the action ID in your manifest.

### Common input fields

The examples in this table use `i: &Invocation`. For a wrapper, set
`let i = &input.invocation;` first.

| Wire field | SDK Rust / JSON type | Meaning and when supplied | Reading it |
| --- | --- | --- | --- |
| `action` | `String` / string | Required manifest action ID, such as `show` or `format-draft`. It is not the action's display label. | `i.action == "show"` |
| `selected_message` | `Option<String>` / string or null | Text of the message chosen by the user. Requires `selected_message` and a `message` action. It contains no message ID or author object. | `i.selected_message.as_deref()` |
| `composer` | `Option<String>` / string or null | The current draft for a `composer` action with the `composer` grant. An empty draft is `Some("")`, not `None`. | `i.composer.as_deref()` |
| `storage` | `Option<String>` / string or null | The plugin's last saved opaque UTF-8 value for this account. Requires `storage`; absent when nothing has been saved. The worker reloads it before execution. | `i.storage_json::<u64>()` if your plugin stores a JSON number |
| `values` | `BTreeMap<String, String>` / object of string values | Current form values when a panel button invokes an action. Keys are input element IDs. Initial tool/panel opens normally have an empty map; reactive events always do. | `i.value("name")` or `i.parse_value::<bool>("enabled")` |

`values` holds strings even for typed controls: a checkbox supplies `"true"` or
`"false"`, a slider supplies a decimal integer string, and a select supplies the
chosen option string. Missing input and an empty string differ. `parse_value`
returns `Ok(None)` for missing input, `Ok(Some(value))` for valid input, and `Err`
for malformed input. Your handler must check any allowed numeric range.

Each value is at most 4 KiB of UTF-8, with at most 64 keys. Storage has a 1 MiB
disk ceiling, but the whole invocation must fit the smaller 256 KiB serialized
input limit. JSON escaping and the other fields count toward that limit.
`storage_json` reports invalid saved JSON as an error; it does not reset it.

A panel button starts a new invocation using its ID as `action`. It receives
current form values and granted storage, but does not inherit the original
selected message or draft. App data, when available, is collected again.

This complete handler reads a draft and returns its character count in a panel.
Declare `count` as a `composer` action and request `composer`. A panel response
does not need an extra capability.

```rust
use serein_extension_sdk::{Element, Invocation, Output};

fn handle(input: Invocation) -> Output {
    if input.action != "count" {
        return Output::default();
    }
    let text = match input.composer.as_deref() {
        Some(draft) => format!("Your draft has {} characters.", draft.chars().count()),
        None => "No draft was supplied.".into(),
    };
    Output { panel: vec![Element::Text { text }], ..Default::default() }
}

serein_extension_sdk::export!(handle);
```

### Additional wrapper fields

These fields are at the top level of the JSON object too. Absent fields decode
to `None`. New optional fields are omitted by the host when unavailable; the
common optional fields above may instead be serialized as `null`.

| Wire field | SDK Rust / JSON type | Meaning and when supplied | Reading it |
| --- | --- | --- | --- |
| `host` | `Option<HostInfo>` / object or absent | Public host support information, injected on every current-host invocation without a grant. Available through `AppInvocation`; absent on older hosts. It contains supported names, not user grants. | `input.host.as_ref().is_some_and(\|host\| host.supports("message_content"))` |
| `message_event` | `Option<MessageEvent>` / object or absent | Live message change. Available in `EventInvocation` and `AppInvocation`; requires `message_events` and the `message_event` surface. | `input.message_event.as_ref()` |
| `app` | `Option<AppSnapshot>` / object or absent | Independently granted [app data](extension-sdk-reference.md#app-data). Available in `AppInvocation`. The desktop supplies snapshots to foreground actions and app events; activation and message events do not currently receive them. | `input.app.as_ref().and_then(\|app\| app.timeline.as_ref())` |
| `app_event` | `Option<AppEventKind>` / string or absent | Why the host scheduled an app observer. Requires `app_events` and the `app_event` surface. Available in `AppInvocation`. | `input.app_event == Some(AppEventKind::Connection)` |

### HostInfo: discover supported names

Current hosts inject this public, fixed support catalog into every Wasm call;
no capability is required and no account data is present. `AppInvocation.host`
is `None` on older hosts. `Invocation`/`EventInvocation` handlers continue to
ignore this additional wire field. SDK support names are strings so future names
can be inspected without decoding a newer capability/event enum.

| Wire field | SDK Rust / JSON type | Meaning | Reading from `host: &HostInfo` |
| --- | --- | --- | --- |
| `api_version` | `u32` / integer | Current buffer/JSON ABI version, `1`. | `host.api_version` |
| `sdk_revision` | `u32` / integer | Current discovery schema revision, `1`; not a release or protocol compatibility claim. | `host.sdk_revision` |
| `capabilities` | `Vec<String>` / array of strings | Host-supported capability names (42 currently), not this plugin's granted capabilities. | `host.supports("forum_data")` |
| `app_events` | `Vec<String>` / array of strings | Host-supported app-event names (21 currently), not an event subscription or delivery guarantee. | `host.supports_event("typing")` |

A supported capability still needs to be declared and explicitly granted. Older
hosts reject unknown required manifest capabilities **before** a handler can run;
discovery cannot bypass installation validation. Use a compatible manifest to
inspect support, and handle missing `host` without assuming support.

This complete synthetic input shows the current support catalog without any
account snapshot or grant-dependent data:

```json
{
  "action": "show",
  "values": {},
  "host": {
    "api_version": 1,
    "sdk_revision": 1,
    "capabilities": [
      "message_content",
      "forum_data",
      "conversation_activity",
      "channel_metadata",
      "member_details",
      "selected_message",
      "composer",
      "storage",
      "deleted_messages",
      "image_sharing",
      "appearance",
      "message_events",
      "app_context",
      "channel_directory",
      "timeline",
      "members",
      "presence",
      "voice_state",
      "read_state",
      "local_settings",
      "notification_settings",
      "navigation",
      "local_notices",
      "clipboard_write",
      "voice_control",
      "app_events",
      "account_profile",
      "guild_directory",
      "channel_details",
      "data_events",
      "message_details",
      "relationships",
      "message_send", "message_manage", "reactions_control", "read_state_control",
      "threads_control", "relationship_control", "account_control", "audio_settings",
      "voice_connect", "camera_control"
    ],
    "app_events": [
      "ready",
      "navigation",
      "context",
      "connection",
      "voice",
      "settings",
      "account",
      "channels",
      "members",
      "presence",
      "read_state",
      "message_details",
      "relationships",
      "threads",
      "roles",
      "permissions",
      "recovered",
      "reactions",
      "pins",
      "typing",
      "polls"
    ]
  }
}
```

### Message event fields

Declare at most one action with `surface: "message_event"` and request
`message_events`. Eligible changes come from the active, accessible conversation
after the host accepts them into its timeline. Updates and deletes require an
already-loaded message. History loads, search results and other conversations
do not generate this event. Ephemeral messages are excluded. An active DM or
private channel can supply ordinary message text after this grant.

In the following table, `event` is a borrowed `MessageEvent`.

| Wire field | SDK Rust / JSON type | Meaning and when supplied | Reading it |
| --- | --- | --- | --- |
| `kind` | `MessageEventKind` / string | Required: `create`, `update` or `delete`. Rust variants are `Create`, `Update`, `Delete`. | `event.kind == MessageEventKind::Create` |
| `channel_id` | `String` / string | Required ID of the active conversation. | `event.channel_id.as_str()` |
| `message_id` | `String` / string | Required ID of the affected message. | `event.message_id.as_str()` |
| `author_id` | `Option<String>` / string or absent | Required for create; may be absent on update; always absent on delete. Missing author means unknown, not the current user. | `event.author_id.as_deref()` |
| `content` | `Option<String>` / string or absent | Required for create; supplied on update when a text patch is included; always absent on delete. At most 16 KiB of UTF-8. | `event.content.as_deref()` |

An update without `content` says nothing about text. `"content": ""` explicitly
reports empty text. A delete never contains the deleted text. Message events
contain no attachments, embeds or raw Gateway payload. Oversized events are
skipped rather than cut into incomplete text.

This is a complete synthetic update input. It reports an empty text patch, while
leaving the author unknown:

```json
{
  "action": "on-message",
  "values": {},
  "message_event": {
    "kind": "update",
    "channel_id": "100",
    "message_id": "200",
    "content": ""
  }
}
```

Event handlers must leave `panel` and `effects` empty. They cannot replace the
draft or enable activation-only features. Separately granted `storage` and
`appearance` remain available. Show information through a separate user-invoked
panel action.

This handler counts delivered creates and displays the count on `show`. Request
`message_events` and `storage`; declare `on-message` as `message_event` and
`show` as `panel`. It saves only a number, not message content or identifiers.
Invalid saved JSON is preserved instead of silently replaced.

```rust
use serein_extension_sdk::{Element, EventInvocation, MessageEventKind, Output};

fn handle(input: EventInvocation) -> Output {
    let count = match input.invocation.storage_json::<u64>() {
        Ok(value) => value.unwrap_or(0),
        Err(_) => return Output::default(),
    };
    if input.invocation.action == "on-message" {
        let mut output = Output::default();
        if input.message_event.as_ref().is_some_and(|event| {
            event.kind == MessageEventKind::Create
        }) && output.set_storage_json(&count.saturating_add(1)).is_err() {
            return Output::default();
        }
        return output;
    }
    if input.invocation.action == "show" && input.message_event.is_none() {
        return Output {
            panel: vec![Element::Text { text: format!("Delivered creates: {count}") }],
            ..Default::default()
        };
    }
    Output::default()
}

serein_extension_sdk::export!(handle);
```

### AppEventKind: why an app observer ran

Declare at most one `app_event` action and request `app_events`. Each call can
include the snapshot groups permitted by your other grants. `app_events` alone
does not grant the current user's identity, conversation text or settings.

| JSON value | Rust variant | Meaning |
| --- | --- | --- |
| `ready` | `AppEventKind::Ready` | Observation has started for the current plugin/account state, including after enable or observer reset. It can occur more than once. |
| `navigation` | `AppEventKind::Navigation` | The selected conversation changed, including moving to or from Home. |
| `connection` | `AppEventKind::Connection` | The app's Gateway connection state changed. It does not describe the voice transport. |
| `context` | `AppEventKind::Context` | Loaded-data readiness, history/member freshness or access changed. This is not a notification for every message edit, member change or read-state change. |
| `voice` | `AppEventKind::Voice` | The current call identity, phase, local controls, screen-share state or tracked participants changed. |
| `settings` | `AppEventKind::Settings` | An exposed local reading or device-local notification preference changed; each snapshot requires its own grant. |
| `account` | `AppEventKind::Account` | Account identity or loaded own profile may have changed; additionally requires `data_events` and `account_profile`. |
| `channels` | `AppEventKind::Channels` | Joined guilds, visible channels or selected-channel details may have changed; additionally requires `data_events` and at least one of `guild_directory`, `channel_directory`, `channel_details`, `channel_metadata`. |
| `members` | `AppEventKind::Members` | Loaded selected-channel members may have changed; additionally requires `data_events` and either `members` or `member_details`. |
| `presence` | `AppEventKind::Presence` | Known selected-context statuses may have changed; additionally requires `data_events` and `presence`. |
| `read_state` | `AppEventKind::ReadState` | Selected-channel read/mention state may have changed; additionally requires `data_events` and `read_state`. |
| `message_details` | `AppEventKind::MessageDetails` | Loaded selected-message metadata may have changed; additionally requires `data_events` and either `message_details` or `message_content`. |
| `relationships` | `AppEventKind::Relationships` | Loaded friends, requests or restricted-account lists may have changed; additionally requires `data_events` and `relationships`. |
| `threads` | `AppEventKind::Threads` | Loaded selected-context thread/post metadata may have changed; additionally requires `data_events` and either `channel_metadata` or `forum_data`. |
| `roles` | `AppEventKind::Roles` | Loaded member role/catalog data may have changed; additionally requires `data_events` and `member_details`. |
| `permissions` | `AppEventKind::Permissions` | Selected-context permissions may have changed, including loss of data access; additionally requires `data_events` and at least one of `channel_metadata`, `member_details`, `forum_data`, `conversation_activity`, `message_content`. |
| `recovered` | `AppEventKind::Recovered` | The host observed accepted recovery/resynchronization; reread current groups, which may still be absent or partial. Additionally requires `data_events` and at least one of `channel_metadata`, `member_details`, `forum_data`, `conversation_activity`, `message_content`; this is not replay of missed events. |
| `reactions` | `AppEventKind::Reactions` | An accepted selected-message reaction change may affect loaded data; requires `data_events` and `conversation_activity`. Reaction summaries still need `message_details`. |
| `pins` | `AppEventKind::Pins` | Selected-channel loaded pin data may have changed; requires `data_events` and `conversation_activity`. |
| `typing` | `AppEventKind::Typing` | Selected-channel typing indicators may have changed or expired; requires `data_events` and `conversation_activity`. |
| `polls` | `AppEventKind::Polls` | A loaded message's poll-presence marker may have changed; requires `data_events` and `message_content`. This is not a poll vote/results stream. |

The fifteen detailed reasons are opt-in: `data_events` requires `app_events`,
and each reason also needs its corresponding read grant. Existing observers
without `data_events` receive only the original six reasons. These are
invalidation hints from observed app updates, not raw service events or payload
patches. Rejected or no-op updates may also produce a hint. Permission changes may invalidate data without supplying a replacement.

Treat the event as a reason to inspect the supplied snapshot, not as a complete
change log. Pending detailed changes of the same kind are coalesced per plugin.
The snapshot is taken when the call is dispatched, so it can reflect several changes. There is no
periodic timer or persistent plugin process. The [app-data example](extension-sdk-reference.md#app-data) safely
distinguishes app events from its foreground display action.

Both reactive surfaces have empty `values`, no selected message and no composer.
They share a queue of at most 32 pending calls and 64 KiB, with at most 10 event
calls started per second. Overload, account/conversation changes, permission loss
and disabling can discard work. Delivery is best effort: events may be skipped,
and duplicate creates are not an exactly-once counting source. All handlers still
use the usual memory, input/output and execution-fuel limits.

## App data

Read app data through `AppInvocation.app`. The host copies only bounded,
already-loaded state. Reading a group does not fetch history, discover guild
members, join a call or issue a network request. Each group requires its own
capability. A group may still be absent after consent because its data is
unavailable, disconnected, inaccessible or not fresh enough.

### IDs, absence and partial data

Channel, guild, message and user IDs are decimal **strings**, such as `"100"`.
They represent nonzero `u64` values, use at most 20 bytes, and must not be treated
as ordinary JSON numbers: some languages lose precision for large numeric IDs.
Keep them as strings when comparing or returning them in a host proposal.

| Value | How to interpret it |
| --- | --- |
| Missing optional group, such as no `timeline` | No data was supplied. Do not infer that the channel has no messages. |
| `"items": []` or `"messages": []` | The group exists, but contains no eligible rows in this snapshot. Check `truncated` too. |
| `"truncated": true` | The list is known to be partial because of loading state, missing rows or host limits. |
| `"truncated": false` | The host did not mark this snapshot partial. It still is not a promise of a complete service-wide directory or history. |
| `"unread": null` | The read state is unknown; this differs from `false`. |
| `"content": ""` | Known empty text, which is valid for a message containing other content. |

The SDK maps an omitted optional field and explicit JSON `null` to `None`.
Most new optional fields are omitted by the host. `ReadSnapshot.unread` is the
exception: unknown unread state is serialized as `null`.

The complete app snapshot is at most 64 KiB serialized. The collector also has
budgets including item overhead: 10 KiB for channels, 20 KiB for timeline,
6 KiB each for members, presence and channel-detail recipients, and 8 KiB for
guilds. Message details and relationships have 8-KiB and 4-KiB group limits
and also consume the remaining shared 64-KiB snapshot budget. They can truncate
earlier when other groups are present. Channel metadata and member details each
have a 6-KiB group ceiling and share that remaining global budget too. Member
rows are trimmed first to fit; either new group may be omitted if insufficient
space remains. A list may reach its byte budget before
its item limit. These limits do not guarantee that every handler fits the sandbox's
fuel budget; parsing and your own processing also consume fuel.

### AppSnapshot: choose the group you need

For the examples below, `app` is a borrowed `AppSnapshot`. Each field is an
`Option<T>` in Rust and an object when present in JSON.

| Wire field | SDK Rust type | Required capability and availability | Reading it |
| --- | --- | --- | --- |
| `message_content` | [`Option<MessageContentSnapshot>`](extension-sdk-reference.md#messagecontentsnapshot-bounded-rich-message-summaries) | `message_content`; fresh readable selected timeline; rich summaries only, no media URLs or referenced text. | `app.message_content.as_ref()` |
| `forum_data` | [`Option<ForumDataSnapshot>`](extension-sdk-reference.md#forumdatasnapshot-loaded-sibling-posts-and-threads) | `forum_data`; fresh readable selected guild text/forum/media parent or thread with an accessible readable parent. | `app.forum_data.as_ref()` |
| `conversation_activity` | [`Option<ConversationActivitySnapshot>`](extension-sdk-reference.md#conversationactivitysnapshot-loaded-pins-and-typing-indicators) | `conversation_activity`; fresh readable selected conversation; loaded pins and current typing IDs only. | `app.conversation_activity.as_ref()` |
| `channel_metadata` | [`Option<ChannelMetadataSnapshot>`](extension-sdk-reference.md#channelmetadatasnapshot-loaded-channel-settings-threads-and-permissions) | `channel_metadata`; connected, fresh, viewable and readable selected guild channel; optional settings/post fields may remain unknown. | `app.channel_metadata.as_ref()` |
| `member_details` | [`Option<MemberDetailsSnapshot>`](extension-sdk-reference.md#memberdetailssnapshot-loaded-guild-members-and-role-labels) | `member_details`; connected, fresh readable selected guild channel with a matching fresh loaded member pane. | `app.member_details.as_ref()` |
| `message_details` | [`Option<MessageDetailsSnapshot>`](extension-sdk-reference.md#messagedetailssnapshot-loaded-replies-mentions-attachments-and-reactions) | `message_details`; connected, fresh, readable selected timeline, without message text. | `app.message_details.as_ref()` |
| `relationships` | [`Option<RelationshipsSnapshot>`](extension-sdk-reference.md#relationshipssnapshot-loaded-account-relationships) | `relationships`; connected, already-loaded friend/request/restricted lists. | `app.relationships.as_ref()` |
| `account_profile` | [`Option<AccountProfileSnapshot>`](extension-sdk-reference.md#accountprofilesnapshot-the-current-accounts-loaded-profile) | `account_profile`; connected current account, with optional already-loaded own profile. | `app.account_profile.as_ref()` |
| `guilds` | [`Option<GuildDirectorySnapshot>`](extension-sdk-reference.md#guilddirectorysnapshot-loaded-joined-servers) | `guild_directory`; connected, already-loaded joined servers. | `app.guilds.as_ref().map(\|group\| group.items.len())` |
| `channel_details` | [`Option<ChannelDetailsSnapshot>`](extension-sdk-reference.md#channeldetailssnapshot-selected-channel-metadata-and-permissions) | `channel_details`; connected, accessible, fresh selected channel. | `app.channel_details.as_ref()` |
| `context` | [`Option<AppContextSnapshot>`](extension-sdk-reference.md#appcontextsnapshot-current-account-and-selected-chat) | `app_context`; current account and connection summary. Can remain available while disconnected. | `app.context.as_ref()` |
| `channels` | [`Option<ChannelDirectorySnapshot>`](extension-sdk-reference.md#channeldirectorysnapshot-loaded-channel-list) | `channel_directory`; Gateway connected. Only loaded channels the user can view; a selected channel known to be unavailable is excluded. | `app.channels.as_ref().map(\|group\| group.items.len())` |
| `timeline` | [`Option<TimelineSnapshot>`](extension-sdk-reference.md#timelinesnapshot-and-messagesnapshot-loaded-messages) | `timeline`; selected text-capable conversation, connected, readable history and fresh timeline. | `app.timeline.as_ref()` |
| `members` | [`Option<MembersSnapshot>`](extension-sdk-reference.md#memberssnapshot-loaded-people-in-this-conversation) | `members`; connected, accessible selected conversation with a fresh loaded member list, or loaded DM/group-DM recipients. | `app.members.as_ref()` |
| `presence` | [`Option<PresenceSnapshot>`](extension-sdk-reference.md#presencesnapshot-and-presenceentry-known-status-only) | `presence`; the same selected member/recipient scope, using only known status entries. | `app.presence.as_ref()` |
| `voice` | [`Option<VoiceSnapshot>`](extension-sdk-reference.md#voicesnapshot-the-current-call) | `voice_state`; current call summary, or an idle summary when there is no accessible active call. The call may be in a different channel from the selected chat. | `app.voice.as_ref()` |
| `read_state` | [`Option<ReadSnapshot>`](extension-sdk-reference.md#readsnapshot-unread-and-mentions) | `read_state`; selected-channel summary. The group can exist with no channel and unknown unread state. | `app.read_state.as_ref()` |
| `settings` | [`Option<LocalSettingsSnapshot>`](extension-sdk-reference.md#localsettingssnapshot-reading-preferences) | `local_settings`; current local reading/layout preferences. | `app.settings.as_ref()` |
| `notification_settings` | [`Option<NotificationSettingsSnapshot>`](extension-sdk-reference.md#notificationsettingssnapshot-device-local-notifications) | `notification_settings`; device-local notification preferences, absent without the grant or on older hosts. | `app.notification_settings.as_ref()` |
| `audio_settings` | [`Option<AudioSettingsSnapshot>`](extension-sdk-reference.md#audiosettingssnapshot-device-audio) | `audio_settings`; local gain and effective input processing. Absent without the grant or on older hosts. | `app.audio_settings.as_ref()` |
| `own_presence` | [`Option<OwnPresenceSnapshot>`](extension-sdk-reference.md#ownpresencesnapshot-your-status-and-activity-preference) | `account_control`; your local status and activity-sharing preference. Absent without the grant or on older hosts. | `app.own_presence.as_ref()` |

On disconnect, the collector omits account profile, guilds, channel details,
channel directory, timeline, message details, relationships, channel metadata,
member details, message content, forum data, conversation activity, members and presence. It also removes the selected
channel from context and read state. The account label in context,
settings and independently available voice state may remain. A known inaccessible
channel is not exposed through the selected-channel groups. Active private
conversations remain eligible when the user has access and grants the relevant
read capability.

### AppContextSnapshot: current account and selected chat

These fields need `app_context`. In the reading examples, `context` is the
borrowed group.

| Wire field | SDK Rust / JSON type | Meaning and presence | Reading it |
| --- | --- | --- | --- |
| `connected` | `bool` / boolean | Whether the app's Gateway connection is connected. It is not a guarantee that history is loaded or voice is connected. | `context.connected` |
| `user` | `Option<UserSnapshot>` / object or absent | Current account's user label and ID, when available. | `context.user.as_ref().map(\|user\| user.id.as_str())` |
| `channel` | `Option<ChannelSnapshot>` / object or absent | Selected chat/channel when connected and accessible. Missing on Home, disconnect or known access loss. | `context.channel.as_ref().map(\|channel\| channel.name.as_str())` |

### UserSnapshot and ChannelSnapshot: shared identity objects

These objects inherit the grant and scope of their containing group. For example,
`timeline` grants message-author labels; it does not also require `members`.
Names are labels, not stable identifiers or complete profiles. The desktop
removes control characters, keeps at most 128 UTF-8 bytes, and substitutes
`Unnamed` if the resulting label is blank. The wire validator allows at most
256 bytes; authors should not depend on a fixed display-name length.

| User wire field | SDK Rust / JSON type | Meaning | Reading from `user: &UserSnapshot` |
| --- | --- | --- | --- |
| `id` | `String` / string | Required user or message-author ID. | `user.id.as_str()` |
| `name` | `String` / string | Required host-supplied display label. No avatar, roles, nickname record or credentials accompany it. | `user.name.as_str()` |

| Channel wire field | SDK Rust / JSON type | Meaning | Reading from `channel: &ChannelSnapshot` |
| --- | --- | --- | --- |
| `id` | `String` / string | Required channel ID. | `channel.id.as_str()` |
| `guild_id` | `Option<String>` / string or absent | Server ID when this is a guild channel. Normally absent for direct conversations. | `channel.guild_id.as_deref()` |
| `name` | `String` / string | Required host-supplied channel label. | `channel.name.as_str()` |
| `kind` | `u8` / integer | Service channel-type number carried by the host. Its presence does not mean every operation is supported for that type. | `channel.kind == 1` |

The host recognizes these channel kinds. Keep a fallback for other `u8` values;
the snapshot contract does not reject an otherwise valid unknown kind.

| `kind` | Meaning |
| --- | --- |
| `0` | Server text channel |
| `1` | Direct message |
| `2` | Server voice channel, which can also have a text conversation |
| `3` | Group direct message |
| `4` | Server category |
| `5` | Announcement channel |
| `10` | Announcement-channel thread |
| `11` | Public thread, including a forum/media post |
| `12` | Private thread |
| `13` | Stage channel; shown as unimplemented by the native channel list |
| `14` | Directory channel; shown as unimplemented by the native channel list |
| `15` | Forum container |
| `16` | Media container |

### AccountProfileSnapshot: the current account's loaded profile

Requires `account_profile`, separately from `app_context`. The group is absent
while disconnected or when the current account is unavailable. It never fetches
a profile, and never contains email, phone, credentials, connections or billing.

| Wire field | SDK Rust / JSON type | Meaning and presence | Reading from `account: &AccountProfileSnapshot` |
| --- | --- | --- | --- |
| `user` | `UserSnapshot` / object | Current account ID and label. | `account.user.name.as_str()` |
| `avatar` | `Option<String>` / string or absent | Loaded avatar hash, at most 128 bytes; not image bytes or a URL. Absent when no avatar is known. | `account.avatar.as_deref()` |
| `profile` | `Option<OwnProfileSnapshot>` / object or absent | Loaded own-profile data matching this account; absent while unavailable, limited, loading, awaiting reload or failed. Absence does not mean a blank profile. | `account.profile.as_ref()` |

The nested `OwnProfileSnapshot` uses the same grant:

| Wire field | SDK Rust / JSON type | Meaning and presence | Reading from `profile: &OwnProfileSnapshot` |
| --- | --- | --- | --- |
| `display_name` | `Option<String>` / string or absent | Optional global display name, using shared identity-label sanitization (128 bytes in the collector; 256-byte wire limit). | `profile.display_name.as_deref()` |
| `bio` | `String` / string | Loaded biography, trimmed to 2,048 UTF-8 bytes; controls other than newline/tab removed. Empty means known empty. | `profile.bio.as_str()` |
| `pronouns` | `String` / string | Loaded pronouns, at most 256 UTF-8 bytes; empty means known empty. | `profile.pronouns.as_str()` |

### GuildDirectorySnapshot: loaded joined servers

Requires `guild_directory`. This is a bounded view of already-loaded joined
servers while connected, not server discovery or a permission/member directory.

| Wire field | SDK Rust / JSON type | Meaning | Reading from `guilds: &GuildDirectorySnapshot` |
| --- | --- | --- | --- |
| `items` | `Vec<GuildSnapshot>` / array | Up to 100 distinct loaded joined servers, limited further by bytes. Empty means no eligible loaded rows. | `guilds.items.iter().find(\|guild\| guild.id == "100")` |
| `truncated` | `bool` / boolean | Item or byte limits omitted entries. No stable ordering is promised. | `guilds.truncated` |

| Guild wire field | SDK Rust / JSON type | Meaning | Reading from `guild: &GuildSnapshot` |
| --- | --- | --- | --- |
| `id` | `String` / string | Joined server ID. | `guild.id.as_str()` |
| `name` | `String` / string | Bounded server label, using the shared identity-label rules. | `guild.name.as_str()` |
| `icon` | `Option<String>` / string or absent | Loaded server icon hash, at most 128 bytes; absent when none is known. Not a URL or image bytes. | `guild.icon.as_deref()` |

### ChannelDetailsSnapshot: selected-channel metadata and permissions

Requires `channel_details`, independently of `channel_directory` and `members`.
Only the connected, accessible, fresh selected channel is eligible. Home,
disconnect, stale metadata or access loss omit the group. Permissions describe
the current snapshot; they do not authorize a future action or guarantee success.

| Wire field | SDK Rust / JSON type | Meaning and presence | Reading from `details: &ChannelDetailsSnapshot` |
| --- | --- | --- | --- |
| `channel` | `ChannelSnapshot` / object | Selected channel identity. | `details.channel.name.as_str()` |
| `parent_id` | `Option<String>` / string or absent | Loaded category or parent channel ID, when present and viewable. | `details.parent_id.as_deref()` |
| `position` | `i32` / integer | Loaded channel ordering value; not an index into a complete directory. | `details.position` |
| `last_message_id` | `Option<String>` / string or absent | Loaded last-message ID, supplied only with readable history; absent when unknown or unauthorized. No message content is included. | `details.last_message_id.as_deref()` |
| `message_count` | `Option<u32>` / nonnegative integer or absent | Service-supplied loaded count when available and history is readable; not a computed full-history count. | `details.message_count` |
| `recipients` | `Vec<UserSnapshot>` / array | Up to 32 loaded DM/group-DM recipients; empty for channels without loaded recipients. No member fetch occurs. | `details.recipients.len()` |
| `recipients_truncated` | `bool` / boolean | Known recipient rows were omitted by item/byte limits. | `details.recipients_truncated` |
| `can_send` | `bool` / boolean | Current host permission to send in this channel. | `details.can_send` |
| `can_read_history` | `bool` / boolean | Current host permission to read this channel's message history. | `details.can_read_history` |

For example, a detailed channel invalidation may carry this complete synthetic
input. It is not a message event; the empty recipient list belongs to a guild
channel, and no last-message ID is disclosed without history access:

```json
{
  "action": "on-app",
  "values": {},
  "app_event": "channels",
  "app": {
    "channel_details": {
      "channel": {"id": "100", "guild_id": "200", "name": "general", "kind": 0},
      "position": 0,
      "recipients": [],
      "recipients_truncated": false,
      "can_send": true,
      "can_read_history": false
    }
  }
}
```

Declare `on-app` as `app_event`, `show` as `panel`, and request `app_events`,
`data_events`, `channel_details`. This complete handler reads fresh details for
its foreground panel and leaves background output empty. A returned panel is
shown immediately; there is no host command requiring Apply.

```rust
use serein_extension_sdk::{AppInvocation, AppOutput, Element, Output};

fn handle(input: AppInvocation) -> AppOutput {
    if input.app_event.is_some() || input.message_event.is_some()
        || input.invocation.action != "show"
    {
        return AppOutput::default();
    }
    let text = input.app.as_ref()
        .and_then(|app| app.channel_details.as_ref())
        .map_or_else(|| "Channel details unavailable".into(), |details| {
            format!("{}: sending allowed = {}", details.channel.name, details.can_send)
        });
    AppOutput {
        output: Output { panel: vec![Element::Text { text }], ..Default::default() },
        ..Default::default()
    }
}
serein_extension_sdk::export!(handle);
```

### ChannelMetadataSnapshot: loaded channel settings, threads and permissions

Requires `channel_metadata`, independently of `channel_details`. Only a connected,
fresh, viewable and readable selected **guild** channel is eligible. DMs, Home,
access loss, stale state or insufficient snapshot space omit the group. Nothing
fetches channel settings or post details automatically. In particular, ordinary
channel selection often leaves topic, slowmode and NSFW unknown: they come from
the already-loaded settings record, which also requires current permission to
open that channel's settings.

| Wire field | SDK Rust / JSON type | Meaning and absence | Reading from `metadata: &ChannelMetadataSnapshot` |
| --- | --- | --- | --- |
| `channel_id` | `String` / string | Selected guild channel ID. | `metadata.channel_id.as_str()` |
| `guild_id` | `String` / string | Owning guild ID. | `metadata.guild_id.as_str()` |
| `parent` | `Option<ChannelSnapshot>` / object or absent | Loaded, viewable parent in the same guild; absent when unknown or inaccessible. | `metadata.parent.as_ref()` |
| `category` | `Option<ChannelSnapshot>` / object or absent | Loaded, viewable category in the same guild, found at most two parent links above the selected channel. | `metadata.category.as_ref()` |
| `topic` | `Option<String>` / string or absent | Already-loaded settings topic, at most 2,048 UTF-8 bytes with controls removed except newline/tab. Empty means known empty; absent means unavailable. | `metadata.topic.as_deref()` |
| `topic_truncated` | `bool` / boolean | The loaded topic exceeded the collector's byte limit. | `metadata.topic_truncated` |
| `slowmode_seconds` | `Option<u32>` / nonnegative integer or absent | Loaded settings value, 0 through 21,600 seconds; zero means known disabled, absent means unknown. | `metadata.slowmode_seconds` |
| `nsfw` | `Option<bool>` / boolean or absent | Loaded settings flag; absent differs from false. | `metadata.nsfw` |
| `thread` | `Option<ThreadMetadataSnapshot>` / object or absent | Present for a selected thread kind (10 through 12); its fields may still be unknown. Absent for other channel kinds. | `metadata.thread.as_ref()` |
| `permissions` | `BTreeMap<ChannelPermission, Option<bool>>` / object of boolean-or-null values | Loaded permission decisions, not raw permission bitfields or guarantees that an action can run. `true` allows, `false` denies, `null` or a missing key is unknown. | `metadata.permissions.get(&ChannelPermission::ManageThreads).copied().flatten()` |

`ChannelPermission` keys are `view_channel`, `read_message_history`,
`send_messages`, `send_messages_in_threads`, `attach_files`, `embed_links`,
`add_reactions`, `mention_everyone`, `use_external_emojis`, `use_external_stickers`,
`use_application_commands`, `manage_channels`, `manage_messages`, `manage_roles`,
`manage_threads`, `create_public_threads`, `create_private_threads`, `manage_webhooks`,
`connect`, `speak`, `stream`, `mute_members`, `deafen_members`, `move_members`,
`use_vad`, and `pin_messages`. Rust variants use PascalCase, for example
`SendMessagesInThreads` and `UseVad`. Check optional values before acting on them.

| Thread wire field | SDK Rust / JSON type | Meaning and absence | Reading from `thread: &ThreadMetadataSnapshot` |
| --- | --- | --- | --- |
| `owner_id` | `Option<String>` / string or absent | Already-loaded post owner ID. | `thread.owner_id.as_deref()` |
| `message_count` | `Option<u32>` / nonnegative integer or absent | Loaded channel count, not a full-history count. | `thread.message_count` |
| `archived` | `Option<bool>` / boolean or null | Loaded post archival flag; null means no loaded post decision. | `thread.archived` |
| `locked` | `Option<bool>` / boolean or null | Loaded post lock flag; null differs from false. | `thread.locked` |
| `pinned` | `Option<bool>` / boolean or null | Loaded post pin flag; null differs from false. | `thread.pinned` |

### MemberDetailsSnapshot: loaded guild members and role labels

Requires `member_details`, independently of `members` and `presence`. It uses the
fresh loaded member pane matching the selected fresh, readable guild channel.
It is absent without that pane, on disconnect/access loss, in DMs, or when the
remaining snapshot budget cannot hold it. It never searches members or loads
profiles/roles. The 6-KiB group budget includes item and nested-vector overhead;
the role catalog also has a 2-KiB collector sub-budget. Lists can reach byte limits
before item limits. Other groups can force additional member-row truncation or
omission of the whole group.

| Wire field | SDK Rust / JSON type | Meaning and absence | Reading from `members: &MemberDetailsSnapshot` |
| --- | --- | --- | --- |
| `channel_id` | `String` / string | Selected channel whose member pane supplies the records. | `members.channel_id.as_str()` |
| `guild_id` | `String` / string | Matching guild ID. | `members.guild_id.as_str()` |
| `items` | `Vec<MemberDetailSnapshot>` / array | Up to 20 distinct loaded member records, further limited by bytes. | `members.items.first()` |
| `truncated` | `bool` / boolean | The loaded pane or resource limits leave members partial. | `members.truncated` |
| `roles` | `Option<Vec<MemberRoleSnapshot>>` / array or absent | Up to 32 distinct loaded guild role labels. Absent is unknown; `[]` is a known empty catalog. Not every member role ID must have a label in this partial catalog. | `members.roles.as_ref()` |
| `roles_truncated` | `bool` / boolean | Role catalog records were omitted by limits. | `members.roles_truncated` |

| Member wire field | SDK Rust / JSON type | Meaning and absence | Reading from `member: &MemberDetailSnapshot` |
| --- | --- | --- | --- |
| `user` | `UserSnapshot` / object | Loaded account ID and label. | `member.user.id.as_str()` |
| `nick` | `Option<String>` / string or absent | Loaded guild nickname if supplied. | `member.nick.as_deref()` |
| `display_name` | `String` / string | Nickname, else matching loaded profile's global name, else account label. Shared identity-label sanitization applies. | `member.display_name.as_str()` |
| `role_ids` | `Vec<String>` / array of strings | Up to 32 distinct loaded role IDs; not permission bitfields. | `member.role_ids.len()` |
| `roles_truncated` | `bool` / boolean | Member role IDs were omitted by limits. | `member.roles_truncated` |
| `profile` | `Option<MemberProfileSnapshot>` / object or absent | Only the already-loaded, nonlimited, successful profile matching this user and guild, when not loading. Absence is unknown, not a blank profile. | `member.profile.as_ref()` |

| Role wire field | SDK Rust / JSON type | Meaning | Reading from `role: &MemberRoleSnapshot` |
| --- | --- | --- | --- |
| `id` | `String` / string | Role ID. | `role.id.as_str()` |
| `name` | `String` / string | Shared bounded identity label. | `role.name.as_str()` |
| `color` | `u32` / nonnegative integer | RGB value, 0 through `0xffffff`; no alpha. | `role.color` |
| `position` | `i32` / integer | Loaded role ordering value, not a complete hierarchy. | `role.position` |

| Profile wire field | SDK Rust / JSON type | Meaning and absence | Reading from `profile: &MemberProfileSnapshot` |
| --- | --- | --- | --- |
| `nick` | `Option<String>` / string or absent | Loaded server-profile nickname, using shared label sanitization. | `profile.nick.as_deref()` |
| `avatar` | `Option<String>` / string or absent | Valid loaded guild-avatar hash, at most 128 bytes; no image bytes or URL. | `profile.avatar.as_deref()` |
| `bio` | `String` / string | Loaded guild biography, capped at 1,024 UTF-8 bytes; controls removed except newline/tab. Empty means known empty. | `profile.bio.as_str()` |
| `pronouns` | `String` / string | Loaded pronouns, capped at 256 UTF-8 bytes with controls removed. | `profile.pronouns.as_str()` |
| `joined_at` | `Option<String>` / string or absent | Loaded join-time text, capped at 64 UTF-8 bytes with controls removed; unknown is absent. | `profile.joined_at.as_deref()` |

Nicknames and display/role labels use the collector's 128-byte sanitized label
limit (256-byte wire ceiling). No member credentials, notes, presence payloads,
connected accounts or global biography are included.

This complete synthetic foreground input shows a thread with unknown settings
and lock state plus one loaded member and role. No background observer grant is
needed merely to read the two groups from a foreground panel:

```json
{
  "action": "show",
  "values": {},
  "app": {
    "channel_metadata": {
      "channel_id": "100", "guild_id": "200", "topic_truncated": false,
      "thread": {"owner_id": "300", "message_count": 4,
        "archived": false, "locked": null, "pinned": true},
      "permissions": {"send_messages_in_threads": true, "manage_threads": null}
    },
    "member_details": {
      "channel_id": "100", "guild_id": "200", "items": [{
        "user": {"id": "300", "name": "Example"}, "nick": "Guild nickname",
        "display_name": "Guild nickname", "role_ids": ["400"], "roles_truncated": false,
        "profile": {"bio": "Loaded server bio", "pronouns": "they/them"}
      }],
      "truncated": true,
      "roles": [{"id": "400", "name": "Member", "color": 0, "position": 1}],
      "roles_truncated": false
    }
  }
}
```

The complete [Guild Inspector handler](../examples/extensions/guild-inspector/src/lib.rs)
and [manifest](../examples/extensions/guild-inspector/manifest.json) read these
objects, distinguish unknown from false, and return a bounded panel with the
first five member rows. The panel appears immediately; no host effect or Apply
is involved. Its separate `app_event` action requests `app_events`, `data_events`
and the same read grants, and returns empty output. It stores nothing and does
not fetch missing data.

### ChannelDirectorySnapshot: loaded channel list

Requires `channel_directory`. Visibility does not imply permission to read a
channel's message history, and the directory is not a list of every channel on
Discord. In the examples, `directory` is the borrowed group.

| Wire field | SDK Rust / JSON type | Meaning | Reading it |
| --- | --- | --- | --- |
| `items` | `Vec<ChannelSnapshot>` / array | Up to 100 distinct loaded, visible channel records, further limited by the byte budget. No stable sorting contract is promised. | `directory.items.iter().find(\|channel\| channel.id == "100")` |
| `truncated` | `bool` / boolean | The collector stopped because a list or byte limit was reached. | `directory.truncated` |

### TimelineSnapshot and MessageSnapshot: loaded messages

Requires `timeline`. Messages belong to the selected, fresh, readable
conversation. The host takes up to 50 eligible recent rows from the loaded
window and returns them in timeline order. When the same plugin also has
`message_details`, both text and metadata row limits are 12; its 20-KiB byte budget stays
unchanged. The combined limit reduces text/metadata parsing work under the
unchanged execution budget; valid wire size alone still does not
guarantee that a handler fits its fuel budget. This may be a window around an old
message rather than the latest service history. Deleted and ephemeral text is
excluded even when a separate host feature retains deleted rows.

| Timeline wire field | SDK Rust / JSON type | Meaning | Reading from `timeline: &TimelineSnapshot` |
| --- | --- | --- | --- |
| `channel_id` | `String` / string | Conversation shared by every message in this group. | `timeline.channel_id.as_str()` |
| `messages` | `Vec<MessageSnapshot>` / array | Up to 50 loaded, eligible messages, or 12 when `message_details` is also granted. An empty array is valid. | `timeline.messages.last()` |
| `truncated` | `bool` / boolean | More history may exist, the loaded window has boundaries, or rows were omitted by size/item limits. | `timeline.truncated` |

| Message wire field | SDK Rust / JSON type | Meaning | Reading from `message: &MessageSnapshot` |
| --- | --- | --- | --- |
| `id` | `String` / string | Required message ID; use the containing timeline's `channel_id` for navigation. | `message.id.as_str()` |
| `author` | `UserSnapshot` / object | Required author ID and bounded label. | `message.author.name.as_str()` |
| `content` | `String` / string | Loaded message text, possibly empty. Markdown/mention syntax remains text; this is not rendered HTML or a full message object. | `message.content.chars().count()` |
| `attachment_count` | `u16` / integer | Count of attachment records in the loaded message. No attachment URLs, bytes or names are supplied. | `message.attachment_count > 0` |
| `edited` | `bool` / boolean | Whether the loaded message is marked edited. No timestamp or edit history is provided. | `message.edited` |

The current collector skips a whole message when its content exceeds 4 KiB of
UTF-8 and sets `truncated`; it does not shorten the message. The wire validator
allows up to 16 KiB per message, matching the separate message-event content
ceiling. Do not assume all valid wire-sized messages appear in desktop snapshots.

### MessageContentSnapshot: bounded rich-message summaries

Requires `message_content`, independently of `timeline`/`message_details`. Only
the connected, fresh, viewable/readable selected text conversation is eligible.
Deleted and ephemeral rows are excluded. No embed/media URLs, bytes, referenced
message text, or poll questions/options/results are exposed. Embed text itself
can contain private conversation content: this is a read grant, not public data.

The group holds at most 10 messages / 8 KiB including collector overhead, then
shares the remaining 64-KiB snapshot budget. Nested stickers/embeds share 2 KiB
per message; embed fields share 768 bytes per embed. Rows can be trimmed and
the group omitted when global space is exhausted. Nothing fetches content.

| Group field | SDK Rust / JSON type | Meaning | Reading from `content: &MessageContentSnapshot` |
| --- | --- | --- | --- |
| `channel_id` | `String` / string | Selected conversation. | `content.channel_id.as_str()` |
| `items` | `Vec<RichMessageSnapshot>` / array | Up to 10 loaded summaries in timeline order, possibly fewer by bytes. | `content.items.last()` |
| `truncated` | `bool` / boolean | Loaded history/window or limits leave this list partial. | `content.truncated` |

| Message field | SDK Rust / JSON type | Meaning | Reading from `message: &RichMessageSnapshot` |
| --- | --- | --- | --- |
| `id` | `String` / string | Message ID. | `message.id.as_str()` |
| `embeds` | `Vec<EmbedSummarySnapshot>` / array | At most 3 loaded embed summaries. | `message.embeds.first()` |
| `embeds_truncated` | `bool` / boolean | Embed records omitted by limits. | `message.embeds_truncated` |
| `embeds_suppressed` | `bool` / boolean | Loaded message's suppressed-embed flag. | `message.embeds_suppressed` |
| `stickers` | `Vec<MessageStickerSnapshot>` / array | At most 3 loaded sticker labels. | `message.stickers.len()` |
| `stickers_truncated` | `bool` / boolean | Sticker records were unavailable in the retained payload or omitted by limits/invalid IDs. | `message.stickers_truncated` |
| `reference` | `Option<MessageReferenceSnapshot>` / object or absent | Loaded reply/deleted-reference/forward marker, absent if none is known. No referenced body is supplied. | `message.reference.as_ref()` |
| `poll` | `PollAvailability` / string | `absent` (`Absent`): no retained poll marker; `unsupported` (`Unsupported`): marker present, structured poll data not retained. Neither supplies votes or results. | `message.poll == PollAvailability::Unsupported` |

Embed strings are capped in UTF-8 and stripped of controls except newline/tab.
Optional absent text is unknown/unsupplied; `Some("")` is known empty.

| Embed field | SDK Rust / JSON type | Meaning / limit | Reading from `embed: &EmbedSummarySnapshot` |
| --- | --- | --- | --- |
| `kind` | `String` / string | Loaded embed kind, at most 32 bytes. | `embed.kind.as_str()` |
| `title` | `Option<String>` / string or absent | Title, at most 256 bytes. | `embed.title.as_deref()` |
| `description` | `Option<String>` / string or absent | Description, at most 512 bytes. | `embed.description.as_deref()` |
| `author` | `Option<String>` / string or absent | Author label only, at most 128 bytes. | `embed.author.as_deref()` |
| `footer` | `Option<String>` / string or absent | Footer text only, at most 256 bytes. | `embed.footer.as_deref()` |
| `color` | `Option<u32>` / integer or absent | RGB color, 0 through `0xffffff`. | `embed.color` |
| `fields` | `Vec<EmbedFieldSnapshot>` / array | At most 4 fields, further limited by their 768-byte shared budget. | `embed.fields.len()` |
| `fields_truncated` | `bool` / boolean | Fields omitted by limits. | `embed.fields_truncated` |
| `has_image` | `bool` / boolean | Loaded image metadata exists; no URL/bytes. | `embed.has_image` |
| `has_thumbnail` | `bool` / boolean | Loaded thumbnail metadata exists. | `embed.has_thumbnail` |
| `has_video` | `bool` / boolean | Loaded video metadata exists. | `embed.has_video` |
| `limited` | `bool` / boolean | The retained embed was limited or text exceeded summary bounds. Check `fields_truncated` separately too. | `embed.limited` |

| Nested object | Field | SDK Rust / JSON type | Meaning / limit |
| --- | --- | --- | --- |
| `EmbedFieldSnapshot` | `name` | `String` / string | Field label, at most 128 UTF-8 bytes. |
| `EmbedFieldSnapshot` | `value` | `String` / string | Field text, at most 256 UTF-8 bytes. |
| `EmbedFieldSnapshot` | `inline` | `bool` / boolean | Loaded inline-layout hint. |
| `MessageStickerSnapshot` | `id` | `String` / string | Sticker ID. |
| `MessageStickerSnapshot` | `name` | `String` / string | Shared sanitized sticker label, at most 128 collector bytes. |
| `MessageStickerSnapshot` | `format_type` | `u8` / integer | Loaded service format number; retain an unknown-value fallback. |
| `MessageReferenceSnapshot` | `message_id` | `Option<String>` / string or absent | Referenced message ID when known; may be absent for a forward marker. |
| `MessageReferenceSnapshot` | `deleted` | `bool` / boolean | Loaded reference-deleted flag, not deleted content. |
| `MessageReferenceSnapshot` | `forwarded` | `bool` / boolean | Loaded forward marker, not the forwarded body. |

### ForumDataSnapshot: loaded sibling posts and threads

Requires `forum_data`. A fresh, readable selected guild text/announcement/forum/
media parent (kind 0/5/15/16), or a thread under such a parent, is eligible only
when both selected channel and parent are viewable/readable in the same guild.
No directory/archive fetch occurs. Only accessible, readable child threads
already resident in the channel list are supplied; archive-only rows are not
loaded on behalf of the plugin. At most 10 posts / 6 KiB, further reduced by
remaining global snapshot space. Unsupported selected kinds or access loss omit
the group. Forum tags are not retained by this API and have no fabricated fields.

| Group field | SDK Rust / JSON type | Meaning | Reading from `forum: &ForumDataSnapshot` |
| --- | --- | --- | --- |
| `channel_id` | `String` / string | Selected parent or thread ID. | `forum.channel_id.as_str()` |
| `guild_id` | `String` / string | Matching guild. | `forum.guild_id.as_str()` |
| `parent_id` | `String` / string | Parent whose loaded children are summarized. | `forum.parent_id.as_str()` |
| `posts` | `Vec<ForumPostSnapshot>` / array | At most 10 resident readable child threads, not all service posts. | `forum.posts.len()` |
| `truncated` | `bool` / boolean | Loading/paging state or limits leave the list partial. | `forum.truncated` |

| Post field | SDK Rust / JSON type | Meaning | Reading from `post: &ForumPostSnapshot` |
| --- | --- | --- | --- |
| `id` | `String` / string | Thread/post ID. | `post.id.as_str()` |
| `name` | `String` / string | Shared sanitized label, at most 128 bytes. | `post.name.as_str()` |
| `kind` | `u8` / integer | Thread kind 10, 11 or 12. | `post.kind` |
| `message_count` | `Option<u32>` / integer or null | Loaded count, unknown when null. | `post.message_count` |
| `owner_id` | `Option<String>` / string or absent | Loaded post owner when known. | `post.owner_id.as_deref()` |
| `archived` | `Option<bool>` / boolean or null | Loaded post flag, or true when present in an accessible loaded archive page. Unknown differs from false. | `post.archived` |
| `locked` | `Option<bool>` / boolean or null | Loaded post flag, otherwise unknown. | `post.locked` |
| `pinned` | `Option<bool>` / boolean or null | Loaded post flag, otherwise unknown. | `post.pinned` |
| `followed` | `Option<bool>` / boolean or null | Loaded follow state; not proof of private-thread membership. | `post.followed` |

### ConversationActivitySnapshot: loaded pins and typing indicators

Requires `conversation_activity`. The selected connected, fresh, viewable and
readable conversation is eligible; no pin/history fetch is started. This 2-KiB
group may be omitted when the remaining shared snapshot budget is insufficient.
A reaction event is only an invalidation reason; actual reaction summaries still
use the separate `message_details` grant.

| Wire field | SDK Rust / JSON type | Meaning and absence | Reading from `activity: &ConversationActivitySnapshot` |
| --- | --- | --- | --- |
| `channel_id` | `String` / string | Selected conversation. | `activity.channel_id.as_str()` |
| `typing_user_ids` | `Vec<String>` / array | Up to 8 current, unexpired typing user IDs; empty means no currently retained indicator, not proof nobody is composing. No completeness flag. | `activity.typing_user_ids.len()` |
| `pinned_message_ids` | `Option<Vec<String>>` / array or absent | Up to 20 IDs from the current successfully loaded pin page for this channel. Absent when pins are not loaded, loading or failed; `[]` means known empty page. | `activity.pinned_message_ids.as_ref()` |
| `pins_truncated` | `bool` / boolean | The loaded pin page is partial, has another cursor, or exceeds the ID cap. Even false does not promise a full service inventory. | `activity.pins_truncated` |

This complete synthetic input combines a rich summary with unsupported poll data,
loaded forum flags, typing and unknown pin state. It has no media URLs or poll
results. Public host discovery is omitted from this focused offline fixture:

```json
{
  "action": "show", "values": {},
  "app": {
    "message_content": {"channel_id": "100", "items": [{
      "id": "300", "embeds": [{"kind": "rich", "title": "Synthetic title",
        "fields": [{"name": "Label", "value": "Value", "inline": false}],
        "fields_truncated": false, "has_image": true, "has_thumbnail": false,
        "has_video": false, "limited": false}],
      "embeds_truncated": false, "embeds_suppressed": false,
      "stickers": [], "stickers_truncated": false,
      "reference": {"message_id": "299", "deleted": false, "forwarded": false},
      "poll": "unsupported"
    }], "truncated": false},
    "forum_data": {"channel_id": "100", "guild_id": "200", "parent_id": "100",
      "posts": [{"id": "400", "name": "Loaded post", "kind": 11,
        "message_count": null, "archived": null, "locked": false,
        "pinned": null, "followed": null}], "truncated": true},
    "conversation_activity": {"channel_id": "100", "typing_user_ids": ["500"],
      "pins_truncated": false}
  }
}
```

The complete [Conversation Inspector handler](../examples/extensions/conversation-inspector/src/lib.rs)
and [manifest](../examples/extensions/conversation-inspector/manifest.json) show
these groups and public host support without storage or effects. Its foreground
panel is immediate; its observer returns empty output. Missing groups remain
unavailable instead of being labeled empty. Its synthetic fixtures exercise
loaded and partial states without contacting Discord.

### MessageDetailsSnapshot: loaded replies, mentions, attachments and reactions

Requires `message_details`, independently of `timeline`. The group is absent
without a connected, accessible selected text conversation and fresh readable
history. It copies only loaded nondeleted, nonephemeral rows; no text, embeds,
attachment URLs/bytes or network fetch is included. Its window need not match
`timeline`, since the two groups have different item and byte limits. Nested
mentions, attachments and reactions share a 4-KiB per-message budget, including
item overhead, so their individual limits may be reached earlier.

| Wire field | SDK Rust / JSON type | Meaning | Reading from `details: &MessageDetailsSnapshot` |
| --- | --- | --- | --- |
| `channel_id` | `String` / string | Selected conversation shared by all records. | `details.channel_id.as_str()` |
| `items` | `Vec<MessageDetailSnapshot>` / array | Up to 20 loaded records (12 when `timeline` is also granted) in timeline order, further limited by the 8-KiB group and remaining snapshot budget. | `details.items.last()` |
| `truncated` | `bool` / boolean | The loaded window or resource limits leave the list partial. An empty partial list is valid. | `details.truncated` |

| Record wire field | SDK Rust / JSON type | Meaning | Reading from `message: &MessageDetailSnapshot` |
| --- | --- | --- | --- |
| `id` | `String` / string | Message ID. | `message.id.as_str()` |
| `kind` | `u8` / integer | Loaded service message-type number; preserve an unknown-type fallback. | `message.kind` |
| `reply_to` | `Option<String>` / string or absent | Loaded referenced message ID when supplied. No referenced text is exposed. | `message.reply_to.as_deref()` |
| `mention_ids` | `Vec<String>` / array of strings | Up to 32 explicitly mentioned user IDs; not role IDs or a membership expansion. | `message.mention_ids.len()` |
| `mentions_truncated` | `bool` / boolean | Mention IDs were omitted by limits. | `message.mentions_truncated` |
| `mention_everyone` | `bool` / boolean | Loaded everyone/here mention flag. | `message.mention_everyone` |
| `attachments` | `Vec<AttachmentSnapshot>` / array | Up to 10 loaded attachment labels with distinct valid IDs. Invalid/duplicate IDs are skipped and mark the list partial. No downloads occur. | `message.attachments.first()` |
| `attachments_truncated` | `bool` / boolean | Attachment records were omitted by limits or invalid/duplicate IDs. | `message.attachments_truncated` |
| `reactions` | `Option<Vec<ReactionSnapshot>>` / array or absent | Up to 16 loaded reaction summaries. Absent means unknown; `[]` means known empty. | `message.reactions.as_ref().map(\|items\| items.len())` |
| `reactions_truncated` | `bool` / boolean | Reaction summaries were omitted by limits. | `message.reactions_truncated` |

| Attachment wire field | SDK Rust / JSON type | Meaning | Reading from `attachment: &AttachmentSnapshot` |
| --- | --- | --- | --- |
| `id` | `String` / string | Attachment ID. | `attachment.id.as_str()` |
| `filename` | `String` / string | Filename label capped at 256 UTF-8 bytes, with controls removed and `Attachment` substituted if empty. No local filesystem path or download URL is supplied. | `attachment.filename.as_str()` |
| `size` | `u64` / nonnegative integer | Loaded byte size; not a downloaded size measurement. | `attachment.size` |
| `content_type` | `Option<String>` / string or absent | Loaded content-type label capped at 128 UTF-8 bytes, with controls removed; omitted if empty. Not a file-content guarantee. | `attachment.content_type.as_deref()` |
| `spoiler` | `bool` / boolean | Loaded attachment spoiler flag. | `attachment.spoiler` |

| Reaction wire field | SDK Rust / JSON type | Meaning | Reading from `reaction: &ReactionSnapshot` |
| --- | --- | --- | --- |
| `emoji_id` | `Option<String>` / string or absent | Custom emoji ID; absent for Unicode emoji. | `reaction.emoji_id.as_deref()` |
| `emoji_name` | `Option<String>` / string or absent | Loaded Unicode emoji or custom emoji label, at most 128 UTF-8 bytes. | `reaction.emoji_name.as_deref()` |
| `count` | `u32` / nonnegative integer | Loaded positive reaction count; invalid/zero-count summaries are skipped and marked partial. No reacting-user roster. | `reaction.count` |
| `me` | `bool` / boolean | Current account has an ordinary reaction of this kind. | `reaction.me` |
| `me_burst` | `bool` / boolean | Current account has a burst reaction of this kind. | `reaction.me_burst` |

### RelationshipsSnapshot: loaded account relationships

Requires `relationships`. The connected account's already-loaded lists are
projected without fetching missing rows. The group is absent if none of the
three lists is known. Names and IDs use `UserSnapshot`; no
notes, nicknames, presence, credentials or account connections are supplied.
An absent group is unavailable/ungranted. An empty list alone does not mean the
account has no friends, requests or restricted accounts: inspect the known flags
and `truncated` before drawing a conclusion.

| Wire field | SDK Rust / JSON type | Meaning | Reading from `relationships: &RelationshipsSnapshot` |
| --- | --- | --- | --- |
| `items` | `Vec<RelationshipSnapshot>` / array | Up to 100 loaded records, further limited by 4 KiB and the remaining snapshot budget. | `relationships.items.len()` |
| `truncated` | `bool` / boolean | Item/byte limits omitted records. Unloaded categories are reported separately by the known flags, not this flag. | `relationships.truncated` |
| `friends_known` | `bool` / boolean | The host knows the friend list; `false` means unavailable/unloaded. | `relationships.friends_known` |
| `requests_known` | `bool` / boolean | The host knows incoming/outgoing request lists. | `relationships.requests_known` |
| `restricted_known` | `bool` / boolean | The host knows blocked/ignored lists. | `relationships.restricted_known` |

| Record wire field | SDK Rust / JSON type | Meaning | Reading from `relationship: &RelationshipSnapshot` |
| --- | --- | --- | --- |
| `user` | `UserSnapshot` / object | Related account's ID and bounded label. | `relationship.user.name.as_str()` |
| `kind` | `RelationshipKind` / string | `friend`, `incoming_request`, `outgoing_request`, `blocked`, or `ignored`; Rust variants `Friend`, `IncomingRequest`, `OutgoingRequest`, `Blocked`, `Ignored`. | `relationship.kind == RelationshipKind::Friend` |

This complete synthetic input contains a known friend list and unavailable
request/restricted lists. No event observer grant is needed for a foreground
panel that reads these groups:

```json
{
  "action": "show",
  "values": {},
  "app": {
    "message_details": {
      "channel_id": "100",
      "items": [{
        "id": "200", "kind": 0, "mention_ids": [], "mentions_truncated": false,
        "mention_everyone": false, "attachments": [], "attachments_truncated": false,
        "reactions": [], "reactions_truncated": false
      }],
      "truncated": false
    },
    "relationships": {
      "items": [{"user": {"id": "300", "name": "Example"}, "kind": "friend"}],
      "truncated": false, "friends_known": true,
      "requests_known": false, "restricted_known": false
    }
  }
}
```

Declare `show` as `panel` and request `message_details` and `relationships`.
This complete handler displays available row counts immediately, without storage
or a host command requiring Apply. To observe invalidations separately, add an
`app_event` action plus `app_events` and `data_events`; keep its output passive.

```rust
use serein_extension_sdk::{AppInvocation, AppOutput, Element, Output};

fn handle(input: AppInvocation) -> AppOutput {
    if input.app_event.is_some() || input.message_event.is_some()
        || input.invocation.action != "show"
    {
        return AppOutput::default();
    }
    let mut panel = Vec::new();
    if let Some(app) = input.app {
        if let Some(details) = app.message_details {
            panel.push(Element::Text { text: format!("Loaded message details: {}{}",
                details.items.len(), if details.truncated { " (partial)" } else { "" }) });
        }
        if let Some(relationships) = app.relationships {
            panel.push(Element::Text { text: format!("Loaded relationships: {}{}",
                relationships.items.len(), if relationships.truncated { " (partial)" } else { "" }) });
        }
    }
    if panel.is_empty() {
        panel.push(Element::Text { text: "Requested data is unavailable.".into() });
    }
    AppOutput { output: Output { panel, ..Default::default() }, ..Default::default() }
}
serein_extension_sdk::export!(handle);
```

### MembersSnapshot: loaded people in this conversation

Requires `members`. Guild entries come from the fresh, loaded member-list rows
for the selected channel. Direct conversations use their loaded recipients when
no such member list is present. This is not a server member search or full roster.

| Wire field | SDK Rust / JSON type | Meaning | Reading from `members: &MembersSnapshot` |
| --- | --- | --- | --- |
| `channel_id` | `String` / string | Selected conversation to which this list belongs. | `members.channel_id.as_str()` |
| `items` | `Vec<UserSnapshot>` / array | Up to 100 distinct known user labels and IDs. No member roles or permission records are included. | `members.items.iter().map(\|user\| user.name.as_str())` |
| `truncated` | `bool` / boolean | Loaded rows do not cover the host's known member count, or a size/item limit was reached. | `members.truncated` |

### PresenceSnapshot and PresenceEntry: known status only

Requires `presence`, independently of `members`. Entries are drawn from the
selected fresh member-list scope or known statuses of loaded DM recipients.
No custom status text, activities or desktop/mobile session details are exposed.
A user with no entry has unknown/unsupplied presence, not necessarily offline.

| Group wire field | SDK Rust / JSON type | Meaning | Reading from `presence: &PresenceSnapshot` |
| --- | --- | --- | --- |
| `items` | `Vec<PresenceEntry>` / array | Up to 100 distinct user/status pairs. Users with unknown status are omitted. | `presence.items.iter().find(\|entry\| entry.user_id == "300")` |
| `truncated` | `bool` / boolean | Entries may be incomplete because of member coverage or size/item limits. `false` still does not prove every person's status is known. | `presence.truncated` |

| Entry wire field | SDK Rust / JSON type | Meaning | Reading from `entry: &PresenceEntry` |
| --- | --- | --- | --- |
| `user_id` | `String` / string | User whose known status is reported. | `entry.user_id.as_str()` |
| `status` | `String` / string | Current producer values: `online`, `idle`, `dnd` (Do Not Disturb), or `offline`. Bounded to 32 bytes; handle future strings without failing. | `entry.status == "online"` |

### VoiceSnapshot: the current call

Requires `voice_state`. This is the current active call, not a directory of calls
or everyone in the selected server. An inaccessible or absent call produces an
idle summary: no `channel_id`, phase `idle`, false flags and no participants.

| Wire field | SDK Rust / JSON type | Meaning | Reading from `voice: &VoiceSnapshot` |
| --- | --- | --- | --- |
| `channel_id` | `Option<String>` / string or absent | Channel of the accessible active call; may differ from the selected chat. | `voice.channel_id.as_deref()` |
| `phase` | `String` / string | Host call phase from the table below, at most 64 bytes. Keep an unknown-value fallback. | `voice.phase == "connected"` |
| `muted` | `bool` / boolean | Current account's call mute state. Not a claim about every participant or measured microphone activity. | `voice.muted` |
| `deafened` | `bool` / boolean | Current account's call deafen state. | `voice.deafened` |
| `camera` | `bool` / boolean | Current account's call camera flag. No frames or device details are provided. | `voice.camera` |
| `streaming` | `bool` / boolean | Local screen sharing is busy: starting, active, stopping or retiring. It is not proof that frames are currently being transmitted. | `voice.streaming` |
| `participants` | `Vec<String>` / array of ID strings | At most 64 tracked participant IDs. No per-user voice flags or media are supplied. There is no `truncated` flag, so treat this as a bounded roster. | `voice.participants.len()` |

| `phase` | Meaning in the host |
| --- | --- |
| `idle` | No accessible active call is exposed. |
| `connecting` | Starting the call connection. |
| `connecting_transport` | Connecting to the voice server. |
| `discovering` | Checking the voice network. |
| `opening_audio` | Opening audio devices. |
| `ringing` | The call is ringing. |
| `securing` | Securing the call's audio. |
| `connected` | The voice connection is established. |
| `waiting` | Connected and waiting for others. |
| `failed` | The current call failed. |

### ReadSnapshot: unread and mentions

Requires `read_state`. Read the optional channel and optional unread value before
displaying a conclusion. In particular, unknown does not mean caught up.

| Wire field | SDK Rust / JSON type | Meaning | Reading from `read: &ReadSnapshot` |
| --- | --- | --- | --- |
| `channel_id` | `Option<String>` / string or absent | Selected channel when connected and accessible; otherwise absent. | `read.channel_id.as_deref()` |
| `unread` | `Option<bool>` / boolean or null | `Some(true)`: known unread; `Some(false)`: known read; `None`: unknown or unavailable. | `match read.unread { Some(true) => "Unread", Some(false) => "Read", None => "Unknown" }` |
| `mentions` | `u32` / nonnegative integer | Host's current mention count for the selected channel. Zero when no channel is available; not a list of mentions or a count of all messages. | `read.mentions` |

### LocalSettingsSnapshot: reading preferences

Requires `local_settings`. These are current local values, not a general settings
object or a guarantee that the latest change has been saved to disk. Changing
them requires a separate `set_local_settings` proposal and the user's Apply.

| Wire field | SDK Rust / JSON type | Meaning and range | Reading from `settings: &LocalSettingsSnapshot` |
| --- | --- | --- | --- |
| `zoom_percent` | `u16` / integer | App zoom percentage, 80 through 150 inclusive; `100` is normal zoom. | `settings.zoom_percent` |
| `sidebar_width` | `u16` / integer | Preferred channel/conversation sidebar width, 190 through 360 logical pixels. A narrow window can constrain actual width. | `settings.sidebar_width` |
| `show_members` | `bool` / boolean | Keep the People/member list visible when the window is wide enough. Does not force a panel into a narrow window. | `settings.show_members` |
| `animate_gifs` | `bool` / boolean | Automatically animate visible GIFs. | `settings.animate_gifs` |
| `hide_media_links` | `bool` / boolean | Hide standalone image/GIF links when their media preview is displayed. It does not hide the image itself. | `settings.hide_media_links` |
| `smooth_scrolling` | `Option<bool>` / boolean or absent | Animate scrolling; absent on older hosts, distinct from disabled. | `settings.smooth_scrolling` |
| `scroll_speed_percent` | `Option<u16>` / integer or absent | Scroll speed, 25 through 300 inclusive; `100` is normal speed. Absent on older hosts. | `settings.scroll_speed_percent` |

Current hosts supply both scrolling fields. The SDK can decode older JSON
snapshots without them; use `None` to show unavailable controls. Adding fields
is not Rust struct-literal source compatibility: use the current fields when
constructing a snapshot. See [reading patches](extension-sdk-actions.md#change-local-reading-settings).

### AudioSettingsSnapshot: device audio

The `audio_settings` grant allows reading these device preferences and proposing
changes through `AppAction::SetAudioSettings`. No device enumeration, microphone
test or raw audio is exposed. Fields describe the **effective** processing preset;
editing a processing field switches to Custom through the native settings path.

| Wire field | Rust / JSON type | Meaning |
| --- | --- | --- |
| `input_percent`, `output_percent` | `u16` / integer | Input/output gain, 0 through 200. |
| `push_to_talk` | `bool` / boolean | Whether push to talk is enabled. |
| `input_profile` | `String` / string | `voice_isolation`, `studio`, or `custom`. |
| `suppression` | `String` / string | `off`, `rnnoise`, or `webrtc`. |
| `suppression_level` | `u8` / integer | Suppression strength, 0 through 3. |
| `echo_cancellation`, `automatic_gain` | `bool` / boolean | Effective processing options. |
| `sensitivity_db` | `Option<i16>` / integer or null | Threshold from -80 through 0 dBFS; null means open microphone. |

Read `input.app.as_ref().and_then(|app| app.audio_settings.as_ref())` before
accessing the fields. The group is absent on an older host or without its grant;
an absent group does not imply default audio settings. Changes invalidate the
existing `settings` app event when subscribed with `app_events`.

### OwnPresenceSnapshot: your status and activity preference

The `account_control` grant allows reading this group and proposing own-account
changes. This is the current local choice, not proof that Discord has accepted
or publicly displayed it. It contains no detected process names or activity list.

| Wire field | Rust / JSON type | Meaning |
| --- | --- | --- |
| `status` | `String` / string | `online`, `idle`, `dnd`, or `invisible`. |
| `custom_status` | `String` / string | Your status text, at most 128 characters/512 UTF-8 bytes; empty means none. |
| `expires_at_ms` | `Option<u64>` / integer or absent | Local Unix expiry in milliseconds; absent means no expiry. |
| `share_game_activity` | `bool` / boolean | Whether local detected-game activity sharing is enabled. This is separate from the server-side account setting. |

Read `input.app.as_ref().and_then(|app| app.own_presence.as_ref())`. As with
audio settings, this optional group is absent without its grant or on older
hosts. Local changes invalidate the existing `settings` event; no new background
write surface is introduced.

This complete synthetic input shows both groups:

```json
{
  "action": "show",
  "app": {
    "audio_settings": {
      "input_percent": 100, "output_percent": 100, "push_to_talk": false,
      "input_profile": "voice_isolation", "suppression": "rnnoise",
      "suppression_level": 2, "echo_cancellation": true,
      "automatic_gain": true, "sensitivity_db": -55
    },
    "own_presence": {
      "status": "online", "custom_status": "Reviewing a release",
      "share_game_activity": false
    }
  }
}
```

### NotificationSettingsSnapshot: device-local notifications

Requires the separate `notification_settings` grant. Read
`app.notification_settings.as_ref()` before inspecting fields: the whole group
is absent without consent and on older hosts, not a set of disabled values.
These are current device-local preferences, not Discord account, server or
channel notification settings. They contain no cue audio or notification content.
They share the overall 64-KiB snapshot limit and do not fetch data.

| Wire field | SDK Rust / JSON type | Meaning | Reading from `settings: &NotificationSettingsSnapshot` |
| --- | --- | --- | --- |
| `new_message` | `bool` / boolean | Enable the new-message sound. | `settings.new_message` |
| `current_channel` | `bool` / boolean | Enable the current-channel message sound. | `settings.current_channel` |
| `incoming_ring` | `bool` / boolean | Enable incoming-call ringing. | `settings.incoming_ring` |
| `outgoing_ring` | `bool` / boolean | Enable outgoing-call ringing. | `settings.outgoing_ring` |
| `disable_sounds` | `bool` / boolean | Master sound disable; preserves individual cue choices. | `settings.disable_sounds` |
| `unread_badge` | `bool` / boolean | Enable the unread badge. | `settings.unread_badge` |
| `mute` | `bool` / boolean | Enable the mute cue. | `settings.mute` |
| `unmute` | `bool` / boolean | Enable the unmute cue. | `settings.unmute` |
| `deafen` | `bool` / boolean | Enable the deafen cue. | `settings.deafen` |
| `undeafen` | `bool` / boolean | Enable the undeafen cue. | `settings.undeafen` |
| `camera_on` | `bool` / boolean | Enable the camera-on cue. | `settings.camera_on` |
| `screen_share_on` | `bool` / boolean | Enable the screen-share-on cue. | `settings.screen_share_on` |
| `user_join` | `bool` / boolean | Enable the participant-join cue. | `settings.user_join` |
| `user_leave` | `bool` / boolean | Enable the participant-leave cue. | `settings.user_leave` |
| `volume` | `u8` / integer | Sound volume, 0 through 100 inclusive; zero silences sounds. | `settings.volume` |

Changing values requires a [`set_notification_settings` proposal and Apply](extension-sdk-actions.md#change-device-local-notification-settings).
A snapshot is not proof that the latest change has reached disk. The `settings`
app event invalidates both reading and notification snapshots; declare `app_events`
to observe it and retain the respective grant to receive each group. Older hosts
reject manifests requesting the new capability before executing a handler.

### Example: read a snapshot without confusing unknown with zero

Declare `show` as a `panel` action and request `app_context`, `timeline` and
`read_state`. This handler displays only bounded summaries. It does not save
conversation content, fetch anything or propose a host action. The event guard
also makes it safe if you later add an `app_event` action.

```rust
use serein_extension_sdk::{AppInvocation, AppOutput, Element, Output};

fn handle(input: AppInvocation) -> AppOutput {
    if input.app_event.is_some() || input.message_event.is_some()
        || input.invocation.action != "show"
    {
        return AppOutput::default();
    }
    let text = match input.app.as_ref() {
        None => "App data is unavailable.".into(),
        Some(app) => {
            let channel = app.context.as_ref()
                .and_then(|context| context.channel.as_ref())
                .map_or("No accessible chat selected", |channel| channel.name.as_str());
            let messages = match app.timeline.as_ref() {
                Some(timeline) => format!("{} loaded messages{}",
                    timeline.messages.len(),
                    if timeline.truncated { " (partial)" } else { "" }),
                None => "Timeline unavailable".into(),
            };
            let unread = match app.read_state.as_ref().and_then(|read| read.unread) {
                Some(true) => "Unread",
                Some(false) => "Read",
                None => "Read state unknown",
            };
            format!("{channel}\n{messages}\n{unread}")
        }
    };
    AppOutput {
        output: Output { panel: vec![Element::Text { text }], ..Default::default() },
        ..Default::default()
    }
}

serein_extension_sdk::export!(handle);
```

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
| `message_event` | `Option<MessageEvent>` / object or absent | Live message change. Available in `EventInvocation` and `AppInvocation`; requires `message_events` and the `message_event` surface. | `input.message_event.as_ref()` |
| `app` | `Option<AppSnapshot>` / object or absent | Independently granted [app data](extension-sdk-reference.md#app-data). Available in `AppInvocation`. The desktop supplies snapshots to foreground actions and app events; activation and message events do not currently receive them. | `input.app.as_ref().and_then(\|app\| app.timeline.as_ref())` |
| `app_event` | `Option<AppEventKind>` / string or absent | Why the host scheduled an app observer. Requires `app_events` and the `app_event` surface. Available in `AppInvocation`. | `input.app_event == Some(AppEventKind::Connection)` |

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
| `settings` | `AppEventKind::Settings` | One of the five exposed local reading preferences changed. |

Treat the event as a reason to inspect the supplied snapshot, not as a complete
change log. Pending app changes are coalesced per plugin; the snapshot is taken
when the call is dispatched, so it can reflect several changes. There is no
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
budgets including item overhead: 12 KiB for channels, 24 KiB for timeline, and
8 KiB each for members and presence. A list may reach its byte budget before its
item limit. These limits do not guarantee that every handler fits the sandbox's
fuel budget; parsing and your own processing also consume fuel.

### AppSnapshot: choose the group you need

For the examples below, `app` is a borrowed `AppSnapshot`. Each field is an
`Option<T>` in Rust and an object when present in JSON.

| Wire field | SDK Rust type | Required capability and availability | Reading it |
| --- | --- | --- | --- |
| `context` | `Option<AppContextSnapshot>` | `app_context`; current account and connection summary. Can remain available while disconnected. | `app.context.as_ref()` |
| `channels` | `Option<ChannelDirectorySnapshot>` | `channel_directory`; Gateway connected. Only loaded channels the user can view; a selected channel known to be unavailable is excluded. | `app.channels.as_ref().map(\|group\| group.items.len())` |
| `timeline` | `Option<TimelineSnapshot>` | `timeline`; selected text-capable conversation, connected, readable history and fresh timeline. | `app.timeline.as_ref()` |
| `members` | `Option<MembersSnapshot>` | `members`; connected, accessible selected conversation with a fresh loaded member list, or loaded DM/group-DM recipients. | `app.members.as_ref()` |
| `presence` | `Option<PresenceSnapshot>` | `presence`; the same selected member/recipient scope, using only known status entries. | `app.presence.as_ref()` |
| `voice` | `Option<VoiceSnapshot>` | `voice_state`; current call summary, or an idle summary when there is no accessible active call. The call may be in a different channel from the selected chat. | `app.voice.as_ref()` |
| `read_state` | `Option<ReadSnapshot>` | `read_state`; selected-channel summary. The group can exist with no channel and unknown unread state. | `app.read_state.as_ref()` |
| `settings` | `Option<LocalSettingsSnapshot>` | `local_settings`; the five current local reading/layout preferences. | `app.settings.as_ref()` |

On disconnect, the collector omits the directory, timeline, members and presence.
It also removes the selected channel from context and read state. Account,
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
window and returns them in timeline order. This may be a window around an old
message rather than the latest service history. Deleted and ephemeral text is
excluded even when a separate host feature retains deleted rows.

| Timeline wire field | SDK Rust / JSON type | Meaning | Reading from `timeline: &TimelineSnapshot` |
| --- | --- | --- | --- |
| `channel_id` | `String` / string | Conversation shared by every message in this group. | `timeline.channel_id.as_str()` |
| `messages` | `Vec<MessageSnapshot>` / array | Up to 50 loaded, eligible messages. An empty array is valid. | `timeline.messages.last()` |
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

### LocalSettingsSnapshot: five reading preferences

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

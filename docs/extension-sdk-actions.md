# Extension SDK actions and panels

## Outputs and host actions

A handler returns a result describing what Serein should display or do. Some
fields update the plugin's local state immediately; a draft change or app action
is a proposal that the user must approve. Capability consent and **Apply** are
separate steps.

This reference describes the SDK in this source revision. App capabilities need
a supporting host; older hosts reject an unsupported manifest even when its
`api_version` is `1`. The original `Invocation` and `Output` remain supported.

### Choose the Rust result type

Use `Output` for panels, draft proposals, storage and appearance. Use `AppOutput`
when returning a `HostEffect`. Its `output` field contains the original `Output`,
and its `effects` field contains the app proposal. For an `AppInvocation`, panel
values and saved storage are under `input.invocation`.

This complete handler proposes opening General settings. Declare a `panel`
action named `open-settings` and the `navigation` capability in its manifest.

```rust
use serein_extension_sdk::{AppInvocation, AppOutput, AppView, HostEffect};

fn handle(input: AppInvocation) -> AppOutput {
    if input.app_event.is_some() || input.message_event.is_some() {
        return AppOutput::default();
    }
    match input.invocation.action.as_str() {
        "open-settings" => AppOutput {
            effects: vec![HostEffect::OpenView { view: AppView::Settings }],
            ..Default::default()
        },
        _ => AppOutput::default(),
    }
}

serein_extension_sdk::export!(handle);
```

The Rust wrappers do **not** add JSON nesting. The SDK serializes that result as
one flat object:

```json
{
  "replacement": null,
  "panel": [],
  "storage": null,
  "effects": [{"type": "open_view", "view": "settings"}]
}
```

There is no `"output"` property on the wire. Likewise, input fields such as
`action` and `values` are top-level JSON properties, not an `"invocation"` object.
The complete serialized response must fit 256 KiB, including JSON escaping.
The host rejects unknown output fields and invalid field types.

### Every output field

The Rust paths below are relative to `AppOutput`. When using the original
`Output` directly, omit the `output.` prefix.

| JSON field / Rust field | Value and required capability | When it takes effect |
| --- | --- | --- |
| `replacement` / `output.replacement` | Optional string; `composer`. The invocation must actually contain composer text. | **Apply to Draft** replaces the original, still-unchanged draft. It never sends a message. |
| `panel` / `output.panel` | Array of native `Element` values; no separate panel capability. | A foreground result displays it. Message/app event handlers must return no elements. Activation does not display returned panels. |
| `storage` / `output.storage` | Optional opaque UTF-8 string; `storage`. | A valid foreground or event result replaces the plugin's saved value before result approval. Activation can read storage but this build does not persist its returned storage. |
| `appearance` / `output.appearance` | Optional `Theme` object; `appearance`. | An accepted result updates the plugin's appearance overlay immediately, including activation and event results. No Apply button is involved. |
| `preserve_deleted_messages` / `output.preserve_deleted_messages` | Boolean, default `false`; `deleted_messages` is required for `true`. | Only an `activation` action may enable host retention of already-loaded deleted messages. |
| `image_sharing` / `output.image_sharing` | Boolean, default `false`; `image_sharing` is required for `true`. | Only an `activation` action may enable the host's emoji/sticker image attachment mode. Enabling it does not send anything. |
| `effects` / `effects` | Array containing at most one `HostEffect`; its capability is checked separately. | The native result describes the action. It runs only after its **Apply** button is clicked and current access is rechecked. |

Closing a result discards its pending app/draft proposal. It does not undo
storage or appearance updates already accepted from that invocation. A panel's
own button runs another handler; it is different from the host's Apply button.

### Missing, null and empty values

| Field | Omitted or `null` | Explicit empty value |
| --- | --- | --- |
| `replacement` | No draft proposal. | `""` proposes clearing the draft; Apply is still required. |
| `storage` | Leave the saved value unchanged. | `""` saves an empty string. It does not remove the value and is not valid JSON for `storage_json`. |
| `appearance` | Leave this plugin's current overlay unchanged. | `{}` removes this plugin's overrides, exposing the underlying theme and other overlays. |
| `panel` | Omitted means `[]`; `null` is invalid. | `[]` supplies no panel elements; it is not a command to close a foreground result. |
| `effects` | Omitted means `[]`; `null` is invalid. | `[]` makes no app proposal. |
| Activation booleans | Omitted means `false`; `null` is invalid. | `false` does not enable the feature in an activation result. These are not runtime toggle commands for other surfaces. |

Every returned appearance object replaces the previous object from that plugin;
it is not a patch to the plugin's old overlay. Omitted theme fields inherit from
the underlying appearance. To save choices across account loads, store them in
a normal action and restore the overlay from activation's storage input. See the
[theme API](theme-api.md) for palette and style fields.

A complete immediate appearance result with the `appearance` grant is:

```json
{"appearance":{"dark":{"colors":{"accent":"#55CBD7"}}}}
```

Draft proposals must also satisfy the client's ordinary draft rules at Apply:
at most 2,000 characters and the account's bounded draft budget. If the account,
conversation or original draft changed, rerun the composer action. A subsequent
panel-button invocation does not receive the previous composer text and cannot
return a replacement just because its manifest also requests `composer`.

### Rules for every host action

Only foreground `message`, `composer` and `panel` actions may return `effects`.
An `activation`, `message_event` or `app_event` action cannot return them. Return
at most one proposal, at most 8 KiB for the serialized `effects` array, and do not
combine a proposal with a non-null `replacement` in the same result.

The user sees the actual operation and values before applying it. Apply rechecks
that the plugin is enabled, its capability is still granted, the account and
originating conversation are unchanged, and that conversation is accessible.
Each operation also has its own checks below. A stale proposal fails instead of
following a later navigation or controlling a replacement call.

All examples below are complete JSON output objects. Other output fields are
optional. IDs such as `"20"` are illustrative: use real IDs from granted input.
Channel, message and user IDs are strings of decimal digits representing nonzero
`u64` values, at most 20 bytes, not JSON numbers or names.

### Open conversations, profiles and search

These six action types require `navigation`. Read capabilities remain separate:
request `channel_directory`, `app_context` or another read grant only when your
handler needs the corresponding input data.

| JSON `type` / Rust variant | Required fields | What Apply does |
| --- | --- | --- |
| `navigate` / `Navigate` | `channel_id`: string ID. | Opens a channel already known and readable in this session, using normal navigation. |
| `home` / `Home` | None. | Opens Friends/Home and clears the current conversation selection. |
| `open_view` / `OpenView` | `view`: one `AppView` string from the table below. | Opens the named native view. |
| `open_profile` / `OpenProfile` | `user_id`: string ID. | Opens your profile, a known friend's profile, or a user known in the readable current conversation. Arbitrary unknown users are rejected. |
| `jump_to_message` / `JumpToMessage` | `channel_id` and `message_id`: string IDs. | Uses normal message navigation in a known readable text channel. Requires a connected session; a known-deleted target is unavailable. |
| `search` / `Search` | `query`: nonblank search string, at most 256 UTF-8 bytes, without control characters. | Runs the normal search in the current readable conversation when connected. Normal search syntax validation also applies. |

Open a known channel:

```json
{"effects":[{"type":"navigate","channel_id":"20"}]}
```

Return Home:

```json
{"effects":[{"type":"home"}]}
```

Open Appearance settings:

```json
{"effects":[{"type":"open_view","view":"appearance"}]}
```

Open a known user's profile:

```json
{"effects":[{"type":"open_profile","user_id":"300"}]}
```

Jump to a message:

```json
{"effects":[{"type":"jump_to_message","channel_id":"20","message_id":"200"}]}
```

Search the active conversation:

```json
{"effects":[{"type":"search","query":"release notes"}]}
```

Navigation, profile and search views may load ordinary service data after Apply.
They do not give Wasm a network API or a search-results callback. Channel/Home
navigation also respects the native guard for unfinished server-settings edits.

### All 18 AppView values

Every row uses `open_view` and requires `navigation` plus a valid proposal context.
The thirteen settings destinations simply open a page; they do not change its
settings. Other views have the extra conditions described here.

| JSON value / Rust variant | Native destination | Use conditions or effect |
| --- | --- | --- |
| `friends` / `Friends` | Friends / Home | Same behavior as `home`. |
| `search` / `Search` | Conversation search | A readable active text conversation and connected session; opens the search controls, preserving an existing query for that conversation. |
| `pins` / `Pins` | Pinned messages | A readable active text conversation and connected session; requests its pins. |
| `members` / `Members` | People / member list | A readable active text conversation; opens the list, enables its wide-window visibility preference and uses normal member loading when available. |
| `threads` / `Threads` | Threads / archived posts | An accessible selected guild text, announcement, forum or media parent channel; requires connection and history access. It cannot browse an arbitrary unselected parent. |
| `settings` / `Settings` | General | Opens General settings. |
| `account` / `Account` | My Account | Opens the current account's page. |
| `profile_settings` / `ProfileSettings` | Profile | Opens your profile editor; does not submit edits. |
| `appearance` / `Appearance` | Appearance | Opens appearance and reading controls. |
| `messaging_permissions` / `MessagingPermissions` | Messaging Permissions | Opens messaging privacy controls. |
| `notifications` / `Notifications` | Notifications | Opens notification preferences. |
| `activity` / `Activity` | Game Activity | Opens activity settings. |
| `voice_settings` / `VoiceSettings` | Voice & Video | Opens device and voice settings; does not join a call or start media. |
| `keybinds` / `Keybinds` | Keybinds | Opens keyboard shortcuts. |
| `storage` / `Storage` | Data & Privacy | Opens local storage controls; does not clear data. |
| `updates` / `Updates` | Updates | Opens update settings; does not install an update. |
| `extensions` / `Extensions` | Extensions | Opens the plugin shop and installed plugins. |
| `themes` / `Themes` | Themes | Opens the theme shop and installed themes. |

`profile_settings` edits your own profile. `open_profile` instead opens a known
user's profile card. The view named `storage` opens app settings; it is unrelated
to the plugin `storage` output field.

### Show a local notice or copy text

| JSON `type` / Rust variant | Required fields | Capability and behavior |
| --- | --- | --- |
| `notice` / `Notice` | `text`: nonblank string, at most 1,024 UTF-8 bytes. | `local_notices`; shows an in-app toast prefixed by the plugin name after Apply. It is not an operating-system notification. |
| `copy_text` / `CopyText` | `text`: string, at most 4,096 UTF-8 bytes; empty is allowed. | `clipboard_write`; replaces clipboard text after Apply. It never reads the clipboard. |

```json
{"effects":[{"type":"notice","text":"Your local review is ready."}]}
```

```json
{"effects":[{"type":"copy_text","text":"Synthetic meeting notes\nReview the release checklist."}]}
```

These are alternatives: two effects in one output are invalid. Ask the user to
run a separate action for each operation.

### Control the current call

Both action types require `voice_control`. The optional `voice_state` read grant
lets a handler inspect the call first, but does not authorize changing it.

| JSON `type` / Rust variant | Required fields | What Apply does |
| --- | --- | --- |
| `set_voice` / `SetVoice` | `muted`: boolean; `deafened`: boolean. Both must be supplied. | Sets the current call's local mute and deafen choices through the ordinary call controls. |
| `leave_voice` / `LeaveVoice` | None. | Leaves the same call that was active when this invocation was created. |

Mute without deafening:

```json
{"effects":[{"type":"set_voice","muted":true,"deafened":false}]}
```

Leave that call:

```json
{"effects":[{"type":"leave_voice"}]}
```

`muted: true` stops microphone transmission. `deafened: true` deafens the call and
also mutes transmission, even if `muted` is `false`. `muted` remains the separate
self-mute choice: to undeafen while staying muted, send `true, false`; to request
both off, send `false, false`. These are explicit values, not toggles. Normal
call availability and speaking permissions still apply; the host can retain a
required mute and rejects an unavailable unmute.

The host records both the call channel and its local request identity **before**
running the plugin. If the call ends, switches channels, or is replaced by a new
call on the same channel, Apply rejects the old proposal. The plugin cannot
choose or forge that identity. There must already be a call: neither action can
join a channel, start a call, enable a camera, share a screen or record media.

### Change local reading settings

`set_local_settings` / `HostEffect::SetLocalSettings` requires `local_settings`.
Its required `settings` object is a `LocalSettingsPatch`: include only the fields
you want to change. Apply reads the other preferences at that moment and preserves
them.

| Optional field | Type and allowed values | Meaning |
| --- | --- | --- |
| `zoom_percent` | Integer, 80 through 150 inclusive. | App zoom percentage. |
| `sidebar_width` | Integer, 190 through 360 inclusive. | Channel/conversation sidebar width in logical pixels, before display scaling. |
| `show_members` | Boolean. | Keep the People/member list open in wide windows. |
| `animate_gifs` | Boolean. | Allow visible chat GIFs to animate automatically. |
| `hide_media_links` | Boolean. | Hide standalone image/GIF links when their preview is shown. |
| `smooth_scrolling` | Boolean. | Animate scrolling. |
| `scroll_speed_percent` | Integer, 25 through 300 inclusive. | Scroll speed percentage; 100 is normal. |

```json
{
  "effects": [{
    "type": "set_local_settings",
    "settings": {"zoom_percent": 110, "animate_gifs": false, "smooth_scrolling": true, "scroll_speed_percent": 125}
  }]
}
```

An omitted or `null` patch field means unchanged. At least one field must contain
a value: `{}` or an all-null patch is invalid. `false` is a real boolean change,
not an omitted value. A value equal to the current preference is allowed.
Unknown fields and out-of-range values are rejected. Other preferences, including
external-link confirmation, are outside this patch.

### Change device-local notification settings

`set_notification_settings` / `HostEffect::SetNotificationSettings` requires the
separate `notification_settings` capability. Its required `settings` object is a
`NotificationSettingsPatch`. Every field in
[`NotificationSettingsSnapshot`](extension-sdk-reference.md#notificationsettingssnapshot-device-local-notifications)
has a matching optional patch field: `Option<bool>` for the fourteen toggles and
`Option<u8>` for `volume` (0 through 100 inclusive). JSON uses booleans and an
integer respectively. `disable_sounds` is the master switch; it does not overwrite
individual cue toggles. Zero volume also silences sounds.

These are local preferences saved through the app's ordinary device-preference
path. This action does not update Discord account/server/channel notification
settings, play a sound, start a call or change microphone/camera state.

As with reading settings, omitted or `null` fields preserve their **Apply-time**
values, and at least one field must have a value. Empty/all-null patches, unknown
fields and out-of-range values are rejected without applying any part of the
patch. Foreground `message`, `composer` and `panel` actions may propose one effect;
background activation/message/app events cannot change settings. The result shows
the proposed values and requires **Apply**, which rechecks the plugin, grant and
account/conversation context. Closing the result makes no change.

#### Complete notification interaction

Declare a `panel` action named `quiet` and request `notification_settings` in the
manifest. This synthetic input supplies all fields of the current snapshot:

```json
{
  "action": "quiet",
  "values": {},
  "app": {
    "notification_settings": {
      "new_message": true,
      "current_channel": false,
      "incoming_ring": true,
      "outgoing_ring": true,
      "disable_sounds": false,
      "unread_badge": true,
      "mute": true,
      "unmute": true,
      "deafen": true,
      "undeafen": true,
      "camera_on": true,
      "screen_share_on": true,
      "user_join": true,
      "user_leave": true,
      "volume": 75
    }
  }
}
```

This complete SDK handler proposes silencing sounds only when the snapshot is
available and sounds are currently enabled. It preserves the volume and every
individual cue choice.

```rust
use serein_extension_sdk::{
    AppInvocation, AppOutput, HostEffect, NotificationSettingsPatch,
};

fn handle(input: AppInvocation) -> AppOutput {
    if input.app_event.is_some() || input.message_event.is_some()
        || input.invocation.action != "quiet"
    {
        return AppOutput::default();
    }
    let Some(settings) = input.app.as_ref()
        .and_then(|app| app.notification_settings.as_ref())
    else {
        return AppOutput::default();
    };
    if settings.disable_sounds {
        return AppOutput::default();
    }
    AppOutput {
        effects: vec![HostEffect::SetNotificationSettings {
            settings: NotificationSettingsPatch {
                disable_sounds: Some(true),
                ..Default::default()
            },
        }],
        ..Default::default()
    }
}

serein_extension_sdk::export!(handle);
```

For the input above, the effect is:

```json
{
  "effects": [{
    "type": "set_notification_settings",
    "settings": {"disable_sounds": true}
  }]
}
```

The host displays the proposal; returning this JSON alone changes nothing.
After Apply, the current local sound preferences use the master disable while
other values remain unchanged. Plugins observing `settings` events receive a
fresh notification snapshot only with the matching grant. An older host rejects
a manifest requesting `notification_settings`; support discovery cannot bypass
that install-time check.

## Panels and storage

A panel is a list of native controls returned by your handler. Serein renders
those controls; the plugin does not run while they are drawn. Editing a field
changes the panel's local form values. A **panel button** runs another declared
action with those values. It does not automatically approve a host action that
the next result proposes.

### The nine element types

Every element is a JSON object with a `type` tag. Supply every field listed in
the middle column. The same names are fields of the corresponding Rust `Element`
variant; unknown fields are rejected by the host.

| JSON `type` / Rust variant | Fields | Display or input behavior |
| --- | --- | --- |
| `text` / `Text` | `text`: string, at most 4 KiB. | Plain wrapping text. It may be empty or contain newlines; it is not HTML or executable markup. |
| `heading` / `Heading` | `text`: nonempty string, at most 128 bytes, no control characters. | A native heading. |
| `separator` / `Separator` | None. | A visual divider. |
| `row` / `Row` | `children`: array of elements. | Places children in a horizontal row that wraps when needed. |
| `button` / `Button` | `id`: action ID; `label`: display text. | Invokes the manifest's `panel` action with this exact ID. |
| `text_input` / `TextInput` | `id`, `label`, `value`: initial string, at most 4 KiB. | A single-line text field. The native editor limits typing to 1,024 characters; submitted values also have the 4 KiB byte limit. |
| `checkbox` / `Checkbox` | `id`, `label`, `checked`: initial boolean. | Returns the string `"true"` or `"false"`. |
| `select` / `Select` | `id`, `label`, `options`: string array; `value`: initial selected string. | A dropdown. `value` must exactly match one option. |
| `slider` / `Slider` | `id`, `label`, `min`, `max`, `value`: signed 32-bit integers. | An integer slider with `min < max` and an initial value in the inclusive range. Returns a decimal string. |

Control labels are nonempty, at most 128 UTF-8 bytes, and contain no control
characters. Dropdowns have 1 through 32 unique nonempty options, each at most
128 bytes without control characters. A label is what the user sees; an ID is
what your code uses.

### IDs and panel limits

Give every button and input a unique ID across the **whole panel**, including
nested rows. A button cannot reuse an input ID. IDs contain lowercase ASCII
letters, digits and hyphens, begin with a letter or digit, and are at most 64
bytes. Windows device names such as `con`, `nul`, `com1` and `lpt1` are reserved.
Spaces, underscores and uppercase letters are not allowed.

A button ID must also name a manifest action with `"surface": "panel"`.
Input IDs do not need manifest actions. A plugin may declare at most 16 actions.
Panels contain at most 64 elements in total, including rows and their children,
with at most eight nested row levels. The full result still shares the 256 KiB
serialized output budget. Byte limits count UTF-8 bytes, not visible letters.

### How a button receives form values

1. A user opens a foreground action, and its output supplies a panel.
2. Serein initializes each input from `value` or `checked` and keeps edits locally.
   Typing, selecting, dragging and checking do not call your handler.
3. Clicking a button creates a fresh invocation whose `action` is the button ID.
   Input IDs become keys in `values`; all values are strings. The button itself
   does not add a value.
4. The worker loads the latest granted storage and runs a fresh Wasm instance.
   Panel callbacks have no previous `selected_message` or `composer` text.
   App-aware callbacks receive a freshly rebuilt, separately granted snapshot.
5. The next output replaces the displayed result and initializes its new form.
   Storage and appearance can take effect immediately; an `effects` proposal
   still waits for the host's Apply button.

For example, checking `enabled` and setting `limit` to 4 before clicking `save`
produces an invocation like this (storage and other granted fields may be added):

```json
{"action":"save","values":{"enabled":"true","limit":"4"}}
```

Use `input.value("name")` to read `Option<&str>` without allocating. Use
`input.parse_value::<bool>("enabled")` or `input.parse_value::<i32>("limit")`
for typed inputs. `Ok(None)` means the ID was absent; `Ok(Some(value))` means
parsing succeeded; `Err(_)` means malformed text. Boolean parsing accepts
`"true"` and `"false"`, not `"1"`, `"yes"` or an empty string. Integer parsing
does not enforce your slider range: check it in the handler too.

The invocation allows at most 64 form entries, each with a valid ID and at most
4 KiB of string data, within the shared 256 KiB input budget. Treat missing and
invalid values explicitly; do not silently turn either into a saved preference.

### Storage is one value, not a filesystem

The `storage` capability gives this plugin one opaque UTF-8 string for the
current account. Each non-null `output.storage` replaces the **whole** saved
value. Omit it to leave storage alone. To store multiple settings, encode one
JSON object yourself; Serein does not merge object fields for you.

For example, this is a complete output that saves a JSON object inside the
storage string:

```json
{"storage":"{\"enabled\":true,\"limit\":4}"}
```

`input.storage_json::<T>()` returns `Ok(None)` if there is no saved value,
`Ok(Some(value))` for valid JSON of type `T`, and `Err(_)` for invalid or
incompatible JSON. An empty stored string is an error, not missing storage.
`output.set_storage_json(&value)` serializes into the same string field. If
serialization fails, it leaves that output field unchanged; it does not itself
write to disk. The host writes it only after accepting the complete result.

Save from foreground actions or separately granted message/app event handlers.
Activation receives saved storage for restoring settings, but this build ignores
storage returned from activation. It also does not display an activation panel.
An ordinary panel action is the appropriate place for a settings editor.

The disk ceiling is 1 MiB per plugin, but storage also travels through the
256 KiB invocation/output buffers alongside other fields. Nested JSON escaping
counts toward that smaller practical limit; keep settings small. Each call gets
a fresh instance, so globals are not persistent storage. Disabling or logging out
clears the account's extension data; cleanup failures are reported and retried.
Re-enabling starts fresh. Storage
is ordinary local data, not encrypted secret storage; never put credentials in it.

### Complete settings panel and Save handler

This example stores **plugin preferences**, not Serein's reading settings. It
requests only `storage`; changing the app's reading preferences instead requires
a `local_settings` host proposal and Apply.

Use this complete manifest:

```json
{
  "api_version": 1,
  "id": "panel-settings",
  "name": "Panel Settings Example",
  "version": "1.0.0",
  "author": "Example Author",
  "license": "MIT",
  "source": "https://example.com/panel-settings",
  "kind": "plugin",
  "capabilities": ["storage"],
  "actions": [
    {"id":"show","label":"Open plugin settings","surface":"panel"},
    {"id":"save","label":"Save plugin settings","surface":"panel"}
  ]
}
```

In a copied example workspace, add the plugin as a workspace member and use this
`Cargo.toml`. The parent workspace already defines these two dependencies.

```toml
[package]
name = "panel-settings"
version = "1.0.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
serde.workspace = true
serein-extension-sdk.workspace = true
```

Put this complete handler in `src/lib.rs`:

```rust
use serein_extension_sdk::{Element, Invocation, Output};

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Settings {
    enabled: bool,
    limit: i32,
}

fn note(text: &str) -> Output {
    Output {
        panel: vec![Element::Text { text: text.into() }],
        ..Default::default()
    }
}

fn form(settings: &Settings) -> Output {
    Output {
        panel: vec![
            Element::Checkbox {
                id: "enabled".into(), label: "Enable summaries".into(),
                checked: settings.enabled,
            },
            Element::Slider {
                id: "limit".into(), label: "Maximum items".into(),
                min: 0, max: 10, value: settings.limit,
            },
            Element::Button { id: "save".into(), label: "Save preferences".into() },
        ],
        ..Default::default()
    }
}

fn handle(input: Invocation) -> Output {
    match input.action.as_str() {
        "show" => {
            let settings = match input.storage_json::<Settings>() {
                Ok(None) => Settings::default(),
                Ok(Some(value)) if (0..=10).contains(&value.limit) => value,
                _ => return note("Saved preferences are invalid; nothing was overwritten."),
            };
            form(&settings)
        }
        "save" => {
            let enabled = match input.parse_value::<bool>("enabled") {
                Ok(Some(value)) => value,
                _ => return note("Choose whether summaries are enabled."),
            };
            let limit = match input.parse_value::<i32>("limit") {
                Ok(Some(value)) if (0..=10).contains(&value) => value,
                _ => return note("Choose a maximum from 0 through 10."),
            };
            let settings = Settings { enabled, limit };
            let mut output = form(&settings);
            if output.set_storage_json(&settings).is_err() {
                return note("Preferences could not be encoded; nothing was saved.");
            }
            output.panel.push(Element::Text { text: "Preferences saved.".into() });
            output
        }
        _ => Output::default(),
    }
}

serein_extension_sdk::export!(handle);
```

Open **Open plugin settings**, edit the controls, then click **Save preferences**.
The save action returns both the updated form and its new storage string. The
host commits storage before presenting the returned success text; there is no
extra Apply step for plugin storage. Opening the manifest's save action directly
without form values shows a validation message instead of changing storage.
Invalid saved data is reported on open, without an automatic reset or overwrite.

Use the [local development commands](../examples/extensions/README.md#test-and-develop-locally)
to test your copied example. Because adding a workspace member changes the
lockfile, first run `cargo check -p panel-settings` from `examples/extensions/`
in your development copy. Review and commit the updated `Cargo.lock`, then build
and package this plugin from the same directory:

```powershell
cargo build --locked --release --target wasm32-unknown-unknown -p panel-settings
python pack.py panel-settings/manifest.json target/wasm32-unknown-unknown/release/panel_settings.wasm packages/panel-settings.serein-extension
```

For a larger working panel with app
proposals, read [App Toolbox](../examples/extensions/app-toolbox/src/lib.rs).

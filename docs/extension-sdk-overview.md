# SDK overview

> **Preview SDK — PR #405, not yet released.** This branch adds approved reply,
> sticker, forward, channel, server, role, moderation and host-mediated media
> operations.

Build tools that run inside Serein: inspect loaded app data, show native panels,
format a draft, or propose an app action for the user to approve. Plugins are
Rust code compiled to WebAssembly. Themes are declarative packages and do not
need a handler.

**Start here:** [Build your first plugin](../examples/extensions/README.md).
For an existing plugin, use the [data reference](extension-sdk-reference.md#app-data)
or [action reference](extension-sdk-actions.md#outputs-and-host-actions).

## How a plugin runs

1. **Declare.** A manifest names your plugin, the actions users can run, and the
   capabilities it needs. The host validates the manifest before installation.
2. **Grant.** The user reviews and grants those capabilities when enabling the
   plugin. A supported capability is not automatically a granted capability.
3. **Invoke.** A user action or subscribed event starts a fresh Wasm instance on
   the extension worker. The host supplies JSON containing the action ID and
   permitted input data.
4. **Return.** Your handler returns one JSON result. The host validates its shape,
   capabilities, size and permitted action surface.
5. **Present or apply.** A foreground panel is displayed as native controls.
   App-changing proposals wait for the user to choose **Apply**. The Wasm instance
   is discarded; saved plugin state uses the separately granted storage field.

There is no persistent plugin process, network connection or render callback.
Reading a snapshot uses data Serein already has; it does not fetch missing data.
See [runtime limits](extensions.md#resource-and-privacy-limits) for the bounded
worker and [storage](extension-sdk-actions.md#storage-is-one-value-not-a-filesystem)
for state that survives invocations.

## Choose what to build

| Goal | Start with | Declare |
| --- | --- | --- |
| Show a tool in the extension panel | [First-plugin tutorial](../examples/extensions/README.md) | A `panel` action; add only the capabilities its handler needs |
| Inspect the selected conversation | [Current context](extension-sdk-reference.md#appcontextsnapshot-current-account-and-selected-chat) | `app_context` |
| Inspect loaded messages | [Timeline](extension-sdk-reference.md#timelinesnapshot-and-messagesnapshot-loaded-messages) and [rich summaries](extension-sdk-reference.md#messagecontentsnapshot-bounded-rich-message-summaries) | `timeline`, `message_content`, or `message_details` for the fields needed |
| Inspect channels, members or roles | [Channel metadata](extension-sdk-reference.md#channelmetadatasnapshot-loaded-channel-settings-threads-and-permissions) and [member details](extension-sdk-reference.md#memberdetailssnapshot-loaded-guild-members-and-role-labels) | `channel_metadata` and/or `member_details` |
| Format the current draft | [Composer input](extension-sdk-reference.md#common-input-fields) and [replacement output](extension-sdk-actions.md#every-output-field) | `composer` and a `composer` action |
| Open a conversation or native settings page | [Navigation actions](extension-sdk-actions.md#open-conversations-profiles-and-search) | `navigation`; read grants remain separate |
| Change scrolling or sound preferences | [Reading settings](extension-sdk-actions.md#change-local-reading-settings) and [notification settings](extension-sdk-actions.md#change-device-local-notification-settings) | `local_settings` or `notification_settings`; each change requires Apply |
| Send/edit messages, react, or manage threads | [App actions](extension-sdk-actions.md#app-actions) | Separate write grants; native permissions and Apply |
| Manage channels, server settings, roles or members | [Channel/server actions](extension-sdk-actions.md#channels-conversations-and-servers) and [roles/moderation](extension-sdk-actions.md#roles-and-moderation) | `channel_control`, `server_control`, `role_control` or `moderation_control`; native permissions and Apply |
| Change your profile, status, relationships or audio | [App actions](extension-sdk-actions.md#app-actions) | `account_control`, `relationship_control` or `audio_settings` |
| Join a call or control host-mediated devices/screen share | [App actions](extension-sdk-actions.md#app-actions) | `voice_connect`, `camera_control`, `audio_settings` or `media_control`; explicit Apply |
| React to app changes | [App events](extension-sdk-reference.md#appeventkind-why-an-app-observer-ran) | `app_events`, the relevant data grants, and `data_events` for the event kinds that require it |
| Save plugin preferences | [Panels and storage](extension-sdk-actions.md#panels-and-storage) | `storage` |
| Change the app's visual appearance | [Theme guide](theme-api.md) and [appearance output](extension-sdk-actions.md#every-output-field) | A declarative theme, or `appearance` for a plugin overlay |

For all names and consent rules, see the [capability reference](extensions.md#capability-reference).
No SDK capability gives a plugin credentials, unrestricted files, a network API,
or automatic Discord actions. Separately granted actions can propose sending
messages, editing profiles, managing threads and more; every operation requires
the user's Apply confirmation. Host actions expose only the operations listed
in the action reference.

## Read one interaction

A plugin declares an action with ID `show`, surface `panel`, and the `app_context`
capability. When the user runs it, a synthetic invocation could be:

```json
{
  "action": "show",
  "values": {},
  "app": {
    "context": {
      "connected": true,
      "channel": {"id": "20", "name": "general", "kind": 0}
    }
  }
}
```

A handler can return this complete result to display a native text panel:

```json
{"panel": [{"type": "text", "text": "Current conversation: general"}]}
```

No Apply step is needed for that panel. Reading the invocation or changing its
Rust value does not change the app.

A foreground action with the **separate** `local_notices` grant can instead return:

```json
{"effects": [{"type": "notice", "text": "Review complete."}]}
```

That result is a proposal. The host displays it and waits for **Apply** before
showing the notice. The host rechecks the plugin's permission and the originating
account/conversation context at that point. An event handler cannot return this
effect. See [host action rules](extension-sdk-actions.md#rules-for-every-host-action).

The [first-plugin tutorial](../examples/extensions/README.md#write-the-handler)
contains the complete Rust handler and manifest. These examples show the JSON
contract; they are not standalone packages.

## Understand the reference tables

| Term | Meaning |
| --- | --- |
| Wire field | The JSON key passed across the Wasm boundary |
| Rust type | The corresponding type exported by `serein_extension_sdk` |
| Capability | A manifest permission that must also be granted by the user |
| Action ID | Your manifest's stable action name; received in `action` |
| Surface | Where the host can invoke an action: `panel`, `message`, `composer`, `activation`, `message_event`, or `app_event` |
| Snapshot | Bounded app data captured for one invocation, not a live object or fetch API |
| Effect | A validated proposal for an app operation; at most one per foreground result |
| Partial / truncated | Some data was not loaded or did not fit the stated bound |

Discord resource IDs (users, guilds, channels and messages) are decimal
**strings**, such as `"20"`, not JSON numbers. Plugin action and control IDs are
author-defined strings such as `"show"`.

A missing snapshot group means unavailable or ungranted; do not convert it to an
empty list. Individual optional fields can have other meanings: an absent
`guild_id` is normal for a direct message. Read that field's table before assigning
a default. An available empty list and an unknown list can mean different things.

Form values are strings too: `"true"` for a checkbox and `"100"` for a slider.
Use `value`, `parse_value`, and `storage_json` rather than assuming fields are
present. The [input reference](extension-sdk-reference.md#common-input-fields)
explains their return values and failure behavior.

## Version and compatibility

`api_version: 1` identifies the Wasm buffer/JSON ABI, not a Discord API version
or a promise that every host supports every field. The wiki's source banner
identifies the exact Serein revision documented here. A merged source change
may not be in the installed release yet.

[Host discovery](extension-sdk-reference.md#hostinfo-discover-supported-names)
reports supported capability and event names. It does not report user grants.
A host rejects unsupported required manifest capabilities before your handler
runs, so discovery cannot make an incompatible manifest installable.

Treat optional fields as optional, and test against the host revision you plan
to support. Existing compiled plugins and rebuilding Rust source are different
compatibility questions: new struct fields can require updates to Rust literals
when you rebuild.

## Coverage and boundaries

The SDK exposes loaded conversation/server/member data, panels, themes, storage,
foreground navigation, messaging, reactions, read markers, thread/forum actions,
relationships, text profile edits, presence, reading/notification preferences,
audio processing/mixing and call controls. The preview adds message replies,
loaded stickers and forwards, a host attachment picker, channel administration,
channel/DM notification state, group/DM controls, server settings and emoji,
invites, roles, member moderation, host device selection and native screen-share
controls. These operations use the same permission checks, queues and failure
handling as native app actions.

It is not a complete Discord API. Poll voting and slash-command execution are
not generic SDK operations in this revision. File/device/screen choices stay in
native host UI: plugins cannot read arbitrary file paths or bytes, enumerate
private devices or capture frames. Snapshot reads do not fetch missing data, and
writes return no private service response to Wasm. Opening a settings page does
not expose unrelated controls.

## Next steps

- [Build and run your first plugin](../examples/extensions/README.md).
- [Look up snapshot objects](extension-sdk-reference.md#app-data).
- [Build a form and Save handler](extension-sdk-actions.md#complete-settings-panel-and-save-handler).
- [Diagnose import, execution or Apply failures](extension-sdk-troubleshooting.md).
- [Package and publish a reviewed extension](extensions.md#creator-workflow).

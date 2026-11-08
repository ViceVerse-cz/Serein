# Equicord Community Tools

A bounded Serein extension that adapts four Equicord workflows without
JavaScript injection, Webpack patches, raw Gateway/store access, network
requests, file access or automatic account actions.

## Provenance

Upstream: [`Equicord/Equicord`](https://github.com/Equicord/Equicord) at commit
`200dba124066146a8f043fde7fc8b05e3dc5e68e`.

Upstream files inspected for this port:

- `src/equicordplugins/exportMessages/index.tsx`
- `src/equicordplugins/hopOn/index.tsx`
- `src/equicordplugins/fastDeleteChannels/index.tsx`
- `src/equicordplugins/ignoreCalls/index.tsx`

The upstream projects are licensed `GPL-3.0-or-later`. This port is distributed
under the same license. See the header in `src/lib.rs` and
<https://www.gnu.org/licenses/gpl-3.0.html>.

This is an independent Rust adaptation, not upstream JavaScript parity and not
evidence of Discord interoperability.

## Supported flows

### Export one selected message as text

The message action **Copy the selected message as text** copies one header line
and the exact selected message text to the clipboard after **Apply**. The header
contains only the public message, channel and author IDs already supplied by the
host. The complete export is capped at **4,096 UTF-8 bytes**.

Attachments, embeds, images, timestamps, file saving and arbitrary message
lookup are not implemented. The upstream `saveWithDialog`/`saveFile` path needs
a native save-file action that this extension does not request.

### HopOn link proposal

The panel stores up to **eight** literal trigger phrases (64 characters and 256
bytes each) and one complete public **HTTPS** link (2,048 bytes). The message
action **Check the selected message for a HopOn trigger** performs a
case-insensitive literal phrase match and, on a match, proposes exactly one
`OpenExternalLink` action. Serein still performs its ordinary external-link
confirmation after Apply.

Upstream regular expressions, the custom `com.epicgames.launcher://` scheme,
automatic opening and message-event-driven opening are not implemented. The
host rejects effects from `message_event`/`app_event` surfaces, so an automatic
event handler cannot truthfully open a link in this SDK.

### Delete the current channel

The panel action **Propose deleting the current channel** returns one
`DeleteChannel` action for the current server channel or thread. Serein applies
its native destructive confirmation and rechecks `MANAGE_CHANNELS` or the
thread-management rule at Apply. Direct messages, unknown channel kinds, pending
messages and active calls are rejected by the host.

There is no keybind, channel-row icon, bulk deletion or background deletion.

### Local call ignore list and explicit decline

The panel can add or remove the current **DM or group DM** in a local list of at
most **16** conversation IDs. The action **Propose declining the call in an
ignored conversation** returns one `DeclineCall` action only when the current
conversation is in that list. The host rechecks that the exact conversation still
has an incoming call.

Serein's bounded `VoiceSnapshot` exposes the active call, not the incoming-call
channel or caller. The extension therefore cannot suppress ringing
automatically, rewrite `CALL_UPDATE`, or decide that a ringing call belongs to an
ignored user. The local list is a user-controlled gate for the explicit decline
proposal, not an automatic call filter.

## Bounds and safety

- One `HostEffect` per response; no background work, timers or event handlers.
- Clipboard export: 4,096 bytes.
- HopOn: 8 triggers, 64 characters/256 bytes per trigger, 2,048-byte HTTPS URL.
- Call ignore list: 16 unique nonzero decimal conversation IDs.
- Storage: one versioned JSON object, at most 48 KiB. Corrupt or oversized
  storage is preserved on read and replaced only by an explicit save/toggle.
- Channel deletion and call decline are typed `AppAction` proposals. The host
  revalidates permissions, current context and call state at Apply.
- No attachment bytes/URLs, credentials, tokens, hidden channels, arbitrary
  message lookup, generic URL fetch, filesystem access or automatic sends.

## Deliberate omissions and blockers

- **ExportMessages:** no file save, open-after-export, attachment/embed export or
  contact-list export. The safe subset is one selected message to the clipboard.
- **HopOn:** no regex engine, custom launcher scheme, automatic launch or
  message-event effect. The safe subset is a manual selected-message check.
- **FastDeleteChannels:** no keybind, channel-row trash icon, one-keypress delete
  or bulk deletion. The safe subset is one Apply-reviewed current channel.
- **IgnoreCalls:** no automatic ringing suppression or `CALL_UPDATE` rewrite.
  The safe subset is a local list plus one Apply-reviewed decline.
- **FollowVoiceUser:** not implemented. The bounded snapshot does not expose
  another user's voice channel, and automatic follow/leave loops are forbidden.
- **FriendCodes:** not implemented. Serein exposes no bounded friend-invite
  list/create/revoke action; adding one would require a new native REST surface
  and sensitive-code handling.
- **Declutter, DisableCameras, EquicordToolbox, KeyboardNavigation,
  CustomStatusTimeouts:** audited separately in `WAVE.md`; no duplicate package
  was added for behavior already covered natively or by another reviewed
  package.

## Tests

Focused inline tests cover:

- selected-message export bounds and missing/stale selection;
- literal trigger matching, HTTPS-only URL validation and one-effect output;
- trigger count/deduplication and storage merge;
- DM rejection and manageable-kind channel deletion;
- local ignore-list bounds, explicit decline gating and corrupt-storage
  preservation.

Run them with:

```sh
cargo test --manifest-path extensions/plugins/equicord-community-tools/Cargo.toml
```

The parent delivery workflow runs the workspace checks and packages the Wasm
centrally. This directory intentionally contains no generated package bytes.

## Parent integration

See `WAVE.md` for the exact workspace, packaging, catalog and documentation
steps the parent must perform. No host or SDK change is required for the
implemented subset.

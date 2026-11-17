# Ventrilo Mode

> **Preview SDK — not yet released.** This plugin needs a Serein host build that
> supports the `interface_ui` capability; released clients do not have it yet.

Ventrilo Mode replaces Serein's navigation, conversation and member panes with a
slim, Ventrilo-style tree: one emphasized row per server, the text and voice
channels of the server you picked, and the people in each voice channel. One
native window can show beside the tree: the call stage or one text channel's
chat. The title bar, dialogs, overlays and settings stay, and a bottom strip
keeps the settings gear, mute/deafen and disconnect controls. The plugin draws
nothing itself: it restates a declarative interface tree on every result, and
the host renders it generically.

## Grants

| Capability | Why |
| --- | --- |
| `interface_ui` | Replace the main panes with the plugin's tree and choose the window beside it. |
| `storage` | Remember the on/off choice, picked server and the open window. |
| `guild_directory` | List your servers as the tree's top rows. |
| `channel_directory` | List each server's text and voice channels. |
| `voice_state` | Read the current call and the voice roster: who sits in which voice channel, with names, mute/deafen (self and server) and speaking flags. |
| `app_context` | Recognize your own row for the `You` badge. |
| `voice_connect` | Propose joining a voice channel when you double-click one. |
| `voice_control` | Keep the call-control lane available; today the host's bottom strip already provides mute, deafen and disconnect. |
| `app_events` | Refresh the tree when voice state changes (`voice` events), so speaking and mute indicators update. |
| `data_events` | Refresh the tree when servers or channels change (`channels` events). |

## Behavior

- **Restore Ventrilo layout** (activation) re-applies the saved choice whenever
  the plugin is enabled or Serein starts: on by default, with the first server
  in directory order picked.
- **Server rows** switch the tree to that server without joining or leaving a
  call; the picked server's row is filled.
- **Channel rows** follow the order of the client's channel list. Text channels
  (server text and announcement channels) carry a `#` icon; voice channels a
  speaker. Categories, threads, forums and stage channels are not listed.
- **Text-channel rows**: a double-click opens that channel's chat beside the tree
  and highlights the row; double-clicking it again closes the chat. A single
  click does nothing. Opening a text channel never proposes anything.
- **Voice-channel rows**: a single click does nothing. A double-click proposes
  joining it (`join_voice`; Serein still shows
  the proposal and its **Apply** button) and shows the call window beside the
  tree. Double-clicking the channel you are already in closes the call window
  (or brings it back) — the call itself stays connected.
- **One window at a time**: opening a chat replaces the call window and opening
  the call replaces a chat.
- **People rows** appear under every voice channel of the picked server that the
  voice roster places people in, one row per person, with the status icon left
  of the name: headphones-slash while deafened (self or server), microphone-slash
  while muted (self or server), a speaker otherwise. The icon is tinted danger
  while muted or deafened, positive while speaking, and muted otherwise. Your own
  row carries the `You` badge. People rows are not clickable.
- **Ventrilo Mode settings** describes the current state with a button that
  flips the layout; **Toggle Ventrilo layout** does the same from the panel
  list. Toggling off restores the normal layout immediately and is remembered
  for the next activation, together with the picked server and the open window.

With nothing saved, the layout is on. Fields this version no longer uses (such as
an older `folded` list) are ignored. If a field it does use has the wrong type
(for example an older boolean `window`), the plugin falls back to the same default
and leaves storage unchanged; the next change overwrites it with a valid value.

## Limitations

- A person's name can be missing until the client has loaded their member data;
  until then the row shows their user ID.
- The roster covers what the client currently knows; servers it has not loaded
  show their channels without people.
- The channel snapshot carries no category or position fields, so rows follow
  the order the client supplies its loaded channels in rather than re-sorting by
  category.
- At most 100 loaded channels are visible to the plugin across all servers.

## Build and install

Run from `examples/extensions`:

```sh
rustup target add wasm32-unknown-unknown
cargo build --locked --release --target wasm32-unknown-unknown -p ventrilo
python ../../extensions/pack.py ventrilo/manifest.json target/wasm32-unknown-unknown/release/ventrilo.wasm packages/ventrilo.serein-extension
```

Import `packages/ventrilo.serein-extension` in **Settings > Extensions**, grant
the capabilities above, then enable the plugin.

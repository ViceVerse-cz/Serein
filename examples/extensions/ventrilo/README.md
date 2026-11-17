# Ventrilo Mode

> **Preview SDK — not yet released.** This plugin needs a Serein host build that supports
> the `voice_only_layout` interface tweak; released clients do not have it yet.

Ventrilo Mode switches Serein to a slim, Ventrilo-style voice-only tree: servers, their
voice channels and the users connected to them, with talking indicators. Text channels,
chat and the member list are hidden. The plugin draws nothing itself; it asks the host for
the `voice_only_layout` interface tweak, and the host shows the layout while any enabled
plugin requests it.

## Grants

| Capability | Why |
| --- | --- |
| `interface_tweaks` | Turn the host's `voice_only_layout` tweak on or off. |
| `storage` | Remember whether the layout is on between sessions. |

## Behavior

- **Restore Ventrilo layout** (activation) re-applies the saved choice whenever the plugin
  is enabled or Serein starts.
- **Ventrilo Mode settings** shows whether the layout is on, with a button that flips it.
- **Toggle voice-only layout** flips the layout, saves the new choice and confirms it.
- The call window stays hidden by default. Double-click a voice channel to join it and
  open its call window; double-click the same channel again to close the window (the call
  stays connected). Leaving the call or the Hide button hides it too.
- A header picker lists every server you are in; picking one switches the tree to it and
  loads it like the server rail, without joining or leaving a call.

The layout is on by default: with nothing saved, activation turns it on. If saved storage
is corrupt, the plugin falls back to the same default and leaves storage unchanged; the
next toggle overwrites it with a valid value.

To restore the normal layout, toggle it off or disable the plugin. Disabling removes the
plugin's tweaks.

## Build and install

Run from `examples/extensions`:

```sh
rustup target add wasm32-unknown-unknown
cargo build --locked --release --target wasm32-unknown-unknown -p ventrilo
python ../../extensions/pack.py ventrilo/manifest.json target/wasm32-unknown-unknown/release/ventrilo.wasm packages/ventrilo.serein-extension
```

Import `packages/ventrilo.serein-extension` in **Settings > Extensions**, grant
**interface tweaks** and **storage**, then enable the plugin.

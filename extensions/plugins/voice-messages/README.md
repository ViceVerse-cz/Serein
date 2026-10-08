# Voice Messages (preview)

An opt-in gate and settings surface for Serein's native recorder. Requires a host
with the preview `voice_messages` capability. A bounded bundled card makes it
available without downloading the catalog. It is not enabled automatically;
review grants in Settings > Extensions and enable it, or import the package.
Older hosts reject the unknown capability before execution.

Open tool to edit maximum duration (5–120 seconds), waveform style (Bars or Line),
and noise suppression. **Save voice message settings** persists with the separate
`storage` grant and applies preferences to new recordings. Editing controls or
opening the panel makes no changes. Saving never starts capture or sends audio.

Choose **Record voice message** in the composer's **+** menu, review the native
recording, then choose **Send**. The host owns devices, capture, processing and
the existing attachment/message queue; Wasm receives no audio, device list, files
or networking API. Recordings are session-only and bounded to 120 seconds and
8 MiB. Native mute, push-to-talk and OS permission gates still apply. Cancel,
navigation, disable, logout and call teardown release capture.

Invalid saved settings provide no recorder contribution until explicit Save
repairs them. Disable removes the plugin and saved preferences under ordinary
account-isolated lifecycle rules. Re-enabling starts fresh. This is unofficial
client functionality; synthetic checks do not prove live Discord interoperability.

From `extensions/`:

```sh
cargo test --locked -p voice-messages
cargo build --locked --release --target wasm32-unknown-unknown -p voice-messages
python pack.py plugins/voice-messages/manifest.json target/wasm32-unknown-unknown/release/voice_messages.wasm plugins/packages/voice-messages.serein-extension
```

The source and built package must be reviewed together before catalog publication.
The shared catalog is pinned after merge; do not edit `catalog.json` in the PR.
See the [SDK contract](../../../docs/extension-sdk-actions.md#voice-messages).

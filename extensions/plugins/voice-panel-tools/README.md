# Voice Panel Tools

A small, local Serein port of the volume controls from Equicord's
**VCPanelSettings** plugin. It reads only the granted `audio_settings` snapshot and
proposes one reviewed `set_audio_settings` change. Nothing is written until the
user presses **Apply**, and no microphone is opened, no call is joined and no
network request is made.

## Upstream source

Adapted from Equicord upstream commit
[`200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e):

- `src/equicordplugins/vcPanelSettings/index.tsx` — volume sliders, device
  selectors, camera selector and the voice-panel React patch.
- `src/equicordplugins/vcPanelSettings/README.md` — one-line feature summary.

The upstream file is GPL-3.0-or-later and its copyright notice is preserved in
`src/lib.rs`. This package uses the same license.

## What is implemented

The **Open voice panel tools** action shows the current host-supplied output and
input gain together with the current processing profile and suppression mode.

- **Output volume** uses upstream's 0-200 percent range.
- **Input volume** uses upstream's 0-100 percent range.
- **Review volume change** returns at most one `set_audio_settings` proposal with
  only the gain fields that actually differ from the fresh snapshot. The host
  displays the proposal and applies it only after **Apply**.
- If only one gain changes, the other gain field is omitted so the host preserves
  it rather than rewriting it.
- Input gain above 100 percent (Serein permits up to 200 percent) is shown as
  text but no input slider is rendered. A volume-only edit therefore cannot
  silently lower it.
- A missing `audio_settings` snapshot is reported as unavailable; no default
  values are invented and no effect is returned.
- Invalid, missing, negative or non-numeric slider values propose nothing and
  re-show the form with the error.
- Output values are bounded to 0-200 and input values to 0-100 before a proposal
  is built. `open_microphone` stays `false`.
- Processing profile, suppression, echo cancellation, automatic gain, sensitivity,
  push-to-talk, devices, calls and camera state are never touched.

## Deliberate omissions from upstream

Upstream injects sliders and selectors directly into Discord's voice panel and
patches Discord's React tree. This port cannot and does not:

- patch or draw inside the voice panel;
- enumerate or select input/output devices, because the SDK does not expose device
  names or lists to Wasm (the separate preview `select_audio_devices` action takes
  host-local IDs but provides no enumeration);
- enumerate or select cameras, or enable/disable the camera;
- subscribe to Discord's `MediaEngineStore`/`FluxDispatcher` events;
- change the "uncollapse settings" or header-visibility preferences;
- write any setting automatically or on slider movement.

The current host already persists `set_audio_settings` through its ordinary
runtime/preferences path; this plugin adds no host or native change.

## Tests

Inline unit tests exercise the same JSON path as the Wasm exports. They cover:

- only changed gain fields appearing in the proposal;
- a 0-200/0-100 range check plus missing, malformed and negative values;
- input gain above 100 percent being reported but never lowered by a
  volume-only edit;
- unchanged values producing no write;
- a missing snapshot producing no form controls and no effect;
- `open` rendering host values and hiding an above-cap input slider.

These are native handler tests, not Wasm sandbox, live Discord or microphone
behaviour evidence.

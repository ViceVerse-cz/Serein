# Theme border evidence

Baseline: `c2842aa7` (clean `main`, matching `origin/main`). Both demo binaries were built
with Rust 1.98.1, `--locked -p serein --features demo`, including voice.

These are native X11 window captures of synthetic `--demo` content on Linux,
using Vulkan, with `WINIT_X11_SCALE_FACTOR=0.93`, 100% egui zoom
and a 1120 × 800 client area. This exercises 0.93 physical pixels per UI point, also
reachable with 62% zoom and a native DPI factor of 1.5. No account, network, microphone
or capture session was used.

- `before.png` / `after.png`: `--demo --demo-theme=ember --demo-settings=appearance`,
  at the effective scale above. The baseline reproduces the bright bottom/right edges;
  the changed build keeps the borders softly tinted. Settings cards, the font selector,
  preset swatches and the blue selection rings were inspected.
- `before-stream.png` / `after-stream.png`: native captures cropped to the call controls
  in `--demo --demo-call-stream --demo-focus`, at the same effective scale, with the
  stream hovered.

Additional checks used `WINIT_X11_SCALE_FACTOR=1`: Appearance with the zoom value entered
as exactly 62%, call controls at 70%, and the changed light appearance at 100%. Those
integer-native-scale checks did not reproduce the harsh edge. No performance improvement
is claimed. Built-in gradient presets use explicit opaque border colors; extension
border tokens keep their authored alpha. Call controls blend their border over their
own fill. This adds no dependencies, geometry, textures or extra draw calls. Release
performance and a baseline package-size comparison were not measured for this cosmetic change.

## Custom-theme review regression

The initial PR head (`130eed23`) precomposed every border over `raised`, including
extension overrides. The follow-up removes that palette mutation. The four built-in
gradient presets now declare the same opaque border colors that the initial fix
produced; custom-theme borders remain available for compositing over each actual fill.

`custom-before.png` / `custom-after.png` are native 600 × 260 X11 captures at scale 1
of the same temporary offline preview calling `design::input`. The authored theme uses
`base = #0c3658`, `raised = #762438` and `border = #ffffff60` in both appearances.
The reference swatch blends that border directly over the input's blue base. The
initial head incorrectly gives the input a red-tinted opaque border; the follow-up
preserves its translucent token so the rendered border matches the reference.
This extra diagnostic fixture contains only synthetic text and a reference swatch;
it is not a live client screenshot. The Ember `after.png` was recaptured after the
follow-up; the unchanged call-control comparison is retained.

The regression test exercises author-supplied translucent border preservation through
surface composition. It failed against the initial PR head and passed after the fix.

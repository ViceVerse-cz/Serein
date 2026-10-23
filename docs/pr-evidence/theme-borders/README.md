# Theme border evidence

Baseline: `c2842aa7` (clean `main`, matching `origin/main`). Both demo binaries were built
with Rust 1.98.1, `--locked -p serein --features demo`, including voice.

These are native X11 window captures of synthetic `--demo` content on Arch Linux,
AMD Radeon RX 6900 XT / RADV Vulkan, with `WINIT_X11_SCALE_FACTOR=0.93`, 100% egui zoom
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
is claimed. The change composites colors when resolving the palette and building call
controls; it adds no dependencies, geometry, textures or extra draw calls. Release
performance and a baseline package-size comparison were not measured for this cosmetic change.

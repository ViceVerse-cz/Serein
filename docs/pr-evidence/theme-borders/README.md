# Theme border evidence

Baseline: `c2842aa7` (clean `main`, matching `origin/main`). Both demo binaries were built
with Rust 1.98.1, `--locked -p serein --features demo`, including voice.

These are native X11 window captures of synthetic `--demo` content on Arch Linux,
AMD Radeon RX 6900 XT / RADV Vulkan, with `WINIT_X11_SCALE_FACTOR=1` and a 1120 × 800
client area. No account, network, microphone or capture session was used.

- `before.png` / `after.png`: `--demo --demo-theme=ember --demo-settings=appearance`,
  with Appearance → Layout → Zoom entered as **62%**. Settings cards, the font selector,
  preset swatches and the blue selection rings were inspected.
- `before-stream.png` / `after-stream.png`: native captures cropped to the call controls
  in `--demo --demo-call-stream --demo-focus`, at **70%** zoom, with the stream hovered.
  The synthetic stream's placement differs slightly between captures; the control sizes
  and zoom match.

The changed light appearance was also inspected at 100% zoom. The user's exact bright-edge
artifact was not reproduced on this renderer; the comparison demonstrates the surface-tinted
border treatment, not verification of that specific GPU artifact. No performance improvement
is claimed. The change composites colors when resolving the palette and building call controls;
it adds no dependencies, geometry, textures or extra draw calls. Release performance and a
baseline package-size comparison were not measured for this cosmetic change.

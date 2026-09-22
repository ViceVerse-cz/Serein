# Synthetic slash command previews

These are inspected eframe/WGPU framebuffer exports on Windows at 125% display
scale, using the default dark and light themes. The commands, application names,
icons and conversation are synthetic. No account or service adapter is connected.

```powershell
cargo run --locked -p serein --features demo --example profile_preview -- --demo --page=slash-commands --width=1400 --height=1000 --output=dark.png
cargo run --locked -p serein --features demo --example profile_preview -- --demo --page=slash-commands --light --width=800 --height=900 --output=light.png
```

The fixture explicitly keeps the picker visible without keyboard/window focus.
It exercises the same layout, grouping, icon rendering and clipping as the app.
These exports are not OS screenshots or evidence of pointer, accessibility, IME,
live command discovery or interaction behavior. The native Computer Use helper
was unavailable with OS error 2; native before/after capture remains blocked.

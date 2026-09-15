# Theme editor readability

These inspected images are synthetic egui/WGPU framebuffer captures from the
offline debug example, not native OS window captures or live Discord evidence.
Native capture/control is unavailable in this session.

Before: `235cf01` with only the demo editor fixture/tab-selection helpers from
`1f5349b` added. After: `1f5349b`. Both use the Ocean package, renamed My ocean
by You, with the bundled synthetic Ocean preview as the background image.
Both use dark appearance, a 1400 x 1000 logical viewport, and 125% display scale
(1750 x 1250 framebuffer). No account or network adapter is involved.

```powershell
cargo run --locked -p serein --features demo --example profile_preview -- --demo --page=extensions --themes --theme-editor=background --width=1400 --height=1000 --output=target/theme-editor-background-after.png
```

Use `--theme-editor=colors` for the color page. Basics and Advanced were also
inspected locally, as were the narrow color layout (`--width=800 --height=900`)
and light background page (`--light`). The light check exposed and verified the
shared action-button text-color fix. Focused egui tests cover section selection
through both the map and menu, save validation, and palette-aware button text.
Native keyboard/scroll interaction and release UI timing remain unmeasured.

## Back button visibility

`back-before.png` uses `e452b0f` (the code baseline for `96a3a05`);
`back-after.png` uses `310ad5e`. Same Background fixture, viewport and scale as
above. Back now uses the shared outlined action style. The light/narrow variant
was also inspected locally. These remain synthetic framebuffer captures.

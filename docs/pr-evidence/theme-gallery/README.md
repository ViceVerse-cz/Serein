# Theme gallery comparison

These are inspected, synthetic egui/WGPU framebuffer captures from the offline
debug `profile_preview` example. They are not OS window captures or live Discord
evidence. Native window capture/control is unavailable in this session.

- Before: `cf4bcc2`, with only the theme fixture from `ae36f54` added to the example.
- After: `ae36f54`.
- Both: Windows, dark appearance, 1400 x 1000 logical viewport, 125% display scale
  (1750 x 1250 framebuffer), same six synthetic entries and Ocean cover image.
- The final light appearance and 800 x 900 logical viewport were also rendered
  and inspected locally; the narrow viewport uses two columns without clipping actions.

```powershell
cargo run --locked -p serein --features demo --example profile_preview -- --demo --page=extensions --themes --width=1400 --height=1000 --output=target/theme-gallery-after.png
```

For light appearance add `--light`; for the narrow check use `--width=800 --height=900`.
The before image requires the baseline UI plus the identical example fixture.
Native keyboard/navigation checks and release UI performance remain unmeasured.

# Gallery customization and full-app theme preview

These are inspected synthetic egui/WGPU framebuffer captures, not native OS window
captures or live Discord evidence. Windows, 125% scale, 1400 x 1000 logical viewport.

- `gallery-before.png`: verified pre-change example from `1f5349b`; through baseline
  `fd0cf4e` only documentation changed. Bundled originals offer Install theme only.
- `gallery-after.png`: same fixture with Customize available before installation,
  Use theme on an installed inactive theme, and a filled Active badge.
- `app-preview.png`: new `--theme-preview` fixture loads bundled BlackTheme into the
  production full-app preview/return flow. The example consumes appearance requests
  locally; it has no account or service adapters. This is a new-state illustration,
  not a matched before/after comparison of the old preview modal.

```powershell
cargo run --locked -p serein --features demo --example profile_preview -- --demo --page=extensions --themes --width=1400 --height=1000 --output=target/gallery.png
& target/debug/examples/profile_preview.exe --demo --page=extensions --themes --theme-preview --width=1400 --height=1000 --output=target/app-preview.png
& target/debug/examples/profile_preview.exe --demo --page=extensions --themes --theme-preview --light --width=800 --height=900 --output=target/app-preview-light.png
```

The light/narrow render was also inspected locally. Behavioral tests exercise
Customize before installation, preview requests, returning to the gallery, retaining
the editable copy, all bundled theme loads, and rejecting corrupt installed packages.
The gallery interaction test also checks Use theme, disabling it during work, and the
confirmed Active state at narrow and wide widths while retaining Edit/Disable actions.
Native keyboard/scroll interaction and matched release CPU/memory/frame-time evidence
remain unavailable: native desktop control/capture is disabled and Orca is absent.

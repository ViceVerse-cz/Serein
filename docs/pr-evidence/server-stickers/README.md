# Synthetic server sticker settings

These screenshots use the native egui renderer with the repository's offline demo
state. They contain no account session and make no Discord requests.

```powershell
cargo run --locked -p serein --example profile_preview --features demo -- --demo --page=server-stickers --output=docs/pr-evidence/server-stickers/after.png --width=1320 --height=900
```

The before image is the existing `docs/screenshots/server-settings.png` at the
base commit. `after.png` is rendered from this branch. The after fixture shows a
bounded synthetic catalog and verifies native layout only. It does not prove live
Discord interoperability.

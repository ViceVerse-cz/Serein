# Synthetic sticker previews

These inspected images come from the native egui framebuffer via the existing
offline `profile_preview` example. They are **not OS-captured screenshots** and
do not establish native input, accessibility, or live Discord interoperability.
The Windows computer-use connection failed with native-pipe OS error 2.

```powershell
cargo run --locked -p serein --features demo --example profile_preview -- --demo --page=stickers --output=target/sticker-preview.png
target/debug/examples/profile_preview.exe --demo --page=stickers --light --width=900 --height=700 --output=target/sticker-preview-light.png
```

The fixtures use original locally generated artwork, a synthetic server catalog,
a standard pack, recent usage, and one received sticker message. No service
credentials, network adapters, or account data are used by this example.

The dark image uses the default 1120 x 760 logical viewport; the light image
uses 900 x 700. Windows display scaling is 125% in the captured output.

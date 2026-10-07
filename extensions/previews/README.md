# Catalog previews

Every current package has a hash-pinned PNG preview under 256 KiB.

- `ocean.png` is the original synthetic native Ocean-theme screenshot.
- `forest-piano.png` and `soft-white.png` are 640x360 downscaled copies of their
  packaged covers, retaining the artwork's existing package provenance and terms.
- Other theme previews render Serein's synthetic workspace with the corresponding
  packaged theme applied in dark mode, including its spacing and typography.
- The original plugin preview renders the synthetic workspace with a preserved
  deleted message. The retired emoji/sticker plugin preview has been removed.
  No live account or Discord traffic is used.

These original previews were rendered by Serein's existing `profile_preview` example at
`ddbed3c456167e827d088aeac3ae74db46c07744`, using a 1280x720 window and its built-in
640x360 thumbnail export. They are captures of the actual egui framebuffer, not
drawn mockups, OS window-capture evidence, or proof of live interoperability.

Reproduce from that Serein revision, replacing `ID` with the package's manifest ID:

```sh
cargo run --locked -p serein --features demo --example profile_preview -- --demo --extension=ID --width=1280 --height=720 --thumbnail --output=preview.png
```

Synthetic UI previews use the repository's MIT OR Apache-2.0 license. Package
bytes are unchanged. To replace a preview, commit its PNG and regenerate the catalog
as described in the [extensions README](../README.md) so its URL, SHA-256, and byte count stay pinned.

`api-proxy.png` renders the API Proxy package's actual native extension panel with
synthetic offline data through the same `profile_preview` framebuffer pipeline.
It is not an OS window capture, a drawn mockup, or proof of live proxy connectivity.
No account or proxy credentials are used in this preview. Reproduce using a Serein
revision with the API Proxy host support:

```sh
SEREIN_PREVIEW_PACKAGE=/path/to/plugins/packages/api-proxy.serein-extension cargo run --locked -p serein --features demo --example profile_preview -- --demo --extension=api-proxy --width=1280 --height=720 --thumbnail --output=api-proxy.png
```

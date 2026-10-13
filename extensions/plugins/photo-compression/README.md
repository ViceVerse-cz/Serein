# Photo compression (preview)

Import the packaged plugin in Settings > Extensions, review and grant
`image_compression` and `storage`, then enable it. Selected JPEG and opaque
static PNG photos can become smaller JPEGs automatically. Defaults are a
1280-pixel longest edge and JPEG quality 80. Open **Configure photo compression**
to change these values and apply them to the next selection.

The composer shows the resulting name, size and preview. Review before sending.
Original disk files stay unchanged. To attach an original instead, remove the
selection, disable the plugin and select it again. Settings affect future
selections; already preparing selections keep their policy snapshot.

Compression can lose detail and removes metadata; decoded EXIF orientation is
applied. Transparency, animation, HDR, WebP and videos keep the original, as do
inputs beyond the safety limits, errors and conversions that are not smaller.
This partially addresses issue #128; video/AVIF/AV1 support is deferred. It does
not guarantee a file will fit Discord's account limit.

The plugin receives no file paths, bytes or Discord credentials. The native host
uses one bounded codec worker. See the [SDK contract](../../../docs/extension-sdk-actions.md#photo-compression-preview)
for limits and cancellation behavior.

Build and package from the repository root:

```powershell
cargo build --manifest-path extensions/Cargo.toml --locked --release --target wasm32-unknown-unknown -p photo-compression
python extensions/pack.py extensions/plugins/photo-compression/manifest.json extensions/target/wasm32-unknown-unknown/release/photo_compression.wasm extensions/plugins/packages/photo-compression.serein-extension
```

The package is reviewed in its pull request. The catalog workflow lists it after
merge; no catalog availability or release is claimed by this preview.

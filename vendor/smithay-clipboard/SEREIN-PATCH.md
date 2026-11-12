# Wayland image paste

Source: crates.io `smithay-clipboard` 0.7.3 (MIT). Retains upstream source, manifest provenance (`Cargo.toml.orig`) and the license file; the unpublished example is dropped from the manifest.

- `Clipboard::load_mime(mime, limit)` reads the raw bytes of one exact offered MIME type through the focused surface's `wl_data_device`, failing once the selection exceeds `limit` bytes. `load` and `load_primary` keep their text behavior.
- The worker replies with bytes instead of `String`; text loads still pick and normalize the upstream text MIME types before replying.

Why: upstream only reads text, so egui-winit fell back to arboard for images. On Wayland arboard needs the privileged data-control protocol or an Xwayland clipboard bridge; KDE Plasma under Flatpak provides neither to Serein, so copied images reported nothing to paste while text worked ([#900](https://github.com/ViceVerse-cz/Serein/issues/900)). The vendored egui-winit now reads `image/png` here first.

Remove this patch when an upstream smithay-clipboard release can load arbitrary MIME types and egui-winit uses it for image paste.

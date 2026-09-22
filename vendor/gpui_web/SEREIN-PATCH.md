# Native-only GPUI web backend stand-in

`apps/gpui` uses `gpui_platform` from Zed commit
`96e95edac45b0e0696179b8139aa521cf48db392`. That crate depends on `gpui_web` only for
`target_family = "wasm"`, but Cargo still resolves every target in the workspace lockfile.
`gpui_web` pins `unicode-properties = "=0.1.3"`, while egui's `epaint` requires `^0.1.4`, so the
workspace cannot resolve both.

Serein never builds GPUI for WebAssembly. This empty crate replaces `gpui_web` through
`[patch."https://github.com/zed-industries/zed"]`; no native GPUI code is changed. A wasm build
of `gpui_platform` would fail to compile against it. Remove this patch once upstream relaxes the
pin or the GPUI experiment moves to a published release.

# Catalog plugins

The published plugins, built against the one [SDK](../sdk) that
the [authoring guide](../../examples/extensions/README.md) documents. Any language
producing compatible WebAssembly can use this ABI; Rust authors can call
`serein_extension_sdk::export!(handler)`.

Build and package from `extensions/`, the workspace root (Python 3 is used only by the author):

```powershell
rustup target add wasm32-unknown-unknown
cargo build --locked --release --target wasm32-unknown-unknown
python pack.py plugins/message-delete-protector/manifest.json target/wasm32-unknown-unknown/release/message_delete_protector.wasm plugins/packages/message-delete-protector.serein-extension
python pack.py plugins/emoji-sticker-images/manifest.json target/wasm32-unknown-unknown/release/emoji_sticker_images.wasm plugins/packages/emoji-sticker-images.serein-extension
```

A rebuilt module is not a release by itself: committed packages are the reviewed bytes
clients install. Replace one only with a reviewed rebuild and a bumped `manifest.version`.

Import the package in Settings > Extensions, review the capabilities, and enable it.
Message delete protector is an example activation plugin. Its `activation` action returns
`preserve_deleted_messages: true` after the user grants `deleted_messages`. The host
keeps already-loaded messages in bounded session memory and displays deleted text in red
by default. Hover and a local context menu can toggle that highlight or remove the
retained row. They never call Discord. The host never sends message bodies to the plugin,
saves deleted bodies to disk, restores messages deleted before loading, or gives deleted
messages live service actions.
Disabling, logout, permission revocation and timeline eviction release retained content.
Emoji & Sticker Images requests `image_sharing` and returns `image_sharing: true`
from activation. While enabled, custom emoji and sticker selections stage image attachments. Wasm receives no conversation text or image bytes and cannot fetch
or send anything. Selecting artwork authorizes an immediate image send after validation; text drafts
remain intact. Disable/logout revoke the option.
Declarative themes are under `../themes/`.
The author packages compiled bytes; Serein never runs a repository's build scripts.

## ABI and limits

See [ABI version 1](../../examples/extensions/README.md#abi-version-1) in the SDK guide
for exports, input/output fields, panels and sandbox limits.

## Publishing

Add or update a package through a pull request; see the [extensions README](../README.md).
Maintainers must review the source and built artifact together before listing a version.
All versions and updates require explicit user consent.

## Custom Rich Presence (preview)

See [Custom RPC](custom-rpc/README.md) for setup and builds. Requires the preview
rich-presence SDK from Serein PR #465. Install through the shared catalog or import
the package locally. Older clients without this capability cannot install it.

## API Proxy (preview)

See [API Proxy](api-proxy/README.md) for the native REST-only proxy editor.
Requires the API proxy SDK capability; Gateway, CDN/media and calls stay direct.

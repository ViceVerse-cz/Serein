# Compiled legacy SDK fixture

`app-toolbox.serein-extension` is copied byte-for-byte from
[`3d94c76228d9f2918fa8d22e78f40233ae4f90dc`](https://github.com/ViceVerse-cz/Serein/blob/3d94c76228d9f2918fa8d22e78f40233ae4f90dc/examples/extensions/packages/app-toolbox.serein-extension),
not rebuilt with the current SDK. Source and package license: MIT OR Apache-2.0.

- Package bytes: 574992
- Wasm bytes: 198370
- Package SHA-256: `45cc1f3951ba59d365f9e29696659f5f474545756e6441755bb92fad8428546f`

This preserves the App Toolbox binary from before message-details/relationships
and later channel-metadata/member-details additions. Its original manifest and
strict event vocabulary remain unchanged. `legacy_sdk_check` runs synthetic
foreground and app-data inputs through the current real Wasm sandbox, then
checks that newer event kinds are rejected by the original manifest before
execution. It does not exercise enable/disable, reload, install or account
lifecycle behavior and does not establish live Discord compatibility.

Run from the repository root:

```sh
cargo run --locked --release -p extensions --example legacy_sdk_check
```

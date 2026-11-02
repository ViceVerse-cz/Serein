# Encoded Rich Presence proxy artwork

Baseline: `ee9a746d365f267889a62f4eb1910b5a72aa701b` plus the same new
`profile_preview --proxy-artwork` fixture scaffold used after the fix. The baseline
keeps the original activity-image validator. After: the source committed with
these images.

The fixture decodes a synthetic Gateway listening activity containing an `mp:`
reference with an encoded source query, slash and filename dot. Its corner badge
uses a numeric application asset. The preview supplies generated checkerboard
pixels only when the projected artwork key is requested; it has no network or
image-worker adapter. Profile metadata is deliberately not supplied, so the
unrelated profile loading label remains visible in both versions.

Before, the decoder substitutes the application icon for the valid proxy image
and the cover stays blank. After, it preserves the proxy image, which requests and
paints the generated cover. These images verify projection and native painting;
they do not verify a live Discord response or the owner's exact presence payload.

Native wgpu/Vulkan windows on Linux x86_64, software llvmpipe, Xvfb, pinned Rust
1.98.1, 1120 × 760 application pixels, scale 1. Both versions use identical flags,
fixtures and isolated XDG directories. Captures come from the native framebuffer
callback after at least five frames and a one-second warmup. The light pair adds
`--light`.

```sh
cargo run --locked -p serein --features demo --example profile_preview -- --demo --page=profile-card --proxy-artwork --output=preview.png --width=1120 --height=760
```

Executable/test builds used `CARGO_PROFILE_DEV_DEBUG=0`,
`CARGO_PROFILE_TEST_DEBUG=0` and `CARGO_INCREMENTAL=0` to limit local build storage.
No live account, external image server, call or microphone was used.

Verification:

- The extended desktop regression failed before the fix: the projected primary
  image was `Application(Id(10))` instead of the encoded `Proxy` reference.
- The same regression passes after the fix for an encoded filename dot, a source
  query containing an encoded slash, and an encoded source path separator. Both
  the cover and small artwork survive projection; the resulting download URL
  retains its opaque path and the fixed Discord media host.
- The model security test passes for malformed routing, encoded authorities,
  decoded traversal, backslashes and the existing size limit.
- All 13 presence-related protocol tests pass, including snapshot/update image
  selection, malformed data and resource bounds.
- `cargo fmt --all -- --check` and diff review passed. Both native example builds
  and all four framebuffer captures completed successfully.

The validator remains allocation-free and references remain limited to 1,024
bytes. Release package size, CPU, RSS and frame timing were not measured. Full
workspace/Clippy/package checks are left to GitHub runners under the owner's
request; this follow-up ran focused regression checks locally.

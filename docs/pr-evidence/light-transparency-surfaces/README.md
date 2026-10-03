# Transparency control-color regression

`before.png` renders baseline library source
`f3f244c97f223280b49966ac96284aa72fd03a1f`; `after.png` renders this PR's library.
Both use the same offline `profile_preview` harness, including the new bounded
`--transparency=0..100` option. The baseline harness was given that option before
any production palette changes. These are actual native WGPU framebuffer exports,
not generated images or live Discord conversations.

The committed pair uses Standard/Light, 100% window-effect palette transparency,
1120 × 760 pixels, display scale 1, and the unchanged synthetic Appearance page.
Dark mode, 540 × 760 narrow settings, and the controls reached by `--scroll=450`
were also rendered and inspected locally. The harness's native viewport is opaque:
this verifies rendered appearance and control contrast, not desktop compositing,
native blur, or Windows/macOS compositor behavior.

## Reproduce

```sh
cargo build --locked -p serein --features demo --example profile_preview
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET target/debug/examples/profile_preview \
  --demo --page=appearance --light --transparency=100 --output=target/appearance.png
cargo test --locked -p ui --lib transparency
```

Omit `--light` for Dark; add `--width=540` for the narrow layout or `--scroll=450`
to inspect the effects section. The X11 environment selection above is local
capture setup only; the application's native backend selection is unchanged.
Negative, greater-than-100, overflowing and nonnumeric percentages are rejected
before any native window is created.

Rendered tests compare settings, mode swatches, an expanded tinted server folder,
private text/voice lock halos, and enabled shared/plain buttons against their
same-fixture effects-disabled colors, at 0/50/100% in Light and Dark. Additional
palette tests cover all seven presets, translucent custom themes, five opacity
levels, single application of image-backdrop tint, and retained main-surface alpha.

## Measurements

`measurements.json` records the standard voice-inclusive DEB package and a separate
optimized native offline Appearance harness. For that harness, both revisions use:

```sh
cargo rustc --release --locked -p serein --features demo --example profile_preview -- -C lto=off
# Run the preserved binary:
profile_preview --demo --interactive --page=appearance --light --transparency=100
```

The final-crate LTO override is identical before and after. Standard packages retain
normal fat LTO and `cargo xtask package`'s no-default-features configuration.
Native sampling uses Python psutil after eight seconds of warmup and three seconds
of settling: twenty one-second process CPU/RSS samples, with settled RSS defined as
the median of the final five. CPU percentages refer to one logical core. Child
processes are recorded separately; none were present. No builds run during sampling.
The scripted workload opens the same synthetic Appearance modal and then idles;
it does not measure event dispatch, frame latency, startup peaks, or a live client.
Small RSS differences are allocator/driver/sampling noise, not a performance claim.

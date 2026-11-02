# Discord profile cosmetics

Baseline `178bde35`; the changed build is this PR's branch. Both are release builds of
`profile_preview --features demo`, run with the same `collectibles` fixture page. The baseline
worktree received only that page and the fixture-only `preview_profile_popout` helper. Its users
have no cosmetics because the field does not exist there.

```sh
cargo build --release --locked -p serein --features demo --example profile_preview
gamescope --backend headless -w 1440 -h 900 -W 1440 -H 900 -- env -u WAYLAND_DISPLAY \
  target/release/examples/profile_preview --demo --width=1440 --height=900 \
  --page=collectibles --output=after-popout.png   # also: --full, --light, --page=dm-tags
```

These images are eframe framebuffer exports, not OS window captures. All decoration, nameplate,
effect and frame artwork is drawn locally and is synthetic. None of it is Discord artwork, and no
network request is made. Pointer hover (animated nameplates and decorations) and screen-reader
behavior were not exercised natively.

## Review follow-up: cosmetics opt-out, image cache settings, frame overflow

`after-settings-storage.png`, `after-settings-cosmetics.png` and the refreshed `after-full.png`
come from a debug `profile_preview --features demo` build of the follow-up commit. They were
exported the same way at 1440 × 900 on Fedora Asahi (Apple M1, labwc):

```sh
cargo build --locked -p serein --features demo --example profile_preview
target/debug/examples/profile_preview --demo --width=1440 --height=900 --page=storage --output=after-settings-storage.png
target/debug/examples/profile_preview --demo --width=1440 --height=900 --page=appearance --scroll=900 --output=after-settings-cosmetics.png
target/debug/examples/profile_preview --demo --width=1440 --height=900 --page=collectibles --full --output=after-full.png
```

The storage page's usage figures (318 MB media, 9.29 MB shared artwork) are synthetic values
injected by the preview. The demo image worker keeps no disk cache, and demo mode disables the
Clear actions. These settings groups did not exist before, so there is no baseline image.

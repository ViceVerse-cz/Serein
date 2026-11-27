# Inline playback of link-embed videos

Baseline `e19b15cd`: `main` (`e5fe9f0b`) plus only this PR's synthetic fixture commit, which adds a
video link embed to `--demo-video`. The changed build is this PR's head (`28f6fc40`). Both are
release builds with demo support (thin LTO, 16 codegen units) on Windows 11, 100% scale, 1120×760
window, captured with `PrintWindow`:

```sh
CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
  cargo build --release --locked -p serein --features demo
serein.exe --demo --demo-video --demo-accent=c084fc
```

- `before.png`: the embed shows its preview image, "Video preview · playback opens in your browser"
  and "Open video…".
- `after.png`: the same embed shows the inline player with a decoded first-frame poster and
  "Open original…"; no Download action for embeds.
- `after-playing.png`: 1.2 s after a native left click on the embed's play button; the seek bar,
  pause, volume and fullscreen controls are drawn and the fixture's timecode has advanced.

All content is synthetic. In `--demo` the embed plays the bundled MOV fixture; no network request
is made, so this does not show Discord's media proxy serving a real embed video.

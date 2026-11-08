# Safe Equicord ecosystem evidence

These are native eframe framebuffer exports from isolated synthetic fixtures, not logged-in
account captures. No Discord network, credentials, messages, microphone or attachment URLs were
used by the history-preview example.

- `before.png`: Detached `21f8870b3849f24f9b0fb0bb17b9e3e272e2b93a` chat fixture at 1120x760
  points before Escape closes its Chat settings overlay. Shows the existing account-control icon
  rendering and the lack of a durable history viewer.
- `after.png`: The real native history browser at 1120x760 with synthetic rows, edit/delete
  versions and attachment metadata. Includes the revised icon resampling/proportions and bounded
  dialog layout. Outgoing export/download requests were not executed.
- `after-narrow-light.png`: The same renderer at 760x520 with the light preference, confirming
  scrollable modal bounds and readable filter/export rows outside the normal viewport.
- `after-composer-preview.png`: The later native composer-preview fixture at 1120x760 with a
  bounded synthetic draft, plain-text wrapping and the loaded friends/server rail indicator.
- `after-composer-preview-narrow-light.png`: The same new fixture at 760x520 with the light
  preference. The preview, composer and loaded indicator remain within the viewport. This is a
  new fixture state, so it is not presented as a pixel-matched baseline comparison.

Capture display: isolated `Xvfb :99`, software GL fixture, no window manager decorations. The
fixture enables only local `MessageLogger` consent needed to display its real browser; it does not
write to the development account database or execute extensions. Fixture command:

```sh
DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 cargo run --locked --offline -p serein \
  --example message_history_preview --features demo -- --demo \
  --width=1120 --height=760 --output=/tmp/opencode/serein-history-after.png
```

For light/narrow capture substitute `--light --width=760 --height=520`. The old profile fixture
received an Escape event to present its channel instead of Chat settings before capturing. No
production source was changed to obtain the baseline image.

The later composer captures used private Xvfb displays and the demo-only fixture hook, with no
attachment staging, upload, send or extension execution:

```sh
xvfb-run -a -s '-screen 0 1600x1000x24' env LIBGL_ALWAYS_SOFTWARE=1 \
  cargo run --locked --offline -p serein --example profile_preview --features demo -- \
  --demo --composer-preview --width=1120 --height=760 \
  --output=docs/pr-evidence/safe-equicord-ecosystem/after-composer-preview.png
```

Add `--light --width=760 --height=520` for the narrow image. The lower Chat settings controls
were also inspected after scripted scrolling; that diagnostic capture was not needed in the PR set.

The images demonstrate UI state only. They are not evidence of service interoperability, durable
reconstruction of deleted-before-load content, an actual HTTP attachment transfer, completed file
export or a live Discord session.

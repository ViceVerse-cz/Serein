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

The images demonstrate UI state only. They are not evidence of service interoperability, durable
reconstruction of deleted-before-load content, an actual HTTP attachment transfer, completed file
export or a live Discord session.

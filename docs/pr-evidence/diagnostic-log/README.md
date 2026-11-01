# Diagnostic log evidence

Both images come from the offline fixture renderer, not from a window capture:

```sh
cargo run --locked -p serein --features demo --example profile_preview -- \
  --demo --page=updates --output=after.png --width=1120 --height=760 --scroll=150
```

`before.png` was rendered from `main` (`1b2f8942`) with local fixture-only patches to the
page allowlist and a missing `managed` field on a synthetic role. The exact patch was not
retained, so this baseline cannot be reproduced from clean `main` with the command above. `after.png` comes from this branch. The
`--scroll=150` offset shows the whole Support & diagnostics card.

The page is the same on both sides apart from the added controls: **Save diagnostics** next to
**Copy**, a level menu (shown at its default, `Off`), and the copy/open actions for the log. Nothing
is seeded: the fixture starts with the default preference, so the controls are what a first launch
shows. No account, network, microphone or capture session was used, and the fixture keeps its
changes in the session.

# Diagnostic log evidence

Both images come from the offline fixture renderer, not from a window capture:

```sh
cargo run --locked -p serein --features demo --example profile_preview -- \
  --demo --page=updates --output=after.png --width=1120 --height=760 --scroll=150
```

`before.png` was rendered from clean `main` (`1b2f8942`) with the same command; only the
fixture's page allowlist and one missing `managed` field on a synthetic role differed, so
the same page could be rendered on both revisions. `after.png` comes from this branch. The
`--scroll=150` offset shows the whole Support & diagnostics card.

The page is the same on both sides apart from the added controls: **Save diagnostics** next to
**Copy**, a level menu (shown at its default, `Off`), and the copy/open actions for the log. Nothing
is seeded: the fixture starts with the default preference, so the controls are what a first launch
shows. No account, network, microphone or capture session was used, and the fixture keeps its
changes in the session.

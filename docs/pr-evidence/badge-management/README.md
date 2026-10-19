# Profile badges, directory and badge settings

Baseline `ee93c1d0` (main). The changed build is this PR's branch. Both are debug builds of
`profile_preview --features demo` running the same `collectibles` fixture page. The baseline
worktree received only an evidence-only `--own` flag, which opens the account's own card.

```sh
cargo build --locked -p serein --features demo --example profile_preview
gamescope --backend headless -w 1440 -h 900 -W 1440 -H 900 -- env -u WAYLAND_DISPLAY \
  target/debug/examples/profile_preview --demo --width=1440 --height=900 \
  --page=collectibles --own --output=after-own.png
# also: (no flag) another user's card, --badge-customize, --badge-directory [--light]
```

These are framebuffer exports, not OS window captures. The badge catalog, tiers, dates and
artwork are synthetic and drawn locally; no network request is made. Hover cards, drag
reordering and keyboard navigation were not exercised with native input. Click-to-hide and
save state are covered by unit tests.

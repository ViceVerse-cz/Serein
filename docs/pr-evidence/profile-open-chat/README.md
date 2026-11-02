# Profile footer open chat

Baseline `1b2f8942` (main) and this PR's branch, each a debug `profile_preview --features demo`
build. Both fixtures needed the same local, uncommitted one-field compile fix (`managed: false`)
in the example's synthetic role payload before they could run. Both screenshots were exported at
1440x900 with the same scale factor (1800x1125 native pixels); no resizing or compositing was used.

```sh
target/debug/examples/profile_preview --demo --width=1440 --height=900 --page=collectibles --output=before.png
target/debug/examples/profile_preview --demo --width=1440 --height=900 --page=collectibles --output=after.png
target/debug/examples/profile_preview --demo --width=1440 --height=900 --page=dm-intro --output=dm-sidebar.png
```

These are native framebuffer exports of the synthetic member-list profile popout. The new chat
button sits left of the send button, so a conversation can be opened with an empty composer.
`dm-sidebar.png` shows the docked profile beside a direct message; it intentionally has no chat
button. Light and dark themes were inspected. No account data or network requests were used.

# Preload on hover setting

Baseline `2df8256c` (main) and this PR's branch, each a debug `profile_preview --features demo`
build with the `chat` fixture page (added by this PR; the baseline build had the same one-line
page addition, uncommitted):

```sh
target/debug/examples/profile_preview --demo --width=1440 --height=900 --page=chat --scroll=1600 --output=after.png
```

These are framebuffer exports of the synthetic Chat settings page. No account data or network
requests are involved. Preloading itself has no visible change beyond opening sooner.

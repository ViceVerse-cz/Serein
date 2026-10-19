# Direct message introduction

Baseline `ee93c1d0` (main), built with only the evidence fixture page added. The changed build
is this PR's branch. Both are debug `profile_preview --features demo` builds:

```sh
target/debug/examples/profile_preview --demo --width=1440 --height=900 --page=dm-intro --output=after.png
```

These are framebuffer exports of a synthetic four-message conversation whose whole history
fits one page. No account data or network requests are involved.

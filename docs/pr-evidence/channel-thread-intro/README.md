# Channel, thread and forum-post introductions

Baseline `ee93c1d0` (main) with only the evidence fixture pages added; the changed build is
this branch. Debug `profile_preview --features demo` framebuffer exports at 1440×900:

```sh
target/debug/examples/profile_preview --demo --width=1440 --height=900 --page=channel-intro --output=ch-after.png
target/debug/examples/profile_preview --demo --width=1440 --height=900 --page=post-intro --output=post-after.png  # add --light
```

The conversations are synthetic four-message histories that fit one page; no network access.
The blue banner is unrelated: read-on-open is a separate PR (#632).

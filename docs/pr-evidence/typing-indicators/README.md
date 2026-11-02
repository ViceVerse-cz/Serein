# Typing indicator demo evidence

Baseline: `7cd14efe2cd22b59ea8569fb858c6324b5c09a38` (`main`, before PR #705).
After: the conflict-resolution source revision committed with these images.

Native Serein windows, Linux x86_64, pinned Rust 1.98.1, wgpu/Vulkan llvmpipe,
Xvfb at 1440 × 1080, application window 1120 × 760 at scale 1. Each capture follows
a ten-second warmup with the pointer outside the application. Executable and test
builds used `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0` and
`CARGO_INCREMENTAL=0` to limit local build storage; these are debug builds.

All content is synthetic. Runs used `--features demo -- --demo --demo-reply`,
plus `--demo-light` for the light pair or `--demo-group-dm` for the group pair.
Each run used isolated XDG configuration, data and cache directories. No live
account, service message, call or microphone was used.

- `before.png` / `after.png`: the People status disc becomes an animated typing pill.
- `before-light.png` / `after-light.png`: the same comparison in the light theme.
- `before-group.png` / `after-group.png`: the old group fixture remains on loading
  placeholders; the corrected fixture supplies synthetic history, then displays
  the group list's grey pill, member pills and named typing footer.

The initially merged demo build failed because its exhaustive command dispatcher
omitted `Command::Typing`. The fix explicitly discards that command offline.
The group fixture separately ignored the history command returned by selection;
it now applies the same bounded local history response used by demo navigation.

Verification on the resolved source:

```sh
cargo fmt --all -- --check
cargo check --locked -p serein -p ui --all-targets --features serein/demo
cargo test --locked -p serein -p ui -p client-core -p discord-api --features serein/demo --lib --bins typing
cargo build --locked -p serein --features demo --bin serein
```

All passed. The filtered suite contains 21 tests: seven client-core, one HTTP
transport, six desktop (including the new group-history regression) and seven UI
checks. CI now compiles demo targets and runs this filtered suite in every native
matrix job, alongside the existing production build checks.

The full workspace/Clippy/package suites were not rerun locally under the owner's
request to leave those checks to GitHub runners. No release package, CPU, RSS or
frame-time comparison was measured. These screenshots and synthetic checks do not
establish live Discord interoperability.

# Graphics API picker evidence

Baseline: `354d033b31e89079e50c16684852fd2b039c225a` (`origin/main` when work began).
The candidate is this PR's implementation. Both use the pinned Rust 1.98.1 toolchain
and lockfile. No account, Discord connection, microphone or live media was used.

## Native synthetic screenshots

The screenshots are actual Serein X11 windows in a dedicated Xvfb display, using
Mesa llvmpipe (LLVM 20.1.2) through Vulkan, at scale 1.0. They are debug builds with
`--features demo`, launched with `--demo --demo-settings=general`. The source for
the baseline is an untouched separate worktree. Workspace artifacts were cleared
before switching the shared build target between source revisions.

The wide window is 1280 × 950; the narrow window is 840 × 650. General is scrolled
to the bottom in both baseline and candidate. The fixture ignores saved preferences
and never persists these edits.

- `before.png`: Graphics has the GPU preference and steady refresh switch.
- `after.png`: the new Graphics API row, with Automatic selected.
- `after-menu.png`: Linux build choices: Automatic, Vulkan and OpenGL.
- `after-dark-narrow.png`: OpenGL selected in dark mode at the narrow size. The
  current adapter still says Vulkan until restart; changing the choice does not
  recreate an active renderer.
- `after-env.png`: a separate launch with `WGPU_BACKEND=gl`; the running renderer
  is OpenGL and the picker is disabled with an explanation of the override.

Pointer selection of OpenGL, both palettes, the menu and the narrow description/control
layout were inspected. Windows/macOS surfaces and the reporter's older GPU remain untested.

To reproduce, build each revision with:

```sh
cargo build --locked -p serein --features demo --bin serein
DISPLAY=:99 target/debug/serein --demo --demo-settings=general
```

Use a dedicated Xvfb display with a 1360 × 1040 screen, resize the app window to the
sizes above, scroll the General pane to its bottom and capture only that window.
For the override state, launch the candidate with `WGPU_BACKEND=gl` as well.

## Measurements

Standard voice-enabled release packages were built separately with `cargo xtask package`
and the repository's unchanged fat-LTO release profile. Executable bytes come from
`dist/serein`; installed bytes sum regular files extracted from the DEB; compressed
bytes are the DEB size. Both packages undergo the existing package smoke validation.

The supplementary native process samples use the debug demo builds above, not the
standard release binaries. Each uses one 10-second warmup after scrolling, followed
by 20 samples at one-second intervals. RSS is `/proc/<pid>/status`'s `VmRSS`; CPU is
the difference in user/system ticks divided by elapsed time and clock ticks per
second (100% means one core). The process is terminated after sampling. The UI is
idle with the picker closed and Automatic selected in both cases. Concurrent
compilation is uncontrolled noise. Xvfb and compiler resource use are excluded and
were not sampled. These measurements establish no GPU speed improvement, release
CPU/RSS result, frame latency or affected-Windows-hardware compatibility.

The environment is Ubuntu 24.04 x86_64 in Docker on an Intel Core i5-1235U host with
30 GiB RAM visible to the container. Exact values and package deltas are in
`docs/performance.md` under Graphics API picker.

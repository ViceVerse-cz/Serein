# DeepFilterNet native evidence

Both images are actual offline Serein windows on macOS 27.0, Apple M1,
1120×760 logical pixels at 2× scale, eframe/wgpu Metal, dark appearance.
No account, microphone, call or fabricated runtime status is used.

- `before.png`: baseline `306bccdbb4d28fa83dac09260772917d3d8b0018`, freshly built
  with `cargo build --locked -p serein --features demo`, launched
  `--demo --demo-settings=voice`; original Voice Isolation uses RNNoise.
- `after.png`: this branch, same build flags and viewport, launched
  `--demo --demo-settings=voice-processing`. This new fixture selects Custom and
  scrolls to the new Auto and strength controls. It starts no model or probe.
- Voice controls are dimmed by the existing offline-preview policy in both images.
  Dark and light native renderings were inspected; narrow 320/640-point layouts
  and selection events are covered by headless UI tests.

Captured through macOS ScreenCaptureKit for the owned demo process/window only.
Accessibility input injection is unavailable, so native keyboard and interactive
scroll checks remain outstanding; fixture positioning is not evidence of input
automation. No UI frame, startup or process CPU timings are inferred from images.

`performance.txt` preserves synthetic component measurements and resource reports;
see `docs/performance.md` for methods, package sizes and limitations.

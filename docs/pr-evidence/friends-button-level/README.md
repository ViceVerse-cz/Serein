# Friends button evidence

Baseline: `e5fe9f0b` (`main`). Both demo binaries were built with the pinned Rust 1.99.1 and
`--locked -p serein --features demo` on Windows 11 (10.0.26200), as native wgpu windows at
100% scale (96 dpi) and 1120 × 760. The baseline is `main` plus this change's demo-only
`--demo-accent=rrggbb` flag, which `main` lacks; nothing else differs. Both captures use
`--demo --demo-friends --demo-accent=c084fc`, a custom light purple accent. No account,
network or live session was used.

- `before.png` / `after.png`: 6× nearest-neighbour crops of the Direct Messages search row.
- `before-window.png` / `after-window.png`: the full native captures.

Measured on the captures: on the baseline, Find conversation spans rows 93–124 (32 pt, the
theme's control height) and the Friends square rows 94–123 (30 pt). In the changed build both
span rows 92–123, with the 6 pt gap and the 8 pt sidebar margin kept, and the list below starts
2 pt lower with the taller row. Outside that sidebar the two captures are identical. A theme
with a taller control height, such as the 38 pt one in the report, gets a square of that height
instead of 30 pt; that palette was not captured.

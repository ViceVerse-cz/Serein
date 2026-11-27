# Accent text coverage evidence

Baseline: `994b3128`, the head of #1176 (`main` plus the levelled Friends button and the
demo-only `--demo-accent` flag). Both demo binaries were built with the pinned Rust 1.99.1 and
`--locked -p serein --features demo` on Windows 11 (10.0.26200), as native wgpu windows at
100% scale (96 dpi) and 1120 × 760.

`--demo-accent=c084fc` is a light purple whose accent text is black, since white stays under
4.5:1 on it. No account, network or live session was used.

- `before.png` / `after.png`: 6× nearest-neighbour crops of the friends header around
  "Add Friend" (`--demo --demo-friends --demo-accent=c084fc`), above the Themes toolbar around
  "Create theme" (`--demo --demo-settings=themes --demo-accent=c084fc`).
- `*-friends-window.png` and `*-themes-window.png`: the full native captures.

Measured on the captures (one pixel per point): the only pixels that differ between the two
builds are label glyphs, 375 px in "Add Friend" and 486 px in "Create theme". Glyph coverage
summed over "Add Friend" falls from 309 to 249 (−19%) and over "Create theme" from 350 to 276
(−21%), and the label bounds do not move. With the default palette (white on azure) no twin
faces are published; a capture of that palette on `main` and on a build with only this change
was pixel-identical.

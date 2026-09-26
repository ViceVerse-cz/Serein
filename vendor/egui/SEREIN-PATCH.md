# Arabic and bidirectional text

Source: https://github.com/emilk/egui/tree/fe6d63efa4a4df6f56ceab814d4f3a6efab69b88
(version 0.36.2). Only `crates/egui` and `crates/epaint` are vendored, with the upstream
README, MIT/Apache-2.0 licenses and lint configuration. Workspace members are narrowed
to those two crates; other sibling dependencies retain that exact upstream git revision.
Serein's root Cargo.lock remains authoritative. No dependency versions are upgraded.

The shared renderer replaces Serein's message-only bidi workaround. `text_layout.rs` and
`text_layout/bidi.rs` resolve directions using the already-resolved unicode-bidi 0.3.18,
shape directional font runs with HarfRust, retain logical scalar slots (including marks,
ligatures and glyph expansions), and place wrapped lines in visual order. Shaped clusters
are not split when wrapping. Backgrounds/underlines use visual order; glyph mesh intervals
stay logical for selection. ASCII skips bidi analysis and reordering.

`text_layout_types.rs` supplies directional caret geometry and visual arrow navigation.
`egui/text_selection/visuals.rs` paints disjoint selection/IME ranges. AccessKit text and
selection indices stay logical; homogeneous RTL chunks get right-edge-relative geometry.
Mixed-direction AccessKit chunks omit detailed character geometry. A small test helper in
`font_provider.rs` clones the now non-Copy Glyph, which can retain extra glyph artwork.

The application removes pre-layout message-string reordering and keeps inline artwork
positions compatible with RTL glyph slots. Stored/sent text is never reversed.

Offline debug check (no account/network/devices):

```sh
cargo run --locked -p ui --features demo --example arabic
```

This fast local pass does not validate native Windows/IME/screen readers or measure
performance. Word-wise navigation and complex-script shaping across separate rich-text
format sections retain upstream limitations. Remove the fork once upstream provides a
compatible bidi layout/editing implementation; do not substitute string reversal.

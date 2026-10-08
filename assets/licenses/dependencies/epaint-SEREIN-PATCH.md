# Exact RTL shaping metadata for Serein

Source: epaint 0.36.2 from egui revision `72bc6574978d87fe0929b1d590c35222e6fd8935`.
All upstream Rust sources and README were copied from that revision. The original manifest is
retained in Cargo.toml.orig, with the exact upstream MIT and Apache-2.0 texts. The standalone
manifest expands inherited dependency versions/features and keeps sibling emath, ecolor and
epaint_default_fonts on the same Git revision. Unused upstream benchmarks/dev dependencies
are omitted; the consuming workspace owns validation. No font, shaping-library, rasterizer,
system-font or encryption dependency is added or replaced.

The patch adds an opt-in single-line shaping API exposing original UTF-8 cluster ranges and
their exact native advances, plus explicit per-section direction on that API. Existing TextFormat/LayoutJob structs and default layout calls
retain upstream behavior. Serein performs bounded logical line breaking and per-line Unicode
bidi ordering, then uses the native cluster map for painting, hit testing and logical copy.
The single-line path avoids upstream's ascending-only continuation-glyph bookkeeping for RTL.

Remove this fork when upstream exposes equivalent direction and cluster mapping APIs with
correct wrapped bidi layout. This patch does not establish bidirectional TextEdit, IME or
accessibility parity. No source instructions in upstream files were adopted as repository policy.

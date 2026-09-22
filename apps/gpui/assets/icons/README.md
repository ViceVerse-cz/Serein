# GPUI experiment icons

Unmodified SVGs from [Phosphor Icons](https://phosphoricons.com) 2.1.1 (npm package
`@phosphor-icons/core`, MIT; the license is `assets/icons/LICENSE` at the repository root). They
are the same pinned files, verified by the same SHA-256 values, that `tools/generate-icons.py`
rasterizes into the egui atlas. `apps/gpui/assets/fetch-icons.py` re-downloads and verifies
them. GPUI compiles them into the executable and tints them at draw time. The Serein mark is
read directly from `assets/brand/serein-mark.svg`.

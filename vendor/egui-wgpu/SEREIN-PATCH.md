# Serein HDR surface patch

This copies only egui-wgpu from egui revision
`4821fc54ae083ac516e137a61e70550330e55169` (0.36.2). The rest of the GUI stack remains
at that revision. Upstream MIT and Apache-2.0 license texts are retained unchanged.
The standalone manifest expands upstream workspace dependencies without upgrading them.

The patch selects linear extended-sRGB HDR presentation only when that exact
format/color-space pair is advertised on an opaque surface. It polls current display
capabilities, retains an SDR fallback, and preserves textures and callbacks while
switching render pipelines. Ordinary UI colors still map to SDR reference white.
Screenshots use the ordinary SDR path. Media callbacks receive current output format,
reference white and display headroom; tone mapping remains the media renderer's job.

The parent's October 2026 egui update leaves egui-wgpu sources and licenses
unchanged. The standalone dependency pins follow that update, retaining the
HDR patch without a second egui/epaint revision.

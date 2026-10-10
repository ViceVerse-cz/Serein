# Nix package

The flake exposes `serein` (also the default package) and a development shell for
`x86_64-linux` and `aarch64-darwin`. Build on the matching host:

```sh
nix build .#serein
nix run .#serein
nix develop        # cargo, rustc and the native build inputs
```

Linux installs `result/bin/serein`, a `cz.viceverse.serein.desktop` entry and icons.
macOS installs `result/Applications/Serein.app` with `result/bin/serein` linking to
the bundle executable. Both include the notices, font/asset licenses and the
corresponding MPL-2.0 hpke-rs source under `result/share/doc/serein` (inside the
bundle `Resources` on macOS).
The outgoing-video libraries use the same pinned LGPL-only FFmpeg/OpenH264
source recipe as native releases, as a separate Nix store dependency. FFmpeg's
complete corresponding source and build recipe ship under `ffmpeg-source` in
that documentation directory. `nix develop` sets `FFMPEG_DIR` to this build.
No system GPL FFmpeg is linked into this package.
All native encoder dependencies use the Nix stdenv C/C++ compiler wrappers,
including FFmpeg's explicit configure compiler options on macOS.
The macOS encoder build uses nixpkgs' `darwin.sigtool` and `cctools` for
ad-hoc signing after changing library install names; no host signing tools or
Developer ID credentials are needed.
Linux x64 includes pinned AMF headers and a static oneVPL dispatcher. libva/libdrm
are Nix dependencies for Quick Sync device setup, while compatible Intel/AMD/NVIDIA
GPU runtimes remain system-provided. Experimental FFmpeg's H264/HEVC/AV1 VA-API
encoders are excluded; H264 has OpenH264 software fallback, while HEVC/AV1 need
compatible hardware. Stable retains its original GStreamer VA-API/NVENC H264
path from the existing Base/Good/Bad plugin dependencies, with Rust OpenH264
fallback. Vendor factories depend on nixpkgs' plugin build and installed drivers.
The package retains exact vendor source
and notices for the same offline rebuild recipe as native distributions.

The package version is the Cargo workspace version, as with `cargo build`. Nix
installations are updated through Nix; the in-app updater does not replace store
paths. The macOS bundle is not Developer ID signed or notarized.

CI builds both systems and compares installed notices and source with the checkout.
A successful build does not establish live Discord, keyring, screen capture or
physical audio compatibility. Linux still needs a graphical session, GPU driver,
Secret Service provider and an XDG desktop portal backend.

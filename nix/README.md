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
The outgoing-video libraries use the same pinned FFmpeg/OpenH264
source recipe as native releases, as a separate Nix store dependency. FFmpeg's
complete corresponding source and build recipe ship under `ffmpeg-source` in
that documentation directory, alongside the FFmpeg-derived GPU camera filter source.
`nix develop` sets `FFMPEG_DIR` to this build.
The bundled FFmpeg is GPL-2.0-or-later; no system FFmpeg is linked into this package.
All native encoder dependencies use the Nix stdenv C/C++ compiler wrappers,
including FFmpeg's explicit configure compiler options on macOS.
The macOS encoder build uses nixpkgs' `darwin.sigtool` and `cctools` for
ad-hoc signing after changing library install names; no host signing tools or
Developer ID credentials are needed.
Linux x64 includes pinned AMF headers and a static oneVPL dispatcher. FFmpeg can
use compatible NVIDIA, AMD and Intel encoders when their GPU runtimes are available;
H.264 also has an OpenH264 software fallback. Direct VA-API encoders are excluded,
though VA-API is used for supported decoding and Quick Sync device setup.
The package retains the vendor source and notices for the same offline rebuild
recipe as native distributions.

AMD encoding requires a compatible AMF runtime and its dependencies. On NixOS,
the launcher searches `/run/opengl-driver/lib`; `pkgs.amf` can be added to
`hardware.graphics.extraPackages` when available in the system's nixpkgs. AMF and
its `amdenc` dependency are unfree and need the corresponding nixpkgs permission.
The runtime, Vulkan driver and GPU must also be compatible. If the probe reaches
AMF and cannot load it, Video encoding settings show that result instead of
claiming the GPU is unsupported.

A first source build may fetch a Rust toolchain and other build dependencies.
The installed closure and fresh-build download are different sizes; use
`nix path-info -Sh .#serein` after building to measure the closure, including
paths already shared with other packages.
Nix builds use the minimal official Rust toolchain pinned in
`rust-toolchain.toml`. The development shell adds that file's Clippy and
rustfmt components.

The package version is the Cargo workspace version, as with `cargo build`. Nix
installations are updated through Nix; the in-app updater does not replace store
paths. The macOS bundle is not Developer ID signed or notarized.

CI builds both systems and compares installed notices and source with the checkout.
A successful build does not establish live Discord, keyring, screen capture or
physical audio compatibility. Linux still needs a graphical session, GPU driver,
Secret Service provider and an XDG desktop portal backend.

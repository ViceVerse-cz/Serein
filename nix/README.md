# Nix package

The flake exposes `serein` and the default package for `x86_64-linux` and
`aarch64-darwin`. Build on the matching host:

```sh
nix build .#serein
# Explicit equivalents:
nix build .#packages.x86_64-linux.serein
nix build .#packages.aarch64-darwin.serein
```

Linux installs `result/bin/serein`, a desktop entry and icons. macOS also installs
`result/Applications/Serein.app`. Both include notices, font/asset licenses and
bundled corresponding source under `result/share/doc/serein`; the macOS bundle
links to these files from `Contents/Resources/documentation`.

The pinned nixpkgs snapshot comes from `staging-next`, after the Swift 6.2.4 update
(NixOS/nixpkgs#557896). As of September 26, 2026, that update is not yet in
`nixos-unstable`; returning to that channel requires a revision with Swift 6.
The Swift toolchain supplies its compatible Apple SDK. SwiftPM builds the capture
bridge, while Cargo owns the application build and install phases.

CI evaluates both systems, builds on Linux and macOS, and compares installed
notices and source with the checkout. Build/package checks do not establish live
Discord, desktop, keyring, screen capture or physical audio compatibility. Linux
still needs a graphical session, GPU driver, Secret Service provider and a portal
backend appropriate for the desktop. Manage updates through Nix; the store is
read-only. The macOS bundle is not Developer ID signed or notarized.

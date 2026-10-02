# AUR binary recipe

This directory prepares `serein-bin` for an AUR maintainer. It is **not an AUR
publication**, and no AUR package availability or ownership is claimed. The recipe
repackages the project's existing, voice-enabled x86_64 Arch release without
compiling Rust or configuring an additional pacman repository. It currently pins
nightly `v1.0.0-nightly.20261001.53`, verified against that release's `SHA256SUMS.txt`.
The native package's dependency version floors, desktop integration, icons, notices
and bundled licenses are preserved. ARM Arch binaries are not provided.

Review `PKGBUILD`, then run on an up-to-date Arch host as an unprivileged user:

```sh
cd packaging/aur
makepkg --verifysource
makepkg -si
```

These are owner installation instructions; automated validation never installs
or launches Serein. `serein-bin` conflicts with the repository/source `serein`
package and a future `serein-git` variant because they install the same executable.
Pacman handles a deliberate replacement. Account caches and saved credentials
are unaffected by package removal; see the normal storage policy.

The AUR maintainer should review the source URL/hash and synchronize dependencies
from the verified Arch release's `.PKGINFO` whenever updating `pkgver`. Keep the
`pre.nightly` version conversion used by native Arch packaging; do not put `-`
in `pkgver`, and never replace the checksum with `SKIP`. Generate and review
`.SRCINFO` with `makepkg --printsrcinfo > .SRCINFO` before submitting. Publishing
requires a maintainer-controlled AUR account and package name; this repository's
GitHub push/release automation does not publish to the AUR.

[Arch's PKGBUILD reference](https://man.archlinux.org/man/PKGBUILD.5.en) documents
checksum, metadata and package-function behavior. Native Arch makepkg validation
runs with the Linux packaging job; desktop startup, media devices and live Discord
interoperability still need owner testing.

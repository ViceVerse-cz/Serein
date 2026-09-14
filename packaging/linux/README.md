# Linux packages

Release builds provide native packages for Ubuntu 26.04 (`apt`), Fedora 44 (`dnf`),
openSUSE Tumbleweed (`zypper`) and Arch (`pacman`), plus a
[Flatpak bundle](../flatpak/README.md) for distributions with a compatible Flatpak runtime.
The [AppImage](../appimage/README.md) supports in-app updates on Linux x86_64 with
the documented host GTK4/WebKit6 runtime; its release build targets Ubuntu 26.04.
Download the file labelled for your distribution from
[Releases](https://github.com/ViceVerse-cz/Serein/releases), then use its actual filename:

```sh
sudo apt install ./serein-*.deb                     # Ubuntu 26.04
sudo dnf install ./serein-*.fc44.*.rpm              # Fedora 44
sudo zypper install ./serein-*.suse.*.rpm           # openSUSE Tumbleweed
sudo pacman -U ./serein-*.pkg.tar.zst               # Arch
flatpak install --user ./serein-*.flatpak           # Flatpak bundle
```

Use a directory containing only the selected package. Local build artifacts are
unsigned; package-manager signature policy may require an operator-signed package.
The [signed repository setup](../repositories/README.md) prepares apt, dnf/zypper
and pacman repositories for normal package-manager upgrades. Hosting and signing
credentials must be configured before those repository URLs are usable. Serein is
not listed in distribution archives, AUR or Flathub by this change.

Native DEB, RPM and Arch packages require the GStreamer Good plugin set, which
provides `autoaudiosink` used by WebKit. Package-manager installation pulls it in
even when recommended/optional packages are disabled. This changes future packages,
not already-published releases. Source builds and extracted directory archives do
not install system dependencies; on Arch/CachyOS install `gst-plugins-good` yourself.

## Native builds

Build on the target distribution; converting an Ubuntu binary to RPM or Arch does
not make its shared libraries compatible. The release workflow builds each format
inside its matching distribution container, as an unprivileged user. CI currently
targets x86_64; native Debian/RPM staging also validates aarch64 ELF headers.

```sh
cargo xtask package --format deb    # Debian/Ubuntu; also the default Linux format
cargo xtask package --format rpm    # Fedora or openSUSE
cargo xtask package --format arch   # Arch; makepkg must run without root
cargo xtask package --format dir    # dist/linux-root/usr, for the Flatpak SDK build
cargo xtask package --format appimage # requires packaging/appimage/install-tools.sh first
```

`install-build-deps.sh` installs build dependencies as root on the explicitly
supported CI distributions. It is intended for fresh build containers. Normal
packaging runs without root and never installs or starts Serein. RPM uses
`rpmbuild` dependency generation; Arch derives native library package dependencies
from the host package database and uses `makepkg`. Both inspect package metadata,
payload contents/permissions and the native executable's library closure. Runtime
Vulkan/EGL, Wayland/X11, portal and credential-service requirements remain explicit.

Run the synthetic package checks on the corresponding build distribution:

```sh
python3 packaging/linux/test_package.py --format deb
python3 packaging/linux/test_package.py --format rpm
python3 packaging/linux/test_package.py --format arch
```

These package `/bin/true`, never Serein or a live account. Native application
builds and package inspection do not prove desktop login, graphics or physical audio.

## Debian / Ubuntu details

On a Debian/Ubuntu Linux build host, `cargo xtask package` produces the standard
`dist/serein_<version>-1_<architecture>.deb` including voice. Native `amd64` and `arm64`
ELF headers are accepted; a build does not prove desktop or audio support on that
architecture. These are unsigned host-distribution packages, not portable Linux
archives or a promise of compatibility with older distributions.

Install the native source-build dependencies in `docs/platform-support.md`, plus
`python3 dpkg-dev desktop-file-utils`. The packager uses Python's standard library,
`dpkg-shlibdeps`, `dpkg-deb`, `desktop-file-validate`, and `ldd`. It needs no root
privileges. It creates a fresh temporary staging tree on the native Linux
filesystem, normalizes file permissions and desktop-file line endings, and copies
only current license paths from the staged release output. Repository documentation
and source trees are not included. It excludes stale archives, nested voice outputs,
logs, and stale license files. Temporary files are removed when packaging finishes
or raises an error.

The archive installs `/usr/bin/serein`, a launcher in
`/usr/share/applications/org.serein.desktop.desktop`, and notices and licenses under
`/usr/share/doc/serein`. No maintainer
scripts, background updater, automatic launch or user-profile writes are added.
For a deliberate manual installation, use the local file:

```sh
sudo apt install --reinstall ./dist/serein_0.1.0-1_amd64.deb
```

These are user installation instructions; the build and smoke checks do not run
them. Remove the application with `sudo apt remove serein`; normal package
removal does not delete account data. Use in-app logout/cache controls as described
in the storage policy.

Dependencies are derived from the actual ELF using the host distribution's
installed shared-library symbols metadata. Missing libraries or dependency
metadata fail packaging. Explicit dependencies additionally cover dynamically
loaded Vulkan/EGL, X11/Wayland libraries and the D-Bus/desktop-portal services that
ELF inspection cannot discover. A working graphical session, graphics driver,
portal backend and unlocked Secret Service provider are still necessary for the
corresponding features. The package recommends a GTK or KDE portal backend and
GNOME Keyring; an existing compatible provider can be used instead. The GStreamer
good plugin set is required; base and libav remain recommended for inline attachment
video loaded at run time through `decodebin`; without the needed codecs the player
reports an unsupported format. GTK/WebKit
and voice library requirements come from the built executable. The
resulting version constraints target the build distribution; inspect `Depends`
with `dpkg-deb --field <package.deb> Depends` before distributing elsewhere.

Every package is inspected before being copied to `dist`: metadata, allowed file
paths, root ownership, executable/data permissions, exact contents, absence of
maintainer scripts, valid desktop syntax and the host ELF library closure must
pass. This does not launch the application, install the package, access credentials,
initialize audio, or prove Wayland/X11, authentication, accessibility or live calls.
Run the small additional regression check without building the application:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 packaging/linux/test_package.py
```

It deliberately supplies synthetic documentation and source trees, verifies they are
omitted, then checks mismatched payload and invalid ELF detection. The fixture is not
a Serein build.

Tool contracts: [dpkg-shlibdeps](https://manpages.debian.org/trixie/dpkg-dev/dpkg-shlibdeps.1.en.html)
and [dpkg-deb](https://manpages.debian.org/trixie/dpkg/dpkg-deb.1.en.html).

# Linux AppImage

Download `serein-<version>-Linux-X64.AppImage` from
[Releases](https://github.com/ViceVerse-cz/rustcord/releases), keep it in a writable
directory, and make it executable:

```sh
chmod +x ./serein-<version>-Linux-X64.AppImage
./serein-<version>-Linux-X64.AppImage
```

The x86_64 image contains Serein including voice, its desktop entry, icon and
application licenses. It uses **host runtime libraries**, including GTK4 and
WebKitGTK 6.0; it is not a self-contained distribution of those libraries. Release
builds use Ubuntu 26.04, so older distributions and incompatible native library
versions are not supported by this artifact. Use the distribution packages or
Flatpak where their supported runtime is a better match.

On Ubuntu 26.04, install the runtime dependencies once:

```sh
sudo apt update
sudo apt install libgtk-4-1 libwebkitgtk-6.0-4 libasound2t64 libfontconfig1 \
  libgstreamer1.0-0 libgstreamer-plugins-base1.0-0 \
  libvulkan1 libegl1 libxkbcommon0 libxkbcommon-x11-0 \
  libwayland-client0 libx11-6 libx11-xcb1 libxcursor1 libxi6 libxrandr2 \
  dbus-user-session xdg-desktop-portal xdg-desktop-portal-gtk gnome-keyring \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-libav
```

The WebKitGTK package supplies its matching browser subprocesses, data files and
GTK dependencies. Serein does not relocate or patch WebKit, disable its sandbox,
or override the host library search path. A graphical session, graphics driver,
portal backend and unlocked Secret Service provider are still required. A KDE
portal/keyring provider can replace the GNOME choices above. No library or desktop
service is installed by launching the AppImage.

The embedded runtime includes FUSE support without requiring the old `libfuse2`
package. Systems that cannot mount AppImages can run:

```sh
./serein-<version>-Linux-X64.AppImage --appimage-extract-and-run
```

Settings → Updates uses the same Production/Nightly channels and automatic-download
preference as Windows/macOS. Restart applies a checked download to the original
AppImage filename. Both the file and its directory must be writable, and the
filesystem must support hard links for the rollback copy (for example ext4 or
Btrfs; FAT/exFAT require manual replacement). An extracted
`squashfs-root/AppRun` and native/Flatpak installations use manual or package-manager
updates. Keep the outer AppImage file in place while Serein is running.

## Build and pipeline

On the Ubuntu 26.04 x86_64 build host, first install the existing
[native build dependencies](../linux/README.md), then:

```sh
bash packaging/appimage/install-tools.sh
cargo xtask package --format appimage
```

The installer downloads [appimagetool 1.9.1](https://github.com/AppImage/appimagetool/releases/tag/1.9.1)
and [Type 2 runtime 20251108](https://github.com/AppImage/type2-runtime/releases/tag/20251108),
verifying each against its pinned SHA-256 before use. The packager reuses the native
payload allowlist, rejects unresolved host libraries, verifies the output's Type 2
x86_64 header, and extracts it to compare packaged file contents and executable
permissions. It never starts Serein or opens a session. Packages over the updater's
512 MiB limit fail the build. Runtime sources and build instructions are available
at the pinned runtime release; its license and third-party notices are included.
The runtime statically links third-party components including LGPL libfuse; its
[exact source revision](https://github.com/AppImage/type2-runtime/tree/dd6cebedcbddde9c82f89b011e8e1d40b6e43868)
and [build/relink instructions](https://github.com/AppImage/type2-runtime/blob/dd6cebedcbddde9c82f89b011e8e1d40b6e43868/BUILD.md)
identify the build recipes and dependency sources. A replacement runtime can be
supplied to appimagetool with `--runtime-file`. These references preserve upstream
source/relink information; this fast pass does not certify redistribution license
coverage. License review remains in the dedicated license CI workflow.

The Ubuntu job in `linux-packages.yml` builds the AppImage alongside the `.deb`,
then uploads the exact release name above in its existing artifact. The release
workflow gathers it, writes `SHA256SUMS.txt`, and publishes both through the existing
release flow. Local artifacts use the workspace version; release filenames use the
release tag, including its `v` prefix and any prerelease suffix. Release publication
is not performed by local packaging.

AppImage packaging, Linux desktop startup, live authentication/audio and an actual
release-to-release AppImage upgrade remain unverified by the initial fast local pass.

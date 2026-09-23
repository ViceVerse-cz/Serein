# Flatpak

Build on native Linux with Python 3.11+, Git, rustup and the repository's pinned
Rust 1.98.1 toolchain installed. GNOME SDK/Platform 49 supplies GTK4/WebKit6 and
native media/build libraries. The toolchain is copied into build-only sources;
no moving Rust SDK extension or compiler is shipped in the application.

```sh
flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install --user --noninteractive --no-related flathub org.gnome.Sdk//49 org.gnome.Platform//49
python3 packaging/flatpak/build.py target/flatpak-build
```

Install `flatpak` and `flatpak-builder` with your distribution's package manager
first. The destination must not exist. Preparation copies tracked working-tree
sources and downloads exactly Cargo.lock's registry/Git dependencies with
`cargo vendor --locked`. `--prepare-only` stops after this network-enabled step.
The actual application build is offline inside Flatpak's build sandbox, using
the standard release configuration including voice and bundled notices/source.
The build produces:
- `target/flatpak-build/Serein-linux.flatpak`: single-file standalone bundle
- `target/flatpak-build/repo/`: exported static OSTree repository with static deltas
- `target/flatpak-build/serein.flatpakref`: one-click repository install file

### Installing and Automatic Updates

#### Option A: One-click repository install (Recommended for automatic updates)
To install Serein configured to receive automatic updates from the hosted OSTree repository:

```sh
flatpak install --user packaging/flatpak/serein.flatpakref
# Or from a published URL:
# flatpak install --user https://viceverse-cz.github.io/Serein/flatpak/serein.flatpakref
```

Once installed via `.flatpakref`, your desktop environment (GNOME Software, KDE Discover)
and `flatpak update` will automatically discover and install new releases.

If installation reports `No such ref 'app/cz.viceverse.serein/x86_64/master'`,
check that the hosted `flatpak/repo/summary` exists. Publishing the `.flatpakref`
alone is insufficient. The package-repository workflow imports the release's
checksum-verified Flatpak bundle and requires the app ref before uploading the
site; missing bundles or refs fail publishing. To repair an incomplete site,
run the corrected workflow with deployment enabled for a release tag containing
all native build-matrix assets (including Fedora 43) and the Flatpak bundle.
Until it is republished, use the standalone release bundle below.

The in-app **Settings -> Updates** screen automatically detects when Serein is running
inside Flatpak, checks GitHub releases, and prompts you to update through `flatpak update`
or your desktop software manager when a new release is available.

#### Option B: Standalone bundle (Offline install)
```sh
flatpak install --user ./target/flatpak-build/Serein-linux.flatpak
flatpak run cz.viceverse.serein
flatpak uninstall --user cz.viceverse.serein
```

### Sandbox Permissions

The sandbox grants network, graphics, Wayland and X11 access, audio and
specific Secret Service/notification D-Bus names and `org.kde.StatusNotifierWatcher`
for the tray. A StatusNotifier host must be running; no additional bus-name ownership
or blanket session-bus permission is required. X11 access is required as an
automatic clipboard fallback on Wayland compositors without the data-control
protocol. Files are selected through
the existing desktop portal; home and session-bus access are not granted.
Caches/preferences use Flatpak's isolated XDG directories under
`~/.var/app/cz.viceverse.serein`; a native installation's data is not imported.
Saved credentials still require the host's unlocked Secret Service and never
fall back to files. That service permission is not an application-specific
credential isolation guarantee. Audio access permits microphone use, but Serein's
existing explicit call/device-testing gates still apply.

Sandboxed login, keyring, file chooser, notifications and physical audio require
owner-controlled Linux desktop validation. Screen sharing uses the ScreenCast portal
and PipeWire; optional system audio monitors individual applications through native
libpulse and the existing PulseAudio socket. Serein's playback and applications without
a usable identity are excluded. No additional sandbox permissions are needed.
The camera adapter uses direct V4L2, with no camera portal; camera capture is
unavailable under these permissions. Do not grant blanket devices/home access to hide
these limitations.

Game Activity binds `discord-ipc-N` in the sandbox's private `$XDG_RUNTIME_DIR`,
which the host sees as `$XDG_RUNTIME_DIR/.flatpak/cz.viceverse.serein/xdg-run`
(the same layout as the Vesktop Flatpak). Leftovers from a crash are replaced on
the next start. Host games need a link, either for the current session:
`ln -sf "$XDG_RUNTIME_DIR"/{.flatpak/cz.viceverse.serein/xdg-run,}/discord-ipc-0`,
or on every login:

```sh
mkdir -p ~/.config/user-tmpfiles.d
echo 'L %t/discord-ipc-0 - - - - .flatpak/cz.viceverse.serein/xdg-run/discord-ipc-0' > ~/.config/user-tmpfiles.d/discord-rpc.conf
systemctl --user enable --now systemd-tmpfiles-setup.service
```

Flatpak games additionally need `--filesystem=xdg-run/.flatpak/cz.viceverse.serein:create`
and `--filesystem=xdg-run/discord-ipc-0`. The loopback WebSocket transport needs no setup.

References: [Flatpak sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html),
[Cargo vendoring](https://doc.rust-lang.org/cargo/commands/cargo-vendor.html),
[GNOME 49 developer platform](https://release.gnome.org/49/developers/).

Preparation regression check (no network, compiler build, installation or login):
`python3 -m unittest discover -s packaging/flatpak -p 'test_*.py'`.

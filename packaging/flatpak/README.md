# Flatpak

Build on native Linux with Python 3.11+, Git, rustup and the repository's pinned
Rust 1.98.1 toolchain installed. GNOME SDK/Platform 49 supplies GTK4/WebKit6 and
native media/build libraries. The toolchain is copied into build-only sources;
no moving Rust SDK extension or compiler is shipped in the application.

```sh
flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install --user --noninteractive flathub org.gnome.Sdk//49 org.gnome.Platform//49
python3 packaging/flatpak/build.py target/flatpak-build
```

Install `flatpak` and `flatpak-builder` with your distribution's package manager
first. The destination must not exist. Preparation copies tracked working-tree
sources and downloads exactly Cargo.lock's registry/Git dependencies with
`cargo vendor --locked`. `--prepare-only` stops after this network-enabled step.
The actual application build is offline inside Flatpak's build sandbox, using
the standard release configuration including voice and bundled notices/source.
The generated bundle is `target/flatpak-build/Serein-linux.flatpak`. CI can upload
that file as an artifact; building it does not publish an application or repository.

```sh
flatpak install --user ./target/flatpak-build/Serein-linux.flatpak
flatpak run org.serein.desktop
flatpak uninstall --user org.serein.desktop
```

This is an unsigned local bundle, not a Flathub listing or an update repository.
Install a newer bundle to update; the in-app Linux updater delegates installation
to package management. Runtime updates remain managed by Flatpak.

The sandbox grants network, graphics, Wayland with X11 fallback, audio and
specific Secret Service/notification D-Bus names. Files are selected through
the existing desktop portal; home and session-bus access are not granted.
Caches/preferences use Flatpak's isolated XDG directories under
`~/.var/app/org.serein.desktop`; a native installation's data is not imported.
Saved credentials still require the host's unlocked Secret Service and never
fall back to files. That service permission is not an application-specific
credential isolation guarantee. Audio access permits microphone use, but Serein's
existing explicit call/device-testing gates still apply.

Sandboxed login, keyring, file chooser, notifications and physical audio require
owner-controlled Linux desktop validation. Linux screen sharing is not implemented.
The camera adapter uses direct V4L2, with no camera portal; camera capture is
unavailable under these permissions. Host game IPC is also isolated. Do not grant
blanket devices/home access to hide these limitations.

References: [Flatpak sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html),
[Cargo vendoring](https://doc.rust-lang.org/cargo/commands/cargo-vendor.html),
[GNOME 49 developer platform](https://release.gnome.org/49/developers/).

Preparation regression check (no network, compiler build, installation or login):
`python3 -m unittest discover -s packaging/flatpak -p 'test_*.py'`.

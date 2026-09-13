# Platform support and packaging

Target platforms are Windows, macOS and Linux. **macOS arm64, Windows x64 and Linux x64 have local build evidence.** macOS has native visual checks; Windows has offline tests and a process/window startup smoke check only. Minimum OS versions, other architectures, real screen-reader support and native login-method support are not certified.

The custom title strip requests a native window move on the initial primary-button press,
including over its nonselectable context title. It does not wait for egui's text/drag threshold.
Caption buttons and other clickable title-strip controls keep their own actions; Windows
double-click maximize/restore remains available. Synthetic egui input tests check command
dispatch, not native OS window movement, which still requires a desktop interaction check.

| Platform | Build/runtime requirements | Status |
|---|---|---|
| macOS | Rust 1.98.1, Xcode command-line tools; Metal/wgpu, system WKWebView, Keychain | Local arm64 build and native synthetic window tested on macOS 27.0 beta, Apple M1 Pro / 16 GiB |
| Windows | Rust MSVC toolchain, Visual Studio C++ build tools, system graphics drivers, WebView2 Runtime 101+ (current supported runtime recommended), Credential Manager | Local x64 checks and unsigned release packaging on Windows 11 build 26200; synthetic process/window startup passed. Visual interaction, InPrivate behavior, IME and accessibility unverified |
| Linux | Rust, C compiler, pkg-config, GTK >=4.10, WebKitGTK 6.0, fontconfig, libxkbcommon, X11/Wayland development packages, Vulkan-compatible GPU/driver, Secret Service session bus/keyring | Ubuntu 26.04 x64 / WSL2 text and voice releases and Debian package smoke passed; X11/Wayland rendering and login window unverified |

Debian/Ubuntu development packages typically include `build-essential pkg-config libgtk-4-dev libwebkitgtk-6.0-dev libfontconfig1-dev libxkbcommon-dev libwayland-dev libx11-dev libxi-dev libxrandr-dev libxcursor-dev libvulkan-dev`. Package names vary by distribution. SQLite is bundled through rusqlite; it is an embedded client cache, with no database service.

Linux packaging uses the target distribution's native tools: `dpkg-dev` for Debian,
`rpm-build` for Fedora/openSUSE, or `makepkg` for Arch, plus Python 3 and
`desktop-file-utils`. See [Linux packaging](../packaging/linux/README.md) for build
dependencies, installation commands and supported distribution versions.

`cargo xtask package` builds the locked default release configuration. macOS gets `dist/Serein.app`; Windows gets an executable plus license files; Debian/Ubuntu Linux additionally produces a `.deb` with desktop integration and dependency metadata. On macOS, packaging replaces the executable through a fresh sibling file and rename, then seals the completed bundle with `codesign --force --sign -` and runs `codesign --verify --strict`. This is a **local ad-hoc signature**, with no signing identity, Developer ID certificate, or notarization. It verifies the staged bundle's integrity and does not certify Gatekeeper acceptance or a trusted publisher. The distinction between signature validity and trust is described in [Apple's code-signing guidance](https://developer.apple.com/library/archive/technotes/tn2206/_index.html).

Windows/Linux local staging artifacts remain unsigned. These are not certified installers. Use `ditto -c -k --keepParent dist/Serein.app dist/Serein-macos.zip` on macOS; normal archive tools may package Windows staging output. Do not modify bundle resources after sealing; rerun packaging when source documentation changes. Linux additionally supports `--format rpm`, `--format arch` and `--format dir`; release jobs build on Ubuntu 26.04, Fedora 44, openSUSE Tumbleweed and Arch independently. [Flatpak](../packaging/flatpak/README.md) builds offline against GNOME SDK 49 with the pinned Rust compiler and locked vendored sources. Its sandbox currently excludes direct V4L2 camera access and host game IPC; desktop login/keyring/audio still need Linux runtime validation. [Signed repository preparation](../packaging/repositories/README.md) supports apt, dnf/zypper and pacman, but requires configured signing credentials and an HTTPS host; preparing artifacts does not publish repositories. Windows installer/signing and release reproducibility remain open work.

The webview lives only during login: WKWebView on macOS, WebView2 on Windows, GTK/WebKitGTK on Linux. Linux uses a separate GTK authentication window and pumps it only while login is active. Voice is built in. Audio devices open only for explicit playback, device testing, or a call reaching required encrypted readiness. Popup-dependent authentication and third-party embedded challenges may not work; do not claim all Discord login methods without live tests.

## Built-in voice

`cargo run --locked` includes native DM and guild audio. Source builds require CMake and a C/C++ toolchain for statically bundled libopus; Linux also needs ALSA development headers (`libasound2-dev` on Debian/Ubuntu). CPAL uses native system audio. See [the voice adapter](../crates/discord-voice/README.md) for codec/protocol dependencies and limitations.

`cargo xtask package` stages the standard release including voice under `dist` (`dist/Serein.app` on macOS). The macOS bundle includes its microphone-use description; actual microphone permission, capture/playback, device switching and sleep/resume have not been exercised. Windows x64 voice release packaging and synthetic protocol/audio tests pass; physical audio and live calls remain unverified on Windows. Linux x64 text/voice release builds and Debian package smoke passed on Ubuntu 26.04 under WSL2; native Linux desktop/audio runtime remains unverified. CMake is a source-build dependency, not a runtime voice service.

Device choices and push-to-talk settings last only for the current session. Focused V push-to-talk has no global-key guarantee; use headphones because there is no acoustic echo cancellation. No signing, desktop integration, physical audio or live-compatibility claim follows from compilation alone.

Native emoji use eframe system-font fallback and installed OS color fonts. macOS
Apple Color Emoji was visually checked with a synthetic moon status on September
10, 2026. Windows/Linux emoji coverage is unverified and depends on installed fonts
(e.g. Segoe UI Emoji / Noto Color Emoji); no OS font is redistributed.


## Camera capture (September 12, 2026)

Outgoing in-call capture uses AVFoundation on macOS, Media Foundation and DirectShow on Windows,
and V4L2 on Linux. The existing camera button becomes available after the voice
server negotiates H264; capture starts only after an explicit click in a connected
call. All adapters send 640×480 video at most 15 encoded frames/s.
Windows needs desktop camera permission; Linux needs an accessible streaming
`/dev/videoN` node supporting progressive YUYV or MJPEG. Linux portal-only camera
access is not implemented. Windows has a device picker in settings and call controls,
including DirectShow virtual cameras; macOS/Linux still use their default selection.
See [camera limits and validation](voice.md#camera-in-calls-macos-windows-and-linux).
Windows compilation and isolated Linux adapter tests do not establish working
physical capture or delivery to an official Discord client; these remain unverified.

## Invite verification (September 13, 2026)

Invite verification uses a temporary WebView2 child on Windows, WKWebView child on
macOS, and a separate GTK4/WebKit6 window on Linux. It
loads a local verification page and hCaptcha's official widget after the user
chooses Verify. The local custom-protocol origin is
`https://serein-captcha.verification.invalid/` on Windows/Linux and
`serein-captcha://verification.invalid/` on macOS; it is not a Discord page, public
server or account-login surface. No account token enters it. Domain restrictions,
provider rejection and missing native webview runtimes fail visibly. macOS/Linux
live CAPTCHA acceptance remains unverified. Widget
loading and synthetic checks do not establish live Discord challenge acceptance.

## Opt-in tray icon (September 13, 2026)

Windows General settings offer Show Serein in System Tray, off by default. Minimizing
keeps the window in the taskbar, including taskbar clicks and automatic startup. The icon supports
keyboard/mouse restore and a Show Serein / Quit menu. Quit uses the normal unsaved
work/download exit checks; the window Close button retains normal exit behavior.
Disabling removes the tray icon without changing the window's minimized state.
The adapter uses existing user32/Shell APIs and dependencies, with no background
polling. A synthetic native Windows test verifies registration,
minimize/restore, own-window taskbar recovery, Quit event and cleanup. Linux/macOS have
an explicitly disabled control; their tray integration is not implemented.

## Opt-in automatic startup

General settings offer automatic launch at Windows sign-in and a dependent Start
Serein minimized preference. Both default off. Registration uses the current user's
Run key; no administrator access, service, scheduled task or new dependency is needed.
Windows Startup Apps can override this registration. Disable startup before deleting
a portable installation, or re-enable it after moving the executable.
Minimized launches stay in the taskbar even when the saved tray preference is enabled;
the tray can attach safely after a minimized launch. Tray failures leave the window
recoverable. The Close button still exits, and the tray Quit action retains unsaved
work checks. macOS/Linux autostart remains explicitly unavailable.
Offline tests cover isolated registry writes/removal, launch flags and settings
interaction; an actual Windows sign-out/sign-in has not been exercised.

## In-app updates

Settings → Updates provides automatic checking/downloading, Production and Nightly
release channels, a manual check and an explicit restart action. The title strip
shows an available or downloaded update on macOS and Windows. Update controls are
also accessible from the signed-out screen. Automatic checking runs at startup
once saved preferences are available, then every six hours while running; turning
it off disables automatic downloads while background checks and title-bar notices
remain active. Nightly is the default channel and automatic downloads are off by
default. Switching channels never installs an
older semantic version. Nightly checks inspect the latest 100 published releases.

Packages come from this repository's existing GitHub releases and must match the
platform/architecture asset name, published length and `SHA256SUMS.txt`. Downloads
and installation preparation run outside rendering; installation is handed off
only after the application's existing close/unsaved-work gates permit shutdown.
GitHub HTTPS and repository access are the update trust boundary; release checksums
alone are not an independent publisher signature. Linux uses its package manager.

The local `--features demo -- --demo --demo-check-updates` debug path exercises
synthetic update states, preference compatibility and settings rendering without
network access or replacing an installation. It is not evidence of a successful
live release upgrade or of Windows native installation behavior.

In-app installation requires an extracted Windows release or an installed,
writable macOS `.app` outside a mounted disk image/App Translocation. macOS checks
strict code-signature validity, the existing publisher's TeamIdentifier and bundle
identifier, and Gatekeeper acceptance. Windows currently relies on the repository's
HTTPS/checksum trust boundary because its published packages are unsigned. Native
helpers wait for the old process to exit, retain a rollback copy during replacement,
and relaunch Serein. A failed recovery leaves its backup available with a visible
recovery path on the next update attempt. Package-manager metadata is not modified.

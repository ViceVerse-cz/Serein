# Platform support and packaging

Target platforms are Windows, macOS and Linux. **macOS arm64, Windows x64 and Linux x64 have local build evidence.** The release workflow also targets Windows arm64 on a native GitHub Actions runner; build and runtime validation remain pending. macOS has native visual checks; Windows has offline tests and a process/window startup smoke check only. Minimum OS versions, other architectures, real screen-reader support and native login-method support are not certified.

Windows defaults to DirectX 12 to avoid reported startup access violations in Intel's
Vulkan driver (`igvk64.dll`). The existing `WGPU_BACKEND` environment override remains
available (for example, `dx12` or `vulkan`). Affected users confirmed that forcing
DX12 launches successfully; the new default still needs native Windows validation.
macOS and Linux retain their existing backend defaults.

Windows turns off winit's undecorated drop-shadow hack after the window exists, including
after title-bar changes. egui-winit enables that hack for custom chrome. While restored it
adds one pixel to `WM_NCCALCSIZE` top and bottom. Maximizing skips the shift, which is why
only that state looked sharp. `ViewportBuilder::with_has_shadow` is macOS-only and does not
disable the Windows hack. Native DPI and eframe's physical surface sizing remain unchanged.
macOS/Linux window creation is unchanged.
For offline inspection, run `cargo run --locked -p serein --features demo -- --demo --demo-rendering`.
The diagnostic shows the physical client size, logical viewport, native/egui scale and WGPU
surface dimensions sampled by a render callback, plus alternating one-pixel stripes.
The surface sample is from the previous paint: compare at rest after resizing, maximizing,
restoring, toggling the title bar and moving between monitors (including mixed DPI).
Surface/client dimensions should match and stripes/text/images should stay sharp. If dimensions
match but blur persists, the surface-resolution hypothesis is not established; investigate
driver/DWM presentation on that machine. Windows visual acceptance remains unverified.

The custom title strip requests a native window move on the initial primary-button press,
including over its nonselectable context title. It does not wait for egui's text/drag threshold.
Caption buttons and other clickable title-strip controls keep their own actions; Windows
double-click maximize/restore remains available. Synthetic egui input tests check command
dispatch, not native OS window movement, which still requires a desktop interaction check.
On Windows and macOS, Appearance settings can hide this 36 px strip and use the native title bar
and window buttons instead. The device preference defaults to showing the custom strip and
is saved with other app preferences; older saved settings keep that default. macOS switches
without restarting and keeps its native traffic lights. Linux always omits the app strip and
requests system decorations; on Wayland compositors without server decorations, including GNOME,
winit uses its Adwaita frame instead of the basic fallback. GNOME rendering remains unverified
in this macOS fast local pass.

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

`cargo xtask package --format appimage` creates a Linux x86_64 Type 2 AppImage
including voice. The Ubuntu 26.04 release job publishes it alongside the native
packages and release checksums. It uses host GTK4/WebKitGTK 6.0, audio and graphics
libraries, rather than bundling a separate browser runtime. See [AppImage setup and
builds](../packaging/appimage/README.md) for installation requirements, pinned tooling
and package inspection. Native AppImage startup and upgrading remain unverified in
the initial fast local pass.

`cargo xtask package` builds the locked default release configuration. macOS gets `dist/Serein.app`; Windows gets an executable plus license files; Debian/Ubuntu Linux additionally produces a `.deb` with desktop integration and dependency metadata. On macOS, packaging replaces the executable through a fresh sibling file and rename, then seals the completed bundle with `codesign --force --sign -` and runs `codesign --verify --strict`. This is a **local ad-hoc signature**, with no signing identity, Developer ID certificate, or notarization. It verifies the staged bundle's integrity and does not certify Gatekeeper acceptance or a trusted publisher. The distinction between signature validity and trust is described in [Apple's code-signing guidance](https://developer.apple.com/library/archive/technotes/tn2206/_index.html).

Windows/Linux local staging artifacts remain unsigned. These are not certified installers. Use `ditto -c -k --keepParent dist/Serein.app dist/Serein-macos.zip` on macOS; normal archive tools may package Windows staging output. Do not modify bundle resources after sealing; rerun packaging when source documentation changes. Linux additionally supports `--format rpm`, `--format arch` and `--format dir`; release jobs target Ubuntu 26.04, Fedora 43/44, openSUSE Tumbleweed and Arch independently. Fedora 43 packaging was added in a local fast pass; its build and installation remain unverified until Linux CI and desktop validation. [Flatpak](../packaging/flatpak/README.md) builds offline against GNOME SDK 49 with the pinned Rust compiler and locked vendored sources. Its sandbox currently excludes direct V4L2 camera access and host game IPC; desktop login/keyring/audio still need Linux runtime validation. [Signed repository preparation](../packaging/repositories/README.md) supports apt, dnf/zypper and pacman, but requires configured signing credentials and an HTTPS host; preparing artifacts does not publish repositories. Windows installer/signing and release reproducibility remain open work.

The webview lives only during login: WKWebView on macOS, WebView2 on Windows, GTK/WebKitGTK on Linux. Linux uses a separate GTK authentication window and pumps it only while login is active. Voice is built in. Audio devices open only for explicit playback, device testing, or a call reaching required encrypted readiness. Popup-dependent authentication and third-party embedded challenges may not work; do not claim all Discord login methods without live tests.

## Built-in voice

Linux device discovery and call streams prefer CPAL's PulseAudio backend, including
PipeWire through `pipewire-pulse`. This lists the server's sources and sinks, including
connected Bluetooth devices and virtual filter-chain endpoints exposed by that server,
instead of ALSA hardware modes. ALSA remains the fallback when the desktop audio server
is unavailable. Native Linux device discovery and physical playback/capture still need
verification on the affected system.

`cargo run --locked` includes native DM and guild audio. Source builds require CMake and a C/C++ toolchain for statically bundled libopus; Linux also needs ALSA development headers (`libasound2-dev` on Debian/Ubuntu). CPAL uses native system audio. See [the voice adapter](../crates/discord-voice/README.md) for codec/protocol dependencies and limitations.

`cargo xtask package` stages the standard release including voice under `dist` (`dist/Serein.app` on macOS). The macOS bundle includes its microphone-use description; actual microphone permission, capture/playback, device switching and sleep/resume have not been exercised. Windows x64 voice release packaging and synthetic protocol/audio tests pass; physical audio and live calls remain unverified on Windows. Linux x64 text/voice release builds and Debian package smoke passed on Ubuntu 26.04 under WSL2; native Linux desktop/audio runtime remains unverified. CMake is a source-build dependency, not a runtime voice service.

Device choices, voice keybinds and the owner's mute/deafen intent are device-local. Mute and deafen can be changed while idle, survive restart and apply to the first voice-state packet when the next call is joined. Voice bindings use native global registration on supported Windows/macOS/Linux X11 setups and the desktop GlobalShortcuts portal on Wayland when they include a modifier. Unmodified focused Push to Talk (V by default) observes key state without consuming typed text and is never registered as an OS-global shortcut. Missing/denied portal access and unavailable/conflicting registrations fall back to focused input. Use headphones because there is no acoustic echo cancellation. No signing, desktop integration, physical audio or live-compatibility claim follows from compilation alone.

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

## Opt-out tray icon (September 13, 2026)

Windows General settings offer Show Serein in System Tray, on by default; turning it off
falls back to ordinary window minimize/close. Minimizing
keeps the window in the taskbar, including taskbar clicks and automatic startup. The icon supports
keyboard/mouse restore and a Show Serein / Quit menu. Quit uses the normal unsaved
work/download exit checks; while the icon is live, the window Close button hides the
window instead of exiting, and Serein keeps running with its logic ticking so
notifications and calls continue. Show restores the window. Disabling the setting,
or a tray that reports itself unavailable, restores a hidden window immediately, so
Close can never strand the application without a way back.
The adapter uses existing user32/Shell APIs and dependencies, with no background
polling. A synthetic native Windows test verifies registration,
minimize/restore, own-window taskbar recovery, Quit event and cleanup. macOS uses a native menu bar icon with Show Serein / Quit actions; it draws Serein's own
mark (`assets/brand/serein-tray.png`, rendered from the brand SVG) as an 18-point template
image, so the system tints it for light, dark and highlighted menu bars. Minimized windows
remain in the Dock.

Linux now uses ksni's StatusNotifierItem on the session bus with Show Serein,
Minimize Serein and Quit actions. Enable a StatusNotifier host (for example a panel's
tray module). Until registration succeeds, or after host loss, Close retains normal
exit behavior. Start/restart the host and toggle the tray off/on to retry registration.
The existing on-by-default tray preference is reused; demo changes are session-only.

**Hyprland / native Wayland:** winit cannot hide, unhide, focus or unminimize a native
Wayland window. Close requests minimization and keeps Serein running; the compositor
may ignore this request. Show requests restoration, but native Wayland users may need
the compositor's own window controls. A still-visible window is no longer marked hidden
inside Serein. Native Wayland remains the default on Wayland sessions, with no
application-level XWayland fallback or backend override.
Quit remains explicit and runs the existing unsaved-work/download/extension checks;
cancelling Quit restores close-to-tray behavior.

Offline lifecycle check: `cargo run --locked -p tray-debug`. On Linux, use
`dbus-run-session -- cargo run --locked -p tray-debug` to additionally exercise
registration, missing/lost host, icon activation and all three menu actions on a private
synthetic bus. These checks do not establish compositor behavior. This refresh was
prepared on macOS; NixOS/Hyprland and Flatpak desktop validation remain pending.

## Opt-in automatic startup

General settings offer automatic launch at Windows sign-in and a dependent Start
Serein minimized preference. Both default off. Registration uses the current user's
Run key; no administrator access, service, scheduled task or new dependency is needed.
Windows Startup Apps can override this registration. Disable startup before deleting
a portable installation, or re-enable it after moving the executable.
Minimized launches stay in the taskbar even when the saved tray preference is enabled;
the tray can attach safely after a minimized launch. Tray failures leave the window
recoverable. Without a tray icon the Close button still exits, and the tray Quit action retains unsaved
work checks. macOS registers a per-user `~/Library/LaunchAgents/cz.viceverse.serein.startup.plist`
for the next graphical login, with the same launch flags. Turning it off removes only
that file. It does not launch a second client when enabled or restart after Quit.
Re-enable startup after moving the executable; disable it before uninstalling. macOS
Login Items settings can independently block launch. Linux autostart remains unavailable.
Native macOS sign-out/sign-in remains unverified.
Offline tests cover isolated registry writes/removal, launch flags and settings
interaction; an actual Windows sign-out/sign-in has not been exercised.

## In-app updates

Settings → Updates provides automatic checking/downloading, Production and Nightly
release channels, a manual check and an explicit restart action. The title strip
shows an available or downloaded update on macOS, Windows and Linux. Update controls are
also accessible from the signed-out screen. Automatic checking runs at startup
once saved preferences are available, then every hour while running; turning
it off disables automatic downloads while background checks and title-bar notices
remain active. Nightly is the default channel and automatic downloads are off by
default. Switching channels never installs an
older semantic version. Nightly checks inspect the latest 100 published releases.

Packages come from this repository's existing GitHub releases and must match the
platform/architecture asset name, published length and `SHA256SUMS.txt`. Downloads
and installation preparation run outside rendering; installation is handed off
only after the application's existing close/unsaved-work gates permit shutdown.
GitHub HTTPS and repository access are the update trust boundary; release checksums
alone are not an independent publisher signature. Linux AppImages use this same
trust boundary; other Linux installations use their package manager.

The local `--features demo -- --demo --demo-check-updates` debug path exercises
synthetic update states, preference compatibility and settings rendering without
network access or replacing an installation. It is not evidence of a successful
live release upgrade or of Windows native installation behavior.

In-app installation requires an extracted Windows release or an installed,
writable macOS `.app` outside a mounted disk image/App Translocation, or a running
x86-64 AppImage in a writable directory on a filesystem supporting hard links.
AppImages retain their original filename, validate the Type 2 ELF architecture,
and atomically replace the outer image after shutdown. An immediate launch failure
restores the previous image; the two-second check is not an application-health test.
macOS checks
strict code-signature validity, the existing publisher's TeamIdentifier and bundle
identifier, and Gatekeeper acceptance. Windows currently relies on the repository's
HTTPS/checksum trust boundary because its published packages are unsigned. When
installed via the per-user installer (`%LOCALAPPDATA%\Programs\Serein`), write
permissions are maintained without administrator elevation, and the update helper
automatically updates the Windows uninstall `DisplayVersion` registry key upon
successful upgrade. Native helpers wait for the old process to exit, retain a rollback
copy during replacement, and relaunch Serein. A failed recovery leaves its backup
available with a visible recovery path on the next update attempt.

## Linux screen sharing

The system screen-sharing picker requires PipeWire, a ScreenCast-capable portal backend for the current
desktop (GNOME, KDE or the compositor-specific backend), and GStreamer Base/Good plus
the PipeWire source plugin. GStreamer 1.24+ is recommended; GPU scaling/encoding also
needs the applicable VA, NVCodec and OpenGL plugins and working driver support.
Native packages declare the PipeWire and Base runtime plugins; hardware codec availability
still depends on distribution packaging and drivers. The software fallback reuses bundled
OpenH264. Flatpak needs compatible plugins/GPU access inside its runtime; no extra sandbox
permission or host socket access is added. Native Linux validation remains pending.

Native X11 sessions can instead explicitly select “Entire X11 desktop · all monitors ·
no portal”. This uses `ximagesrc` from GStreamer Good, already a native package
dependency, and shares the whole desktop. No direct capture starts after a failed or
cancelled portal request. X11 capture and live delivery remain unverified.

Optional Linux stream audio uses native `libpulse` per-application monitoring on
PulseAudio or PipeWire's PulseAudio server. Source builds need the libpulse development
package (`libpulse-dev`, `pulseaudio-libs-devel`, `libpulse-devel` or Arch's `libpulse`).
The existing Flatpak PulseAudio socket permission covers this access; the ScreenCast
portal's PipeWire remote grants video only. Windows uses native process loopback on
build 20348+ (Windows 11 / Server 2022), with a visible audio error on older systems.
Both exclude Serein's playback and capture other applications even when sharing one
window. There is no whole-output fallback. Hardware exclusion and receiving sound in
an official client remain unverified.

## Additional macOS attachment codecs

WebM and MOV codecs unavailable in the native inline decoder can use an installed
FFmpeg from `/opt/homebrew/bin`, `/usr/local/bin`, or `/usr/bin`. Nothing is installed
automatically. The helper must include H.264 (`libx264`) and AAC encoding; converted
video plays through the existing native controls. Conversion may take up to two
minutes before playback and is limited to 100 MiB input/output, 1080p and two hours.
Missing FFmpeg or conversion failures appear in the video card. Linux and Windows
continue to use their installed native codecs. This optional fallback is not bundled
in release packages; actual codec coverage depends on the local FFmpeg build.

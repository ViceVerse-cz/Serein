# Platform support and packaging

## Window size and placement

Normal window size is saved in logical pixels in the existing device-local SQLite
preferences. Outer position is saved in physical pixels where the window system
supports it (including Linux X11); disconnected-monitor positions are ignored.
Restoration fits the complete outer window to the selected monitor's dimensions,
shrinking it and moving it inward when the display has become smaller. The normal
minimum size is relaxed if necessary to fit that display and recalculated when
the window moves or its native scale changes, restoring 760 × 520 logical pixels
when the current display can accommodate the window and its decorations.
Minimized, hidden, maximized and fullscreen states do not replace normal geometry.
Closing frames do not overwrite the saved geometry.
A changed placement is saved once sampled unchanged for at least 500 ms, so a drag or resize queues no
write per frame. Maximizing retains the latest sampled normal placement for restoration; a placement sampled just before the window closes or hides is still saved.
The offline `--demo` ignores saved geometry and does not save its window state.

Wayland does not expose global window coordinates to ordinary clients, so window
placement belongs to the compositor. Niri also controls tiled column sizes and can
override the application's requested size. To allow Serein's saved width, add to
`~/.config/niri/config.kdl`:

```kdl
window-rule {
    match app-id=r#"^cz\.viceverse\.serein$"#
    default-column-width {}
}
```

For compositor-controlled placement, use Niri's `open-on-workspace`, `open-on-output`
or floating-window rules. See [Niri window rules](https://github.com/niri-wm/niri/wiki/Configuration:-Window-Rules)
and [Niri's application-selected width](https://github.com/niri-wm/niri/wiki/Configuration:-Layout#default-column-width),
and [winit's platform limitations](https://docs.rs/winit/0.30.13/winit/window/struct.Window.html#method.outer_position).
Native restart/resize behavior on Niri and other Linux desktops remains unverified.

## Build and runtime support

Target platforms are Windows, macOS and Linux. **macOS arm64, Windows x64 and Linux x64 have local build evidence.** The release workflow also targets Windows arm64 and Ubuntu 26.04 ARM64 on native GitHub Actions runners; build and runtime validation remain pending. macOS has native visual checks; Windows has offline tests and a process/window startup smoke check only. Minimum OS versions, other architectures, real screen-reader support and native login-method support are not certified.

Windows defaults to DirectX 12 to avoid reported startup access violations in Intel's
Vulkan driver (`igvk64.dll`). The existing `WGPU_BACKEND` environment override remains
available (for example, `dx12` or `vulkan`). Affected users confirmed that forcing
DX12 launches successfully; the new default still needs native Windows validation.
macOS and Linux retain their existing backend defaults.

General → Graphics now offers a **Graphics API** picker alongside the GPU preference.
It lists Automatic and the APIs compiled for the current platform (Vulkan, DirectX 12,
Metal or OpenGL); an installed driver and a presentable adapter are still required.
The device-local choice is saved in SQLite and applies on the next launch. An explicit
API is ranked before the GPU preference; when no adapter for it can present, selection
falls back to the normal Automatic candidates and reports the fallback on stderr.
Automatic retains the Windows DirectX 12 / OpenGL policy and other platform defaults.
`WGPU_BACKEND` takes precedence over the saved API and disables the picker for that
launch. The current adapter/backend remains visible in settings, and bug reports include
the requested API and whether an environment override is active. Driver initialization
crashes cannot be caught by this fallback; Windows Vulkan remains an explicit opt-in.

Adapter selection prefers a real GPU over a software rasterizer whichever backend presents it.
Windows offers WARP ("Microsoft Basic Render Driver") as its Direct3D adapter on hardware that
predates DX12, and drawing the client on the CPU that way freezes a machine whose GPU still
presents through OpenGL. Windows therefore enumerates DirectX 12 and OpenGL (Vulkan stays out
for the reason above), so both candidates exist for that ranking, and the chosen adapter is
still printed to stderr. When only a software rasterizer is left, the client logs that fact and
names the `WGPU_BACKEND=gl` override, which forces the OpenGL driver; when that software
adapter is already an OpenGL one, it suggests installing the GPU driver instead.

Windows turns off winit's undecorated drop-shadow hack after the window exists, including
after title-bar changes. egui-winit enables that hack for custom chrome. While restored it
adds one pixel to `WM_NCCALCSIZE` top and bottom. Maximizing skips the shift, which is why
only that state looked sharp. `ViewportBuilder::with_has_shadow` is macOS-only and does not
disable the Windows hack. Native DPI and eframe's physical surface sizing remain unchanged.
macOS/Linux window creation is unchanged.
The custom Windows frame requests DWM's rounded-corner treatment on Windows 11. Windows keeps
maximized windows square, and older releases ignore the unsupported preference.
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
On Windows, the custom title strip paints native-fidelity Windows 11 caption buttons
(minimize, maximize/restore, close) flush in the top-right corner. Custom resize hit testing
excludes the caption buttons area so edge/corner resizing does not intercept corner clicks on
Close. Caption buttons keep their own actions; Windows double-click maximize/restore remains
available. Synthetic egui input tests check command dispatch, including corner hit testing.
On Windows, dragging the strip runs the OS move loop, which paints synchronously and waits for
vsync before the next move. Through a patched egui-winit (see `vendor/egui-winit/SEREIN-PATCH.md`)
Serein therefore ignores raw mouse motion and repaints for window moves at most once per 500 ms,
enough to sample and save where the window comes to rest; it also reapplies the minimum size only
when it changes. An otherwise idle window follows the cursor like a native one. Continuous
rendering, such as an animation or playing video, still paces a drag to the display refresh rate.
Synthetic `SendInput` drags on Windows 11 are measured in `docs/performance.md`; physical-mouse
behavior on other machines and native macOS/Linux window movement remain unverified.
On Windows and macOS, General settings can hide this 36 px strip and use the native title bar
and window buttons instead. The device preference defaults to showing the custom strip and
is saved with other app preferences; older saved settings keep that default. macOS switches
without restarting and keeps its native traffic lights. Linux always omits the app strip and
defaults to system decorations; on Wayland compositors without server decorations, including GNOME,
winit uses its Adwaita frame instead of the basic fallback. GNOME rendering remains unverified
in this macOS fast local pass. Linux General → Window includes **Hide window decorations**,
which requests a borderless window immediately and persists across restarts. Moving, resizing
and closing then use the window manager's controls. The compositor controls whether decoration
requests are honored; native X11/Wayland verification of this toggle remains pending.

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
including voice. A separate Ubuntu 24.04 (glibc 2.39) release job publishes it
alongside the native packages and release checksums. It uses host GTK4/WebKitGTK 6.0,
audio and graphics libraries, rather than bundling a separate browser runtime. See
[AppImage setup and builds](../packaging/appimage/README.md) for installation
requirements, pinned tooling and package inspection. Native AppImage startup and
upgrading remain unverified in
the initial fast local pass.

`cargo xtask package` builds the locked default release configuration. macOS gets `dist/Serein.app`; Windows gets an executable plus license files; Debian/Ubuntu Linux additionally produces a `.deb` with desktop integration and dependency metadata. On macOS, packaging replaces the executable through a fresh sibling file and rename, then seals the completed bundle with `codesign --force --sign -` and runs `codesign --verify --strict`. This is a **local ad-hoc signature**, with no signing identity, Developer ID certificate, or notarization. It verifies the staged bundle's integrity and does not certify Gatekeeper acceptance or a trusted publisher. The distinction between signature validity and trust is described in [Apple's code-signing guidance](https://developer.apple.com/library/archive/technotes/tn2206/_index.html).

Windows/Linux local staging artifacts remain unsigned. These are not certified installers. Use `ditto -c -k --keepParent dist/Serein.app dist/Serein-macos.zip` on macOS; normal archive tools may package Windows staging output. Do not modify bundle resources after sealing; rerun packaging when source documentation changes. Linux additionally supports `--format rpm`, `--format arch` and `--format dir`; release jobs target Ubuntu 26.04 (x86_64 and ARM64), Fedora 43/44, openSUSE Tumbleweed and Arch independently. Fedora 43 packaging was added in a local fast pass; its build and installation remain unverified until Linux CI and desktop validation. [Flatpak](../packaging/flatpak/README.md) builds offline against GNOME SDK 51 with the pinned Rust compiler and locked vendored sources. Its sandbox currently excludes direct V4L2 camera access, and host game IPC needs the documented socket link; desktop login/keyring/audio still need Linux runtime validation. [Signed repository preparation](../packaging/repositories/README.md) supports apt, dnf/zypper and pacman, but requires configured signing credentials and an HTTPS host; preparing artifacts does not publish repositories. Windows installer/signing and release reproducibility remain open work.

The webview lives only during login: WKWebView on macOS, WebView2 on Windows, GTK/WebKitGTK on Linux. Linux uses a separate GTK authentication window and pumps it only while login is active. Voice is built in. Audio devices open only for explicit playback, device testing, or a call reaching required encrypted readiness. Popup-dependent authentication and third-party embedded challenges may not work; do not claim all Discord login methods without live tests.

## Linux system appearance

Appearance set to System reads the freedesktop Settings portal's color-scheme on
the existing runtime, then listens for changes. Signals trigger a fresh property
read so an older queued signal cannot replace a newer initial value. A closed or
unavailable portal is reconnected after a three-second delay, with one connection
and subscription active at a time. Reads have a three-second timeout; the latest
known preference remains in effect during reconnection.

When the initial portal attempt fails, a one-shot GNOME gsettings fallback has a
two-second process limit and at most 256 output bytes. An explicit dark preference
selects Dark; no preference selects Light. If detection fails entirely, the
existing Dark fallback remains. Windows/macOS retain winit's system theme events.
Real Linux desktop/portal rendering is unverified in this macOS repair session.

## Go idle when away

The Notifications setting shows Idle after the account's inactivity timeout
(default 10 minutes) without system-wide keyboard or mouse input, or right away
while the computer is locked, so Discord can send notifications to the phone.
It is not applied during voice calls. Input age comes from `GetLastInputInfo` on
Windows and CoreGraphics on macOS. On Linux Serein asks GNOME Mutter's
IdleMonitor, then the Wayland `ext-idle-notify-v1` protocol (KDE Plasma, wlroots
compositors, COSMIC), then `org.freedesktop.ScreenSaver` (KDE on X11), then X11
XScreenSaver, which is skipped under Wayland. Wayland compositors with none of
these stay online, and idle inhibitors such as video playback keep the session
active. A lock is read from the WTS session state on Windows, the CGSession
dictionary on macOS, and logind's `LockedHint` (then the ScreenSaver service's
`GetActive`) on Linux. The check runs every 5 seconds off the UI thread, so locking applies
within about that long. The timeout is read when Serein connects and saved to
the account when changed here; edits from another device apply on the next
connect. Serein also shows your own status as Idle while it applies. macOS and
Linux behavior is unverified on a live session. A setting change invalidates an
in-flight idle sample; its result is discarded before another query starts.
The poller retains one blocking query at a time. A stalled Linux bus handshake,
Wayland roundtrip or X11 query can stop idle detection without accumulating
workers; the setting can still clear AFK. Wayland currently observes one seat,
so activity on other seats in a multi-seat session is not included.

## Window transparency and blur

Enable Transparency & blur in Appearance and restart to create an alpha-capable
rendering surface. Transparency works where the OS/compositor supports alpha windows.
Blur is requested through macOS winit support, Windows 11 22H2+ desktop Acrylic,
Wayland [`ext-background-effect-v1`](https://isaacfreund.com/docs/wayland/ext-background-effect-v1/)
(including compatible Mutter/GNOME), with the
existing KDE Wayland protocol as fallback, and the KDE blur property on X11.
Unsupported compositors/older Windows retain transparency without native blur;
system accessibility and appearance preferences can also suppress effects.
The compositor determines blur strength; zero disables it. Standard Wayland blur
capability changes are processed without blocking the rendering loop. Linux and
Windows native appearance remains unverified in this macOS local pass.

## Variable refresh rate displays

Serein redraws only when something changes: about twice a second while the composer's text
cursor blinks, and up to the display's refresh rate during input. On Windows its DirectX 12
swap chain moves to hardware independent flip while frames arrive quickly, so a variable refresh
rate (VRR, G-SYNC, FreeSync) display can follow that cadence while Serein is the active window.
Many panels flicker visibly when the rate swings between their limits, OLED panels in particular.

General › Graphics › Steady refresh rate is off by default. When it is on, Serein draws at the
display's refresh rate while its window is focused and not minimized, so the display can hold one
rate. A window that macOS or X11 reports as covered pauses it as well; Windows and Wayland do not
report that, so a focused window under an always-on-top one keeps drawing. Every refresh runs a
full interface pass, which costs CPU time and power in proportion to the refresh rate: about a
quarter of one core at 3840×2160 and 120 Hz in the November 5, 2026 entry of
[performance findings](performance.md). Unfocused, minimized and hidden windows redraw only on
change again. The setting applies immediately and is saved with the other device preferences.

Excluding `serein.exe` from VRR in the graphics driver, such as NVIDIA's per-program Monitor
Technology setting, avoids the swings without the extra frames. Present cadence was verified
with PresentMon on Windows 11 with an RTX 5060 Ti; the panel's own refresh readout and macOS and
Linux VRR behavior remain unverified.

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

Wayland shortcut requests use the [XDG shortcut specification](https://specifications.freedesktop.org/shortcuts/latest/)
and XKB keysyms (for example `CTRL+SHIFT+m`, `CTRL+1`, and `CTRL+Down`),
rather than physical key-code names such as `KeyM` or `Digit1`. To check a portal
registration without joining voice, enable global keybinds, approve the desktop
dialog, move focus to another application and use Toggle Mute/Toggle Deafen.
The mute/deafen intent should change when returning to Serein. Flatpak needs a host
portal backend implementing GlobalShortcuts; the encoding regression test does not
verify compositor approval or delivery on a particular desktop. Outside Flatpak, Serein
registers the portal app ID `cz.viceverse.serein` through the host Registry
(xdg-desktop-portal 1.19+), which needs the installed `cz.viceverse.serein.desktop` entry;
source runs and unintegrated AppImages without it fall back to focused input. Shortcut IDs
are stable per action (`toggle-mute`, `toggle-deafen`, `push-to-talk`, `push-to-mute`);
extra triggers for one action are numbered, such as `toggle-mute-2`. Hyprland only delivers
portal shortcuts to keys bound in its config and does not apply the suggested trigger, so
the check above needs a bind such as
`bind = CTRL SHIFT, M, global, cz.viceverse.serein:toggle-mute`. Update existing binds that
used the earlier `cz.viceverse.serein:serein-N` IDs.

Native emoji use eframe system-font fallback and installed OS color fonts. macOS
Apple Color Emoji was visually checked with a synthetic moon status on September
10, 2026. Windows/Linux emoji coverage is unverified and depends on installed fonts
(e.g. Segoe UI Emoji / Noto Color Emoji); no OS font is redistributed.


## Camera capture (September 12, 2026)

Outgoing in-call capture uses AVFoundation on macOS, Media Foundation and DirectShow on Windows,
and V4L2 on Linux. The existing camera button becomes available after the voice
server negotiates H264; capture starts only after an explicit click in a connected
call. All adapters send 640×480 video at most 15 encoded frames/s. Native capture now
selects the closest supported dimensions/rate within a 1280×720 input ceiling
(DirectShow preserves its existing 1920×1080 fallback) and
converts to the existing encoder size; hardware negotiation remains unverified.
Windows needs desktop camera permission; Linux needs an accessible streaming
`/dev/videoN` node supporting progressive YUYV or MJPEG. Linux portal-only camera
access is not implemented. All three platforms have a device picker in settings and call controls,
including DirectShow virtual cameras on Windows, AVFoundation discovery on macOS,
and single-plane streaming V4L2 devices on Linux. Settings also provide an explicit
local camera preview outside calls. Physical selection/preview remains unverified.
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

## Persistent tray icon and close behavior (October 2, 2026)

Serein keeps its native tray/menu-bar icon while running, independently of the
on-by-default minimize-to-tray preference. The preference controls window Close:
when enabled and registration is available, Close hides the window; otherwise
Close follows ordinary exit checks. Turning the preference off or losing the
tray host restores a hidden window. Normal taskbar/Dock minimizing is unchanged.
Show restores the window, and Quit follows the existing unsaved-work and download
checks.

Windows tray activation shows a hidden or already visible window without resetting
maximization; only an actually minimized window receives native restoration.
The synthetic native regression covers tray clicks, the Show menu, repeated Show,
minimized maximized windows and preserved normal geometry. On an interactive
Windows desktop, run:

```text
cargo test --locked -p platform --lib tray::native::tests::native_minimize_restore_restart_quit_and_cleanup -- --exact --ignored --test-threads=1
```

Cross-compilation alone does not verify desktop behavior.
The user-run Windows x64 synthetic regression reproduced the maximization failure
before this fix and passed after it. Full-application visual validation remains pending.

Windows voice state changes the icon and tooltip for speaking, muted and deafened
states. Fixed-size icons are created once and destroyed with the registration;
failed updates are retried on the next logic tick. Explorer recovery restores the
current icon and tooltip. Voice and tray updates run once in background-capable
logic, with a 50 ms repaint request while a call is active. No audio device opens
merely to update the icon. Windows shell behavior remains unverified in this macOS
repair session; synthetic update-state tests do not prove native shell behavior.

macOS retains its 18-point template Serein menu-bar mark and Show/Quit actions.
Linux uses ksni StatusNotifierItem with Show, Minimize and Quit. A StatusNotifier
host is required. Registration failures remain visible independently of the Close
preference; toggling that preference retries registration while healthy icons stay
registered. Demo setting changes are session-only.

**Hyprland / native Wayland:** winit cannot hide, unhide, focus or unminimize a native
Wayland window. On Hyprland, Close and tray Minimize instead park Serein on
`special:serein-tray` through the compositor socket; Show moves it to the active
workspace. This first uses `movetoworkspacesilent` with the numeric workspace `id`
for restoration or the named special workspace for hiding. A compositor rejection
falls back to `hl.dsp.window.move` with `follow = false`, preserving the workspace
`address` when supplied, otherwise its numeric `id`. Transport failures are not
retried. Workspace names are bounded and escaped before Lua dispatch.
On KDE Plasma (`XDG_CURRENT_DESKTOP` contains `KDE`), Close loads a small KWin script
over D-Bus (`org.kde.KWin` `/Scripting`) that minimizes Serein and skips the taskbar,
pager and task switcher; Show and Quit unminimize and activate it first, so Quit no
longer renders into a minimized surface. Tray Minimize keeps the taskbar entry. Native
installs match the process ID; Flatpak matches its app ID and writes scripts to its
shared per-app runtime directory. If KWin scripting is unreachable, Close falls back
to minimizing. KDE Plasma behavior is not yet verified on a live session.
Other Wayland compositors receive minimize/restore requests and may require their
own window controls. Native Wayland remains the default on Wayland sessions, with no
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
Login Items settings can independently block launch. Built-in Linux autostart remains
unavailable; desktop-session entries can launch `serein --start-minimized`, while ordinary
launches without the flag remain visible.
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

AppImage downloads first try the selected release's `.zsync` sidecar, verified
against the same checksum list. The updater scans the installed image for reusable
blocks and downloads missing HTTPS ranges, then verifies the entire result with
SHA-256. Missing/incompatible delta metadata or reconstruction failure falls back
to a full download. The built-in implementation supports the zsync 0.6.2/MD4 format
produced by the Ubuntu packaging job; it does not run an external updater. Offline
debug checks cover shifted-block reuse and integrity rejection; real release-to-release
bandwidth savings and Linux upgrade behavior remain unverified.

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

Niri portal capture normalizes frame timestamps at arrival before frame-rate filtering,
including on Niri 26.04 where presentation timestamps remain constant. This preserves
the existing VA-API/NVENC/OpenH264 selection and bounded buffers. Other desktops and
native X11 retain their existing timestamp handling. The offline `linux_screen --niri-timestamps` regression is synthetic; native capture and Discord delivery remain
unverified. See [the screen-sharing checks](voice.md) for build/run commands.

Screen sharing also tries the legacy `vaapih264enc` element when modern VA encoding
fails. This optional system plugin uses CPU scaling and hardware H.264 encoding;
it does not require `vaapipostproc`. Check availability with
`gst-inspect-1.0 vaapih264enc` in the same runtime as Serein. Installing the modern
`va` plugin alone does not provide this legacy element. Driver compatibility still
requires an actual encode test; `vainfo` only advertises capabilities.

Native X11 sessions can instead explicitly select “Entire X11 desktop · all monitors ·
no portal”. This uses `ximagesrc` from GStreamer Good, already a native package
dependency, and shares the whole desktop. No direct capture starts after a failed or
cancelled portal request. X11 capture and live delivery remain unverified.

Optional Linux stream audio uses native `libpulse` per-application monitoring on
PulseAudio or PipeWire's PulseAudio server. Source builds need the libpulse development
package (`libpulse-dev`, `pulseaudio-libs-devel`, `libpulse-devel` or Arch's `libpulse`).
The existing Flatpak PulseAudio socket permission covers this access; the ScreenCast
portal's PipeWire remote grants video only. Windows uses native process loopback: one
exclude-Serein capture on build 20348+ (Windows 11 / Server 2022), and per-application
captures on older Windows 10 builds. Both exclude Serein's playback. Sharing a window captures
only that window's application, as Discord does; sharing a screen captures other applications. There is no whole-output fallback. Hardware
exclusion and receiving sound in an official client remain unverified.

## High-resolution video attachments

Inline playback accepts source resolutions supported by the installed native codec and device,
including sources larger than 4K. Larger sources are fitted, without upscaling, into the existing
playback surface: at most 3840 pixels per side and 3840×2160 pixels in total. Portrait and
ultrawide aspect ratios are preserved to integer-pixel rounding. This is a playback surface
budget, not a source-resolution rejection. Windows requests resized Media Foundation output,
macOS scales into a bounded VideoToolbox pixel buffer, and Linux scales after applying rotation and
before queuing decoded video. The original attachment is unchanged.

Codec profiles, hardware capabilities, the 100 MiB encoded attachment limit and the two-hour
duration limit still apply. Supporting a source does not promise native-resolution rendering
or a particular frame rate. Live screen sharing has its separate existing limits.

## Additional macOS attachment codecs

WebM, MOV and MP4 codecs unavailable in the native inline decoder can use an installed
FFmpeg from `/opt/homebrew/bin`, `/usr/local/bin`, or `/usr/bin`. Nothing is installed
automatically. The helper must include H.264 (`libx264`) and AAC encoding; converted
video plays through the existing native controls. Conversion may take up to two
minutes before playback and is limited to 100 MiB input/output, the same bounded playback surface and two hours.
Native format failures during opening or playback retry conversion once, resuming at
the last displayed position. A decoder that cannot produce a bounded native picture can also use this
downscaling path. AAC track timestamps need not use the audio sample rate
as their timescale. Missing FFmpeg or conversion failures appear in the video card. Linux and Windows
continue to use their installed native codecs. This optional fallback is not bundled
in release packages; actual codec coverage depends on the local FFmpeg build.
Windows passes MPEG-4/MOV and WebM/Matroska attachments to Media Foundation; a recognized
container can still fail when its video or audio codec is not installed.

## HEIC still images

Windows HEIC/HEIF attachment previews and composer thumbnails use Windows Imaging
Component (WIC) and its installed HEIF/HEVC codecs. Install the Microsoft HEIF
Image Extensions and an applicable HEVC codec if decoding is unavailable. No codec
library, installer or DLL is bundled; only the existing `windows` crate gains its
Imaging bindings. Executable/package size changes have not been measured.

Only the primary still frame is decoded, on existing blocking workers, within the
existing encoded-byte, source-dimension and scaled RGBA allocation limits (also
capped at 32 MiB encoded, 8192 pixels per edge and 128 MiB RGBA). OS codec scratch allocations are
not controlled by Serein. WIC scales to the requested output edge before Serein
allocates RGBA pixels, so large sources can produce bounded composer thumbnails.
Originals remain unchanged when uploaded, with image/heic
or image/heif MIME types. Incoming attachments prefer Discord's existing image
proxy; attachment originals can fall back to the validated CDN URL even without
a HEIC filename suffix.
On Windows, attachment viewers prefer that original, with the existing 4096-pixel
longest-edge display limit; larger photos are downscaled. Missing service dimensions
use decoded pixels for the aspect ratio and displayed resolution, so that resolution
is the decoded rendition, not necessarily the full source photo. Original downloads
remain unchanged.
Missing codecs or corrupt/oversized files retain the existing failed-preview state.
macOS and Linux have no local HEIC decoder in this implementation; proxy-provided
PNG/JPEG/WebP renditions can still display. The offline Windows check decodes an
original synthetic 6000x4000 HEIC both at source size and as a 320px thumbnail,
checking dimensions and pixel conversion. Owner photos and live Discord remain unverified.

## Windows screenshots-and-recording permission — November 4, 2026

Windows 11 builds that expose Settings > Privacy & security > Screenshots and screen recording keep
a per-application consent for programmatic Graphics Capture. On Windows 11 build 26300 a locally
built, unpackaged executable was denied at the capture item with `E_ACCESSDENIED` ("You do not have
permission to capture the given monitor"), and the Settings page listed no request; the installed
copy at its own path had a consent record from earlier capture and worked. Serein now asks the
platform for that consent with `GraphicsCaptureAccess.RequestAccessAsync(Programmatic)` when a share
starts. An unpackaged executable answers `NotDeclaredByApp`, so the capture item keeps deciding;
`DeniedByUser` or `DeniedBySystem` names the setting in the share dialog. The diagnostic log records
the start failure with its reason instead of a generic message. The permission is per executable
path, so a build run from a different folder (a local release build beside an installed copy) needs
its own allowance even when the other copy works. The capture pipeline itself is unchanged.

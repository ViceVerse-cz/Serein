## Build from a clean Windows checkout

Install Rust 1.98.1 for MSVC, Visual Studio C++ Build Tools with the Windows SDK and CMake,
Windows Python 3.12+, and [MSYS2](https://www.msys2.org/). Install NSIS if you also want
`dist-installer/serein-<version>-setup.exe`. The client requires WebView2 for login.

In the MSYS2 shell, install the build helpers:

```sh
pacman -S --needed make pkgconf nasm diffutils
```

Return to Developer PowerShell for Visual Studio, initialized for the same architecture as
Rust (x64 or ARM64). From the repository root, use native Windows Python and launch MSYS2
from that shell so it inherits the compiler, linker, and Windows SDK:

```powershell
python -m pip install meson==1.9.2 ninja==1.13.0
$env:MSYS2_PATH_TYPE = 'inherit'
$env:MSYSTEM = 'MSYS'
$env:CHERE_INVOKING = '1'
& 'C:\msys64\usr\bin\bash.exe' -lc 'python scripts/build-ffmpeg.py'
if ($LASTEXITCODE -ne 0) { throw 'Pinned FFmpeg build failed' }
$env:FFMPEG_DIR = (Resolve-Path .\target\ffmpeg\prefix).Path
$env:Path = "$env:FFMPEG_DIR\bin;$env:Path"
cargo xtask check
cargo xtask package
```

Adjust `C:\msys64` to the MSYS2 installation path. Leave MSYS2 path conversion enabled:
`MSYS2_ARG_CONV_EXCL=*` can leave POSIX include paths unreadable by MSVC, causing the
FFmpeg configure step to report that `libvpl` is missing even after it was built. A regular
FFmpeg executable does not contain the private headers and import libraries needed by the build. If bundled Opus chooses a newer
Visual Studio generator than the one installed, set `CMAKE_GENERATOR` to the installed version
(for example, `Visual Studio 17 2022`) and use a fresh `CARGO_TARGET_DIR`. Choose short local
paths for the FFmpeg build and Cargo output if MSVC reports file-tracking or include-path
errors. See [FFmpeg build and packaging](../ffmpeg/README.md) for the pinned recipe and reuse
rules. Distribute `dist/serein.exe` with its adjacent DLLs, docs, and licenses.

Run `dist/serein.exe --demo` for an offline synthetic preview with no saved-login lookup or account storage. Run without `--demo` only when the owner is ready to operate their account. See [platform support](../../docs/platform-support.md) for the live-test boundary and limitations.

September 10, 2026: Windows x64 workspace checks pass, including all 70 offline Rust tests. The text release created a responsive native window in a process smoke check; visual interaction could not be inspected because the Computer Use helper was unavailable. Installer creation, signing, native Save As, authentication, physical audio and accessibility remain unverified.

System notifications require a Start Menu shortcut carrying the existing AppUserModelID (`cz.viceverse.serein`). The per-user installer creates this shortcut automatically. For a portable archive, extract it to its final folder, then run `powershell -File .\install-notifications.ps1` from that folder (the script must be beside `serein.exe`). It creates only the current user's `NuSerein.lnk`, without administrator rights or autostart, and refuses to overwrite an existing shortcut. `powershell -File .\install-notifications.ps1 -Remove` removes that shortcut before moving/uninstalling the portable package. If Windows marks the downloaded script as internet-sourced and `RemoteSigned` blocks it, review its source and run `Unblock-File -LiteralPath .\install-notifications.ps1` from the extracted folder; this removes the mark for that file without changing the machine execution policy. Normal builds and startup never run this script automatically. A legacy `Serein.lnk` may remain after an upgrade; remove it manually only if it belongs to the old installation. The new script leaves it untouched.

Desktop notifications are on by default for new installs and can be turned off in Serein; Windows may still block them in Settings. Message alerts include a bounded sender name and content preview; an already-cached avatar PNG is added when available. Other alerts remain generic. Windows may retain notification content in its history; logout/disable requests removal of Serein's outstanding notification history through WinRT, without claiming forensic erasure. Native activation/deep links are not implemented. On Windows 11, `ToastNotifier.Setting()` can return `0x80070490` for Serein even when its notifier can submit a toast; this no longer blocks delivery. The [Microsoft shortcut/AUMID requirement](https://learn.microsoft.com/windows/win32/shell/enable-desktop-toast-with-appusermodelid) and the pinned notify-rust/WinRT source APIs were reviewed September 16, 2026.

## Windows Installer & Setup

NuSerein supports both portable zip extraction and a per-user Windows installer:
- **NSIS Installer**: `packaging/windows/installer.nsi` builds `dist-installer/serein-<version>-setup.exe` via `makensis`. It installs per-user to `%LOCALAPPDATA%\Programs\Serein` (`RequestExecutionLevel user`) without requiring administrator elevation. This preserves full user write permissions for the in-app autoupdater. Setup and uninstall check running processes with native Windows Toolhelp APIs; setup writes the notification shortcut's AppUserModelID through the native COM property store. These actions do not launch PowerShell, bypass execution policy or compile C# at installation time. The setup payload excludes the optional portable PowerShell utilities and removes older copies on upgrade. Silent setup stops with code 1 if Serein is running, or code 2 if its process check or shortcut registration fails. Silent uninstall also stops when Serein is running; its normal NSIS launcher detaches a temporary uninstaller process, so the launcher exit code alone does not indicate the uninstall outcome.
- **Portable Archive**: The ZIP includes `install-notifications.ps1` for optional shortcut registration. Use the matching `-Setup.exe` asset for a per-user installation. The source-only `packaging/windows/setup.ps1` script stays in the repository.
- **Autoupdate Compatibility**: The NSIS installer and source PowerShell setup register NuSerein under the existing key `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Serein`. When Serein autoupdates, the update helper automatically synchronizes `DisplayVersion` in the registry upon file replacement, keeping Windows Settings and Installed Apps accurate. The release zip packages remain strictly decoupled from the installer executable, preventing allowlist check failures during in-app update extraction.

## Moving an existing Windows install to the media-runtime build

Windows releases use `serein-<tag>-Windows-X64-media-v5.zip` and
`serein-<tag>-Windows-ARM64-media-v6.zip` for portable installs and in-app updates.
The installer is named `serein-<tag>-Windows-<arch>-Setup.exe`.
ARM64 media-v6 removes the x64-only `vcruntime140_1.dll` from its payload.
Earlier updaters look for their matching archive name and require its original contents.

Upgrading from the original, media-v2, media-v3, media-v4, or ARM64 media-v5 format
requires one manual install:

1. Close Serein completely, then download the matching architecture's Setup.exe
   from the project's trusted release page. Run it over the existing per-user
   installation. The installer replaces application files; it does not remove
   account settings, local data or credentials.
2. For a portable installation, extract the complete ZIP for your architecture into a new
   folder and start `serein.exe` from that folder. Keep the adjacent DLLs,
   `licenses`, `ffmpeg-source` and `voice-enhancement`; copying just the executable is insufficient.
   If you registered a portable notification shortcut, remove the old shortcut
   using its `install-notifications.ps1 -Remove` and register it again from the
   new folder.

The release workflow checks the actual archive's name, required DLLs, notices
and corresponding source before upload. Run the offline checks with
`python packaging/windows/test_update_archive.py`; they use synthetic ZIPs and
never start the client. Manual migration on physical Windows installations
still needs native verification before release.

Run `python packaging/windows/test_installer.py` in a disposable Windows user with Rust and NSIS installed. It uses an offline fixture executable to check the running-process guard, Unicode install paths, shortcut target/working directory/AppUserModelID and icon, upgrades, registry entries and uninstall cleanup. It also checks the installer and uninstaller artwork; it never launches the real client. CI runs this check on Windows x64 and ARM64, including draft PRs. `packaging/windows/check-brand-icon.ps1` also checks the icon Windows extracts from the real packaged executable and installer against the bundled artwork.

Removing script-driven setup behavior addresses the installer heuristics reported in [issue #546](https://github.com/ViceVerse-cz/Serein/issues/546). Windows signing is not configured in the release workflow; a rebuilt installer still needs vendor scanning and can receive reputation or antivirus warnings. Portable notification setup and the app's existing update helper remain separate PowerShell flows.

The local speech assets include LavaSRv2 with denoising, CPU ONNX Runtime and
app-local MSVC CRT DLLs. Uninstall removes the installed models; account logout
does not. The application never downloads a model during a call.

Build `cargo xtask package` using Rust 1.98.1 MSVC and Visual Studio C++ build tools. The executable is `dist/serein.exe`; distribute it with the adjacent docs and licenses. The login flow requires WebView2. Source builds require CMake for the built-in voice engine.

Run `dist/serein.exe --demo` for an offline synthetic preview with no saved-login lookup or account storage. Run without `--demo` only when the owner is ready to operate their account. See [platform support](../../docs/platform-support.md) for the live-test boundary and limitations.

September 10, 2026: Windows x64 workspace checks pass, including all 70 offline Rust tests. The text release created a responsive native window in a process smoke check; visual interaction could not be inspected because the Computer Use helper was unavailable. Installer creation, signing, native Save As, authentication, physical audio and accessibility remain unverified.

System notifications require a Start Menu shortcut carrying Serein's own AppUserModelID (`cz.viceverse.serein`). The per-user installer creates this shortcut automatically. For a portable archive, extract it to its final folder, then run `powershell -File .\install-notifications.ps1` from that folder (the script must be beside `serein.exe`). It creates only the current user's `Serein.lnk`, without administrator rights or autostart, and refuses to overwrite an existing shortcut. `powershell -File .\install-notifications.ps1 -Remove` removes that shortcut before moving/uninstalling the portable package. If Windows marks the downloaded script as internet-sourced and `RemoteSigned` blocks it, review its source and run `Unblock-File -LiteralPath .\install-notifications.ps1` from the extracted folder; this removes the mark for that file without changing the machine execution policy. Normal builds and startup never run this script automatically.

Desktop notifications are on by default for new installs and can be turned off in Serein; Windows may still block them in Settings. Message alerts include a bounded sender name and content preview; an already-cached avatar PNG is added when available. Other alerts remain generic. Windows may retain notification content in its history; logout/disable requests removal of Serein's outstanding notification history through WinRT, without claiming forensic erasure. Native activation/deep links are not implemented. On Windows 11, `ToastNotifier.Setting()` can return `0x80070490` for Serein even when its notifier can submit a toast; this no longer blocks delivery. The [Microsoft shortcut/AUMID requirement](https://learn.microsoft.com/windows/win32/shell/enable-desktop-toast-with-appusermodelid) and the pinned notify-rust/WinRT source APIs were reviewed September 16, 2026.

## Windows Installer & Setup

Serein supports both portable zip extraction and a per-user Windows installer:
- **NSIS Installer**: `packaging/windows/installer.nsi` builds `dist-installer/serein-<version>-setup.exe` via `makensis`. It installs per-user to `%LOCALAPPDATA%\Programs\Serein` (`RequestExecutionLevel user`) without requiring administrator elevation. This preserves full user write permissions for the in-app autoupdater. Setup and uninstall check running processes with native Windows Toolhelp APIs; setup writes the notification shortcut's AppUserModelID through the native COM property store. These actions do not launch PowerShell, bypass execution policy or compile C# at installation time. The setup payload excludes the optional portable PowerShell utilities and removes older copies on upgrade. Silent setup stops with code 1 if Serein is running, or code 2 if its process check or shortcut registration fails. Silent uninstall also stops when Serein is running; its normal NSIS launcher detaches a temporary uninstaller process, so the launcher exit code alone does not indicate the uninstall outcome.
- **PowerShell Setup**: `packaging/windows/setup.ps1` provides a zero-dependency installer/uninstaller script (`powershell -File .\setup.ps1` to install, `powershell -File .\setup.ps1 -Uninstall` to remove).
- **Autoupdate Compatibility**: Both installers register Serein in `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Serein`. When Serein autoupdates, the update helper automatically synchronizes `DisplayVersion` in the registry upon file replacement, keeping Windows Settings and Installed Apps accurate. The release zip packages remain strictly decoupled from the installer executable, preventing allowlist check failures during in-app update extraction.

## Moving an existing Windows install to the FFmpeg build

Windows releases with the FFmpeg runtime use
`serein-<tag>-Windows-<arch>-media-v2.zip` (`X64` or `ARM64`). The installer name
remains `serein-<tag>-Windows-<arch>-Setup.exe`. The updated client selects the
media-v2 ZIP for subsequent in-app updates.

Clients released before FFmpeg accept only the original payload and cannot
install the new DLLs or their corresponding source. They look for the original
ZIP filename and will report a missing package on a media-v2 release. Updating
the allowlist in the new binary cannot repair the old running updater, and users
can skip an intermediate release, so publishing a bridge release alone is
insufficient.

The first migration requires one manual install:

1. Close Serein completely, then download the matching architecture's Setup.exe
   from the project's trusted release page. Run it over the existing per-user
   installation. The installer replaces application files; it does not remove
   account settings, local data or credentials.
2. For a portable installation, extract the complete media-v2 ZIP into a new
   folder and start `serein.exe` from that folder. Keep the adjacent DLLs,
   `licenses` and `ffmpeg-source`; copying just the executable is insufficient.
   If you registered a portable notification shortcut, remove the old shortcut
   using its `install-notifications.ps1 -Remove` and register it again from the
   new folder.

Do not rename a media-v2 ZIP to the legacy filename when publishing a release.
The release workflow checks the actual archive's name, required DLLs, notices
and corresponding source before upload. Run the offline checks with
`python packaging/windows/test_update_archive.py`; they use synthetic ZIPs and
never start the client. Manual migration on physical Windows installations
still needs native verification before release.

Run `python packaging/windows/test_installer.py` in a disposable Windows user with Rust and NSIS installed. It uses an offline fixture executable to check the running-process guard, Unicode install paths, shortcut target/working directory/AppUserModelID, upgrades, registry entries and uninstall cleanup. It never launches the real client. CI runs this check on Windows x64 and ARM64, including draft PRs.

Removing script-driven setup behavior addresses the installer heuristics reported in [issue #546](https://github.com/ViceVerse-cz/Serein/issues/546). Windows signing is not configured in the release workflow; a rebuilt installer still needs vendor scanning and can receive reputation or antivirus warnings. Portable notification setup and the app's existing update helper remain separate PowerShell flows.

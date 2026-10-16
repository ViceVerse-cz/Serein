# Embed styling review evidence

The baseline is PR #650's original head, `c01461edf02da38bd0428200bfd9e032e3357e46`.
The changed application is `9354335e2a3828d4fbd6946ba6cc84ed506fa48d`.
The new `profile_preview --page=embeds` fixture was copied into the baseline worktree too;
its application code was otherwise unchanged. These are synthetic native Windows captures,
with no account, Discord transport, microphone or browser navigation.

The fixture shows three untitled cards: a source URL already present in the message, a
missing source URL with a footer, and a missing source URL with no footer or timestamp.
The second card's footer becomes a source link; the third gains the existing localized
"Open link" action. Author destinations stay separate from post destinations.

All six PNGs were inspected. Cards and source controls fit both themes and the narrow
viewport. The UI reuses the existing palette, typography, spacing and localized link;
the footer underline identifies its source action. No new artwork or motion was added.
Manual native clicks and keyboard interaction were stopped by the user's physical Escape
key. Synthetic interaction tests verify source, author and provider destinations and
preserve media copy/save actions. The PR remains draft while native interaction and CI
evidence are incomplete.

## Reproduce the captures

Use separate worktrees for the baseline and changed revision. Copy only the changed
`apps/desktop/examples/profile_preview.rs` into the baseline before building it.
Run the following from each worktree, using distinct build/output paths:

```powershell
$env:CARGO_TARGET_DIR = 'E:/codex-builds/serein-review650-target'
cargo build --release --locked -p serein --no-default-features --features demo --example profile_preview
$previewExe = "$env:CARGO_TARGET_DIR/release/examples/profile_preview.exe"
& $previewExe --demo --page=embeds --width=1120 --height=900 --output=before.png
& $previewExe --demo --page=embeds --width=1120 --height=900 --light --output=before-light.png
& $previewExe --demo --page=embeds --width=760 --height=900 --output=before-narrow.png
```

Use `after` in the output filenames for the changed revision. Preserve the baseline
executable before rebuilding in a shared target directory. The screenshots are actual
eframe/wgpu framebuffer exports, inspected alongside the native window. On this display,
125% scaling produced 1400 x 1125 and 950 x 1125 pixel images from the requested DIP sizes.

## Reproduce package and process measurements

Run `cargo xtask package` in both worktrees, retaining their separate `dist` directories.
In this run, a later cleanup removed baseline binaries, ZIPs and raw sampling files;
the recorded aggregate measurements and native PNG captures remain the evidence.
Measure `dist/serein.exe` and sum the lengths of all files under `dist`. Create each ZIP
with the same .NET compression implementation:

```powershell
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory(
    $distDirectory, $zipPath, [IO.Compression.CompressionLevel]::Optimal, $false)
```

Run each release `profile_preview` with
`--demo --page=embeds --width=1120 --height=900 --interactive`. After a five-second warmup,
take 20 samples at one-second intervals using `Get-Process`:
`WorkingSet64`, `PrivateMemorySize64`, and `TotalProcessorTime.TotalSeconds`.
Report the maximum observed memory and the final sample as settled memory. Idle CPU is
`100 * (last CPU seconds - first CPU seconds) / elapsed wall seconds`, with 100% meaning
one logical core. Sample the preview's `conhost` child separately; it is a console host,
not a media or voice helper. This is one run per revision, not a statistical performance
claim. Other compilation work was active on the host during measurement.

Host: Windows 11 Home 10.0.26200, Ryzen 7 7800X3D, 33,410,678,784 bytes physical RAM,
wgpu renderer, 125% display scale. Preview builds use `--no-default-features --features demo`;
the standard unsigned portable packages use `--no-default-features` and include voice.
The selected GPU/backend, startup latency and frame percentiles were not instrumented.

NSIS is unavailable on this host, so `xtask package` creates the portable tree but skips
the Windows installer executable. Live Discord, macOS and Linux behavior remain untested.

# Symbols fallback evidence

The four inspected PNGs are native Windows screen captures of the offline
`profile_preview` example, running production profile widgets with synthetic
content. Before, U+2726 (`✦`) appears as missing-glyph bars in the bio editor and
profile card. After, the four-pointed stars render in both places. The wide dark
viewport is 1120×760 logical pixels; the narrow light viewport is 640×760.
At 125% display scale, the captured windows are 1400×950 and 800×950 pixels.

The baseline is `38d919d74054b885b992ce607f0bfa93ab449696`; the repaired runtime
is `7894963e05d6a617b1765df04be0f51a17c5000f`, before this evidence-only commit.
The baseline has two disclosed local adjustments: the same Windows lockfile
correction as the PR, and the same synthetic bio fixture. Without the correction,
baseline WGPU fails to compile because `gpu-allocator` and WGPU resolve different
Windows binding types. The baseline renderer and font assets are unchanged.

The fixture adjustment in `prime_profile` is:

```rust
let mut profile = ui::synthetic_own_profile(state.user.as_ref().unwrap());
profile.bio = "✦ quiet corners ✦\nSynthetic preview with a four-pointed star.".into();
```

The lockfile adjustment changes only the `gpu-allocator` dependency reference
from `windows 0.61.3` to the already-locked `windows 0.62.2`.

Both revisions use Rust 1.98.1, the lockfile, normal release fat LTO and the same
Cargo target directory, built sequentially. The preview uses default features
plus `demo`. Standard packages use `cargo xtask package`, include voice and omit
the demo feature. The preserved preview executable is copied before building
the next revision. Source/binary/image hashes, raw process samples and package
sizes are in [measurements.json](measurements.json).

Host: Windows 11 Home 10.0.26200, Ryzen 7 7800X3D with 16 logical CPUs, 32 GiB
physical RAM, NVIDIA RTX 5070 Ti and AMD integrated graphics. Both native preview
launches set `WGPU_BACKEND=dx12`. Adapter selection and GPU memory are not measured.

Build each revision from its source worktree, preserving its preview:

```powershell
$env:CARGO_TARGET_DIR='E:/codex-builds/serein-compact-after-target'
cargo clean -p xtask -p ui
cargo xtask package
cargo build --release --locked -p serein --features demo --example profile_preview
Copy-Item "$env:CARGO_TARGET_DIR/release/examples/profile_preview.exe" PHASE_PREVIEW
```

Invalidate the helper and UI caches when switching revisions in a shared target
directory. The baseline helper initially omitted the new license, and the demo
UI cache initially omitted the new font. The corrected package is verified after
rebuilding the task's helper; the demo UI is rebuilt after invalidating its source
mtime. The final production and preview binaries are checked for the embedded
subset before capture. These are build-cache corrections, with no source change.

Capture and sample each preserved executable with the scripts in this directory:

```powershell
$env:WGPU_BACKEND='dx12'
$preview=Start-Process -FilePath PHASE_PREVIEW -WindowStyle Hidden -PassThru `
  -ArgumentList '--demo --interactive --page=profile --width=1120 --height=760'
Start-Sleep -Seconds 3
./capture-window.ps1 -ProcessId $preview.Id -OutputPath PHASE_DARK_PNG
Add-Type -AssemblyName UIAutomationClient
$preview.Refresh()
$root=[System.Windows.Automation.AutomationElement]::FromHandle($preview.MainWindowHandle)
$root.FindAll([System.Windows.Automation.TreeScope]::Descendants,
  [System.Windows.Automation.Condition]::TrueCondition).Count
./sample-preview.ps1 -ProcessId $preview.Id -OutputPath PHASE_PROCESS_JSON
$preview.CloseMainWindow()
```

For the light capture, launch a fresh preview with
`--demo --interactive --page=profile --width=640 --height=760 --light`, capture
it and close its window. The capture helper restores and foregrounds only the
specified synthetic preview, verifies focus and copies its native screen bounds.
It uses Win32 and `System.Drawing`, not egui's framebuffer export. The installed
computer-use helper's native pipe is unavailable (`os error 2`); this is the
native platform fallback. The preview's Windows UI Automation tree was also read
in both revisions. This does not establish a complete accessibility audit.

Sampling follows the wide capture and accessibility-tree read. After five
seconds of warmup, twenty 500 ms samples record `TotalProcessorTime`,
`PrivateMemorySize64` and `WorkingSet64`. CPU is percent of one logical core;
settled memory is the last sample, and peak memory is the sampled maximum.
Only the preview process is included. No task build runs during sampling;
unrelated compiler activity on the shared machine remains a source of noise.
One launch pair cannot establish a causal memory or CPU regression. Startup,
frame-time p95, GPU allocations and live traffic are unmeasured.

Installed size is the sum of regular files below `dist`. Each complete ZIP uses
`Compress-Archive -LiteralPath dist -CompressionLevel Optimal`, including the
base directory. NSIS is unavailable locally, so these measurements cover the
unsigned directory package and ZIP, rather than a Windows installer. The changed
package's Symbols 2 license is verified byte-for-byte against its source.

`cargo xtask check` passes: 1,166 tests passed, 27 ignored, with formatting,
strict workspace Clippy, production and policy checks. Five focused font tests,
fuzz formatting and `node tests/xtask-workspace.cjs` also pass. The eight stale UI
test failures reproduced on the baseline before repair. Linux package tests are
covered by Linux CI, not by this Windows host.

The subset is 87,460 bytes and was reproduced byte-for-byte from the pinned
upstream using FontTools 4.66.1; provenance is in [assets/README.md](../../../assets/README.md).
The embedded font total is 15,667,889 bytes, below the existing 16 MiB ceiling.
These captures and tests use no Discord account, microphone or call. They do not
prove live Discord interoperability or complete Unicode coverage.

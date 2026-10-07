# Compact message alignment evidence

The four inspected PNGs use the existing offline `profile_preview` native
eframe/WGPU framebuffer callback and production message widgets. The `member-tags`
fixture covers short, wrapped, Unicode, quote and code messages. All content is
synthetic. The same `--compact` harness was applied to the baseline before building;
its original UI sources were retained.

These are framebuffer exports, not OS window captures or native input,
accessibility, or live Discord evidence. Windows computer-use failed with
`os error 2`, and Orca was unavailable. The existing synthetic wheel injection
exposes text rows; native hover toolbars/tooltips differ between captures.

Runtime/package revisions are baseline `2423f600ad3cdb266c4c5ea8a840b2b2ade4795c`
and after `05af1e6aee17a44f5821f8abe5d20350722121dc`, before the later evidence-only
commit. Source, executable, UI-library and PNG hashes are in
[measurements.json](measurements.json).

| Phase | Source worktree | Release target | Preserved preview |
| --- | --- | --- | --- |
| Before | `E:/codex-builds/serein-compact-baseline-src` | `E:/codex-builds/serein-compact-baseline-target` | `target/native-baseline/profile_preview.exe` in baseline source |
| After | `C:/Users/Jakub/Desktop/Projects/Serein-compact-alignment` | `E:/codex-builds/serein-compact-after-target` | `target/native-after/profile_preview.exe` in task source |

Each revision uses an isolated Cargo target directory. Both normal release builds
use pinned Rust 1.98.1, `--no-default-features --features demo`, and
`CARGO_BUILD_JOBS=1`; neither overrides LTO. The after build explicitly compiled
`ui` from the task worktree. Run from each source worktree, substituting its target:

```powershell
$env:CARGO_BUILD_JOBS='1'
cargo build --release --locked -p serein --no-default-features --features demo --example profile_preview --target-dir PHASE_TARGET
```

Windows 11 Home 10.0.26200, Ryzen 7 7800X3D/16 logical CPUs, 33,410,678,784 bytes
RAM. `WGPU_BACKEND=vulkan` selects the NVIDIA GeForce RTX 5070 Ti, driver 591.86;
both sampled processes mapped `vulkan-1.dll` and `nvoglv64.dll`. Display scale is
1.25: wide 1400×900 logical / 1750×1125 pixels; narrow 760×900 / 950×1125.

Capture commands, using the preserved executables above and this evidence folder:

```powershell
$before='E:/codex-builds/serein-compact-baseline-src/target/native-baseline/profile_preview.exe'
$after='C:/Users/Jakub/Desktop/Projects/Serein-compact-alignment/target/native-after/profile_preview.exe'
$evidence='C:/Users/Jakub/Desktop/Projects/Serein-compact-alignment/docs/pr-evidence/compact-message-alignment'
$env:WGPU_BACKEND='vulkan'
& $before --demo --compact --page=member-tags --width=1400 --height=900 --scroll=-1200 "--output=$evidence/before.png"
& $after --demo --compact --page=member-tags --width=1400 --height=900 --scroll=-1200 "--output=$evidence/after.png"
& $before --demo --compact --page=member-tags --width=760 --height=900 --light --scroll=-1200 "--output=$evidence/before-light-narrow.png"
& $after --demo --compact --page=member-tags --width=760 --height=900 --light --scroll=-1200 "--output=$evidence/after-light-narrow.png"
```

Idle sampling uses one fresh process per revision, dark 1400×900, at the default
bottom of the compact timeline with message 500 media. It uses no injected
scroll/input and therefore differs from the scrolled screenshot view. No build or
ZIP work ran during either valid sample. For each preserved executable:

```powershell
$exe='C:/Users/Jakub/Desktop/Projects/Serein-compact-alignment/target/native-after/profile_preview.exe'
# Repeat with $exe='E:/codex-builds/serein-compact-baseline-src/target/native-baseline/profile_preview.exe' for baseline.
$stdout='C:/Users/Jakub/Desktop/Projects/Serein-compact-alignment/target/compact-reproduction.stdout.log'
$stderr='C:/Users/Jakub/Desktop/Projects/Serein-compact-alignment/target/compact-reproduction.stderr.log'
$env:WGPU_BACKEND='vulkan'
$scenario=@('--demo','--interactive','--compact','--page=member-tags','--width=1400','--height=900')
$p=Start-Process -FilePath $exe -ArgumentList $scenario -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
try {
    Start-Sleep -Seconds 8
    $p.Refresh()
    if ($p.HasExited) { throw 'Preview exited before warmup' }
    $p.Modules | Where-Object ModuleName -Match 'vulkan|nvoglv' | Select-Object ModuleName,FileName
    $clock=[Diagnostics.Stopwatch]::StartNew()
    $lastCpu=$p.TotalProcessorTime.TotalSeconds
    $lastWall=$clock.Elapsed.TotalSeconds
    $samples=@(for ($i=0; $i -lt 20; $i++) {
        Start-Sleep -Seconds 1
        $p.Refresh()
        if ($p.HasExited) { throw 'Preview exited during sample' }
        $wall=$clock.Elapsed.TotalSeconds
        $cpu=$p.TotalProcessorTime.TotalSeconds
        $children=@(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($p.Id)" | Select-Object ProcessId,Name)
        [pscustomobject]@{elapsed_s=$wall;interval_s=($wall-$lastWall);cpu_percent_one_core=(100*($cpu-$lastCpu)/($wall-$lastWall));working_set_bytes=$p.WorkingSet64;children=$children}
        $lastCpu=$cpu
        $lastWall=$wall
    })
    $samples | ConvertTo-Json -Depth 6
} finally {
    if (-not $p.HasExited) {
        $null=$p.CloseMainWindow()
        if (-not $p.WaitForExit(10000)) { $p.Kill(); $p.WaitForExit() }
    }
    $p.Dispose()
}
```

Raw samples include polling overhead. CPU is percent of one logical core;
`WorkingSet64` is process working set. Peak is the maximum post-warmup sample;
settled working set is the median of the final five. One `conhost.exe` child was
observed per process; its CPU/memory are unmeasured and excluded. Baseline has one
nonzero CPU interval and after has five; both medians are zero. This single launch
pair cannot establish a causal improvement or regression. Startup, full-frame p95,
GPU memory, native input/accessibility and live workloads remain unmeasured.

Standard voice-inclusive packages use `cargo xtask package` and the two recorded
runtime revisions. Installed bytes sum the 213 regular files in each `dist` tree.
The complete distribution ZIP uses `.NET ZipFile.CreateFromDirectory` with
`CompressionLevel.Optimal` and `includeBaseDirectory=false`. NSIS was unavailable;
the package routine skipped installer generation on both revisions. Package sizes
and hashes are preserved separately from the demo-preview measurements.

## Verification

- `cargo test --workspace --locked`: 1,153 passed, 27 ignored.
- `cargo test --locked -p ui --lib compact`: all three tests passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo clippy --locked -p serein --no-default-features --features demo --example profile_preview -- -D warnings`: passed.
- Task-path formatting, `cargo xtask policy`, and `cargo check --locked -p serein --no-default-features`: passed.
- `cargo xtask package`: both standard voice-inclusive packages passed.
- `cargo xtask check`: stops at pre-existing `forum.rs` formatting, independently reproduced with `cargo fmt --all -- --check` on the untouched baseline.

The commands above reproduce native framebuffer captures and process sampling;
native input, other OS rendering and live Discord behavior remain unverified.

# Native PDF preview: synthetic evidence

Collected on October 11, 2026. No live Discord account, original user PDF,
authentication, messages, calls or microphone was used.

## Source and build provenance

Starting upstream `ViceVerse-cz/Serein/main`:
`134e556fb469f3fa72425adf24acb7df56811a59` (October 10, 2026).
The baseline used a separate disposable worktree; the owner's existing
`feat/native-message-links` checkout was not changed.

The standard baseline package was built from that exact revision **before**
adding the demo fixture. `baseline-fixture.patch` records the baseline-only
additive message fixture and `--demo-pdf` selector used for native captures.
This changes the scenario, not its attachment renderer: baseline has no
Preview button or PDF modal. The same synthetic messages are in the changed
build. Neither capture depicts live Discord.

Toolchain: pinned Rust/Cargo 1.98.1 (Fedora), locked dependencies.
Standard packages use `cargo xtask package --format dir`: release, no default
features, voice included, the repository's fat-LTO profile. Package outputs
were retained separately; demo executables are not used for package sizes.
Native binaries use matching release demo builds:

```sh
cargo rustc --release --locked -p serein \
  --no-default-features --features demo \
  -- -C lto=thin -C codegen-units=8
```

Dependencies retain their release settings. The demo-only thin-LTO override
does not describe the standard distributable.

The changed binaries were built from the uncommitted feature tree on the
baseline above; no application source changed between those builds,
measurements and the delivery commit. Evidence-only additions followed.
SHA-256 hashes:

| Executable | SHA-256 |
| --- | --- |
| Baseline standard package | `6b469f9295142a59fe794b9227b1f6c2fe68d689055b4582659b0c5895902495` |
| Changed standard package | `4f01ded674c1319887b2ec95e1be9cceff30850cb7ad5e5c627bacec3e2f75f8` |
| Baseline demo, with additive fixture | `c6e4e2fe06376aa305310735064d4a19da303f2e6565bfba09c8bbda162e4447` |
| Changed demo | `8fd201ea4394793f038123f0ecf2cdb8e4d66a6e9798682a528900614b761e8f` |

## Environment and methods

Fedora 43, Intel Core i5-1035G1, eight logical CPUs, 23 GiB RAM.
Native X11 display: Xvfb, 1440×1000, 96 dpi / 100% scale; app 1120×760.
Software Vulkan renderer: llvmpipe (LLVM 21.1.8, 256 bits).
Light and 760×520 / long-filename checks use the same changed binary.

`measure-native.py` launches only `--demo --demo-pdf` with isolated
configuration/cache directories, places the pointer outside the window,
waits 12 seconds, clears any initial hover and allows another second for
repainting, captures the app window, then samples `/proc` every 100 ms
for 50 samples (~5 seconds). RSS is resident pages, not private memory/GPU
usage. Peak **sampled** RSS is not the process lifetime high-water mark.
Idle CPU is process user+system clock ticks divided by elapsed time, as
percent of one logical CPU. One run per revision is noisy and does not
establish an improvement or a universal memory bound.

`measure-helper.py` runs the **changed standard package executable**'s
pre-GUI entrypoint with a cleared environment and the 2,116-byte ordinary
fixture. It validates the bounded pipe response and uses `wait4` for child
user/system CPU and peak RSS (Linux KiB). One warmup and five measured runs
are performed **per page**. Helper wall time includes process startup, parsing,
rasterization and pipe transfer; it is not UI startup/frame/interaction latency.
Baseline has no helper, so helper metrics have no baseline comparison.

Installed-package bytes sum `linux-root` file lengths; staged-distribution
bytes sum every staged file (including the executable's two copies).
Compressed distribution is the full staged tree, using:

```sh
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
  -cf - -C PACKAGE . | gzip -n -9 > PACKAGE.tar.gz
```

The Linux package is an unsigned directory distribution, not a tested DEB/RPM.
No generated executable, distribution archive or build directory is committed.

## Results

Raw package values are in `package-measurements.json`. Both standard package
builds include voice and use the same release settings.

| Package metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Executable | 87,338,088 B | 92,696,008 B | +5,357,920 B / +6.13% |
| Installed Linux tree | 91,870,032 B | 97,435,353 B | +5,565,321 B / +6.06% |
| Full staged distribution | 183,385,083 B | 194,515,725 B | +11,130,642 B / +6.07% |
| Full staged tar.gz | 106,078,659 B | 111,018,749 B | +4,940,090 B / +4.66% |

The dependency/font/CMap addition has a measurable size cost. The installed
tree adds 42 notice/provenance files; all 41 copied notice hashes were verified.

Matched closed-card native process samples:

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| Settled / peak sampled RSS | 248,544 KiB | 250,776 KiB | +2,232 KiB / +0.90% |
| Idle CPU, percent of one core | 0.00% | 0.20% | +0.20 percentage points |
| Sample duration, 50 samples | 4.933 s | 4.932 s | Approximately equal |

`before-samples.json` and `after-card-samples.json` retain the matching pair.
The CPU difference is a single clock tick; this noisy single-run comparison
does **not** establish a meaningful idle CPU regression or improvement.
Closed cards do not fetch, parse or render a PDF.

`after-page1-samples.json` samples the first-page modal in the **same changed
process**: settled/peak sampled RSS 267,536 KiB, +16,760 KiB / +6.68% over its
closed card; idle CPU 0.20% over 4.934 seconds. This is a changed-only workload,
not a before/after viewer comparison. No helper remained after the render.

`helper-measurements.json` retains one warmup plus five measured helper runs
per page. Independent medians:

| Changed standard-package helper metric | Page 1 | Page 2 |
| --- | ---: | ---: |
| Wall time, including startup/parse/render/pipe | 67.83 ms | 66.60 ms |
| User CPU | 39.89 ms | 43.25 ms |
| System CPU | 26.31 ms | 21.66 ms |
| Peak RSS | 64,468 KiB | 64,520 KiB |
| Raster dimensions | 1,099×1,429 | 1,076×1,461 |

These roughly 63-MiB child measurements exclude the desktop and apply only
to the tiny synthetic fixture. They are not UI latency or arbitrary-document
performance claims. There is no baseline PDF helper.

Performance behavior is deliberately demand-driven: one encoded original is
reused during navigation, only the current raster is retained, and zoom/Fit/
scroll reuse that texture without re-rendering. A synthetic scheduling test
coalesces rapid page changes to the latest request, rejects superseded results
and verifies cache release on close/replacement. No page prefetch, persistent
renderer or growing work queue is introduced.

## Native interaction checks

`before.png` and `after.png` are original 1120×760 dark native captures.
The baseline shows the existing download-only PDF card; the changed capture
shows page 1 of the new modal. `after-light-page.png` and
`after-light-narrow.png` record light appearance and the 760×520 long-filename
case. Every retained image is a window-only PNG under 2 MiB, without rescaling
or generated UI content.

Native input checks exercised:

- Next/Previous and Left/Right navigation; Right at page 2 stays at 2/2.
- Four zoom-in clicks reach 2.00×, zoom-out returns to 1.75×, Fit returns to 1.00×.
- Vertical scrolling at 2× changes the page region and reveals lower-page text.
- Horizontal scrolling at 2× in a 760×960 window: after explicit X11 window
  focus, Shift+wheel moves shared page text 266 pixels left with unchanged
  vertical coordinates. An earlier ineffective horizontal attempt was excluded.
- Escape, backdrop and header-close dismissal.
- Light/dark minimum 760×520 layouts and long filenames. OCR/pixel/bounds
  checks found the title, toolbar and Download footer within the viewport and
  nonempty rendered page content.

These are interaction and image-screening checks, not direct visual approval.

## Reproduction

Use a separate baseline worktree at the exact revision above. Build its
standard package before applying `baseline-fixture.patch`; then build its
demo. Build the task's standard package and matching demo separately.
Keep both standard packages outside `dist` so a subsequent build cannot
overwrite them.

```sh
python3 docs/pr-evidence/native-pdf-preview/measure-native.py \
  /absolute/path/to/baseline-demo /tmp/pdf-evidence before
python3 docs/pr-evidence/native-pdf-preview/measure-native.py \
  /absolute/path/to/changed-demo /tmp/pdf-evidence after-card --keep
# With DISPLAY=:78, use the native window's Preview, page/zoom controls
# and keyboard; capture only that window. Terminate the owned demo to finish.
python3 docs/pr-evidence/native-pdf-preview/measure-helper.py \
  /absolute/path/to/changed-standard-package/serein /tmp/helper-measurements.json
```

Use `--light` for the light appearance and `xdotool windowsize WINDOW 760 520`
for the minimum native viewport. A fresh display can be selected with
`--display :79`. Do not pass the baseline binary to the helper script.
Run screenshots/measurements with no build active; keep the same procedure on
both revisions.

## Verification and limitations

See `checks.txt` for the completed full workspace gate, focused PDF/helper
tests, both gateway compression backends, packaging and dependency checks.
The two synthetic fixtures and their generation/provenance are documented
in `apps/desktop/tests/fixtures/README.md`.

Native screenshots are genuine window captures, not generated mockups.
Direct visual-image inspection was unavailable: the image-viewing tool
explicitly rejected image inputs. OCR, image dimensions/pixel-region checks
and native keyboard/mouse interactions can screen these captures, but are
**not visual approval** of the screenshots. Human visual review remains
necessary.

The resource-limited helper is **not an OS sandbox**; upstream experimental
renderer/unsafe-code risks and arbitrary-document fidelity remain.
Windows/macOS runtime limits, screen-reader behavior, live attachment CDN
compatibility and authenticated source changes are unverified here.
Synthetic regressions exercise source/account/permission/spoiler retirement,
timeouts, cancellation/reaping, coalescing and malformed/password/limit errors.
PDF text selection/search and password entry are not implemented.
Frame/p95 latency, hardware-GPU memory and whole-session/live performance are
unmeasured. No synthetic result establishes Discord interoperability.

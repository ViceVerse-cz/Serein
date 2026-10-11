# PDF-only optimization evidence

Collected October 11, 2026, using synthetic offline documents. This is a
comparison against the **already-implemented PDF feature**, not against
upstream without a viewer. Original upstream-versus-feature evidence in the
parent directory remains historical and unchanged.

## Provenance and scope

Baseline: `72efba34aa2bebb7e827a4705b748801489b30bd`, the first PDF-preview
commit on PR #625. Its separately retained standard package and demo hashes
match the parent evidence. The optimized binaries use the application source
in this optimization commit; later evidence/documentation additions do not
change application source.

The optimization changes only PDF render targeting, pixel transfer/conversion
and cancellation handling. An explicit page request samples modal size and
display density with headroom for 2× zoom; zoom/Fit/scroll/resize reuse the
current texture. The helper moves its premultiplied raster allocation rather
than making a second page-sized copy. Already-cancelled rasters skip conversion.
No parser, standard-font/CMap, dependency, resampling-SIMD or other media
configuration is removed or changed. No thread pool, page prefetch, growing
queue, persistent renderer or disk PDF cache is added.

Two related PR findings are also fixed: Unix timeout tests use `/bin/sleep`,
and unrelated reaction revisions no longer retire a valid PDF source.
The reaction regression failed before the fix, then passed for timeline,
search and pin sources, preserving document identity, texture and work count.
Existing source/edit/spoiler/account/permission retirement remains tested.

Upstream was fetched again: `main` advanced from the original baseline
`134e556fb469f3fa72425adf24acb7df56811a59` to
`178bde35620a0eb6594aa22ce33aa4a8ca47ae0f` on October 11. Besides the earlier
README-only update, that revision adds unrelated video work. Its command-block
insertion overlapped the PDF dispatch. Only the PDF dispatch position was
relocated, after notification configuration, to avoid that conflict.
A read-only three-way merge inspection succeeds; no merge/rebase, upstream
video code or upstream dependencies are included in these local builds.
Comparisons remain against the recorded initial PDF implementation, not the
newer upstream video implementation. The original message-links checkout
remains untouched.

## Methods and reproduction

Fedora 43, Intel Core i5-1035G1, eight logical CPUs, 23 GiB RAM; pinned
Rust/Cargo 1.98.1, locked dependencies. Standard packages use
`cargo xtask package --format dir`: no default features, release fat LTO and
voice included. Native demo executables use the matching final-executable
thin-LTO override described in the parent evidence, not package size builds.

Helper comparisons use separately preserved standard executables. Run with
no build active:

```sh
EVIDENCE=docs/pr-evidence/native-pdf-preview/optimization
FIXTURES=apps/desktop/tests/fixtures
python3 "$EVIDENCE/measure-helper-comparison.py" \
  /absolute/path/to/baseline-standard/serein /tmp/helper-before.json \
  --protocol 1 --fixture "$FIXTURES/pdf-preview.pdf"
python3 "$EVIDENCE/measure-helper-comparison.py" \
  /absolute/path/to/optimized-standard/serein /tmp/helper-after.json \
  --protocol 2 --fixture "$FIXTURES/pdf-preview.pdf"
# Repeat with --fixture "$FIXTURES/pdf-images.pdf" --pages 1 for image resampling.
# Repeat optimized runs with --target 1536 1536 for the maximum envelope.
```

`posix_spawn` starts only the pre-GUI helper with a cleared environment.
Bounded nonblocking pipes validate its response; `wait4` reports total
user+system CPU and child peak RSS (Linux KiB). There is one warmup and five
measured runs **per page**. Independent medians include process startup,
PDF parsing/rendering and pipe transfer, not UI interaction/frame latency.
The original 2,116-byte two-page fixture covers vectors/text and crop/rotation.
The new 27,015-byte image fixture deterministically covers 1800×1200 RGB,
grayscale softmask and grayscale resampling. It is test-only, not a runtime
asset. Both source revisions render these same documents.

Native comparisons use Xvfb at 96 dpi / 100% scale, a 1120×760 window and
software Vulkan llvmpipe (LLVM 21.1.8, 256 bits). Isolated config/cache and
explicit `--demo --demo-pdf` prevent a saved authenticated session.

```sh
python3 "$EVIDENCE/measure-pdf-native.py" \
  /absolute/path/to/baseline-demo /tmp/pdf-native before --display :81
python3 "$EVIDENCE/measure-pdf-native.py" \
  /absolute/path/to/optimized-demo /tmp/pdf-native after --display :82
# Add --keep to exercise the native window; --light checks light appearance.
```

The script warms up for 12 seconds, clears the pointer and settles for another
second, then captures/samples the closed card. It opens Preview, settles for
three seconds, then captures/samples page 1 in that same process. Each state
has 50 samples at 100 ms (~5 seconds). RSS is resident pages, not private/GPU
memory or lifetime peak. Idle CPU is a percentage of one core, calculated
from process ticks. Helpers are measured separately; none remains after
settling. Single native runs and short helper samples are noisy.

Native input checks exercise Left/Right and Previous/Next navigation, the
last-page bound, four zoom-in clicks to 2×, vertical and horizontal wheel
scrolling, zoom-out to 1.75×, Fit to 1×, Escape/backdrop/header-close dismissal
and reopening. Dark 760×520 and light 1120×760 / 760×520 long-filename cases
are also captured. At 760×960 / 2× zoom, Shift-wheel moves matching page words
266 pixels horizontally with unchanged vertical positions. See
[`native-checks.json`](native-checks.json) for inputs, capture hashes and
OCR/locator limitations. This is input/pixel/OCR screening, not visual approval.

Package values sum file bytes in the installed `linux-root` tree and full
staging tree. Full staging counts the executable twice, as in the baseline.
Compressed bytes use sorted, zero-timestamp tar with numeric owner/group 0
and `gzip -n -9`. Evidence is excluded from installed packages. No executable
or distribution archive is committed.

## Results

At 1120×760 / 100% scale, both fixtures' first pages change from 1099×1429
to 723×940: RGBA bytes fall from 6,281,884 to 2,718,480
(−3,563,404 B / −56.73%). The second vector page changes from 1076×1461
to 692×940 (−58.62% raster bytes). These are allocation/transfer dimensions,
not measured whole-app or GPU-memory savings.

Independent helper medians from the committed `helper-*.json` and
`image-*.json`:

| Ordinary target / metric | Baseline | Optimized | Delta |
| --- | ---: | ---: | ---: |
| Vector page 1 wall | 80.71 ms | 62.57 ms | −18.14 ms / −22.47% |
| Vector page 1 total CPU | 78.53 ms | 61.74 ms | −16.79 ms / −21.38% |
| Vector page 1 peak RSS | 64,152 KiB | 55,980 KiB | −8,172 KiB / −12.74% |
| Vector page 2 wall | 78.42 ms | 61.25 ms | −17.17 ms / −21.90% |
| Vector page 2 total CPU | 76.90 ms | 60.30 ms | −16.59 ms / −21.58% |
| Vector page 2 peak RSS | 64,204 KiB | 55,788 KiB | −8,416 KiB / −13.11% |
| Image page wall | 98.85 ms | 87.53 ms | −11.32 ms / −11.45% |
| Image page total CPU | 97.37 ms | 86.36 ms | −11.01 ms / −11.30% |
| Image page peak RSS | 72,596 KiB | 63,892 KiB | −8,704 KiB / −11.99% |

The maximum envelope retains the baseline raster dimensions and caps.
At that target, vector page 1/page 2 total CPU is 65.95/64.87 ms
(−16.02%/−15.65%) and peak RSS is 58,216/58,064 KiB
(−9.25%/−9.56%). The image page is 92.23 ms total CPU versus 97.37 ms
(−5.28%) and 66,708 KiB peak RSS (−8.11%). The ordinary-view comparison
includes smaller rasters; the maximum-envelope comparison retains the same
dimensions while reducing buffer-copy overhead. These short five-sample series
also include startup, scheduler and code-layout variation, so they cannot
attribute every timing delta to a particular optimization. No arbitrary-PDF,
universal HiDPI CPU improvement or UI-latency claim follows.

Matched native-process samples from the final demo binary:

| Native state / metric | Baseline | Optimized | Delta |
| --- | ---: | ---: | ---: |
| Closed card settled / peak sampled RSS | 268,608 KiB | 269,372 KiB | +764 KiB / +0.28% |
| Page 1 settled / peak sampled RSS | 281,516 KiB | 275,072 KiB | −6,444 KiB / −2.29% |
| Closed card CPU, percentage of one core | 12.57% | 9.73% | No PDF-improvement claim |
| Page 1 CPU, percentage of one core | 0.00% | 0.00% | Unchanged at tick resolution |
| Helpers after settling | None | None | No persistent renderer |

These are one matched native run per build, 50 samples per state at 100 ms,
with the warmup, renderer and viewport described above. Page RSS is lower in
this scenario; the tiny closed-card RSS difference and the closed-card CPU
difference are noisy and are not attributed to this PDF optimization.
The modal is idle at process-tick resolution on both builds. This does not
measure PDF render latency, frame/p95 latency, private memory or GPU memory.
The `*-card-samples.json` / `*-page-samples.json` files retain all raw samples.

Actual standard packages from `package-measurements.json`:

| Package metric | Baseline | Optimized | Delta |
| --- | ---: | ---: | ---: |
| Executable | 92,696,008 B | 92,700,104 B | +4,096 B / +0.0044% |
| Installed Linux tree | 97,435,353 B | 97,439,449 B | +4,096 B / +0.0042% |
| Full staged distribution | 194,515,725 B | 194,523,917 B | +8,192 B / +0.0042% |
| Full staged tar.gz | 111,018,749 B | 111,024,243 B | +5,494 B / +0.0049% |

**There is no disk-size reduction.** The changes reduce transient raster RAM
and measured helper CPU/RSS without sacrificing fonts/CMaps or resampling
compatibility. A rejected SIMD-removal trial saved only 4 KiB and was restored;
it is not used for any final measurement. The original feature's roughly
5.36-MB executable addition versus upstream remains a real cost. Installed
packages have no evidence images, measurement scripts or new image-test fixture.

Optimized standard executable SHA-256:
`35e2c5dd187c4c4523606389a8255ec24bdb69a1b8faec92786e646db7b99e01`.
Baseline standard:
`4f01ded674c1319887b2ec95e1be9cceff30850cb7ad5e5c627bacec3e2f75f8`.
See `checks.txt` and the raw JSON for fixture/binary hashes and actual
verification commands.
Final demo executable SHA-256:
`a2012f6fb643afdfd63c467ad109c1a3decc5be748cb5589a115ef3b9f6a3eb1`.
The application-source diff SHA-256 is recorded in both
`native-checks.json` and `package-measurements.json`.

## Evidence limitations

Captures are genuine, unscaled native synthetic window images, not mockups.
Native keyboard/mouse interactions, OCR and pixel/bounds screening are
recorded, but direct image inspection is unavailable because the image-viewing
tool rejected image inputs. **These checks are not visual approval; human
visual review remains necessary.**

These results do not establish arbitrary-PDF fidelity, live Discord/CDN
compatibility, whole-session memory bounds, hardware-GPU memory, screen-reader
behavior or frame/p95 latency. Windows/macOS runtime is not tested locally;
CI results are reported separately. The helper's resource limits are not an
OS sandbox or a security certification. No live Discord, authentication,
messages, calls or microphone tests were run.

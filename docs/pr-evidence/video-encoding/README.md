# Video encoding evidence for #567

This is the compact review record for FFmpeg encoding, hardware capabilities,
GPU routing and camera/screen presets. Current behavior is documented in
[Voice & Video](../../voice.md); this directory contains development evidence.

All captures and measurements below are **historical**, with their original
revision labels, source hashes and limitations. They do not validate the latest
PR head. The complete previous record remains in the
[commit-pinned archive](https://github.com/MCShotty/Serein/tree/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence).
That commit identifies the archive snapshot, not every measured revision.

## Before / after

These inspected native egui/eframe captures use offline synthetic data, not a
Discord session or physical GPU/camera/screen. Positive capability rows are
explicit fixtures. Each image is below 2 MiB. The matched dark pairs use the
same default Inter font, 1120×760 viewport and 100% scale.

| Addition | Before | After |
| --- | --- | --- |
| Backend/codec selection | ![Before video settings](../video-backend-settings/before.png) | ![Experimental and AV1](../video-backend-settings/after.png) |
| Driver queries and optional testing | ![Automatic encode scan](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence/video-driver-detection/before.png) | ![Driver support and unrun optional test](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence/video-8k-resolution/camera-before.png) |
| Camera resolution / frame rate | ![Before camera controls](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence/video-8k-resolution/camera-before.png) | ![8K and 60 fps camera controls](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence/video-8k-resolution/camera-after.png) |
| Screen resolution | ![Original screen presets](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence/video-8k-resolution/screen-before.png) | ![Screen presets through 8K](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence/video-8k-resolution/screen-after.png) |

The original backend pair starts at `65811f2`; it predates moving the controls
to the bottom of Voice & Video. The driver pair starts at
`16a09a986b73c7bf6abc3a80559ecdefb919c84a`. The 8K pairs start at
`623ef63514c1b68c516bc4b047503cead6b23511`. These illustrate separate stages,
not one before/after comparison against today's parent `main`.

The driver After and camera Before files were byte-for-byte identical; both
rows reuse the camera Before image. The camera pair is scrolled to the bottom
with Experimental/H.265; After selects 8K/60 fps. Both screen captures select
the same synthetic source and 720p/30 fps, showing the new available presets.
Sharing is disabled and no source is captured.

The retained [narrow/light camera](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence/video-8k-resolution/camera-narrow-light-after.png)
and [narrow/light screen](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence/video-8k-resolution/screen-narrow-light-after.png)
captures use 760×900 at 125% and 150% respectively. Camera controls wrap; the
screen picker body scrolls independently of its Cancel/Share footer. These
are auxiliary UI helpers excluding the production native desktop adapters.
The original capture commands, fixture definitions and verification records
remain in the archived
[backend](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence/video-backend-settings/README.md),
[driver](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence/video-driver-detection/README.md)
and [8K](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/pr-evidence/video-8k-resolution/README.md)
methods.

## Performance

[archived measurements](https://github.com/MCShotty/Serein/blob/918a4abe391a9e1619ba678af2b3f204f594cf6e/docs/pr-evidence/video-encoding/measurements.json) retains six original records without
changing their data: backend/software encoding, driver detection, 8K UI,
GPU-routing/software encoding, split-option setup and AMF split-request diagnostics.
Each record includes its
original path, file SHA-256 and immutable archive URL. Revision labels, samples,
environment, compiler/source identities and unmeasured fields are preserved.
Relative paths inside a record resolve in its archived original directory;
additional referenced build metadata and scripts remain there.

| Historical workload | Baseline | After | Delta / interpretation |
| --- | ---: | ---: | --- |
| Windows release executable, bytes | 86,008,832 | 86,554,112 | +545,280 |
| Windows installed package, bytes | 90,199,948 | 124,070,028 | +33,870,080 |
| Windows complete ZIP, bytes | 50,001,798 | 78,054,030 | +28,052,232 |
| FFmpeg software camera, 300 frames, median of five runs | 993.070 ms | 1,024.365 ms | +3.15%; overlapping ranges |
| Driver detection, median of five fresh processes | 262.167 ms | 265.354 ms | +1.216%; no hardware drivers available |
| 8K controls, sampled peak/settled UI RSS | 162,140,160 B | 163,909,632 B | +1,769,472 B; one helper pair |
| Split-option setup, median of five runs | 1,301.148 ms | 1,371.227 ms | +5.39%; overlapping ranges |

The Windows comparison is `1b3e4a7b` → `eefa0fe5`, Rust 1.98.1 on Windows 11
build 26200, Ryzen 7 7800X3D and 32 GiB RAM. Both standard releases include
voice and omit demo. Installed bytes sum every file in `dist`; ZIPs use .NET
`ZipFile.CreateFromDirectory` with Optimal compression. A stale cached xtask
omitted FFmpeg staging, so the recorded package was repaired by replaying
`packaging/ffmpeg/bundle.py`. All 241 non-executable files then matched their
expected sources. This is a portable directory/ZIP measurement, not a clean
end-to-end package rerun or an NSIS installer measurement. The complete
[Windows method and caveats](https://github.com/MCShotty/Serein/blob/faa9660761a2099cf2e6f5b748cf48bfa6871b63/docs/performance.md#windows-ffmpeg-integration-package---october-8-2026)
remain archived.

The software camera uses deterministic 640×480 RGB, 30-frame warmup and 300
timed calls, with one process warmup and five alternating pairs. Driver-query
timing uses actual detector/native code, one warmup and five alternating
fresh-process samples, with all nine NVENC/AMF/QSV codec paths unavailable.
The 8K UI helper uses software OpenGL, default camera preferences, six-second
warmup, 100 wheel events and fifteen one-second CPU/RSS samples per revision;
it performs no encoding or capture. Split setup allocates/configures/frees
60,000 HEVC/AV1 contexts at 1440p without opening encoders or drivers. Its
diagnostic FFmpeg build is not the standard distribution recipe.

These are separate workloads and recorded revisions. They establish neither
general speed/quality improvements nor current-head whole-app performance.
Helper sizes are not package sizes; sampled RSS is not a memory ceiling.
Physical 8K/60 fps, frame/startup p95, GPU memory and active hardware throughput
remain unmeasured in these records.

## Verification and reproduction

The historical records remain immutable. Current native fixtures are grouped by vendor
and purpose; separately compiled ABI/mock variants and all original assertions remain.
Fresh implementation measurements will be recorded in [performance](../../performance.md). The recorded split verification at
`faa9660` includes 464 UI tests, 140 scoped voice tests, selected strict Clippy,
native fixtures, packaging fixtures, login handoff and replay/soak. Full native
desktop/package validation is still separate from those scoped results.

Active regression fixtures remain in
[crates/discord-voice/tests/native](../../../crates/discord-voice/tests/native),
and source/license distribution remains in
[packaging/ffmpeg](../../../packaging/ffmpeg). Historical helper programs and
logs are archived rather than kept as additional active PR files. To inspect
the exact recorded methods without changing the task branch:

```sh
git worktree add --detach /tmp/serein-video-evidence faa9660761a2099cf2e6f5b748cf48bfa6871b63
```

Read that worktree's stage README and use each report's recorded revisions and
source identities. Some historical After states were uncommitted working trees
identified by hashes. The archive is not a promise that rerunning an old helper
against today's source reproduces its original measured binaries.

### Ultrawide and HDR implementation — October 11, 2026

The starting head is `c4bc650`; `206b73f` consolidates historical screenshots,
measurement archives and native fixtures before the shared media work. The
complete PR stays within 150 changed paths. Only egui-wgpu is copied from the
already pinned GUI revision, with unchanged upstream licenses.

Offline fixtures now decode the exact 6144×2560 H.264 picture through synthetic
RTP/DAVE, and decode HEVC/AV1 PQ and HLG pictures through that transport. The
ultrawide fixture carries 60 fps timestamps; it is not a sustained throughput
measurement. Attachment fixtures exercise HDR, audio draining, seeking,
cancellation and bounded anonymous reads. Actual native P010 packing tests cover
odd crops, planar/semiplanar/grayscale layouts, black levels and output guards.
The production media shader is executed with software Vulkan, including SDR,
PQ/HLG highlights, rotation, resource cleanup and device recreation.

Normal Linux FFmpeg builds enable NVENC, AMF and QSV, shared decoding and dav1d.
The cached recipe verifies its recorded inputs before reuse. Native query/preset
fixtures, source-patch reproduction, library-identity and packaging fixtures
remain offline; they do not demonstrate physical GPU throughput. Current package
checks and measurements are tracked in [performance](../../performance.md).

Current native HDR display and Discord interoperability remain owner-controlled
checks. The owner has agreed to test the CI Windows installer. Outgoing HDR
captures currently tone-map to SDR because codec selection alone does not
establish receiver 10-bit compatibility. No live account, microphone or capture
test is performed by this automation. The retained screenshots are historical,
with their original revision labels; native window capture is unavailable here.

## Source resolution and aspect-ratio regression scope

The source-resolution implementation starts at `574a8d2`. Earlier screenshots,
packages and measurements in this document retain their original revision labels.
The current work uses tests in the existing core, UI, screen capture and synthetic
transport modules: common/custom landscape and portrait ratios, physical-pixel
geometry, even alignment, capped bitrate interpolation, source refresh/removal,
disabled pointer/keyboard choices, approved crop isolation, output generations,
delayed picker heartbeats/cancellation and source-size announcements.

The exact 6144×2560 synthetic RTP/DAVE receive case remains included. Native
monitor selection, Retina/DPI changes, HDR monitor movement and hardware encoding
need owner-controlled checks. This cloud session has no native display/window
capture capability; no current native screenshot or GUI throughput/CPU/memory
measurement is claimed. Native application and clean/cached packaging checks run
in CI on the pushed revision.

## CodeRabbit review follow-up — October 12, 2026

[The full review of `a65238d`](https://github.com/ViceVerse-cz/Serein/pull/567#pullrequestreview-5489255857)
reported five findings, all verified against that source. The follow-up keeps
video-failure notices advisory when the UI queue is full, retries temporarily
unavailable macOS geometry, accepts and validates existing AV1 OBU size fields,
rebuilds attachment audio resampling when input layout/format/rate changes, and
corrects the packaged-library count. No finding was skipped.

Existing test files cover saturated/disconnected notice queues, AV1 aggregation
and fragmented lengths, malformed length recovery, and real native audio frames
changing channels, layout, sample format and rate. Output guards and resampler
cleanup are checked alongside the existing HDR packing and vendor preset fixtures.
Pinned Rust 1.98.1 `cargo xtask check` passes with 1,388 workspace tests, strict
Clippy, formatting, the minimal application build and policy checks. Native media
fixtures, FFmpeg packaging regressions, login handoff, cached xtask selection and
the offline reducer replay pass. The production HDR shader and 120-second soak
remain separately labeled checks from `a65238d`; neither path changed in these fixes.
The macOS minimized-window scenario requires a native owner-controlled check;
the geometry retry does not enumerate new sources or replace the approved filter.
Comparable native-helper measurements are recorded in
[performance](../../performance.md); they exclude GUI, decode and device playback.

[The follow-up review of `5b8ae38`](https://github.com/ViceVerse-cz/Serein/pull/567#pullrequestreview-5489559654)
found an additional Linux release-build dependency omission. The native dependency
installer now includes Meson and Ninja on Ubuntu, Fedora, openSUSE and Arch, as
required by the pinned dav1d recipe. Shell syntax and mocked package-manager runs
cover every supported distribution/version and reject unsupported hosts without
installing anything. Application code is unchanged from the checked `5b8ae38`.

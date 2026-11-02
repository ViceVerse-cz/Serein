# Shared output-format selection evidence

This is fresh synthetic/offline evidence for the output selector. Baseline is upstream
`main` at `38e46b8f5ec694002ef515fbc2d8dc69297e4f5f`, with a clean worktree before edits.
No historical runtime results are reused. No host, stream, microphone, network connection,
credential store or Discord session is opened by the probe.

## Reproduction

Use the repository's pinned Rust 1.98.1 toolchain on Linux x86_64. At each revision,
run `cargo xtask package`; on this Arch host its Debian stage fails because `dpkg` is
absent. `cargo xtask package --format arch` builds the native unsigned package and
checks its metadata, library closure, payload contents, modes, owners and lack of scripts.
Preserve the two `dist` outputs separately. Executable size is `dist/serein`'s byte length;
installed size sums regular files beneath `usr/` in the inspected native archive;
compressed size is the complete `.pkg.tar.zst` archive. Archive metadata/timestamps can vary.

After the release build, locate its `target/release/deps/libcpal-*.rlib` and run:

```sh
python3 docs/pr-evidence/shared-output-format-selection/selection_probe.py \
  --revision 38e46b8f5ec694002ef515fbc2d8dc69297e4f5f \
  --cpal-lib target/release/deps/libcpal-<hash>.rlib
python3 docs/pr-evidence/shared-output-format-selection/selection_probe.py \
  --revision WORKTREE \
  --cpal-lib target/release/deps/libcpal-<hash>.rlib
```

The probe compiles the selected revision's selector with `rustc -O -C lto=thin`, using
CPAL from the locked release build. It replaces only the device type/queries with synthetic
fixtures. The baseline notification gate mirrors the original supported stream match;
the after probe uses the shared selector source. Both run one warmup and five measured
runs of 1,000,000 selections per workload, using `black_box` inputs/results.

The default is stereo, 44.1 kHz, either F32 or unsupported I24. The 64 advertised ranges
contain 63 unsupported I24 ranges followed by one stereo F32 range supporting 8–192 kHz.
Voice and notification recovery select 48 kHz. The original notification path rejects
I24 before considering any alternatives. Measurements time only configuration selection,
excluding OS queries, decoding, stream creation and audio callbacks. Tiny optimized default
paths approach timer/optimizer limits. Results are workload-specific; no audible latency,
process CPU/RSS, frame timing or live Discord performance claim follows from them.

## Validation boundaries

The four shared-selector regression tests cover unsupported/missing defaults with usable
alternatives, stereo/surround/mono preference, preserved usable defaults, voice fallback,
Windows native-mix selection without enumeration, and format/channel/rate/enumeration bounds.
Existing notification callback/decoder tests exercise stereo content, silent extra channels,
mono downmix, draining, request-local cancellation and bounded cue decoding.

Windows selection policy is simulated on Linux. Native WASAPI, macOS and physical I24
playback remain unvalidated. To check physical reproduction on a non-Windows system, select
a system-default output with an I24 default plus an advertised F32/I16/I32/U16 alternative,
then preview a notification sound in Settings without joining a call. Expected: the preview
plays through the system default, with the notification volume setting. This physical-device
procedure was not performed. There is no visible UI change, so screenshots do not apply.

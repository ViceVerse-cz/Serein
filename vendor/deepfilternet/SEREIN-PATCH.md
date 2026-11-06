# DeepFilterNet real-time runtime and DFN3 model

Source: [DeepFilterNet Plus](https://github.com/ismailivanov/DeepFilterNetPlus) at commit
`2dc5273d16e1e3769527ad0740654c5ccde581cc` (`main`, September 16, 2026; crate
`deep_filter` 0.5.7-pre in `libDF`). DeepFilterNet Plus is a maintained fork of
[Rikorose/DeepFilterNet](https://github.com/Rikorose/DeepFilterNet), whose last
runtime change is `d375b2d8309e0935d165700c91da9de862a99c31` (October 17, 2024). The
directory layout mirrors upstream so `tract.rs` finds the model at `../../models/`.

crates.io publishes `deep_filter` only up to 0.2.5 (July 2022), which has neither the
tract inference runtime nor a bundled model. A git dependency would lock the fork's
training-only dependencies as well (an HDF5 git dependency, dataset loaders, audio
decoders, a CLI, WebAssembly and LADSPA bindings) and fetch about 85 MB of unused models.
The workspace therefore patches `deep_filter` to this directory.

## Changes in the fork that Serein uses

Relative to `d375b2d`, the fork's `lib.rs` and `tract.rs`:

- build against current tract (the 0.21.6 symbol and 0.21.7 ndarray 0.16 APIs) and read
  the local SNR without `to_scalar()`, which tract 0.21.5+ rejects for multi-channel
  models (Serein runs one channel);
- remove the per-frame heap allocations from `DfTract::process` and the STFT
  analysis/synthesis buffers;
- process every frame, including digital silence, instead of zeroing the output after
  more than five near-silent frames, and guard the band normalization against the 0/0
  that sustained silence would then produce;
- make a repeated `init()` clear the noisy-spectrum ring instead of doubling it;
- log the clipping warning once per episode instead of every 10 ms.

`transforms.rs` and `logging.rs` are unchanged from `d375b2d`, as are the license files
and the DeepFilterNet3 model.

## What is here

- `libDF/src/lib.rs`, `transforms.rs`, `logging.rs`: copied unchanged; they match the Git
  blobs of the commit above.
- `libDF/src/tract.rs`: the fork's file plus the Serein change below.
- `libDF/LICENSE`, `LICENSE-MIT`, `LICENSE-APACHE`: copied unchanged (MIT OR Apache-2.0).
- `libDF/Cargo.toml.orig`: the unchanged fork manifest.
- `models/DeepFilterNet3_onnx.tar.gz`: the unchanged DeepFilterNet3 ONNX export
  (7,983,136 bytes, SHA-256
  `c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616`), covered by the
  same repository license. It is embedded in the executable; nothing is downloaded.

Omitted files: the dataset/augmentation/HDF5 modules, `capi.rs`, `wasm.rs`, `util.rs`,
`wav_utils.rs`, the binaries, the Python packages, the LADSPA plugin, the demo and every
other model archive. The features that would compile them no longer exist.

## Serein change to `tract.rs`

`DfTract` holds tract `TValue` buffers (`Rc`), so it cannot be built on one thread and
used on another. Serein splits its constructor so the slow part can run off the audio
worker:

- `DfModels::new(DfParams, &RuntimeParams)` decodes, pulses, optimizes and plans the three
  networks. It holds only the model configuration and tract plans, which are `Send`.
- `DfTract::from_models(DfModels, &RuntimeParams)` builds the runtime state from them.
- `DfTract::new` is now `from_models(DfModels::new(..)?, ..)`; its behavior is unchanged.

The rest of the file is the fork's code. The change is marked `Serein:` in the source.

## Manifest changes

`libDF/Cargo.toml` is otherwise the only modified file:

- Only the `tract`, `transforms`, `logging` and `default-model` features and their
  dependencies remain. Serein enables `tract` and `default-model`.
- `crate-type` no longer requests `cdylib`/`staticlib`; Serein links the Rust library.
- The tract crates require `^0.22.5` instead of the fork's `^0.21.15`. Every tract
  release from 0.21.11 through 0.21.19 pins `half = "=2.4.1"`, which cannot coexist with
  the `half ^2.5` that wgpu's naga requires. tract 0.22 accepts `half >=2.4, <3.0` and
  keeps the 0.21 API this source uses; the source compiles against it unchanged.
- `repository` points at the fork; `license` uses the SPDX `OR` form; `readme`, binaries
  and dev-dependencies are dropped.
- A `[lints]` table silences rustc and Clippy warnings from the third-party source.

tract 0.21.4, used by the first version of this integration, rejected this model's node
names in builds with debug assertions. tract 0.22.5 does not, so the workspace no longer
disables debug assertions for `tract-core`.

## Maintenance

Remove this directory when a crates.io `deep_filter` release ships the tract runtime and
a default model. On any update, re-copy the listed files from one fork commit, re-apply
and re-check the `DfModels` split, record the commit here, re-verify the model hash, and
run `cargo test -p discord-voice deepfilternet` and
`cargo run -p discord-voice --example echo`. Those checks are synthetic; they are not
evidence of call quality, device behavior or live Discord compatibility.

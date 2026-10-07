# DeepFilterNet real-time runtime and DFN3 model

Source: <https://github.com/Rikorose/DeepFilterNet> at commit
`d375b2d8309e0935d165700c91da9de862a99c31` (`main`, October 17, 2024; crate
`deep_filter` 0.5.7-pre in `libDF`). The directory layout mirrors upstream so the
unmodified `tract.rs` finds the model at `../../models/`.

crates.io publishes `deep_filter` only up to 0.2.5 (July 2022), which has neither the
tract inference runtime nor a bundled model. A git dependency on the upstream repository
would lock its training-only dependencies as well (an HDF5 git dependency, dataset
loaders, audio decoders, a CLI, WebAssembly bindings) and fetch about 85 MB of unused
models. The workspace therefore patches `deep_filter` to this directory.

## What is here

- `libDF/src/lib.rs`, `tract.rs`, `transforms.rs`, `logging.rs`: copied unchanged.
  They match Git blobs of the commit above; no Rust source is modified.
- `libDF/LICENSE`, `LICENSE-MIT`, `LICENSE-APACHE`: copied unchanged (MIT OR Apache-2.0).
- `libDF/Cargo.toml.orig`: the unchanged upstream manifest.
- `models/DeepFilterNet3_onnx.tar.gz`: the unchanged upstream DeepFilterNet3 ONNX export
  (7,983,136 bytes, SHA-256
  `c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616`), covered by the
  same repository license. It is embedded in the executable; nothing is downloaded.

Omitted upstream files: the dataset/augmentation/HDF5 modules, `capi.rs`, `wasm.rs`,
`util.rs`, `wav_utils.rs`, the binaries, the Python packages, the LADSPA plugin, the demo
and every other model archive. The features that would compile them no longer exist.

## Manifest changes

`libDF/Cargo.toml` is the only modified file:

- Only the `tract`, `transforms`, `logging` and `default-model` features and their
  dependencies remain. Serein enables `tract` and `default-model`.
- `crate-type` no longer requests `cdylib`/`staticlib`; Serein links the Rust library.
- The four tract crates are pinned to `=0.21.4`, the version in upstream's lockfile.
  The upstream requirement `^0.21.4` now resolves to 0.21.10, whose `symbol_table` and
  ndarray 0.16 changes do not compile with this source (0.21.6 fails the same way).
- A `[lints]` table silences rustc and Clippy warnings from the unmodified source.
- `license` uses the SPDX `OR` form; `readme`, binaries and dev-dependencies are dropped.

tract 0.21.4 runs a graph-name check in builds with debug assertions which rejects
duplicate node names produced by its own optimizer for this model. The workspace disables
debug assertions for `tract-core` in the dev profile; release builds never ran that check.

## Maintenance

Remove this directory when a crates.io `deep_filter` release ships the tract runtime and
a default model. On any update, re-copy the listed files from one upstream commit, record
it here, re-verify the model hash, and run `cargo test -p discord-voice deepfilternet`
and `cargo run -p discord-voice --example echo`. Those checks are synthetic; they are not
evidence of call quality, device behavior or live Discord compatibility.

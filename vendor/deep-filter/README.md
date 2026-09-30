# Embedded DeepFilterNet3 runtime

This is a narrowly vendored Rust inference runtime from
[Rikorose/DeepFilterNet](https://github.com/Rikorose/DeepFilterNet), maintained for Serein.
See [SEREIN-PATCH.md](SEREIN-PATCH.md) for the exact source and local changes.

The embedded standard DeepFilterNet3 model, pre-converted from upstream's ONNX release
to Tract NNEF by `tools/deep-filter-model`, accepts normalized mono 48 kHz audio,
480 samples per call (10 ms). Its 960-sample STFT and two-frame model lookahead
add 30 ms of algorithmic delay. This is speech noise suppression, not an input
activity gate. No Python, GPU, network download, or external model file is needed.

Create and run the processor on a media worker. It performs bounded inference
allocations and is unsuitable for device callbacks. `DfTract::reset()` discards
capture history without rebuilding or cloning the inference graphs. Call it
when a microphone capture generation ends, including mute and push-to-talk
transitions. Initialization and first inference can be expensive; measure them
separately from warmed steady-state frames.

`RuntimeParams::with_atten_lim()` / `DfTract::set_atten_lim()` set the maximum
attenuation in dB: 0 bypasses reduction, 100 removes the attenuation limit.
Use a nonzero deterministic signal when measuring performance; silence can
skip inference. Only the embedded, reviewed model is accepted by `DfParams`.

Upstream source is dual licensed MIT or Apache-2.0; complete license texts are
included. The model is shipped in the upstream repository under its root
license, without a separate model-specific license file. Upstream's explicit
clarification of model-weight redistribution remains unanswered; see provenance.

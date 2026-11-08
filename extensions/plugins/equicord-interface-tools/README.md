# Equicord interface tools — audit only

This directory records the audit of eleven upstream Equicord workflows in the
interface, tags, effects and notification area. **It deliberately contains no
manifest, Rust source or packaged Wasm:** every assigned workflow is either
already implemented by an existing Serein surface or needs a host/native
integration that this package cannot safely provide.

Read [WAVE.md](WAVE.md) for the per-path evidence, the exact omissions and the
parent integration steps.

Upstream: `Equicord/Equicord` at
`200dba124066146a8f043fde7fc8b05e3dc5e68e`.

This note is not a claim of upstream JavaScript parity, complete plugin
coverage, or live Discord interoperability.

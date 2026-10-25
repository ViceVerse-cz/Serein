# Native OpenMLS dependency selection and crypto migration

Source: crates.io `davey` 0.1.4, upstream git `a1e2e741bea06bc3b7167a5c3792844b8975993c`, crate directory `davey` in <https://github.com/Snazzah/davey>. Original archive SHA-256: `25028cc2ec8cd43138ca0d2c71d7f6019196238ebd6edea11dd35276fa85b860`.

The native manifest continues to omit OpenMLS's browser-only `js` timer feature.
The October 25, 2026 security migration additionally selects P-256 0.14 and the
coordinated OpenMLS components pinned by the root manifest to
`be248d4b1306d87a641ce24ed0ffcab2b27967f6`: OpenMLS 0.9.0 snapshot and
credential/RustCrypto providers 0.6.0. These are Git sources, not the similarly
numbered registry releases. Published parent dependencies still select
crypto-bigint 0.5; the pin moves all selected curve paths to crypto-bigint 0.7.7,
addressing RUSTSEC-2026-0387 (fixed >=0.7.6).

Three source adaptations are required:

- P-256 signing keys use `Generate::generate_from_rng`, getrandom's OS `SysRng`
  with rand_core 0.10 `UnwrapErr`, and `to_sec1_point(false)`. The unused old
  `rand` dependency is removed. Random-source failures continue to fail closed.
- OpenMLS now reports `ProcessedMessageContent::OwnPendingCommit` instead of
  `StageCommitError::OwnCommit`; merge the pending commit in that successful
  message branch.
- The exported secret now has an owning zeroizing wrapper. Copy its bytes into
  the existing hash-ratchet API, as the old API returned an owned byte vector.

No DAVE protocol version, ciphersuite, ephemeral identity policy, media bounds
or transport setting changes. The workspace retains `tracing/max_level_off`
for this stack. No advisory is suppressed. This native fork does not support
WebAssembly or claim validation of Davey's optional Python/Node bindings.

`Cargo.toml.orig` and `.cargo_vcs_info.json` retain the original upstream
provenance. Modified files are `Cargo.toml`, `src/signing_key_pair.rs` and
`src/session.rs`. The upstream crate's lockfile and registry marker are omitted;
the workspace lockfile controls resolution. The MIT license comes from the
exact upstream revision because the published archive omits it:
<https://github.com/Snazzah/davey/blob/a1e2e741bea06bc3b7167a5c3792844b8975993c/LICENSE>.

Validation includes the workspace DAVE/MLS transition and encryption tests,
a signed legacy key-package regression and the offline cross-version peers in
`crates/discord-voice/tests/fixtures/legacy-dave`. They cover both creator/joiner
roles, matching epochs/privacy codes, encrypted packets in both directions and
tamper/replay rejection. This is neither an independent cryptographic audit nor
proof of live Discord compatibility. Replace the Git pin and local adaptations
when a compatible released Davey/OpenMLS stack removes the affected paths;
review all components together rather than relaxing the audit policy.

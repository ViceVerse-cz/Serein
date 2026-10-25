# Native HPKE dependency selection

Source: crates.io **hpke-rs 0.8.0**, upstream commit
`42d68bd49b244b09bd02626c6ffa95e54dc64f91`, directory
`crates/protocols/hpke` in <https://github.com/celabshq/libcrux>.
Original archive SHA-256:
`deb74477a6c7f6e8e9c68de06fe928db79b3b8b9e04c900bfb729bf096df7d22`.
`Cargo.toml.orig` and `.cargo_vcs_info.json` preserve release provenance.
The release lockfile, registry marker, publish helper and Libcrux-only
post-quantum benchmark are omitted; workspace Cargo.lock controls resolution.
Native packages include the manifests, provenance/license documents, `src` and
`benches`; archival upstream CI configuration is not shipped.

The October 25, 2026 migration updates the whole vendored component from 0.6.1
to 0.8.0, matching the pinned OpenMLS RustCrypto provider's HPKE dependency.
Its RustCrypto provider 0.8.0 uses curve 0.14/crypto-bigint 0.7 instead of the
vulnerable 0.5 path. The existing native selection patch is carried forward:

- Replace unconditional `libcrux-sha3 ^0.0.11` with RustCrypto `sha3 =0.10.9`.
  Route the two fixed-size SHAKE256 calls in `src/kem.rs` through the existing
  standard XOF adapter in `src/serein_sha3.rs`.
- Remove unused optional/dev `hpke-rs-libcrux`, its feature/re-export and the
  Libcrux-only post-quantum feature/benchmark. Otherwise its unused tooling
  retains the unmaintained proc-macro-error2 path in the all-target lockfile.
  The native RustCrypto provider, KDF tests, AEAD tests, examples and benchmarks
  remain; duplicate Libcrux-only cases are removed. The local seed-derivation
  draft feature remains available without a backend dependency.

The fork does not expose the optional `libcrux` or `draft-ietf-hpke-pq` features.
`Readme.md` documents that selection. No HPKE algorithm or wire-format change is
made by the local SHAKE adapter. No advisory is suppressed or package renamed to
hide a finding. Reassess this selection when a compatible upstream release no
longer retains the unused backend/tooling in the audited graph.

All retained and modified component source remains **MPL-2.0**. The release
archive omits a standalone license file; `LICENSE-MPL-2.0.txt` is the unmodified
canonical Mozilla text, not an invented upstream file. Binary distributions
include this complete corresponding native component source under
`source/hpke-rs`, plus the exact upstream MPL RustCrypto-provider/traits archives
under the dependency license bundle. Distributors must retain notices and offer
corresponding source including modifications as required by MPL-2.0; license
text alone is insufficient.

Validation uses the exact vendored adapter's independent 32/64-byte SHAKE output
vectors, workspace DAVE/MLS transitions and bidirectional encrypted packets,
plus old/new offline compatibility peers. Synthetic fixtures do not establish
constant-time behavior, live Discord interoperability or an independent
cryptographic security assessment.

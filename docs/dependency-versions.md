# Resolved dependencies

September 13 Windows camera selection: the existing `windows 0.61.3` bindings
also enable DirectShow, COM StructuredStorage, Variant, GDI and Ole for virtual
camera enumeration and capture. No package versions or new dependencies are
introduced; existing Windows binding licenses/notices apply.

September 12 egui refresh: `egui` and `eframe` now pin upstream `main` commit
`441971a776322a482e371775219380eca812cfa9`, verified with `git ls-remote`.
The new `ContainerAtom` API lays out the server-menu title and chevron inside
its existing button without adding another interactive response. This replaces
manual text/icon positioning and reserves the icon width when truncating titles.
The workspace remains on Rust 1.98.1 with the same egui feature flags and
transitive versions. Upstream license texts and bundled font files are unchanged.

September 12 community extensions: Wasmi is pinned to 2.0.0 with only `std`,
`validate`, `stable` and `portable-dispatch`. No WASI implementation is linked.
WAT 1.245.1 is test-only for executable adversarial sandbox fixtures. The
application exposes a versioned JSON/Wasm ABI; no JavaScript or web UI runtime
is added. See `docs/extensions.md` for lifecycle and package trust boundaries.

Voice is now included in every desktop build without a feature flag. Dated dependency-selection notes below describe their original snapshots.

September 12 camera capture: Windows reuses the locked `windows 0.61.3` and
`windows-core 0.61.2` bindings with Media Foundation/COM features. Linux uses the
locked `libc 0.2.189` V4L2 interface and `image 0.25.10` JPEG decoder. No package
versions, codec binaries or helper processes were added; existing dependency
notices apply. Camera encoding reuses `openh264 0.9.8` on all three platforms.

September 12 link-opening fix: enable eframe's `links` feature, adding webbrowser 1.2.4
(MIT OR Apache-2.0) to launch the default browser. No existing dependency versions change.
The registry release's MIT license is bundled in `assets/licenses/files/webbrowser-1.2.4-LICENSE-MIT`.

September 10 egui main experiment: `egui` and `eframe` are pinned to upstream
`65e7db3c06d779c60ac56647bdd3011ed8ba1cbd`, the `main` tip returned by
`git ls-remote https://github.com/emilk/egui.git refs/heads/main` during this task.
All ten locked egui ecosystem crates use that Git revision (still version 0.36.2).
The required transitive changes are glifo 0.2.0 → 0.3.0, vello_common/vello_cpu
0.1.0 → 0.2.0, and new unicode-properties 0.1.4; each declares MIT/Apache-2.0.
Rust 1.98.1 is retained. Eframe now enables `system_fonts`, including epaint
`color_fonts`, to resolve native Unicode emoji in ordinary labels and editors.
Other existing feature flags are retained. The four bundled font files
are byte-identical to registry epaint_default_fonts 0.36.2; its combined font
license and exact-version exception remain applicable. This is an experimental
source pin, not a released egui upgrade. UI IDs use the equivalent `Id::unique`
constructor, and the font coverage test reads `FontData::bytes()` for the new
shared font representation. Sidebar state reset uses the parent-scoped panel ID
required by this revision, preserving saved widths across viewport constraints.
Native fallback adds egui_system_fonts 0.36.2, fontique 0.11.1, parlance 0.1.0,
poll-promise 0.3.0, sys-locale 0.3.2 and unicode-script 0.5.8 (MIT OR Apache-2.0);
macOS adds objc2-core-text 0.3.2 (Zlib OR Apache-2.0 OR MIT). Linux resolution
also adds roxmltree 0.20.0 (MIT OR Apache-2.0) and
yeslogic-fontconfig-sys 6.0.1 (MIT), using the system fontconfig library.


September 10 Linux migration: current Cargo.lock has 726 packages. GTK4 0.11.4, WebKit6 0.6.1,
JavaScriptCore6 0.6.0, GLib 0.22.9 and Soup3 0.9.0 replace the earlier GTK3/WebKit2GTK family.
Wry 0.57.0 is a Windows/macOS-only local patch. Earlier inventory/addenda below are historical.
Notices/provenance are in assets/licenses/login; the strict audit now passes without ignores.

September 10 native dependency hardening: the historical macOS table below predates this patch.
Cargo.lock now has 738 packages, down from 775 at e4ef4a8. The unused hpke-rs-libcrux/libcrux/hax
branch and Davey's OpenMLS browser timer branch (including instant) are removed. The table's
entries for those removed packages no longer describe current resolution. Davey 0.1.4 is now a
local MIT-licensed manifest-only patch; all its Rust source files are unchanged. Existing
hpke-rs 0.6.1 remains the documented MPL fork with RustCrypto. No remaining version changes.
See dependency-audit.md and both vendor SEREIN-PATCH.md files for provenance and the remaining
strict-audit failure: zero vulnerability-class findings, two denied Linux warnings.


September 10 pin pagination addendum: `discord-protocol` enables the existing `time 0.3.55` formatting feature to serialize validated pin cursors as RFC3339 with nanosecond precision. Cargo.lock and package versions are unchanged; no new dependency or license text is introduced. Text and optional voice packages are checked separately.

September 10 upload feature addendum: the historical macOS inventory below is not regenerated by this change. Enabling reqwest 0.13.5 `stream` and Tokio `fs` adds native **tokio-util 0.7.19 (MIT)** and uses the existing futures-util 0.3.34. The lockfile also adds **wasm-streams 0.5.0**, referenced exclusively by reqwest's wasm32 target declaration; it is not part of the supported native package builds. These facts were checked against Cargo.lock's diff and downloaded reqwest/tokio-util manifests, rather than a new cross-platform metadata run. No existing package version changed for upload streaming.

The tokio-util license is copied unmodified from its 0.7.19 registry archive into `assets/licenses/files/tokio-util-LICENSE-MIT`, SHA-256 `253cd04c6714889df2d32f3f64d669179a1c95c76ac43c40882c52eb06bc3552`. Cargo.lock records archive checksum `494815d09bf52b5548659851081238f0ca39ff638363907596da739561c62c52`. This addendum does not change the previously documented security-audit outcome or complete the remaining transitive distribution review.

Rust toolchain: 1.98.1, edition 2024. Host resolution: aarch64-apple-darwin. Cargo.lock is the reproducible dependency input. Generated from `cargo metadata --locked --all-features --filter-platform aarch64-apple-darwin`; this workspace inventory includes optional voice and development dependencies, rather than representing the default text executable. Other platforms add native dependencies. Voice uses Davey/OpenMLS with RustCrypto, static bundled libopus, and CPAL. Eframe persistence and the REST cookie jar remain disabled.

| Crate | Version | Declared license |
|---|---|---|
| accesskit | 0.24.1 | MIT OR Apache-2.0 |
| accesskit_consumer | 0.38.0 | MIT OR Apache-2.0 |
| accesskit_macos | 0.26.3 | MIT OR Apache-2.0 |
| accesskit_winit | 0.32.2 | Apache-2.0 |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| aead | 0.5.2 | MIT OR Apache-2.0 |
| aes | 0.8.4 | MIT OR Apache-2.0 |
| aes-gcm | 0.10.3 | Apache-2.0 OR MIT |
| ahash | 0.8.12 | MIT OR Apache-2.0 |
| allocator-api2 | 0.2.21 | MIT OR Apache-2.0 |
| apple-native-keyring-store | 1.0.2 | MIT OR Apache-2.0 |
| arboard | 3.6.1 | MIT OR Apache-2.0 |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 |
| ash | 0.38.0+1.3.281 | MIT OR Apache-2.0 |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT |
| autocfg | 1.5.1 | Apache-2.0 OR MIT |
| aws-lc-rs | 1.18.1 | ISC AND (Apache-2.0 OR ISC) |
| aws-lc-sys | 0.45.0 | ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0) |
| base16ct | 0.2.0 | Apache-2.0 OR MIT |
| base64 | 0.22.1 | MIT OR Apache-2.0 |
| base64 | 0.23.1 | MIT OR Apache-2.0 |
| base64ct | 1.8.3 | Apache-2.0 OR MIT |
| bit-set | 0.10.0 | Apache-2.0 OR MIT |
| bit-vec | 0.9.1 | Apache-2.0 OR MIT |
| bitflags | 1.3.2 | MIT/Apache-2.0 |
| bitflags | 2.13.1 | MIT OR Apache-2.0 |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 |
| block-buffer | 0.12.1 | MIT OR Apache-2.0 |
| block2 | 0.5.1 | MIT |
| block2 | 0.6.2 | MIT |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| bytemuck_derive | 1.12.0 | Zlib OR Apache-2.0 OR MIT |
| byteorder-lite | 0.1.0 | Unlicense OR MIT |
| bytes | 1.12.1 | MIT |
| cc | 1.4.5 | MIT OR Apache-2.0 |
| cfg-if | 1.0.4 | MIT OR Apache-2.0 |
| cfg_aliases | 0.2.2 | MIT |
| cgl | 0.3.2 | MIT / Apache-2.0 |
| chacha20 | 0.10.2 | MIT OR Apache-2.0 |
| chacha20 | 0.9.1 | Apache-2.0 OR MIT |
| chacha20poly1305 | 0.10.1 | Apache-2.0 OR MIT |
| cipher | 0.4.4 | MIT OR Apache-2.0 |
| cmake | 0.1.58 | MIT OR Apache-2.0 |
| cmov | 0.5.4 | Apache-2.0 OR MIT |
| codespan-reporting | 0.13.1 | Apache-2.0 |
| color | 0.3.3 | Apache-2.0 OR MIT |
| const-oid | 0.10.2 | Apache-2.0 OR MIT |
| const-oid | 0.9.6 | Apache-2.0 OR MIT |
| cookie | 0.18.2 | MIT OR Apache-2.0 |
| core-foundation | 0.10.1 | MIT OR Apache-2.0 |
| core-foundation | 0.9.4 | MIT OR Apache-2.0 |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 |
| core-graphics | 0.23.2 | MIT OR Apache-2.0 |
| core-graphics-types | 0.1.3 | MIT OR Apache-2.0 |
| coreaudio-rs | 0.14.2 | MIT/Apache-2.0 |
| cpal | 0.18.2 | Apache-2.0 |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 |
| cpufeatures | 0.3.1 | MIT OR Apache-2.0 |
| crc32fast | 1.5.1 | MIT OR Apache-2.0 |
| crossbeam-deque | 0.8.8 | MIT OR Apache-2.0 |
| crossbeam-epoch | 0.9.21 | MIT OR Apache-2.0 |
| crossbeam-utils | 0.8.23 | MIT OR Apache-2.0 |
| crypto-bigint | 0.5.5 | Apache-2.0 OR MIT |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 |
| crypto-common | 0.2.2 | MIT OR Apache-2.0 |
| ctr | 0.9.2 | MIT OR Apache-2.0 |
| ctutils | 0.4.2 | Apache-2.0 OR MIT |
| cursor-icon | 1.2.0 | MIT OR Apache-2.0 OR Zlib |
| curve25519-dalek | 4.1.3 | BSD-3-Clause |
| dasp_sample | 0.11.0 | MIT OR Apache-2.0 |
| data-encoding | 2.11.1 | MIT |
| davey | 0.1.4 | MIT |
| der | 0.7.10 | Apache-2.0 OR MIT |
| deranged | 0.5.8 | MIT OR Apache-2.0 |
| digest | 0.10.7 | MIT OR Apache-2.0 |
| digest | 0.11.3 | MIT OR Apache-2.0 |
| dirs | 7.0.0 | MIT OR Apache-2.0 |
| dirs-sys | 0.5.0 | MIT OR Apache-2.0 |
| dispatch | 0.2.0 | MIT |
| dispatch2 | 0.3.1 | Zlib OR Apache-2.0 OR MIT |
| displaydoc | 0.2.7 | MIT OR Apache-2.0 |
| dlib | 0.5.3 | MIT |
| document-features | 0.2.12 | MIT OR Apache-2.0 |
| dpi | 0.1.2 | Apache-2.0 AND MIT |
| dunce | 1.0.5 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| ecdsa | 0.16.9 | Apache-2.0 OR MIT |
| ecolor | 0.36.2 | MIT OR Apache-2.0 |
| ed25519 | 2.2.3 | Apache-2.0 OR MIT |
| ed25519-dalek | 2.2.0 | BSD-3-Clause |
| eframe | 0.36.2 | MIT OR Apache-2.0 |
| egui | 0.36.2 | MIT OR Apache-2.0 |
| egui-wgpu | 0.36.2 | MIT OR Apache-2.0 |
| egui-winit | 0.36.2 | MIT OR Apache-2.0 |
| egui_glow | 0.36.2 | MIT OR Apache-2.0 |
| either | 1.18.0 | MIT OR Apache-2.0 |
| elliptic-curve | 0.13.8 | Apache-2.0 OR MIT |
| emath | 0.36.2 | MIT OR Apache-2.0 |
| epaint | 0.36.2 | MIT OR Apache-2.0 |
| epaint_default_fonts | 0.36.2 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 |
| equivalent | 1.0.2 | Apache-2.0 OR MIT |
| euclid | 0.22.14 | MIT OR Apache-2.0 |
| fallible-iterator | 0.3.0 | MIT/Apache-2.0 |
| fallible-streaming-iterator | 0.1.9 | MIT/Apache-2.0 |
| fastrand | 2.5.0 | Apache-2.0 OR MIT |
| fax | 0.2.7 | MIT |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 |
| fearless_simd | 0.4.1 | Apache-2.0 OR MIT |
| ff | 0.13.1 | MIT/Apache-2.0 |
| find-msvc-tools | 0.1.12 | MIT OR Apache-2.0 |
| flate2 | 1.1.10 | MIT OR Apache-2.0 |
| fluvio-wasm-timer | 0.2.5 | MIT |
| foldhash | 0.2.0 | Zlib |
| font-types | 0.12.5 | MIT OR Apache-2.0 |
| foreign-types | 0.5.0 | MIT/Apache-2.0 |
| foreign-types-macros | 0.2.4 | MIT/Apache-2.0 |
| foreign-types-shared | 0.3.1 | MIT/Apache-2.0 |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 |
| fs_extra | 1.3.0 | MIT |
| futures | 0.3.34 | MIT OR Apache-2.0 |
| futures-channel | 0.3.34 | MIT OR Apache-2.0 |
| futures-core | 0.3.34 | MIT OR Apache-2.0 |
| futures-executor | 0.3.34 | MIT OR Apache-2.0 |
| futures-io | 0.3.34 | MIT OR Apache-2.0 |
| futures-lite | 2.6.1 | Apache-2.0 OR MIT |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 |
| futures-sink | 0.3.34 | MIT OR Apache-2.0 |
| futures-task | 0.3.34 | MIT OR Apache-2.0 |
| futures-timer | 3.0.4 | MIT/Apache-2.0 |
| futures-util | 0.3.34 | MIT OR Apache-2.0 |
| generic-array | 0.14.7 | MIT |
| getrandom | 0.2.17 | MIT OR Apache-2.0 |
| getrandom | 0.3.4 | MIT OR Apache-2.0 |
| getrandom | 0.4.3 | MIT OR Apache-2.0 |
| ghash | 0.5.1 | Apache-2.0 OR MIT |
| glifo | 0.2.0 | Apache-2.0 OR MIT |
| glow | 0.17.0 | MIT OR Apache-2.0 OR Zlib |
| glutin | 0.32.3 | Apache-2.0 |
| glutin-winit | 0.5.0 | MIT |
| gpu-allocator | 0.28.0 | MIT OR Apache-2.0 |
| group | 0.13.0 | MIT/Apache-2.0 |
| guillotiere | 0.7.0 | MIT/Apache-2.0 |
| half | 2.7.1 | MIT OR Apache-2.0 |
| harfrust | 0.12.0 | MIT |
| hashbrown | 0.16.1 | MIT OR Apache-2.0 |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 |
| hax-lib | 0.3.6 | Apache-2.0 |
| hax-lib-macros | 0.3.6 | Apache-2.0 |
| hex-literal | 0.3.4 | MIT OR Apache-2.0 |
| hkdf | 0.12.4 | MIT OR Apache-2.0 |
| hmac | 0.12.1 | MIT OR Apache-2.0 |
| hpke-rs | 0.6.1 | MPL-2.0 |
| hpke-rs-crypto | 0.6.1 | MPL-2.0 |
| hpke-rs-libcrux | 0.6.1 | MPL-2.0 |
| hpke-rs-rust-crypto | 0.6.1 | MPL-2.0 |
| http | 1.5.0 | MIT OR Apache-2.0 |
| http-body | 1.1.0 | MIT |
| http-body-util | 0.1.5 | MIT |
| httparse | 1.10.1 | MIT OR Apache-2.0 |
| hybrid-array | 0.4.15 | MIT OR Apache-2.0 |
| hyper | 1.11.1 | MIT |
| hyper-rustls | 0.27.9 | Apache-2.0 OR ISC OR MIT |
| hyper-util | 0.1.20 | MIT |
| icu_collections | 2.3.0 | Unicode-3.0 |
| icu_locale_core | 2.3.0 | Unicode-3.0 |
| icu_normalizer | 2.3.0 | Unicode-3.0 |
| icu_normalizer_data | 2.3.0 | Unicode-3.0 |
| icu_properties | 2.3.0 | Unicode-3.0 |
| icu_properties_data | 2.3.0 | Unicode-3.0 |
| icu_provider | 2.3.1 | Unicode-3.0 |
| idna | 1.1.0 | MIT OR Apache-2.0 |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT |
| image | 0.25.10 | MIT OR Apache-2.0 |
| indexmap | 2.14.2 | Apache-2.0 OR MIT |
| inout | 0.1.4 | MIT OR Apache-2.0 |
| instant | 0.1.13 | BSD-3-Clause |
| ipnet | 2.12.2 | MIT OR Apache-2.0 |
| itertools | 0.15.0 | MIT OR Apache-2.0 |
| itoa | 1.0.18 | MIT OR Apache-2.0 |
| jobserver | 0.1.35 | MIT OR Apache-2.0 |
| k256 | 0.13.4 | Apache-2.0 OR MIT |
| keccak | 0.1.6 | Apache-2.0 OR MIT |
| keyring | 4.2.0 | MIT OR Apache-2.0 |
| keyring-core | 1.0.0 | MIT OR Apache-2.0 |
| khronos-egl | 6.0.0 | MIT/Apache-2.0 |
| kurbo | 0.13.1 | Apache-2.0 OR MIT |
| libc | 0.2.189 | MIT OR Apache-2.0 |
| libcrux-aead | 0.0.7 | Apache-2.0 |
| libcrux-aesgcm | 0.0.7 | Apache-2.0 |
| libcrux-chacha20poly1305 | 0.0.7 | Apache-2.0 |
| libcrux-curve25519 | 0.0.6 | Apache-2.0 |
| libcrux-ecdh | 0.0.6 | Apache-2.0 |
| libcrux-hacl-rs | 0.0.4 | Apache-2.0 |
| libcrux-hkdf | 0.0.6 | Apache-2.0 |
| libcrux-hmac | 0.0.6 | Apache-2.0 |
| libcrux-intrinsics | 0.0.6 | Apache-2.0 |
| libcrux-kem | 0.0.7 | Apache-2.0 |
| libcrux-macros | 0.0.3 | Apache-2.0 |
| libcrux-ml-kem | 0.0.8 | Apache-2.0 |
| libcrux-p256 | 0.0.6 | Apache-2.0 |
| libcrux-platform | 0.0.3 | Apache-2.0 |
| libcrux-poly1305 | 0.0.5 | Apache-2.0 |
| libcrux-secrets | 0.0.5 | Apache-2.0 |
| libcrux-sha2 | 0.0.6 | Apache-2.0 |
| libcrux-sha3 | 0.0.8 | Apache-2.0 |
| libcrux-traits | 0.0.6 | Apache-2.0 |
| libloading | 0.8.9 | ISC |
| libm | 0.2.16 | MIT |
| libopus_sys | 0.3.3 | MIT |
| libsqlite3-sys | 0.38.2 | MIT |
| linebender_resource_handle | 0.1.1 | Apache-2.0 OR MIT |
| litemap | 0.8.3 | Unicode-3.0 |
| litrs | 1.0.0 | MIT OR Apache-2.0 |
| lock_api | 0.4.14 | MIT OR Apache-2.0 |
| log | 0.4.34 | MIT OR Apache-2.0 |
| lru-slab | 0.1.2 | MIT OR Apache-2.0 OR Zlib |
| mac-notification-sys | 0.6.15 | MIT/Apache-2.0 |
| mac-usernotifications | 0.3.1 | MIT OR Apache-2.0 |
| mach2 | 0.6.0 | BSD-2-Clause OR MIT OR Apache-2.0 |
| memchr | 2.8.3 | Unlicense OR MIT |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| mio | 1.2.3 | MIT |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 |
| naga | 30.0.1 | MIT OR Apache-2.0 |
| naga-types | 30.0.1 | MIT OR Apache-2.0 |
| nohash-hasher | 0.2.0 | Apache-2.0 OR MIT |
| notify-rust | 4.18.0 | MIT OR Apache-2.0 |
| num-conv | 0.2.2 | MIT OR Apache-2.0 |
| num-derive | 0.4.2 | MIT OR Apache-2.0 |
| num-traits | 0.2.19 | MIT OR Apache-2.0 |
| objc-sys | 0.3.5 | MIT |
| objc2 | 0.5.2 | MIT |
| objc2 | 0.6.4 | MIT |
| objc2-app-kit | 0.2.2 | MIT |
| objc2-app-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-audio-toolbox | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-audio | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-audio-types | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-data | 0.2.2 | MIT |
| objc2-core-foundation | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-graphics | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-image | 0.2.2 | MIT |
| objc2-core-location | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-encode | 4.1.0 | MIT |
| objc2-exception-helper | 0.1.1 | Zlib OR Apache-2.0 OR MIT |
| objc2-foundation | 0.2.2 | MIT |
| objc2-foundation | 0.3.2 | MIT |
| objc2-io-surface | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-metal | 0.2.2 | MIT |
| objc2-metal | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-quartz-core | 0.2.2 | MIT |
| objc2-quartz-core | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-user-notifications | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-web-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| once_cell | 1.21.4 | MIT OR Apache-2.0 |
| opaque-debug | 0.3.1 | MIT OR Apache-2.0 |
| openmls | 0.8.1 | MIT |
| openmls_basic_credential | 0.5.0 | MIT |
| openmls_memory_storage | 0.5.0 | MIT |
| openmls_rust_crypto | 0.5.1 | MIT |
| openmls_traits | 0.5.0 | MIT |
| option-ext | 0.2.0 | MPL-2.0 |
| opus2 | 0.4.0 | MIT/Apache-2.0 |
| ordered-float | 5.5.0 | MIT |
| p256 | 0.13.2 | Apache-2.0 OR MIT |
| p384 | 0.13.1 | Apache-2.0 OR MIT |
| parking | 2.2.1 | Apache-2.0 OR MIT |
| parking_lot | 0.11.2 | Apache-2.0/MIT |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 |
| parking_lot_core | 0.8.6 | Apache-2.0/MIT |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 |
| pbkdf2 | 0.12.2 | MIT OR Apache-2.0 |
| pem-rfc7468 | 0.7.0 | Apache-2.0 OR MIT |
| peniko | 0.6.1 | Apache-2.0 OR MIT |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT |
| pin-utils | 0.1.0 | MIT OR Apache-2.0 |
| pkcs8 | 0.10.2 | Apache-2.0 OR MIT |
| pkg-config | 0.3.34 | MIT OR Apache-2.0 |
| png | 0.18.1 | MIT OR Apache-2.0 |
| pollster | 1.0.1 | Apache-2.0/MIT |
| poly1305 | 0.8.0 | Apache-2.0 OR MIT |
| polycool | 0.4.0 | MIT OR Apache-2.0 |
| polyval | 0.6.2 | Apache-2.0 OR MIT |
| potential_utf | 0.1.6 | Unicode-3.0 |
| powerfmt | 0.2.0 | MIT OR Apache-2.0 |
| ppv-lite86 | 0.2.21 | MIT OR Apache-2.0 |
| presser | 0.3.1 | MIT OR Apache-2.0 |
| primeorder | 0.13.6 | Apache-2.0 OR MIT |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 |
| profiling | 1.0.18 | MIT OR Apache-2.0 |
| pulldown-cmark | 0.13.4 | MIT |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 |
| quick-error | 2.0.1 | MIT/Apache-2.0 |
| quinn | 0.11.11 | MIT OR Apache-2.0 |
| quinn-proto | 0.11.17 | MIT OR Apache-2.0 |
| quinn-udp | 0.5.15 | MIT OR Apache-2.0 |
| quote | 1.0.47 | MIT OR Apache-2.0 |
| rand | 0.10.2 | MIT OR Apache-2.0 |
| rand | 0.8.8 | MIT OR Apache-2.0 |
| rand | 0.9.5 | MIT OR Apache-2.0 |
| rand_chacha | 0.10.0 | MIT OR Apache-2.0 |
| rand_chacha | 0.3.1 | MIT OR Apache-2.0 |
| rand_chacha | 0.9.0 | MIT OR Apache-2.0 |
| rand_core | 0.10.1 | MIT OR Apache-2.0 |
| rand_core | 0.6.4 | MIT OR Apache-2.0 |
| rand_core | 0.9.5 | MIT OR Apache-2.0 |
| rand_pcg | 0.10.2 | MIT OR Apache-2.0 |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| raw-window-metal | 1.1.0 | MIT OR Apache-2.0 |
| rayon | 1.12.0 | MIT OR Apache-2.0 |
| rayon-core | 1.13.0 | MIT OR Apache-2.0 |
| read-fonts | 0.41.0 | MIT OR Apache-2.0 |
| renderdoc-sys | 1.1.0 | MIT OR Apache-2.0 |
| reqwest | 0.13.5 | MIT OR Apache-2.0 |
| rfc6979 | 0.4.0 | Apache-2.0 OR MIT |
| rfd | 0.17.2 | MIT |
| ring | 0.17.14 | Apache-2.0 AND ISC |
| rtrb | 0.4.0 | MIT OR Apache-2.0 |
| rusqlite | 0.40.2 | MIT |
| rustc-hash | 1.1.0 | Apache-2.0/MIT |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 |
| rustls | 0.23.44 | Apache-2.0 OR ISC OR MIT |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 |
| rustls-platform-verifier | 0.7.0 | MIT OR Apache-2.0 |
| rustls-webpki | 0.103.15 | ISC |
| salsa20 | 0.10.2 | MIT OR Apache-2.0 |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 |
| scrypt | 0.11.0 | MIT OR Apache-2.0 |
| sec1 | 0.7.3 | Apache-2.0 OR MIT |
| security-framework | 3.7.0 | MIT OR Apache-2.0 |
| security-framework-sys | 2.17.0 | MIT OR Apache-2.0 |
| self_cell | 1.3.0 | Apache-2.0 OR GPL-2.0-only |
| semver | 1.0.28 | MIT OR Apache-2.0 |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde_bytes | 0.11.19 | MIT OR Apache-2.0 |
| serde_core | 1.0.229 | MIT OR Apache-2.0 |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 |
| serde_json | 1.0.151 | MIT OR Apache-2.0 |
| sha1 | 0.11.0 | MIT OR Apache-2.0 |
| sha2 | 0.10.9 | MIT OR Apache-2.0 |
| sha3 | 0.10.9 | MIT OR Apache-2.0 |
| shlex | 2.0.1 | MIT OR Apache-2.0 |
| signature | 2.2.0 | Apache-2.0 OR MIT |
| simd-adler32 | 0.3.10 | MIT |
| skrifa | 0.44.0 | MIT OR Apache-2.0 |
| slab | 0.4.12 | MIT |
| smallvec | 1.16.0 | MIT OR Apache-2.0 |
| smol_str | 0.2.2 | MIT OR Apache-2.0 |
| socket2 | 0.6.5 | MIT OR Apache-2.0 |
| spirv | 0.4.0+sdk-1.4.341.0 | Apache-2.0 |
| spki | 0.7.3 | Apache-2.0 OR MIT |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 |
| subtle | 2.6.1 | BSD-3-Clause |
| syn | 2.0.119 | MIT OR Apache-2.0 |
| syn | 3.0.5 | MIT OR Apache-2.0 |
| sync_wrapper | 1.0.2 | Apache-2.0 |
| synstructure | 0.13.2 | MIT |
| termcolor | 1.4.1 | Unlicense OR MIT |
| thiserror | 2.0.20 | MIT OR Apache-2.0 |
| thiserror-impl | 2.0.20 | MIT OR Apache-2.0 |
| tiff | 0.11.3 | MIT |
| time | 0.3.55 | MIT OR Apache-2.0 |
| time-core | 0.1.9 | MIT OR Apache-2.0 |
| time-macros | 0.2.32 | MIT OR Apache-2.0 |
| tinystr | 0.8.4 | Unicode-3.0 |
| tinyvec | 1.13.2 | Zlib OR Apache-2.0 OR MIT |
| tinyvec_macros | 0.1.1 | MIT OR Apache-2.0 OR Zlib |
| tls_codec | 0.4.2 | Apache-2.0 OR MIT |
| tls_codec_derive | 0.4.2 | Apache-2.0 OR MIT |
| tokio | 1.53.1 | MIT |
| tokio-macros | 2.7.2 | MIT |
| tokio-rustls | 0.26.5 | MIT OR Apache-2.0 |
| tokio-tungstenite | 0.30.0 | MIT |
| tower | 0.5.3 | MIT |
| tower-http | 0.6.11 | MIT |
| tower-layer | 0.3.3 | MIT |
| tower-service | 0.3.3 | MIT |
| tracing | 0.1.44 | MIT |
| tracing-attributes | 0.1.31 | MIT |
| tracing-core | 0.1.36 | MIT |
| try-lock | 0.2.5 | MIT |
| tungstenite | 0.30.0 | MIT OR Apache-2.0 |
| type-map | 0.5.1 | MIT/Apache-2.0 |
| typenum | 1.20.1 | MIT OR Apache-2.0 |
| unicase | 2.9.0 | MIT OR Apache-2.0 |
| unicode-general-category | 1.1.0 | Apache-2.0 |
| unicode-ident | 1.0.24 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 |
| unicode-width | 0.2.2 | MIT OR Apache-2.0 |
| universal-hash | 0.5.1 | MIT OR Apache-2.0 |
| untrusted | 0.9.0 | ISC |
| url | 2.5.8 | MIT OR Apache-2.0 |
| utf8_iter | 1.0.4 | Apache-2.0 OR MIT |
| uuid | 1.26.0 | Apache-2.0 OR MIT |
| vcpkg | 0.2.15 | MIT/Apache-2.0 |
| vello_common | 0.1.0 | Apache-2.0 OR MIT |
| vello_cpu | 0.1.0 | Apache-2.0 OR MIT |
| version_check | 0.9.5 | MIT/Apache-2.0 |
| want | 0.3.1 | MIT |
| wayland-sys | 0.31.11 | MIT |
| web-time | 1.1.0 | MIT OR Apache-2.0 |
| webpki-roots | 0.26.11 | CDLA-Permissive-2.0 |
| webpki-roots | 1.0.9 | CDLA-Permissive-2.0 |
| weezl | 0.1.12 | MIT OR Apache-2.0 |
| wgpu | 30.0.1 | MIT OR Apache-2.0 |
| wgpu-core | 30.0.1 | MIT OR Apache-2.0 |
| wgpu-core-deps-apple | 30.0.1 | MIT OR Apache-2.0 |
| wgpu-hal | 30.0.1 | MIT OR Apache-2.0 |
| wgpu-naga-bridge | 30.0.1 | MIT OR Apache-2.0 |
| wgpu-types | 30.0.1 | MIT OR Apache-2.0 |
| winit | 0.30.13 | Apache-2.0 |
| writeable | 0.6.4 | Unicode-3.0 |
| wry | 0.57.0 | Apache-2.0 OR MIT |
| x25519-dalek | 2.0.1 | BSD-3-Clause |
| yoke | 0.8.3 | Unicode-3.0 |
| yoke-derive | 0.8.2 | Unicode-3.0 |
| zerocopy | 0.8.57 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zerocopy-derive | 0.8.57 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zerofrom | 0.1.8 | Unicode-3.0 |
| zerofrom-derive | 0.1.7 | Unicode-3.0 |
| zeroize | 1.9.0 | Apache-2.0 OR MIT |
| zeroize_derive | 1.5.0 | Apache-2.0 OR MIT |
| zerotrie | 0.2.5 | Unicode-3.0 |
| zerovec | 0.11.8 | Unicode-3.0 |
| zerovec-derive | 0.11.6 | Unicode-3.0 |
| zlib-rs | 0.6.7 | Zlib |
| zmij | 1.0.23 | MIT |
| zune-core | 0.5.3 | MIT OR Apache-2.0 OR Zlib |
| zune-jpeg | 0.5.15 | MIT OR Apache-2.0 OR Zlib |

September 10, 2026 Twemoji rendering: `ui` directly reuses already-locked `image 0.25.10`
(PNG only, default features disabled) and `unicode-segmentation 1.13.3`; no new resolved
Cargo package was added. Bundled Twemoji 17.0.3 artwork is CC BY 4.0, with provenance,
modifications and full license in `assets/twemoji`. The atlas generator uses Pillow 11.3.0
only during development; it is not a runtime or Cargo-build requirement.


September 11 inline audio: Symphonia 0.6.1 (MPL-2.0) enables only MP3, PCM and WAV,
with defaults/metadata features disabled. CPAL 0.18.2 (Apache-2.0), already used by
optional voice, now supplies attachment output in the default build as well. It does
not initialize devices until Play. DAVE/Opus/microphone integration remains optional.
Exact unmodified Symphonia source archives and license provenance ship in
`licenses/audio`; Linux runtime library dependencies are derived by dpkg-shlibdeps.

## Voice AEC addition — September 11, 2026

The optional discord-voice dependency adds sonora 0.2.0 with default features
disabled, plus sonora-aec3, sonora-agc2, sonora-common-audio, sonora-fft,
sonora-ns and sonora-simd 0.2.0 (all BSD-3-Clause). Its existing derive_more
dependency also enables unicode-xid 0.2.6 (MIT OR Apache-2.0). Cargo.lock pins
archive checksums; existing packages were not upgraded. This is a Rust DSP
implementation; no new native build tools, SDK credentials or models are needed.
The integration enables only AEC3 and its high-pass filter. Text-only builds
do not select discord-voice or Sonora. See assets/licenses/voice/PROVENANCE.md.

The macOS microphone fix adds objc2-av-foundation 0.3.2
(Zlib OR Apache-2.0 OR MIT), selecting only std, AVCaptureDevice, AVMediaFormat
and block2. Existing objc2 0.6.4 and block2 0.6.2 become direct macOS-only voice
dependencies. AVFoundation remains a system framework. The objc2 licensing
notice already staged in licenses/notifications also covers this framework
binding; complete transitive license assembly remains the existing release gate.

## Optional noise suppression — September 11, 2026

The voice-only dependency nnnoiseless 0.5.2 embeds its default RNNoise model. Default features are disabled: no CLI, WAV I/O, training, C API, or dasp. Cargo.lock adds these ten packages without upgrading existing packages:

- nnnoiseless 0.5.2: BSD-3-Clause.
- easyfft 0.4.2: MIT OR Apache-2.0.
- anymap3 1.1.0: BlueOak-1.0.0 OR MIT OR Apache-2.0.
- array-init 2.1.0: MIT OR Apache-2.0.
- generic_singleton 0.5.3: MIT OR Apache-2.0.
- primal-check 0.3.4: MIT OR Apache-2.0.
- realfft 3.5.0: MIT.
- rustfft 6.4.1: MIT OR Apache-2.0.
- strength_reduce 0.2.4: MIT OR Apache-2.0.
- transpose 0.2.3: MIT OR Apache-2.0.

Available license texts/notices and provenance are under `assets/licenses/voice`; existing voice packaging stages that directory. Realfft omits its standalone MIT text from both the registry archive and the pinned Git tree; full redistribution notice assembly remains outstanding. Default text-only builds do not select this processor.

## September 11 screen sharing

Pinned direct additions: screencapturekit 10.0.3 (MIT OR Apache-2.0, macOS, default features disabled with `macos_14_0` for aspect-preserving output), windows-capture 2.0.1 (MIT, Windows), openh264 and openh264-sys2 0.9.8 (BSD-2-Clause, source build). Image 0.25.10 is already resolved. New support crates: apple-cf 0.10.0, apple-metal 0.9.0, crossbeam-queue 0.3.14, doom-fish-utils 0.4.0 and nasm-rs 0.3.2 (MIT OR Apache-2.0), wide 1.7.0 and safe_arch 1.2.0 (Zlib OR Apache-2.0 OR MIT). No existing locked package version changes. These remain optional voice dependencies, with platform capture dependencies target-specific.


### Game process enumeration (September 11, 2026)

The existing pinned `windows` 0.61.3 dependency additionally enables
`Win32_System_Diagnostics_ToolHelp`; no package or lockfile dependency is added.
Platform keeps `unsafe_code = deny` with one narrowly allowed Windows detector function
for the reviewed ToolHelp/CloseHandle FFI. Other workspace crates retain their existing
forbid policy. The snapshot handle is owned once and closed on every return path.


## Per-build notice inventory - September 11, 2026

The static tables above describe resolved dependencies. Each new text/voice package also
contains `licenses/dependencies/inventory.json`, selected from Cargo compiler-artifact IDs
with locked, offline, host-filtered metadata. Its conservative scope includes build-time
crates; it does not prove linker-level inclusion. Exact-version/source overrides supplement
missing root texts, nested notices and corresponding MPL sources. No dependencies changed.

See `assets/licenses/dependencies/PROVENANCE.md` for hashes and known upstream omissions.
`complete: false` records unresolved grants for realfft/dispatch and modern objc2 components
when selected; source archives and a labeled MIT reference do not close those omissions.
New unreviewed missing texts fail packaging. Native system-library redistribution and a
complete transitive notice review remain separate release gates.


## Inline attachment video on macOS and Linux (September 12, 2026)

Pinned direct additions in `platform`: objc2-video-toolbox 0.3.2 (MIT, macOS, only the
`VTDecompressionSession`, `VTErrors`, `VTSession` and `VTBase` features), plus the already
resolved objc2-core-media, objc2-core-video and objc2-core-foundation 0.3.2 with their
decoding features; symphonia 0.6.1 with the `aac` feature (MPL-2.0, pure Rust, resolves the new
symphonia-codec-aac 0.6.1); gstreamer 0.25.3, gstreamer-app 0.25.2 and gstreamer-video 0.25.3
(MIT OR Apache-2.0, Linux only, dynamically linked against the distribution's GStreamer
1.x; CI and packaging install `libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev`).
New support crates: gstreamer-sys 0.25.2, gstreamer-base 0.25.3, gstreamer-base-sys 0.25.3,
gstreamer-app-sys 0.25.0, gstreamer-video-sys 0.25.3, atomic_refcell 0.1.14, kstring 2.0.4,
muldiv 1.0.1, option-operations 0.6.1 and pastey 0.2.3. No existing locked package version
changes. The Debian package recommends `gstreamer1.0-plugins-base`, `gstreamer1.0-plugins-good`
and `gstreamer1.0-libav`, which `dpkg-shlibdeps` cannot see because plugins are loaded at run
time. `platform` keeps `unsafe_code = deny`; the VideoToolbox module is the only new allowance
and mirrors the audited Media Foundation module.

# Voice dependency license provenance

September 15, 2026 stream-audio exclusion addition: `libpulse-sys 1.23.0`,
registry archive SHA-256 `d74371848b22e989f829cc1621d2ebd74960711557d8b45cfe740f60d0a05e61`.
`libpulse-sys-1.23.0-LICENSE-MIT` is copied unmodified from the release's
[`LICENSE-MIT`](https://docs.rs/crate/libpulse-sys/1.23.0/source/LICENSE-MIT),
SHA-256 `20278f4e2697210305f0a25ef5f3b73fecce789096c0ae55287684ea9282cfcb`.
The library is dynamically linked against the system's libpulse; no PulseAudio server
or native library is bundled. The resolved WinAPI 0.3.9 and architecture support crates
are Windows-only declarations of libpulse-sys and are not selected by Serein's Linux
runtime or macOS development use of these bindings.

Collected September 10, 2026. Except for the separately identified canonical MPL text below, files are unmodified source license/notices, copied from the exact resolved crates.io releases or fetched from the commit recorded in the release's `.cargo_vcs_info.json`. Registry source links identify the shipped source archive; SHA-256 values below verify the copied text. All files are flat for distribution staging.

Davey 0.1.4 and OpenMLS 0.8.1 omit their root license files from their registry archives. Their MIT texts were retrieved from the pinned upstream commits linked below (Davey package path `davey`, OpenMLS package path `openmls`). `libopus_sys` 0.3.3 bundles the codec under `opus/`; the registry archive is authoritative because its VCS metadata records a dirty working tree. Both its binding licenses and the bundled codec's COPYING and LICENSE_PLEASE_READ.txt are retained, including the upstream IETF patent-statement references.

This directory covers newly added direct voice libraries and the bundled codec/binding notices. It is **not** a complete transitive dependency license bundle, legal opinion, patent review or platform redistribution sign-off. The full per-artifact release review remains outstanding as described in THIRD_PARTY_NOTICES.md.

| File | Exact source | SHA-256 |
|---|---|---|
| cpal-LICENSE.txt | [source](https://docs.rs/crate/cpal/0.18.2/source/LICENSE) | `c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4` |
| opus2-LICENSE-MIT.txt | [source](https://docs.rs/crate/opus2/0.4.0/source/LICENSE-MIT) | `6b3b465fa69075348ee5ffc9ba6afa93402743a4a942a463404fac25257bd3ed` |
| opus2-LICENSE-APACHE.txt | [source](https://docs.rs/crate/opus2/0.4.0/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| rtrb-LICENSE-MIT.txt | [source](https://docs.rs/crate/rtrb/0.4.0/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| rtrb-LICENSE-APACHE.txt | [source](https://docs.rs/crate/rtrb/0.4.0/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| chacha20poly1305-LICENSE-MIT.txt | [source](https://docs.rs/crate/chacha20poly1305/0.10.1/source/LICENSE-MIT) | `3c0dfa33fd2e6976038555b52095699452653b1fcabe113074f14e0848a6b11e` |
| chacha20poly1305-LICENSE-APACHE.txt | [source](https://docs.rs/crate/chacha20poly1305/0.10.1/source/LICENSE-APACHE) | `a9040321c3712d8fd0b09cf52b17445de04a23a10165049ae187cd39e5c86be5` |
| libopus_sys-LICENSE.txt | [source](https://docs.rs/crate/libopus_sys/0.3.3/source/LICENSE) | `c5512e4899a9b06c3a286bf69444ebc5314d506e437c334bfd66a2eac7bca3ef` |
| libopus_sys-LICENSE-old.txt | [source](https://docs.rs/crate/libopus_sys/0.3.3/source/LICENSE-old) | `a6c8bb45880edc99bb13feb04d69fb2f0f73b565dfe209affa60414d2aa4c1a7` |
| libopus_sys-opus-COPYING.txt | [source](https://docs.rs/crate/libopus_sys/0.3.3/source/opus/COPYING) | `01e1167d54a096d123cf6dfbbeb19587278845c6481d2d66d545669846079551` |
| libopus_sys-opus-LICENSE_PLEASE_READ.txt | [source](https://docs.rs/crate/libopus_sys/0.3.3/source/opus/LICENSE_PLEASE_READ.txt) | `7efb4989e0cd1b256229bdf2f09300c5d14e35db0e7476bfb87fac243498273d` |
| davey-LICENSE.txt | [source](https://raw.githubusercontent.com/Snazzah/davey/a1e2e741bea06bc3b7167a5c3792844b8975993c/LICENSE) | `90760006b6e6c76a67476de39bee25e42e584679eac5d608765355d5d54e8afa` |
| openmls-LICENSE.txt | [source](https://raw.githubusercontent.com/openmls/openmls/47dbedecad0c1fd8eb5368d582250ebfcc1e1ce6/LICENSE) | `43e5e3c4b5cca67f9ea912f7e1929702a848aa765d3b3e14f25d3838a5a5565d` |
| hpke-rs-LICENSE-MPL-2.0.txt | [canonical Mozilla text](https://www.mozilla.org/media/MPL/2.0/index.txt) | `3f3d9e0024b1921b067d6f7f88deb4a60cbe7a78e76c64e3f1d7fc3b779b9d04` |

Registry package archive checksums from the original Cargo.lock (Davey is now a local
manifest-only patch; its original archive checksum remains recorded below and in
`vendor/davey/SEREIN-PATCH.md`):

- chacha20poly1305 0.10.1: `10cd79432192d1c0f4e1a0fef9527696cc039165d729fb41b3f4f4f354c2dc35`
- cpal 0.18.2: `6f02e8d0327b42d3e2e4ab2119af397344eb9fc54a34bf0ddeaa1277af8681f1`
- davey 0.1.4: `25028cc2ec8cd43138ca0d2c71d7f6019196238ebd6edea11dd35276fa85b860`
- libopus_sys 0.3.3: `b81c32f233fb2507347a93f97b9919493be9db7ec2249ce2c3ed0c89ad8edf60`
- openmls 0.8.1: `dcb512bfe6a55777518853ea535c6241f069cb0e8984678c117151d2a1e7e903`
- opus2 0.4.0: `49521e33fbf825d2abc8d696506c278cb8469d0c6cc05d3785bef5c5f7ee957b`
- rtrb 0.4.0: `9278fb35b3e730abe136e9b395b5b81b96d06b9f5478a50f0c8430a2237b22de`

Vendored hpke-rs 0.6.1 followup: the [release-pinned manifest](https://raw.githubusercontent.com/cryspen/hpke-rs/f3463e7530771d7f7116635335c25e7d2d11e861/Cargo.toml) declares **MPL-2.0**, not MIT/Apache. Neither its registry archive nor the complete pinned Git tree includes a LICENSE/COPYING/NOTICE file. The unmodified canonical Mozilla MPL-2.0 text linked above is therefore supplied in this directory and `vendor/hpke-rs/LICENSE-MPL-2.0.txt`; it is not represented as an upstream repository file. The original source and manifest notices are retained. `vendor/hpke-rs/SEREIN-PATCH.md` identifies the local SHAKE dependency replacement and adapter. Distributors of the modified component must make its corresponding source, including modifications, available as required by MPL-2.0 and tell recipients how to obtain it. This addition does not complete the remaining transitive license review.

## RustCrypto SHAKE backport dependencies
- `keccak-LICENSE-APACHE`: unmodified from crates.io keccak 0.1.6 / `LICENSE-APACHE`; SHA-256 `a9040321c3712d8fd0b09cf52b17445de04a23a10165049ae187cd39e5c86be5`.
- `keccak-LICENSE-MIT`: unmodified from crates.io keccak 0.1.6 / `LICENSE-MIT`; SHA-256 `bdebaf9156a298f8fdab56dd26cb5144673de522d80f4c0d88e0039145f147f9`.
- `sha3-LICENSE-APACHE`: unmodified from crates.io sha3 0.10.9 / `LICENSE-APACHE`; SHA-256 `a9040321c3712d8fd0b09cf52b17445de04a23a10165049ae187cd39e5c86be5`.
- `sha3-LICENSE-MIT`: unmodified from crates.io sha3 0.10.9 / `LICENSE-MIT`; SHA-256 `f18f6229547ab07f0b7b3e1f83acad8bb436f5f4c95a8a98b44f876caa00f04e`.

Sonora 0.2.0 and its sonora-aec3, sonora-agc2, sonora-common-audio, sonora-fft,
sonora-ns and sonora-simd 0.2.0 components belong to the BSD-3-Clause licensed
Sonora workspace. Its license is retained as `sonora-LICENSE.txt`, copied
unmodified from [the sonora 0.2.0 registry archive](https://docs.rs/crate/sonora/0.2.0/source/LICENSE).
The component archives omit a root license file; their VCS metadata points to
[the shared workspace](https://github.com/dignifiedquire/sonora/tree/a024d6ef8351add55be5e8b1d6cc35f555787660).
SHA-256: `d130affc26760004865ad4fb009f8daf7f6d4b1b738f01fb7623e99c1a275a4a`.

## RNNoise noise suppression — September 11, 2026

Unmodified license and notice files from the pinned registry packages. The anymap3 COPYING notice offers a choice of licenses. Realfft 3.5.0 declares MIT, but its registry archive and pinned upstream tree (`d0d4eee0525fd27c96c8a046d6d107acd5ed84a6`) omit the license text; its complete redistribution notice remains part of the existing per-artifact release review.

| File | Exact source | SHA-256 |
|---|---|---|
| nnnoiseless-COPYING.txt | [registry source](https://docs.rs/crate/nnnoiseless/0.5.2/source/COPYING) | `26e8aae6fb3622e281d8d99aa7ba0df8ef3a83465d8717f2e4cacbcbed8efc92` |
| easyfft-LICENSE-APACHE.txt | [registry source](https://docs.rs/crate/easyfft/0.4.2/source/LICENSE-APACHE) | `c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08` |
| easyfft-LICENSE-MIT.txt | [registry source](https://docs.rs/crate/easyfft/0.4.2/source/LICENSE-MIT) | `cb22dce3ea93e49e43b70059c193184e27612526591db29a4cd95a369a2a5645` |
| anymap3-COPYING.txt | [registry source](https://docs.rs/crate/anymap3/1.1.0/source/COPYING) | `cce3eaac7d45535f0c5421e1a7330933e816265e766f4a94f20442de9da548df` |
| array-init-LICENSE-APACHE.txt | [registry source](https://docs.rs/crate/array-init/2.1.0/source/LICENSE-APACHE) | `c8d9a0d15dd76ca3bf277b6bf6da56799e266eac60bdc321a97ebc6d76d5153c` |
| array-init-LICENSE-MIT.txt | [registry source](https://docs.rs/crate/array-init/2.1.0/source/LICENSE-MIT) | `e27fb2953c088c71285a4f2f54a0ac53323460ee7c2b1b838d563bd2687a38af` |
| generic_singleton-LICENSE-APACHE.txt | [registry source](https://docs.rs/crate/generic_singleton/0.5.3/source/LICENSE-APACHE) | `c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08` |
| generic_singleton-LICENSE-MIT.txt | [registry source](https://docs.rs/crate/generic_singleton/0.5.3/source/LICENSE-MIT) | `cb22dce3ea93e49e43b70059c193184e27612526591db29a4cd95a369a2a5645` |
| primal-check-LICENSE-APACHE.txt | [registry source](https://docs.rs/crate/primal-check/0.3.4/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| primal-check-LICENSE-MIT.txt | [registry source](https://docs.rs/crate/primal-check/0.3.4/source/LICENSE-MIT) | `6d3a9431e65e69c73a8923e6517b889d17549b23db406b9ec027710d16af701f` |
| rustfft-LICENSE-APACHE.txt | [registry source](https://docs.rs/crate/rustfft/6.4.1/source/LICENSE-APACHE) | `2e54cd84a645bea25943c75dd8ae67cb291e66a47a11578333c9b4b3b6b86c85` |
| rustfft-LICENSE-MIT.txt | [registry source](https://docs.rs/crate/rustfft/6.4.1/source/LICENSE-MIT) | `8f5442dfa8e9169045697e386bc91d19f393c939635741fa2a665ec36ca6f0ad` |
| strength_reduce-LICENSE-APACHE.txt | [registry source](https://docs.rs/crate/strength_reduce/0.2.4/source/LICENSE-APACHE) | `2e54cd84a645bea25943c75dd8ae67cb291e66a47a11578333c9b4b3b6b86c85` |
| strength_reduce-LICENSE-MIT.txt | [registry source](https://docs.rs/crate/strength_reduce/0.2.4/source/LICENSE-MIT) | `8f5442dfa8e9169045697e386bc91d19f393c939635741fa2a665ec36ca6f0ad` |
| transpose-LICENSE-APACHE.txt | [registry source](https://docs.rs/crate/transpose/0.2.3/source/LICENSE-APACHE) | `8797ef61538ec5ee9222ebef7ca4e0f3ec5761b145ca9943d358c450efb644dd` |
| transpose-LICENSE-MIT.txt | [registry source](https://docs.rs/crate/transpose/0.2.3/source/LICENSE-MIT) | `5080149357fd0be590bdc10cf92165412bb4d61ce496284d56f2d12874ae3121` |

## Screen sharing — September 11, 2026

Unmodified texts from newly locked registry archives, included by voice packaging.

| File | Exact source | SHA-256 |
|---|---|---|
| apple-cf-LICENSE-APACHE | [registry source](https://docs.rs/crate/apple-cf/0.10.0/source/LICENSE-APACHE) | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |
| apple-cf-LICENSE-MIT | [registry source](https://docs.rs/crate/apple-cf/0.10.0/source/LICENSE-MIT) | `a31e68c0715fe30f0fd0a2372b39ed1ccc3cf07d396940d7d1258001d60ea4e4` |
| apple-metal-LICENSE-APACHE | [registry source](https://docs.rs/crate/apple-metal/0.9.0/source/LICENSE-APACHE) | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |
| apple-metal-LICENSE-MIT | [registry source](https://docs.rs/crate/apple-metal/0.9.0/source/LICENSE-MIT) | `a31e68c0715fe30f0fd0a2372b39ed1ccc3cf07d396940d7d1258001d60ea4e4` |
| crossbeam-queue-LICENSE-APACHE | [registry source](https://docs.rs/crate/crossbeam-queue/0.3.14/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| crossbeam-queue-LICENSE-MIT | [registry source](https://docs.rs/crate/crossbeam-queue/0.3.14/source/LICENSE-MIT) | `5734ed989dfca1f625b40281ee9f4530f91b2411ec01cb748223e7eb87e201ab` |
| doom-fish-utils-LICENSE-APACHE | [registry source](https://docs.rs/crate/doom-fish-utils/0.4.0/source/LICENSE-APACHE) | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |
| doom-fish-utils-LICENSE-MIT | [registry source](https://docs.rs/crate/doom-fish-utils/0.4.0/source/LICENSE-MIT) | `a31e68c0715fe30f0fd0a2372b39ed1ccc3cf07d396940d7d1258001d60ea4e4` |
| nasm-rs-LICENSE-APACHE | [registry source](https://docs.rs/crate/nasm-rs/0.3.2/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| nasm-rs-LICENSE-MIT | [registry source](https://docs.rs/crate/nasm-rs/0.3.2/source/LICENSE-MIT) | `c9a75f18b9ab2927829a208fc6aa2cf4e63b8420887ba29cdb265d6619ae82d5` |
| openh264-sys2-upstream-LICENSE | [registry source](https://docs.rs/crate/openh264-sys2/0.9.8/source/upstream/LICENSE) | `dd5c1c9668512530fa5a96e4c29ac4033d70a7eeb0eed7a42fddb6dd794ebdbb` |
| screencapturekit-LICENSE-APACHE | [registry source](https://docs.rs/crate/screencapturekit/10.0.3/source/LICENSE-APACHE) | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |
| screencapturekit-LICENSE-MIT | [registry source](https://docs.rs/crate/screencapturekit/10.0.3/source/LICENSE-MIT) | `5464a5ed6a2143a9d31e5516c7336a312c3c067cdc6ad15323f18fdf8d9755ae` |
| wide-LICENSE-APACHE.txt | [registry source](https://docs.rs/crate/wide/1.7.0/source/LICENSE-APACHE.txt) | `d64ac6ba60c7352115244dcfcb46c24f934603dd92e64835beafe7631c485abc` |
| wide-LICENSE-MIT.txt | [registry source](https://docs.rs/crate/wide/1.7.0/source/LICENSE-MIT.txt) | `69c62cda6938d510467c134dc0b5c83fb9c8ca847b9cd9d9d2fe9ab427c82100` |
| wide-LICENSE-ZLIB.txt | [registry source](https://docs.rs/crate/wide/1.7.0/source/LICENSE-ZLIB.txt) | `3c6921f3dfec0bed0cfa2d632616edd7380a875c8ad76c7b23dc59892259f62c` |
| windows-capture-LICENCE | [pinned upstream](https://raw.githubusercontent.com/NiiightmareXD/windows-capture/c7d106448eb9d9b251345c39047711e1cd408ae2/LICENCE) | `ee09cf4f5e858c3b5105968951825c562120959f6cb53707e326d8f3d9e5a481` |

OpenH264 Rust binding crates declare BSD-2-Clause but their registry archives and pinned upstream tree omit a binding license file. The bundled Cisco OpenH264 codec license is retained separately above; this does not complete the existing distribution license/patent review. The encoder is built from source; no Cisco binary download or binary-license coverage is assumed.

Safe_arch 1.2.0 archive verified against Cargo.lock SHA-256 before reading its unmodified license texts:

| File | Exact source | SHA-256 |
|---|---|---|
| safe_arch-LICENSE-APACHE.md | [registry source](https://docs.rs/crate/safe_arch/1.2.0/source/LICENSE-APACHE.md) | `e3ba223bb1423f0aad8c3dfce0fe3148db48926d41e6fbc3afbbf5ff9e1c89cb` |
| safe_arch-LICENSE-MIT.md | [registry source](https://docs.rs/crate/safe_arch/1.2.0/source/LICENSE-MIT.md) | `e57011537d230b14e790f6666dc00816f7b371ebbd7da8a12491e51086fec278` |
| safe_arch-LICENSE-ZLIB.md | [registry source](https://docs.rs/crate/safe_arch/1.2.0/source/LICENSE-ZLIB.md) | `c43b9a9b1387ed53d2c49263838261129a010e280e3a174a792242c3e2c98db9` |

## Linux desktop audio — September 17, 2026

CPAL’s PulseAudio feature adds these unmodified registry license texts.
They cover the Rust PulseAudio client and its newly resolved helper crates;
no PulseAudio server is bundled.

| File | Exact source | SHA-256 |
|---|---|---|
| pulseaudio-0.3.1-LICENSE.md | [registry source](https://docs.rs/crate/pulseaudio/0.3.1/source/LICENSE.md) | `fb9e808c9dd52f9d00f1168713933087b8fed4b7ecbbc694fc4b1d08c0c82352` |
| enum-primitive-derive-0.3.0-LICENSE | [registry source](https://docs.rs/crate/enum-primitive-derive/0.3.0/source/LICENSE) | `819e0555b295079201b0670bb3302855303bdbbcc739f3819b13e1b3d2ec03bb` |
| futures-0.3.34-LICENSE-MIT | [registry source](https://docs.rs/crate/futures/0.3.34/source/LICENSE-MIT) | `6652c868f35dfe5e8ef636810a4e576b9d663f3a17fb0f5613ad73583e1b88fd` |
| futures-0.3.34-LICENSE-APACHE | [registry source](https://docs.rs/crate/futures/0.3.34/source/LICENSE-APACHE) | `275c491d6d1160553c32fd6127061d7f9606c3ea25abfad6ca3f6ed088785427` |

## DeepFilterNet noise suppression — November 6, 2026

DeepFilterNet source, the DeepFilterNet3 model and their licenses come from
[ismailivanov/DeepFilterNetPlus](https://github.com/ismailivanov/DeepFilterNetPlus/tree/2dc5273d16e1e3769527ad0740654c5ccde581cc)
commit `2dc5273d16e1e3769527ad0740654c5ccde581cc`, a maintained fork of Rikorose/DeepFilterNet
whose root license files are unchanged from upstream commit
`d375b2d8309e0935d165700c91da9de862a99c31`; `vendor/deepfilternet/SEREIN-PATCH.md` records
the vendored files and manifest changes. The tract texts are unmodified from the tract-core
0.22.5 registry archive; the other eight tract 0.22.5 workspace crates ship byte-identical
texts. Support-crate files are unmodified from each pinned registry archive. Hashes are of
the LF-normalized files.

| File | Component(s) | Exact source | SHA-256 |
|---|---|---|---|
| deepfilternet-LICENSE-MIT.txt | DeepFilterNet Plus 2dc5273 | [pinned fork](https://github.com/ismailivanov/DeepFilterNetPlus/blob/2dc5273d16e1e3769527ad0740654c5ccde581cc/LICENSE-MIT) | `24e6bb09c928af8d8e56268082f87413247ce36b39dd5d33add2f9893968065e` |
| deepfilternet-LICENSE-APACHE.txt | DeepFilterNet Plus 2dc5273 | [pinned fork](https://github.com/ismailivanov/DeepFilterNetPlus/blob/2dc5273d16e1e3769527ad0740654c5ccde581cc/LICENSE-APACHE) | `1eaee808c5fb6b4e895ba30425285a5cdc5dd25bba2cd230f264c2200c331aec` |
| tract-LICENSE-MIT.txt | tract 0.22.5 workspace (nine crates) | [registry source](https://docs.rs/crate/tract-core/0.22.5/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| tract-LICENSE-APACHE.txt | tract 0.22.5 workspace (nine crates) | [registry source](https://docs.rs/crate/tract-core/0.22.5/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| anyhow-1.0.104-LICENSE-APACHE | anyhow 1.0.104 | [registry source](https://docs.rs/crate/anyhow/1.0.104/source/LICENSE-APACHE) | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| anyhow-1.0.104-LICENSE-MIT | anyhow 1.0.104 | [registry source](https://docs.rs/crate/anyhow/1.0.104/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| anymap2-0.13.0-LICENSE-APACHE | anymap2 0.13.0 | [registry source](https://docs.rs/crate/anymap2/0.13.0/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| anymap2-0.13.0-LICENSE-MIT | anymap2 0.13.0 | [registry source](https://docs.rs/crate/anymap2/0.13.0/source/LICENSE-MIT) | `b85cb7b51c3f8d600bbac3fb8dbbfe64cde697e460a55fc80978eb0804ca1427` |
| bit-set-0.5.3-LICENSE-APACHE | bit-set 0.5.3 | [registry source](https://docs.rs/crate/bit-set/0.5.3/source/LICENSE-APACHE) | `8173d5c29b4f956d532781d2b86e4e30f83e6b7878dce18c919451d6ba707c90` |
| bit-set-0.5.3-LICENSE-MIT | bit-set 0.5.3 | [registry source](https://docs.rs/crate/bit-set/0.5.3/source/LICENSE-MIT) | `c9a75f18b9ab2927829a208fc6aa2cf4e63b8420887ba29cdb265d6619ae82d5` |
| bit-vec-0.6.3-LICENSE-APACHE | bit-vec 0.6.3 | [registry source](https://docs.rs/crate/bit-vec/0.6.3/source/LICENSE-APACHE) | `8173d5c29b4f956d532781d2b86e4e30f83e6b7878dce18c919451d6ba707c90` |
| bit-vec-0.6.3-LICENSE-MIT | bit-vec 0.6.3 | [registry source](https://docs.rs/crate/bit-vec/0.6.3/source/LICENSE-MIT) | `7b63ecd5f1902af1b63729947373683c32745c16a10e8e6292e2e2dcd7e90ae0` |
| const-random-0.1.18-LICENSE-APACHE | const-random 0.1.18 | [registry source](https://docs.rs/crate/const-random/0.1.18/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| const-random-0.1.18-LICENSE-MIT | const-random 0.1.18 | [registry source](https://docs.rs/crate/const-random/0.1.18/source/LICENSE-MIT) | `ff8f68cb076caf8cefe7a6430d4ac086ce6af2ca8ce2c4e5a2004d4552ef52a2` |
| dlv-list-0.5.2-LICENSE-APACHE | dlv-list 0.5.2 | [registry source](https://docs.rs/crate/dlv-list/0.5.2/source/LICENSE-APACHE) | `95bd3988beee069fa2848f648dab43cc6e0b2add2ad6bcb17360caf749802bcc` |
| dlv-list-0.5.2-LICENSE-MIT | dlv-list 0.5.2 | [registry source](https://docs.rs/crate/dlv-list/0.5.2/source/LICENSE-MIT) | `77adcddfe9e50acd2df63ddbb8e566bb8d34b4d02a9d92e7a5c1b9c2225eda9f` |
| dyn-clone-1.0.20-LICENSE-APACHE | dyn-clone 1.0.20 | [registry source](https://docs.rs/crate/dyn-clone/1.0.20/source/LICENSE-APACHE) | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| dyn-clone-1.0.20-LICENSE-MIT | dyn-clone 1.0.20 | [registry source](https://docs.rs/crate/dyn-clone/1.0.20/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| dyn-hash-0.2.2-LICENSE-APACHE | dyn-hash 0.2.2 | [registry source](https://docs.rs/crate/dyn-hash/0.2.2/source/LICENSE-APACHE) | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| dyn-hash-0.2.2-LICENSE-MIT | dyn-hash 0.2.2 | [registry source](https://docs.rs/crate/dyn-hash/0.2.2/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| filetime-0.2.29-LICENSE-APACHE | filetime 0.2.29 | [registry source](https://docs.rs/crate/filetime/0.2.29/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| filetime-0.2.29-LICENSE-MIT | filetime 0.2.29 | [registry source](https://docs.rs/crate/filetime/0.2.29/source/LICENSE-MIT) | `378f5840b258e2779c39418f3f2d7b2ba96f1c7917dd6be0713f88305dbda397` |
| hashbrown-0.14.5-LICENSE-APACHE | hashbrown 0.14.5 | [registry source](https://docs.rs/crate/hashbrown/0.14.5/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| hashbrown-0.14.5-LICENSE-MIT | hashbrown 0.14.5 | [registry source](https://docs.rs/crate/hashbrown/0.14.5/source/LICENSE-MIT) | `ff8f68cb076caf8cefe7a6430d4ac086ce6af2ca8ce2c4e5a2004d4552ef52a2` |
| itertools-0.12.1-LICENSE-APACHE | itertools 0.12.1 | [registry source](https://docs.rs/crate/itertools/0.12.1/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| itertools-0.12.1-LICENSE-MIT | itertools 0.12.1 | [registry source](https://docs.rs/crate/itertools/0.12.1/source/LICENSE-MIT) | `7576269ea71f767b99297934c0b2367532690f8c4badc695edf8e04ab6a1e545` |
| itertools-0.14.0-LICENSE-APACHE | itertools 0.14.0 | [registry source](https://docs.rs/crate/itertools/0.14.0/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| itertools-0.14.0-LICENSE-MIT | itertools 0.14.0 | [registry source](https://docs.rs/crate/itertools/0.14.0/source/LICENSE-MIT) | `7576269ea71f767b99297934c0b2367532690f8c4badc695edf8e04ab6a1e545` |
| liquid-0.26.11-LICENSE | liquid 0.26.11 | [registry source](https://docs.rs/crate/liquid/0.26.11/source/LICENSE) | `7b4b09e856fad3b9d6e480a9ec8ccaf85080c5ad8e536aade8fe30b2c533a779` |
| liquid-0.26.11-LICENSE-APACHE | liquid 0.26.11 | [registry source](https://docs.rs/crate/liquid/0.26.11/source/LICENSE-APACHE) | `c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08` |
| liquid-0.26.11-LICENSE-MIT | liquid 0.26.11 | [registry source](https://docs.rs/crate/liquid/0.26.11/source/LICENSE-MIT) | `6efb0476a1cc085077ed49357026d8c173bf33017278ef440f222fb9cbcb66e6` |
| liquid-core-0.26.11-LICENSE-APACHE | liquid-core 0.26.11 | [registry source](https://docs.rs/crate/liquid-core/0.26.11/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| liquid-core-0.26.11-LICENSE-MIT | liquid-core 0.26.11 | [registry source](https://docs.rs/crate/liquid-core/0.26.11/source/LICENSE-MIT) | `6a5dfb0adf37850239f4b2388a79355c77b625d6c5542dea7743ac5033efaba2` |
| liquid-lib-0.26.11-LICENSE | liquid-lib 0.26.11 | [registry source](https://docs.rs/crate/liquid-lib/0.26.11/source/LICENSE) | `7b4b09e856fad3b9d6e480a9ec8ccaf85080c5ad8e536aade8fe30b2c533a779` |
| maplit-1.0.2-LICENSE-APACHE | maplit 1.0.2 | [registry source](https://docs.rs/crate/maplit/1.0.2/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| maplit-1.0.2-LICENSE-MIT | maplit 1.0.2 | [registry source](https://docs.rs/crate/maplit/1.0.2/source/LICENSE-MIT) | `7576269ea71f767b99297934c0b2367532690f8c4badc695edf8e04ab6a1e545` |
| matrixmultiply-0.3.11-LICENSE-APACHE | matrixmultiply 0.3.11 | [registry source](https://docs.rs/crate/matrixmultiply/0.3.11/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| matrixmultiply-0.3.11-LICENSE-MIT | matrixmultiply 0.3.11 | [registry source](https://docs.rs/crate/matrixmultiply/0.3.11/source/LICENSE-MIT) | `792d075c7bad6dac258a44e799eb64cbf465e24d9932d27669be08c5ec957e27` |
| ndarray-0.16.1-LICENSE-APACHE | ndarray 0.16.1 | [registry source](https://docs.rs/crate/ndarray/0.16.1/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| ndarray-0.16.1-LICENSE-MIT | ndarray 0.16.1 | [registry source](https://docs.rs/crate/ndarray/0.16.1/source/LICENSE-MIT) | `1fd6747d2c8e80f9fa766f57c5888864774621deb85cc2838ccaed727db32d45` |
| nom-language-0.1.0-LICENSE | nom-language 0.1.0 | [pinned upstream](https://raw.githubusercontent.com/rust-bakery/nom/2cec1b3e4c9ccac62c902d60c00de6d1549ccbe1/LICENSE) | `4dbda04344456f09a7a588140455413a9ac59b6b26a1ef7cdf9c800c012d87f0` |
| ordered-multimap-0.7.3-LICENSE | ordered-multimap 0.7.3 | [registry source](https://docs.rs/crate/ordered-multimap/0.7.3/source/LICENSE) | `047c1d2f1c28c30ced89bd0740ff251d8f51512e81b142711f958a0551729ec4` |
| pest-2.9.5-LICENSE-APACHE | pest 2.9.5 | [registry source](https://docs.rs/crate/pest/2.9.5/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| pest-2.9.5-LICENSE-MIT | pest 2.9.5 | [registry source](https://docs.rs/crate/pest/2.9.5/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| pest_stack_guard-1.0.1-LICENSE-APACHE | pest_stack_guard 1.0.1 | [registry source](https://docs.rs/crate/pest_stack_guard/1.0.1/source/LICENSE-APACHE) | `9b40875025d726c3e2f46fdd88c062462dd0f86d4e6e1deb8da067e0ef337cd8` |
| pest_stack_guard-1.0.1-LICENSE-MIT | pest_stack_guard 1.0.1 | [registry source](https://docs.rs/crate/pest_stack_guard/1.0.1/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| prost-0.11.9-LICENSE | prost 0.11.9 | [registry source](https://docs.rs/crate/prost/0.11.9/source/LICENSE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| psm-0.1.32-LICENSE-APACHE | psm 0.1.32 | [registry source](https://docs.rs/crate/psm/0.1.32/source/LICENSE-APACHE) | `965a63a81d9a2fbeb5f9096954dabb49690f9dffcdac9825f675b25c807252a2` |
| psm-0.1.32-LICENSE-MIT | psm 0.1.32 | [registry source](https://docs.rs/crate/psm/0.1.32/source/LICENSE-MIT) | `3e3714aa69bd874601741fd7d7ad5298740cece37778e279fc1ab4451c5a11af` |
| rand_distr-0.4.3-LICENSE-APACHE | rand_distr 0.4.3 | [registry source](https://docs.rs/crate/rand_distr/0.4.3/source/LICENSE-APACHE) | `6df43f6f4b5d4587f3d8d71e45532c688fd168afa5fe89d571cb32fa09c4ef51` |
| rand_distr-0.4.3-LICENSE-MIT | rand_distr 0.4.3 | [registry source](https://docs.rs/crate/rand_distr/0.4.3/source/LICENSE-MIT) | `a771e4354f6b3ad4c92da1a5c9a239b6c291527db869632ecea4f20e24ca1135` |
| rawpointer-0.2.1-LICENSE-APACHE | rawpointer 0.2.1 | [registry source](https://docs.rs/crate/rawpointer/0.2.1/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| rawpointer-0.2.1-LICENSE-MIT | rawpointer 0.2.1 | [registry source](https://docs.rs/crate/rawpointer/0.2.1/source/LICENSE-MIT) | `7576269ea71f767b99297934c0b2367532690f8c4badc695edf8e04ab6a1e545` |
| rubato-0.14.1-LICENSE.txt | rubato 0.14.1 | [registry source](https://docs.rs/crate/rubato/0.14.1/source/LICENSE.txt) | `36dbd32f27adb0d2477b3d3f6ecbde9ca9ad8d72e24766bebe3ea631d235a7ef` |
| rust-ini-0.21.3-LICENSE | rust-ini 0.21.3 | [registry source](https://docs.rs/crate/rust-ini/0.21.3/source/LICENSE) | `ccf6244964385d34fef3799aa7792e9f8d35517de026f39a4f43f0e89b2079eb` |
| safetensors-0.6.2-LICENSE | safetensors 0.6.2 | [registry source](https://docs.rs/crate/safetensors/0.6.2/source/LICENSE) | `c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4` |
| scan_fmt-0.2.6-LICENSE | scan_fmt 0.2.6 | [registry source](https://docs.rs/crate/scan_fmt/0.2.6/source/LICENSE) | `4d0814fe61e6458a52ce8c744f23c6e7fbdacd20f4f6452bc5f29b7790a98588` |
| stacker-0.1.25-LICENSE-APACHE | stacker 0.1.25 | [registry source](https://docs.rs/crate/stacker/0.1.25/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| stacker-0.1.25-LICENSE-MIT | stacker 0.1.25 | [registry source](https://docs.rs/crate/stacker/0.1.25/source/LICENSE-MIT) | `378f5840b258e2779c39418f3f2d7b2ba96f1c7917dd6be0713f88305dbda397` |
| string-interner-0.15.0-LICENSE-APACHE | string-interner 0.15.0 | [registry source](https://docs.rs/crate/string-interner/0.15.0/source/LICENSE-APACHE) | `1129d26272a056e37e5e7bd2985b8c72e6acd8f55c899c7a2bc136d13ee172b6` |
| string-interner-0.15.0-LICENSE-MIT | string-interner 0.15.0 | [registry source](https://docs.rs/crate/string-interner/0.15.0/source/LICENSE-MIT) | `7f304506d28acecccba0e2ee0bb6aebdf46385d4a7cd602e18dd1a2f123bba75` |
| tar-0.4.46-LICENSE-APACHE | tar 0.4.46 | [registry source](https://docs.rs/crate/tar/0.4.46/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| tar-0.4.46-LICENSE-MIT | tar 0.4.46 | [registry source](https://docs.rs/crate/tar/0.4.46/source/LICENSE-MIT) | `8ca6b96cea9e67c6c5c63f452c31bd396db8bd2406231fdea5d48ef462b48077` |
| ucd-trie-0.1.7-LICENSE-APACHE | ucd-trie 0.1.7 | [registry source](https://docs.rs/crate/ucd-trie/0.1.7/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| ucd-trie-0.1.7-LICENSE-MIT | ucd-trie 0.1.7 | [registry source](https://docs.rs/crate/ucd-trie/0.1.7/source/LICENSE-MIT) | `0f96a83840e146e43c0ec96a22ec1f392e0680e6c1226e6f3ba87e0740af850f` |
| xattr-1.6.1-LICENSE-APACHE | xattr 1.6.1 (Linux/macOS only) | [registry source](https://docs.rs/crate/xattr/1.6.1/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| xattr-1.6.1-LICENSE-MIT | xattr 1.6.1 (Linux/macOS only) | [registry source](https://docs.rs/crate/xattr/1.6.1/source/LICENSE-MIT) | `8b427f5bc501764575e52ba4f9d95673cf8f6d80a86d0d06599852e1a9a20a36` |

The embedded model `vendor/deepfilternet/models/DeepFilterNet3_onnx.tar.gz` has SHA-256
`c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616`, identical in the
Plus fork. The compile-time-only additions — ar_archive_writer 0.5.3, const-random-macro
0.1.16, derive-new 0.5.9, itertools 0.10.5, liquid-derive 0.26.10, object 0.39.1,
pastey 0.1.1, pest_derive/pest_generator/pest_meta 2.9.5, prost-derive 0.11.9,
syn 1.0.109, tiny-keccak 2.0.2 (CC0-1.0, already a `deny.toml` exception) and
unicode-normalization 0.1.26 — are proc-macros, their dependencies and build helpers that
are not linked into the executable; no texts are retained for them.

Gaps: nom-language 0.1.0 ships no license file in its registry archive or its
`nom-language/` VCS path; the workspace root MIT
[`LICENSE`](https://raw.githubusercontent.com/rust-bakery/nom/2cec1b3e4c9ccac62c902d60c00de6d1549ccbe1/LICENSE)
from the archive's pinned commit is retained instead. liquid-lib 0.26.11 declares
MIT OR Apache-2.0 but ships only the MIT text, and string-interner 0.15.0 ships a short
Apache-2.0 notice rather than the full license text; the shipped files are retained
unmodified. No other runtime support crate is missing a license text.

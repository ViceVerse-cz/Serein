# Third-party notices

Packaging copies the repository's bundled notices, license texts and corresponding component
source without scanning dependencies or checking license coverage. Declared-license checks run
separately in the dedicated license CI job (`cargo xtask licenses`); its failure does not block
package jobs. No per-artifact dependency inventory is generated during packaging.

`assets/licenses/dependencies/PROVENANCE.md` records supplement sources and hashes;
`overrides.json` preserves the reviewed package versions, source identities and known evidence
gaps. Corresponding MPL sources retain their original terms. Supplied reference texts,
declarations and source archives do not resolve missing upstream grants.


In-app release updates use **zip 4.6.1** (MIT) for bounded ZIP extraction with only
flate2/zlib-rs decompression enabled, and the already-resolved **semver 1.0.28**
(MIT OR Apache-2.0) for release ordering. Their upstream license texts are retained
under `assets/licenses/files/zip-4.6.1-LICENSE` and
`assets/licenses/files/semver-1.0.28-LICENSE-MIT`; registry archive checksums are
recorded in `Cargo.lock`. The ZIP crate's fuzz-only arbitrary dependencies are not
part of the normal desktop build.


REST gzip decoding adds **async-compression 0.4.42**, **compression-codecs 0.4.38**
and **compression-core 0.4.32**. Gateway zlib-stream decoding directly uses the
already-resolved **flate2 1.1.10**. Rusqlite statement caching adds **hashlink 0.12.2**.
All five declare **MIT OR Apache-2.0** in their corresponding registry release
manifests; their versions and archive checksums are recorded in `Cargo.lock`.

External browser opening enables eframe's `links` feature and adds **webbrowser 1.2.4**
(MIT OR Apache-2.0). Its MIT license is copied unmodified from the registry release
to `assets/licenses/files/webbrowser-1.2.4-LICENSE-MIT`; `Cargo.lock` records the archive checksum.

Inline MP3/WAV/Ogg Vorbis attachment playback uses **Symphonia 0.6.1** and its core, metadata,
MP3, PCM, RIFF, Ogg, Vorbis and common components (MPL-2.0), plus existing **CPAL 0.18.2** (Apache-2.0)
for output in both default and voice builds. Additional resolved dependencies are
extended 0.1.0 (MIT), lazy_static 1.5.0 (MIT OR Apache-2.0), and regex-lite 0.1.9
(MIT OR Apache-2.0). Optional metadata features, recording and FLAC are disabled.
The unmodified corresponding Symphonia source archives, MPL text, direct component
licenses and provenance are in `assets/licenses/audio`, shipped as `licenses/audio`
in both packages. Recipients can extract the supplied `.crate` archives with `tar`;
source remains under its original MPL-2.0 terms, separately from Serein source.
Attachment playback never opens a microphone; Discord voice transport ships in every build.

The native attachment video adapter also uses **symphonia-codec-aac 0.6.1**
(MPL-2.0). Its unmodified source archive and checksum are included in
`assets/licenses/audio/source` and `assets/licenses/audio/PROVENANCE.md`, alongside
the other Symphonia codecs, and ship through the same package copy step.

The egui main experiment pins the egui/eframe ecosystem to upstream commit
`65e7db3c06d779c60ac56647bdd3011ed8ba1cbd` (version 0.36.2, MIT OR Apache-2.0).
It adds unicode-properties 0.1.4 (MIT/Apache-2.0) and updates glifo to 0.3.0 and
vello_common/vello_cpu to 0.2.0 (Apache-2.0 OR MIT). Epaint bundled fonts and
their separate license obligations are unchanged. Native font fallback uses
egui_system_fonts/fontique and platform font discovery; see docs/dependency-versions.md
for the exact added dependency versions and declared licenses. OS emoji fonts
remain installed system resources and are not bundled or redistributed.

Linux login uses gtk4 0.11.4, webkit6 0.6.1, javascriptcore6 0.6.0, glib 0.22.9 and soup3
0.9.0 Rust bindings (MIT). Windows/macOS retain Wry 0.57.0 (MIT OR Apache-2.0) via a documented
manifest/build patch excluding obsolete Linux dependencies; backend sources are unchanged.
Provenance is in vendor/wry/SEREIN-PATCH.md. Component notices and hashes in assets/licenses/login
are staged in licenses/login in both variants. Binding licenses do not relicense system
GTK/WebKitGTK libraries, which remain external prerequisites under their own terms.

Original Serein code is MIT OR Apache-2.0. Dependencies retain their own copyrights and licenses. The full resolved host dependency/license inventory is in [docs/dependency-versions.md](docs/dependency-versions.md), derived from Cargo metadata. Cargo.lock includes target-specific packages too; final distributors must ship the license texts/notices for the exact binaries they build.

`cargo xtask licenses` enforces the declared-license policy in `deny.toml` with pinned
cargo-deny 0.20.2, offline against locked sources for all features/platforms (including vendored
and development crates). MPL-only components and egui's combined font licenses have exact-version
exceptions; they retain their original obligations. This automated check does not establish
complete license-text/notice coverage, source delivery compliance or native-system-library
redistribution clearance. See CONTRIBUTING.md for installation and fixture checks.

Development-only fuzzing uses cargo-fuzz 0.13.2 (MIT OR Apache-2.0) and
libfuzzer-sys 0.4.13 ((MIT OR Apache-2.0) AND NCSA), from the Rust Fuzz project.
The isolated `fuzz/Cargo.lock` is checked separately by `cargo xtask licenses`; its exact-version
NCSA exception does not apply to application dependencies. These tools and LLVM libFuzzer are
not included in text or voice packages. The registry wrapper retains MIT/Apache license texts
and upstream LLVM attribution in its bundled libFuzzer sources. This declaration is not a
redistribution review of independently distributed fuzz executables.

Core components include egui/eframe/wgpu (MIT OR Apache-2.0), Tokio (MIT), serde (MIT OR Apache-2.0), reqwest (MIT OR Apache-2.0), tokio-tungstenite/tungstenite (MIT OR Apache-2.0 / MIT), rustls and its crypto/provider dependencies, Wry (MIT OR Apache-2.0), keyring (MIT OR Apache-2.0), and rusqlite (MIT) with SQLite (public domain). Consult the resolved inventory for precise expressions and native transitive dependencies, including AWS-LC/BoringSSL notices, ring, Unicode data, and egui’s font licenses.

System frameworks and runtimes (Metal, WebKit/WKWebView, WebView2, GTK/WebKitGTK, OS credential stores) are supplied under their vendors’ terms and are not relicensed here. Text-mode packages contain no libdave, Opus, camera, or microphone implementation. No Discord logos, proprietary fonts, official client binaries/source, or emoji collection are redistributed. Abaddon and Discord Userdoccers were consulted as protocol evidence; no implementation source was copied.

The standard build additionally uses **Davey 0.1.4** (MIT, Snazzah; [upstream commit a1e2e741](https://github.com/Snazzah/davey/tree/a1e2e741bea06bc3b7167a5c3792844b8975993c)), **OpenMLS 0.8.1** (MIT, OpenMLS Authors; [upstream commit 47dbedec](https://github.com/openmls/openmls/tree/47dbedecad0c1fd8eb5368d582250ebfcc1e1ce6)), **CPAL 0.18.2** (Apache-2.0), **opus2 0.4.0** (MIT OR Apache-2.0), **rtrb 0.4.0** (MIT OR Apache-2.0), and **chacha20poly1305 0.10.1** (Apache-2.0 OR MIT). Davey implements DAVE with OpenMLS; this build does not link Discord's C++ libdave. The local `vendor/davey` manifest patch removes OpenMLS browser-only timer features from this native build; all Davey Rust sources are unchanged. Exact provenance and the MIT license are retained there. Its DAVE and transport cryptography dependencies retain their own licenses and are not covered merely by naming these direct components.

The voice build statically links the bundled Opus source from **libopus_sys 0.3.3**. Binding notices include its current MIT license and preserved earlier ISC license. The codec's unmodified `COPYING` and `LICENSE_PLEASE_READ.txt` retain its contributor copyrights, BSD-style redistribution conditions and references to IETF patent statements. These notices are distinct from the binding license; no independent patent or licensing conclusion is claimed.

The direct voice library/codec license and notice texts are collected in [assets/licenses/voice](assets/licenses/voice). [PROVENANCE.md](assets/licenses/voice/PROVENANCE.md) records exact registry-release source paths, pinned upstream license URLs, file SHA-256 values and Cargo.lock archive checksums. Davey/OpenMLS root license texts came from their recorded release commits because their registry archives omit them. The libopus_sys registry archive is the authoritative bundled-source reference because its VCS metadata marks that release tree dirty. This collection is ready for voice-package staging and does not complete the transitive per-artifact redistribution review.

The voice dependency tree also contains the locally patched **hpke-rs 0.6.1**, licensed **MPL-2.0** according to its [release-pinned Cargo manifest](https://github.com/cryspen/hpke-rs/blob/f3463e7530771d7f7116635335c25e7d2d11e861/Cargo.toml). The vendored component is under `vendor/hpke-rs/`; `SEREIN-PATCH.md` describes its SHAKE dependency replacement, small standard-XOF adapter and removal of the unused optional libcrux backend. Its original source remains under MPL-2.0, separately from Serein's MIT/Apache code. Upstream's registry archive and pinned Git tree omit a standalone license file, so an unmodified [canonical Mozilla MPL-2.0 text](https://www.mozilla.org/media/MPL/2.0/index.txt) is provided as `vendor/hpke-rs/LICENSE-MPL-2.0.txt` and `assets/licenses/voice/hpke-rs-LICENSE-MPL-2.0.txt`. This text was supplied from Mozilla, not recovered from a nonexistent upstream file. Voice packages include this corresponding component source under `source/hpke-rs` (inside macOS bundle Resources). Binary distributors must provide recipients access to the corresponding hpke-rs source, including modifications, and retain its notices as required by MPL-2.0; distributing only this license text is insufficient.

Bundled fonts are unmodified and licensed under SIL OFL 1.1: **Inter 3.19** (Regular, Medium, SemiBold; the "hinted for Windows" TrueType builds), Copyright (c) 2016-2020 The Inter Project Authors, "Inter" is a trademark of Rasmus Andersson (https://github.com/rsms/inter); **Noto Sans CJK JP Regular 2.004**, © 2014–2021 Adobe (http://www.adobe.com/); and **Noto Sans Arabic 2.012**, Copyright 2022 The Noto Project Authors (https://github.com/notofonts/arabic). The complete license texts are `assets/fonts/Inter-OFL.txt`, `assets/fonts/NotoSansCJK-LICENSE.txt` and `assets/fonts/NotoSansArabic-OFL.txt` in source, and are staged alongside distribution notices. Provenance, hashes, sizes and coverage limitations are in [assets/README.md](assets/README.md). Their font licenses remain separate from Serein's source-code license.

The initial packaging command stages original licenses and this inventory notice. Complete per-artifact transitive license-text assembly and platform redistribution review remain a release-hardening gate; do not treat a development package as a completed legal/distribution review.

Native Save As uses rfd 0.17.2 (MIT) and its pollster 0.4.0 dependency (MIT OR Apache-2.0). Their unmodified license texts are in `assets/licenses/files` and are staged in `licenses/files` in both package variants.

Single-file upload streaming enables existing reqwest's `stream` feature and Tokio's `fs` feature. The native dependency addition is **tokio-util 0.7.19** (MIT, Tokio Contributors); its unmodified registry `LICENSE` is included as `assets/licenses/files/tokio-util-LICENSE-MIT` and staged in both package variants. Existing futures-util remains MIT OR Apache-2.0. Cargo.lock also gains wasm-streams 0.5.0 through reqwest's wasm-only target declaration; it is not selected by these native builds. See the dated addendum in [the dependency inventory](docs/dependency-versions.md).

Twemoji 17.0.3 graphics © Twitter, Inc. and other contributors are licensed under **CC BY 4.0**, separately from Serein code. Source: https://github.com/jdecked/twemoji/tree/v17.0.3. The bundled images are resized and packed into an atlas; see [assets/twemoji/README.md](assets/twemoji/README.md) for provenance and modifications. The full license is `assets/twemoji/LICENSE-GRAPHICS`, staged in both packages as `licenses/Twemoji-CC-BY-4.0.txt`. No Twemoji JavaScript is bundled.

Interface icons are **Phosphor Icons 2.1.1**, Copyright (c) 2023 Phosphor Icons, licensed under the **MIT License** (npm package `@phosphor-icons/core`, https://github.com/phosphor-icons/core). Seventy-five upstream SVGs plus one derived slashed-headphones glyph are rasterized into `assets/icons/atlas.png`; the unmodified license is `assets/icons/LICENSE`, staged in both packages as `licenses/Phosphor-Icons-MIT.txt`. Ten brand marks (PlayStation, Battle.net, Epic Games, League of Legends, Riot Games, Bungie, Roblox, Crunchyroll, eBay, Bluesky) in the same atlas are **Simple Icons 16.30.0** (npm package `simple-icons`, https://github.com/simple-icons/simple-icons), released under **CC0 1.0**; the license is `assets/icons/LICENSE-SIMPLE-ICONS`, staged as `licenses/Simple-Icons-CC0.txt`. Brand marks remain trademarks of their respective owners. Provenance and file hashes are in [assets/icons/README.md](assets/icons/README.md).

The emoji picker's English names and fully qualified sequences derive from Unicode 17.0
`emoji-test.txt`, © 2025 Unicode, Inc., under Unicode License v3. Provenance and transformations
are in `assets/twemoji/NAMES.md`; `assets/twemoji/LICENSE-UNICODE` is shipped in both packages
as `licenses/Unicode-LICENSE.txt`. Custom server emoji are service content fetched on demand,
not redistributed in this repository; demo custom images are original synthetic shapes.

Native system notifications use **notify-rust 4.18.0** (MIT OR Apache-2.0), with the opt-in modern macOS UserNotifications backend **mac-usernotifications 0.3.1** (MIT OR Apache-2.0). Its manifest also links **mac-notification-sys 0.6.15** (MIT/Apache-2.0), although the legacy notification backend is not selected. Additional host dependencies include **futures-timer 3.0.4** (MIT/Apache-2.0) and **objc2-core-location / objc2-user-notifications 0.3.2** (Zlib OR Apache-2.0 OR MIT). The notification adapters’ unmodified MIT texts, objc2’s upstream licensing notice and pinned source provenance are in [assets/licenses/notifications](assets/licenses/notifications). Linux uses the existing zbus dependency family; Windows uses **tauri-winrt-notification 0.7.3** and **windows 0.61.3** (MIT OR Apache-2.0). OS notification services/frameworks retain their platform terms. Notifications carry only generic Serein text; server content is not bundled in these assets.

Optional voice echo cancellation uses **Sonora 0.2.0**, a Rust port of WebRTC
AEC3, and its sonora-aec3/agc2/common-audio/fft/ns/simd 0.2.0 components
(BSD-3-Clause, WebRTC Project Authors, Arun Raghavan and contributors, and
dignifiedquire). The unmodified workspace license is
`assets/licenses/voice/sonora-LICENSE.txt`, staged by existing voice packaging.
Krisp is not bundled. No extra native SDK or model download is required.
Sonora noise suppression and AGC are not enabled by this integration.

The macOS voice permission adapter additionally uses **objc2-av-foundation
0.3.2** (Zlib OR Apache-2.0 OR MIT) and existing objc2 0.6.4 / block2 0.6.2.
The objc2 upstream licensing notice already retained under
`assets/licenses/notifications` covers these generated framework bindings.
AVFoundation is supplied by macOS and is not redistributed.

Optional microphone noise suppression uses **nnnoiseless 0.5.2**, a Rust port of
Xiph RNNoise, with the built-in model (BSD-3-Clause). Its original COPYING is
`assets/licenses/voice/nnnoiseless-COPYING.txt`. Default crate features are disabled.
New support crates are easyfft 0.4.2, anymap3 1.1.0, array-init 2.1.0,
generic_singleton 0.5.3, primal-check 0.3.4, realfft 3.5.0, rustfft 6.4.1,
strength_reduce 0.2.4 and transpose 0.2.3. Their license declarations and retained
texts/notices are recorded in docs/dependency-versions.md and
assets/licenses/voice/PROVENANCE.md and staged by the existing voice packager.

Optional screen sharing adds **screencapturekit 10.0.3** (MIT OR Apache-2.0) on macOS, **windows-capture 2.0.1** (MIT) on Windows and **openh264 / openh264-sys2 0.9.8** (BSD-2-Clause) for source-built Cisco OpenH264 encoding. It reuses **image 0.25.10** (MIT OR Apache-2.0) for bounded scaling. Native frameworks are supplied by the OS. These dependencies stay behind the existing voice feature. Unmodified available license texts and source provenance are retained in `assets/licenses/voice/PROVENANCE.md`; the noted missing binding license text and existing full per-artifact redistribution review remain outstanding.

Camera sending in the macOS build uses **openh264 0.9.8** and
**openh264-sys2 0.9.8** (BSD-2-Clause, Ralf Biedert), built locally with the
`source` feature. The sys crate bundles **Cisco OpenH264 2.6.0**, as identified
by `upstream/codec/api/wels/codec_ver.h`; its BSD-2-Clause notice is reproduced
below verbatim from that registry archive's `upstream/LICENSE`. The Rust
wrapper manifests and README declare BSD-2-Clause, but their published archives
omit a standalone wrapper license text; the codec notice does not invent one.
Registry archive checksums and exact versions are retained in `Cargo.lock`.
This build does not download or redistribute Cisco's prebuilt codec binaries
and makes no claim to their separately described patent-license coverage.

New support dependencies are **wide 1.7.0** and **safe_arch 1.2.0**
(Zlib OR Apache-2.0 OR MIT), and build-time **nasm-rs 0.3.2**
(MIT OR Apache-2.0). Native capture additionally selects **objc2-core-media
0.3.2** and **objc2-core-video 0.3.2** (Zlib OR Apache-2.0 OR MIT), alongside
existing objc2/objc2-foundation/objc2-av-foundation, block2 and dispatch2
bindings. CoreMedia, CoreVideo and AVFoundation remain macOS system frameworks.
Camera and codec dependencies ship on supported platforms in the standard build. Their
exact per-artifact license collection remains subject to the existing packaging
gate described above; no camera package was produced by this local change.

Cisco OpenH264 2.6.0 notice:

```text
Copyright (c) 2013, Cisco Systems
All rights reserved.

Redistribution and use in source and binary forms, with or without modification,
are permitted provided that the following conditions are met:

* Redistributions of source code must retain the above copyright notice, this
  list of conditions and the following disclaimer.

* Redistributions in binary form must reproduce the above copyright notice, this
  list of conditions and the following disclaimer in the documentation and/or
  other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```
# Community extension runtime (September 12, 2026)

Wasmi 2.0.0 and its core, collections and IR crates are MIT OR Apache-2.0.
The upstream license texts at commit `2970aa871cc1001b57b267ccecdcd1e42306199e`
are bundled under `assets/licenses/files/wasmi-2.0.0-LICENSE-*`.
Wasmparser 0.228.0 is used under its MIT option; its upstream license at
`e66235859a6ec0502bf6f9dcc358953eda4cafcc` is bundled alongside it.
New transitive foldhash 0.1.5, hashbrown 0.15.5, spin 0.9.9 and
string-interner 0.19.0 license texts are copied from their registry packages.
Test-only WAT/WAST/wasm-encoder/wasmparser 0.245.1 use the wasm-tools MIT
license at `76927bf4bdbddf4b15f835c5eddfffbdfe3bdbd5` (`wat-1.245.1-LICENSE-MIT`);
leb128fmt license texts are also bundled. These test tools are not a plugin
compiler shipped to end users. Creator packages carry their own license metadata.

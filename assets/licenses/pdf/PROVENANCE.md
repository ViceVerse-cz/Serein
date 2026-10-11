# Native PDF renderer notices

Unmodified license texts for the pinned PDF renderer and newly resolved dependencies.
These files ship in `licenses/pdf`; registry archive checksums remain in `Cargo.lock`.
Existing reused dependencies keep their existing notices. This inventory is not a
claim that all pre-existing repository license evidence gaps have been resolved.

hayro 0.8.0 is experimental. Its PDF parser and CPU rasterizer run only in the
short-lived resource-limited helper, not the UI process. No PDFium/MuPDF DLL or
external renderer is downloaded, bundled, or discovered at runtime.

## Code

- `alloc-no-stdlib-2.0.4-LICENSE`: SHA-256 `c0c56f26d9c051cac4d200c34c84e7ae9aaa853e01a982a1df08b09931e518ae`; unmodified registry `alloc-no-stdlib-2.0.4/LICENSE`; declared `BSD-3-Clause`.
- `alloc-stdlib-0.2.4-LICENSE`: SHA-256 `c0c56f26d9c051cac4d200c34c84e7ae9aaa853e01a982a1df08b09931e518ae`; unmodified Dropbox/rust-alloc-no-stdlib repository root `LICENSE` at `ae42d22078b98549e987d2f03d12df7b984fde47` (registry omits the file; `.cargo_vcs_info.json` pins this source); declared `BSD-3-Clause`.
- `brotli-8.0.4-LICENSE.BSD-3-Clause`: SHA-256 `c0c56f26d9c051cac4d200c34c84e7ae9aaa853e01a982a1df08b09931e518ae`; unmodified registry `brotli-8.0.4/LICENSE.BSD-3-Clause`; declared `BSD-3-Clause AND MIT`.
- `brotli-8.0.4-LICENSE.MIT`: SHA-256 `3d180008e36922a4e8daec11c34c7af264fed5962d07924aea928c38e8663c94`; unmodified registry `brotli-8.0.4/LICENSE.MIT`; declared `BSD-3-Clause AND MIT`.
- `brotli-decompressor-5.0.3-LICENSE`: SHA-256 `c0c56f26d9c051cac4d200c34c84e7ae9aaa853e01a982a1df08b09931e518ae`; unmodified registry `brotli-decompressor-5.0.3/LICENSE`; declared `BSD-3-Clause/MIT`.
- `fearless_simd-0.7.0-LICENSE-APACHE`: SHA-256 `a6cba85bc92e0cff7a450b1d873c0eaa2e9fc96bf472df0247a26bec77bf3ff9`; unmodified registry `fearless_simd-0.7.0/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `fearless_simd-0.7.0-LICENSE-MIT`: SHA-256 `23c23145f6eac25c28cdcc03d38506a5712a9a8cffb5bd5301457f6173ea696e`; unmodified registry `fearless_simd-0.7.0/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.
- `fearless_simd-1.1.0-LICENSE-APACHE`: SHA-256 `a6cba85bc92e0cff7a450b1d873c0eaa2e9fc96bf472df0247a26bec77bf3ff9`; unmodified registry `fearless_simd-1.1.0/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `fearless_simd-1.1.0-LICENSE-MIT`: SHA-256 `23c23145f6eac25c28cdcc03d38506a5712a9a8cffb5bd5301457f6173ea696e`; unmodified registry `fearless_simd-1.1.0/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.
- `glifo-0.4.0-LICENSE-APACHE`: SHA-256 `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a`; unmodified registry `glifo-0.4.0/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `glifo-0.4.0-LICENSE-MIT`: SHA-256 `3252fc481fda4d2bad9e1ae675065889abd35c8abb500499ac01e70aaffa02b9`; unmodified registry `glifo-0.4.0/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.
- `hayro-0.8.0-LICENSE-APACHE`: SHA-256 `0cec06e0e55fbc3dc5cee4fca9b607f66cb8f4e4dbcf3b3c013594dd156732e9`; unmodified registry `hayro-0.8.0/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `hayro-0.8.0-LICENSE-MIT`: SHA-256 `51b50e6120c344dcc146bbd6375ca1c78e079d97c8bea76e67a53d6ff0efebac`; unmodified registry `hayro-0.8.0/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.
- `hayro-ccitt-0.4.0-LICENSE-APACHE`: SHA-256 `0cec06e0e55fbc3dc5cee4fca9b607f66cb8f4e4dbcf3b3c013594dd156732e9`; unmodified registry `hayro-ccitt-0.4.0/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `hayro-ccitt-0.4.0-LICENSE-MIT`: SHA-256 `51b50e6120c344dcc146bbd6375ca1c78e079d97c8bea76e67a53d6ff0efebac`; unmodified registry `hayro-ccitt-0.4.0/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.
- `hayro-cmap-0.1.0-LICENSE-APACHE`: SHA-256 `0cec06e0e55fbc3dc5cee4fca9b607f66cb8f4e4dbcf3b3c013594dd156732e9`; unmodified registry `hayro-cmap-0.1.0/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `hayro-cmap-0.1.0-LICENSE-MIT`: SHA-256 `51b50e6120c344dcc146bbd6375ca1c78e079d97c8bea76e67a53d6ff0efebac`; unmodified registry `hayro-cmap-0.1.0/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.
- `hayro-interpret-0.8.0-LICENSE-APACHE`: SHA-256 `0cec06e0e55fbc3dc5cee4fca9b607f66cb8f4e4dbcf3b3c013594dd156732e9`; unmodified registry `hayro-interpret-0.8.0/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `hayro-interpret-0.8.0-LICENSE-MIT`: SHA-256 `51b50e6120c344dcc146bbd6375ca1c78e079d97c8bea76e67a53d6ff0efebac`; unmodified registry `hayro-interpret-0.8.0/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.
- `hayro-jbig2-0.3.1-LICENSE-APACHE`: SHA-256 `0cec06e0e55fbc3dc5cee4fca9b607f66cb8f4e4dbcf3b3c013594dd156732e9`; unmodified registry `hayro-jbig2-0.3.1/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `hayro-jbig2-0.3.1-LICENSE-MIT`: SHA-256 `51b50e6120c344dcc146bbd6375ca1c78e079d97c8bea76e67a53d6ff0efebac`; unmodified registry `hayro-jbig2-0.3.1/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.
- `hayro-jpeg2000-0.4.1-LICENSE-APACHE`: SHA-256 `0cec06e0e55fbc3dc5cee4fca9b607f66cb8f4e4dbcf3b3c013594dd156732e9`; unmodified registry `hayro-jpeg2000-0.4.1/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `hayro-jpeg2000-0.4.1-LICENSE-MIT`: SHA-256 `51b50e6120c344dcc146bbd6375ca1c78e079d97c8bea76e67a53d6ff0efebac`; unmodified registry `hayro-jpeg2000-0.4.1/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.
- `hayro-postscript-0.1.0-LICENSE-APACHE`: SHA-256 `0cec06e0e55fbc3dc5cee4fca9b607f66cb8f4e4dbcf3b3c013594dd156732e9`; unmodified registry `hayro-postscript-0.1.0/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `hayro-postscript-0.1.0-LICENSE-MIT`: SHA-256 `51b50e6120c344dcc146bbd6375ca1c78e079d97c8bea76e67a53d6ff0efebac`; unmodified registry `hayro-postscript-0.1.0/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.
- `hayro-syntax-0.8.0-LICENSE-APACHE`: SHA-256 `0cec06e0e55fbc3dc5cee4fca9b607f66cb8f4e4dbcf3b3c013594dd156732e9`; unmodified registry `hayro-syntax-0.8.0/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `hayro-syntax-0.8.0-LICENSE-MIT`: SHA-256 `51b50e6120c344dcc146bbd6375ca1c78e079d97c8bea76e67a53d6ff0efebac`; unmodified registry `hayro-syntax-0.8.0/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.
- `pic-scale-0.7.12-LICENSE-APACHE.md`: SHA-256 `90bf2d659c43045111b65c733ab2a6d4cbcb422a098368c8c58a9ba3db4ed0c5`; unmodified registry `pic-scale-0.7.12/LICENSE-APACHE.md`; declared `BSD-3-Clause OR Apache-2.0`.
- `pic-scale-0.7.12-LICENSE.md`: SHA-256 `2aa92cada6431e75615e3fe6cb1a9082c98f777d48ae1c087c0da0e37f7b8bff`; unmodified registry `pic-scale-0.7.12/LICENSE.md`; declared `BSD-3-Clause OR Apache-2.0`.
- `read-fonts-0.43.3-LICENSE-APACHE`: SHA-256 `eba684204073ed715c4abb48129acdfd2bff8ae48339e6a7da3b341d3027b7f0`; unmodified registry `read-fonts-0.43.3/LICENSE-APACHE`; declared `MIT OR Apache-2.0`.
- `read-fonts-0.43.3-LICENSE-MIT`: SHA-256 `7b4c9a3946dfcea7967582760e963e27799e225dcf20565c8ef55324bb017427`; unmodified registry `read-fonts-0.43.3/LICENSE-MIT`; declared `MIT OR Apache-2.0`.
- `skrifa-0.46.2-LICENSE-APACHE`: SHA-256 `eba684204073ed715c4abb48129acdfd2bff8ae48339e6a7da3b341d3027b7f0`; unmodified registry `skrifa-0.46.2/LICENSE-APACHE`; declared `MIT OR Apache-2.0`.
- `skrifa-0.46.2-LICENSE-MIT`: SHA-256 `7b4c9a3946dfcea7967582760e963e27799e225dcf20565c8ef55324bb017427`; unmodified registry `skrifa-0.46.2/LICENSE-MIT`; declared `MIT OR Apache-2.0`.
- `vello_common-0.3.0-LICENSE-APACHE`: SHA-256 `a6cba85bc92e0cff7a450b1d873c0eaa2e9fc96bf472df0247a26bec77bf3ff9`; unmodified registry `vello_common-0.3.0/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `vello_common-0.3.0-LICENSE-MIT`: SHA-256 `adf157330f722fe7cfd5964a2a6974eff851b718228c6735724a11706e6f4dec`; unmodified registry `vello_common-0.3.0/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.
- `vello_cpu-0.3.0-LICENSE-APACHE`: SHA-256 `a6cba85bc92e0cff7a450b1d873c0eaa2e9fc96bf472df0247a26bec77bf3ff9`; unmodified registry `vello_cpu-0.3.0/LICENSE-APACHE`; declared `Apache-2.0 OR MIT`.
- `vello_cpu-0.3.0-LICENSE-MIT`: SHA-256 `adf157330f722fe7cfd5964a2a6974eff851b718228c6735724a11706e6f4dec`; unmodified registry `vello_cpu-0.3.0/LICENSE-MIT`; declared `Apache-2.0 OR MIT`.

## Embedded resources

Bundled standard PDF fonts are unmodified Foxit/PDFium Type 1 fonts; Adobe CMaps
and the ICC profiles have separate notices below. No system font/file/network
resolver is installed by Serein. `LAB.icc` is generated with LCMS2 per the hayro
asset README, rather than linking LCMS2 into Serein.

- `hayro-interpret-0.8.0-LICENSE_FOXIT`: SHA-256 `b578cdd2345840ada550bd12519533812320d5f1d21cf4c1c7e1b1b0a31c98b7`; unmodified registry `hayro-interpret-0.8.0/assets/LICENSE_FOXIT`.
- `hayro-interpret-0.8.0-CGATS_LICENSE.txt`: SHA-256 `469d87b5fa48f8aaf69e25f4fcac3a309c97dbb7d561b917638abdb640e004d9`; unmodified registry `hayro-interpret-0.8.0/assets/CGATS_LICENSE.txt`.
- `hayro-interpret-0.8.0-README.md`: SHA-256 `060a1d5778f49e9de3050c2bb84e5980a29744d9fe25ce590e76d7614c4f172f`; unmodified registry `hayro-interpret-0.8.0/assets/README.md`.
- `hayro-cmap-0.1.0-LICENSE.txt`: SHA-256 `742665db9c8e1bc72603c6d319ca3e90b83bd6d95202f0cd4ef11068a07a9c29`; unmodified registry `hayro-cmap-0.1.0/assets/LICENSE.txt`.

The bundled CMYK profile notice (`CGATS_LICENSE.txt`) is CC0-1.0. It also matches
the `license` file of saucecontrol/Compact-ICC-Profiles at
`bdd84663061bc4ae95ca70decff54f581e27f702`, checked byte-for-byte. Font and CMap
assets come from the hayro source pinned by its crate `.cargo_vcs_info.json`:
`ea9c81dc70cde40c64205986fdcfcd481fe934d8`.

hayro-jpeg2000 test-only assets are not embedded in Serein; the application only
uses the crate decoder. Serein’s two tiny PDF fixtures are generated synthetic
test/demo documents, not copied user files or hayro test assets.

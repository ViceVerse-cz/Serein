No Discord logos, proprietary fonts or copied client assets are bundled. The interface leads proportional text with three unmodified SIL Open Font License 1.1 Inter faces (Regular, Medium, SemiBold; the heavier two form the `medium`/`semibold` families because egui has no synthetic bold), keeps egui's default faces as fallbacks, and appends two unmodified OFL fallback faces (CJK, Arabic) to every family. They are embedded once in the executable (the CJK face as a zstd archive inflated on first use); there is no runtime font download, system-font scan, or render-time filesystem access.

| Asset | Upstream version | Bytes | Copyright and license |
|---|---|---:|---|
| `fonts/NotoSansCJKjp-Regular.otf.zst` | 2.004 | 12,034,822 (16,467,736 inflated) | © 2014–2021 Adobe; [OFL 1.1](fonts/NotoSansCJK-LICENSE.txt) |
| `fonts/NotoSansArabic.ttf` | 2.012 | 844,676 | Copyright 2022 The Noto Project Authors; [OFL 1.1](fonts/NotoSansArabic-OFL.txt) |
| `fonts/Inter-Regular.ttf` | 3.19 (hinted) | 680,240 | Copyright (c) 2016-2020 The Inter Project Authors; [OFL 1.1](fonts/Inter-OFL.txt) |
| `fonts/Inter-Medium.ttf` | 3.19 (hinted) | 694,512 | Copyright (c) 2016-2020 The Inter Project Authors; [OFL 1.1](fonts/Inter-OFL.txt) |
| `fonts/Inter-SemiBold.ttf` | 3.19 (hinted) | 710,040 | Copyright (c) 2016-2020 The Inter Project Authors; [OFL 1.1](fonts/Inter-OFL.txt) |

Downloaded September 10, 2026 from pinned upstream sources:

- [Noto CJK font](https://github.com/notofonts/noto-cjk/blob/f8d157532fbfaeda587e826d4cd5b21a49186f7c/Sans/OTF/Japanese/NotoSansCJKjp-Regular.otf), [license](https://github.com/notofonts/noto-cjk/blob/f8d157532fbfaeda587e826d4cd5b21a49186f7c/Sans/LICENSE). SHA-256: `68a3fc98800b2a27b371f2fb79991daf3633bd89309d4ffaa6946fd587f375b5`. The repository stores it as `zstd -19 NotoSansCJKjp-Regular.otf` (archive SHA-256 `781dc80b10781f348c6701f059d75fe1e41c3c134eac129926c54e1b905df4ef`); the executable embeds the archive and inflates the unmodified font in memory the first time CJK text is displayed.
- [Noto Sans Arabic font](https://github.com/google/fonts/blob/334b789e33413f3aba4264d9aa6c97f7b94c5a2f/ofl/notosansarabic/NotoSansArabic%5Bwdth%2Cwght%5D.ttf), [license](https://github.com/google/fonts/blob/334b789e33413f3aba4264d9aa6c97f7b94c5a2f/ofl/notosansarabic/OFL.txt). The upstream variable-font filename is shortened locally; font bytes are unchanged. SHA-256: `63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e`.

- [Inter 3.19](https://github.com/rsms/inter/releases/tag/v3.19) static TrueType instances `Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf`, taken from the `Inter Hinted for Windows/Desktop/` directory of the official `Inter-3.19.zip` release archive (SHA-256 `150ab6230d1762a57bebf35dfc04d606ff91598a31d785f7f100356ecdcc0032`), re-fetched September 15, 2026. The archive's `LICENSE.txt` is byte-identical to the bundled `fonts/Inter-OFL.txt`. SHA-256: Regular `529be850e06f62f8904f22bda77e45bde4834498fdbec4ff4201fa3177447a3a`, Medium `6df88fcb83ac96582350f801355c6eff55f15710093e9627fb431caa40521151`, SemiBold `2de533bda937a063c595b07c6bd9b70c8c5087d0649a1c8330f7ac11fcc05602`.

  These are the hinted TrueType builds. Upstream also ships CFF outlines. egui paints
  grayscale coverage, not DirectWrite ClearType. Serein leaves the TrueType interpreter
  off and keeps sub-pixel binning on. Dark mode remaps coverage with
  `FontColorTransferFunction::Gamma(0.5)`. Light mode leaves the transfer function off.
  Inter faces set `FontTweak.hinting` to `Some(false)`. The files stay the hinted
  TrueType builds. The interpreter does not run. Glyph designs and advance widths are
  unchanged, so layout is unaffected.

The five font blobs total **19,397,204 bytes (18.50 MiB)**, below the 20 MiB asset ceiling asserted by `cargo test -p ui bundled_fallbacks`. This raw size is separate from compressed distribution size, font-parser/layout memory and GPU glyph-atlas allocations. Package the license files and third-party copyright notices with the executable.

The focused test checks egui glyph availability for synthetic Japanese kana/kanji, simplified/traditional Chinese, Korean, Arabic, Latin and combining accents in both font families. It does not prove complete Unicode coverage, correct Arabic shaping/bidirectional editing, actual IME behavior, screen-reader behavior, or platform rendering. The single CJK face uses Japanese regional Han forms; locale-specific forms and extended emoji remain incomplete. Fallback glyphs in code blocks are not guaranteed to have the primary monospace font's cell width.

Test detail: egui 0.36.2's [`Font::has_glyph`](https://github.com/emilk/egui/blob/49682f8baa058bf49e011035cfbd6e825f88a5ef/crates/epaint/src/text/font.rs#L663) compares the selected face with the replacement face. Our initial test observed a false negative for `H`; inspecting that implementation explains why valid glyphs on the replacement face fail this query. The test therefore uses egui's already-resolved Skrifa parser to check every sample scalar against the actual configured face charmaps, then separately checks egui's installed CJK/Arabic fallback path. It does not skip missing sample glyphs.

## Icons

`icons/atlas.png` (38,534 bytes, 512×320 RGBA) bundles 37 [Phosphor Icons](https://github.com/phosphor-icons/core) 2.1.1 glyphs under the MIT license; see [icons/README.md](icons/README.md) for the pinned sources, hashes and the `resvg` regeneration command.

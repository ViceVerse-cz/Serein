No Discord logos or proprietary fonts are bundled. The notification sounds are Discord audio assets; see [sound provenance and redistribution limitations](sounds/README.md). The interface leads proportional text with three unmodified SIL Open Font License 1.1 Inter faces (Regular, Medium, SemiBold; the heavier two form the `medium`/`semibold` families because egui has no synthetic bold), keeps egui's default faces as fallbacks, and appends four OFL fallback fonts (CJK collection, Arabic, Math, Symbols 2) to every family. The first three are unmodified; Symbols 2 is a modified subset covering punctuation, arrows, technical, miscellaneous symbols and dingbats (including U+2726). They are embedded once in the executable (the CJK collection as a zstd archive inflated on first use); there is no runtime font download or render-time filesystem access.

| Asset | Upstream version | Bytes | Copyright and license |
|---|---|---:|---|
| `fonts/NotoSansCJK-Regular.ttc.zst` | 2.004 | 12,171,653 (19,484,784 inflated) | © 2014–2021 Adobe; [OFL 1.1](fonts/NotoSansCJK-LICENSE.txt) |
| `fonts/NotoSansArabic.ttf` | 2.012 | 844,676 | Copyright 2022 The Noto Project Authors; [OFL 1.1](fonts/NotoSansArabic-OFL.txt) |
| `fonts/NotoSansMath-Regular.otf` | 3.000 (unhinted) | 479,308 | Copyright 2022 The Noto Project Authors; [OFL 1.1](fonts/NotoSansMath-OFL.txt) |
| `fonts/NotoSansSymbols2-Regular.ttf` | 2.008 (modified subset, unhinted) | 87,460 | Copyright 2022 The Noto Project Authors; [OFL 1.1](fonts/NotoSansSymbols2-OFL.txt) |
| `fonts/Inter-Regular.ttf` | 3.19 (hinted) | 680,240 | Copyright (c) 2016-2020 The Inter Project Authors; [OFL 1.1](fonts/Inter-OFL.txt) |
| `fonts/Inter-Medium.ttf` | 3.19 (hinted) | 694,512 | Copyright (c) 2016-2020 The Inter Project Authors; [OFL 1.1](fonts/Inter-OFL.txt) |
| `fonts/Inter-SemiBold.ttf` | 3.19 (hinted) | 710,040 | Copyright (c) 2016-2020 The Inter Project Authors; [OFL 1.1](fonts/Inter-OFL.txt) |

Downloaded from pinned upstream sources (September 10, 2026 unless specified):

- [Noto CJK collection](https://github.com/notofonts/noto-cjk/blob/523d033d6cb47f4a80c58a35753646f5c3608a78/Sans/OTC/NotoSansCJK-Regular.ttc), downloaded October 7, 2026; [license](https://github.com/notofonts/noto-cjk/blob/523d033d6cb47f4a80c58a35753646f5c3608a78/Sans/LICENSE). SHA-256: `b76b0433203017ca80401b2ee0dd69350349871c4b19d504c34dbdd80541690a`. Stored as `zstd -19 NotoSansCJK-Regular.ttc` (archive SHA-256 `4dbf1ffa2c0f9972f8f342f8462ccd870fecfed6852aeaf61c0352f63dde386c`); the unmodified collection is inflated once on first CJK use. Face indices 0 (JP), 2 (SC), and 3 (TC) select Japanese, Simplified Chinese, and Traditional Chinese glyph forms. Changing the interface language reuses the same shared font bytes.
- [Noto Sans Arabic font](https://github.com/google/fonts/blob/334b789e33413f3aba4264d9aa6c97f7b94c5a2f/ofl/notosansarabic/NotoSansArabic%5Bwdth%2Cwght%5D.ttf), [license](https://github.com/google/fonts/blob/334b789e33413f3aba4264d9aa6c97f7b94c5a2f/ofl/notosansarabic/OFL.txt). The upstream variable-font filename is shortened locally; font bytes are unchanged. SHA-256: `63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e`.
- [Noto Sans Math 3.000](https://github.com/notofonts/math/releases/tag/NotoSansMath-v3.000) unhinted OpenType face from the official release archive (SHA-256 `ac351837b41f8a897f020b97fb0f075ad574c1e9669fb5839ada1f92fd748356`). The font bytes are unchanged. SHA-256: `a3a3904ede36039d4ba8177ec0aa9cf90653e7c7927d74d766bf31ea26e2a7c1`.
- [Noto Sans Symbols 2 2.008](https://github.com/google/fonts/blob/7b6724ac7ececc713e9ba93af309f7520c9a80a3/ofl/notosanssymbols2/NotoSansSymbols2-Regular.ttf), downloaded October 4, 2026 (upstream SHA-256 `7d5fb73b7ca67a6798101741f5d280a3d016a56a197afcd4199dbb57b4b82a21`, 1,233,128 bytes; [license](https://github.com/google/fonts/blob/7b6724ac7ececc713e9ba93af309f7520c9a80a3/ofl/notosanssymbols2/OFL.txt)). Modified with `pyftsubset NotoSansSymbols2-Regular.ttf --unicodes="U+2000-206F,U+20A0-20CF,U+2190-21FF,U+2300-23FF,U+2600-26FF,U+2700-27BF,U+2B00-2BFF,U+FE0F,U+200D" --layout-features=* --no-hinting` to 87,460 bytes (SHA-256 `e468ff9d90a79cd8be88aeed26bf6a27f7bc6d37011e1846e96b2e987485f960`). FontTools 4.66.1 reproduced the bundled subset byte-for-byte on October 7, 2026. Coverage and hinting are reduced; retained glyph outlines are unchanged.

- [Inter 3.19](https://github.com/rsms/inter/releases/tag/v3.19) static TrueType instances `Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf`, taken from the `Inter Hinted for Windows/Desktop/` directory of the official `Inter-3.19.zip` release archive (SHA-256 `150ab6230d1762a57bebf35dfc04d606ff91598a31d785f7f100356ecdcc0032`), re-fetched September 15, 2026. The archive's `LICENSE.txt` is byte-identical to the bundled `fonts/Inter-OFL.txt`. SHA-256: Regular `529be850e06f62f8904f22bda77e45bde4834498fdbec4ff4201fa3177447a3a`, Medium `6df88fcb83ac96582350f801355c6eff55f15710093e9627fb431caa40521151`, SemiBold `2de533bda937a063c595b07c6bd9b70c8c5087d0649a1c8330f7ac11fcc05602`.

  These are the hinted TrueType builds. Upstream also ships CFF outlines. egui paints
  grayscale coverage, not DirectWrite ClearType. Serein leaves the TrueType interpreter
  off and keeps sub-pixel binning on. Dark mode remaps coverage with
  `FontColorTransferFunction::Gamma(0.5)`. Light mode leaves the transfer function off.
  Inter faces set `FontTweak.hinting` to `Some(false)`. The files stay the hinted
  TrueType builds. The interpreter does not run. Glyph designs and advance widths are
  unchanged, so layout is unaffected.

The seven embedded font blobs total **15,667,889 bytes (14.94 MiB)**. Their embedded sizes stay below the 16 MiB ceiling asserted by `cargo test -p ui bundled_fallbacks`. This raw size is separate from compressed distribution size, font-parser/layout memory and GPU glyph-atlas allocations. Package the license files and third-party copyright notices with the executable.

The focused test checks egui glyph availability for synthetic Japanese kana/kanji, simplified/traditional Chinese, Korean, Arabic, mathematical alphanumeric symbols, dingbats (including U+2726), Latin and combining accents in both font families. It does not prove complete Unicode coverage, correct Arabic shaping/bidirectional editing, actual IME behavior, screen-reader behavior, or platform rendering. The CJK collection selects Chinese regional Han forms for the Chinese interface locales and Japanese forms otherwise; extended emoji remain incomplete. Fallback glyphs in code blocks are not guaranteed to have the primary monospace font's cell width.

Test detail: egui 0.36.2's [`Font::has_glyph`](https://github.com/emilk/egui/blob/49682f8baa058bf49e011035cfbd6e825f88a5ef/crates/epaint/src/text/font.rs#L663) compares the selected face with the replacement face. Our initial test observed a false negative for `H`; inspecting that implementation explains why valid glyphs on the replacement face fail this query. The test therefore uses egui's already-resolved Skrifa parser to check every sample scalar against the actual configured face charmaps, then separately checks egui's installed CJK/Arabic fallback path. It does not skip missing sample glyphs.

### Display-name fonts

Discord's Nitro display-name styles name a font by id. Serein bundles unmodified SIL Open Font License 1.1 faces for 14 of them, used only when Settings › Appearance › name styles is on. Each face is embedded in the executable but parsed only the first time a visible styled name uses it; there is no runtime font download. Variable faces render at weight 500. Néo-Castel (id 7) is distributed only through a store checkout and Hexagon (id 17) has no identified source, so those names, Default (id 11) and unknown ids keep the interface font. Whether each face matches the exact build Discord serves is unverified.

| Asset | Discord font id | Bytes | Copyright and license |
|---|---:|---:|---|
| `fonts/Bangers-Regular.ttf` | 1 (Bangers) | 93,148 | Copyright 2010 The Bangers Project Authors; [OFL 1.1](fonts/Bangers-OFL.txt) |
| `fonts/BioRhyme.ttf` | 2 (BioRhyme) | 188,500 | Copyright 2016 The BioRhyme Project Authors; [OFL 1.1](fonts/BioRhyme-OFL.txt) |
| `fonts/CherryBombOne-Regular.ttf` | 3 (Cherry Bomb) | 214,940 | Copyright 2019 The Cherry Bomb Project Authors; [OFL 1.1](fonts/CherryBombOne-OFL.txt) |
| `fonts/Chicle-Regular.ttf` | 4 (Chicle) | 41,032 | Copyright (c) 2011 Angel Koziupa; [OFL 1.1](fonts/Chicle-OFL.txt) |
| `fonts/Compagnon-Roman.otf` | 5 (Compagnon) | 54,376 | Copyright (c) 2018 Juliette Duhé, Léa Pradine, Valentin Papon, Sébastien Riollier and Chloé Lozano; [OFL 1.1](fonts/Compagnon-OFL.txt) |
| `fonts/MuseoModerno.ttf` | 6 (MuseoModerno) | 188,724 | Copyright 2020 The MuseoModerno Project Authors; [OFL 1.1](fonts/MuseoModerno-OFL.txt) |
| `fonts/PixelifySans.ttf` | 8 (Pixelify) | 79,160 | Copyright 2021 The Pixelify Sans Project Authors; [OFL 1.1](fonts/PixelifySans-OFL.txt) |
| `fonts/Ribes-Regular.otf` | 9 (Ribes) | 36,208 | Copyright © 2023 Collletttivo; [OFL 1.1](fonts/Ribes-OFL.txt) |
| `fonts/Sinistre-Regular.otf` | 10 (Sinistre) | 120,700 | Copyright © 2023 Collletttivo; [OFL 1.1](fonts/Sinistre-OFL.txt) |
| `fonts/ZillaSlab-Regular.ttf` | 12 (Zilla Slab) | 270,104 | Copyright 2017 The Mozilla Foundation; [OFL 1.1](fonts/ZillaSlab-OFL.txt) |
| `fonts/PlaypenSans.ttf` | 13 (Playpen Sans) | 1,567,080 | Copyright 2023 The Playpen Sans Project Authors; [OFL 1.1](fonts/PlaypenSans-OFL.txt) |
| `fonts/Orbitron.ttf` | 14 (Orbitron) | 38,576 | Copyright 2018 The Orbitron Project Authors; Reserved Font Name "Orbitron"; [OFL 1.1](fonts/Orbitron-OFL.txt) |
| `fonts/NewRocker-Regular.ttf` | 15 (New Rocker) | 173,224 | Copyright (c) 2011 Pablo Impallari; [OFL 1.1](fonts/NewRocker-OFL.txt) |
| `fonts/Kalam-Regular.ttf` | 16 (Kalam) | 427,360 | Copyright (c) 2014 Indian Type Foundry; [OFL 1.1](fonts/Kalam-OFL.txt) |

Downloaded October 15, 2026 from pinned upstream sources. Variable-font filenames are shortened locally; font bytes are unchanged. `Compagnon-OFL.txt` joins the upstream `COPYRIGHT.md` and `LICENSE.txt`.

- [Bangers](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/bangers/Bangers-Regular.ttf), [license](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/bangers/OFL.txt). SHA-256: `4160a7311de9342674cce9160cde9fcbb30f48190397d86ff1b70b455af65824`.
- [BioRhyme](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/biorhyme/BioRhyme%5Bwdth%2Cwght%5D.ttf), [license](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/biorhyme/OFL.txt). SHA-256: `3ef9bcf73838d59683b4144ddd4687bc9023340ee8e2bec33bc7b3cead65c9f7`.
- [Cherry Bomb](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/cherrybombone/CherryBombOne-Regular.ttf), [license](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/cherrybombone/OFL.txt). SHA-256: `9596c6793eb0335057d655b13751ce7ccb50ef0cd15cb52c58465fb62de2bb8f`.
- [Chicle](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/chicle/Chicle-Regular.ttf), [license](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/chicle/OFL.txt). SHA-256: `2c82494b86d3f03821284d60f2209a33218adde0dad7133ce3eb063b4363372b`.
- [Compagnon](https://gitlab.com/velvetyne/compagnon/-/blob/4f2344df5adb6eaf9ffd9215c5406c0729fb7aa1/fonts/Compagnon-Roman.otf), [license](https://gitlab.com/velvetyne/compagnon/-/blob/4f2344df5adb6eaf9ffd9215c5406c0729fb7aa1/LICENSE.txt). SHA-256: `0750c9f473e03177e5ad966fd2d0de84e21ea9b6086dbd432d43e63566f7131e`.
- [MuseoModerno](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/museomoderno/MuseoModerno%5Bwght%5D.ttf), [license](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/museomoderno/OFL.txt). SHA-256: `d03b027cf5cf72d014c8a405a8ceb83ea1d5d8ae2c64a7f602b131d9973e3503`.
- [Pixelify](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/pixelifysans/PixelifySans%5Bwght%5D.ttf), [license](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/pixelifysans/OFL.txt). SHA-256: `9ba86cd010a4de309d263ceff8e8044092c9db7efda869620cb9ff1c4389e8a5`.
- [Ribes](https://github.com/collletttivo/ribes/blob/e5f58f6ef719ff69b599a3155c66f4cecaed0a0f/fonts/Ribes-Regular.otf), [license](https://github.com/collletttivo/ribes/blob/e5f58f6ef719ff69b599a3155c66f4cecaed0a0f/LICENSE.txt). SHA-256: `b9b5955cf70241c3f0eb4f8ef90347b17e731c81712725c2d7b0d23314f5dac7`.
- [Sinistre](https://github.com/collletttivo/sinistre/blob/fbf85f1130670ff2e1b944a17ec13872996b4305/fonts/Sinistre-Regular.otf), [license](https://github.com/collletttivo/sinistre/blob/fbf85f1130670ff2e1b944a17ec13872996b4305/LICENSE.txt). SHA-256: `f78b2cd9af2f9d67f6a6929d24b0d807f4fb45b2dbf96b045f9a2ed2b76c0695`.
- [Zilla Slab](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/zillaslab/ZillaSlab-Regular.ttf), [license](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/zillaslab/OFL.txt). SHA-256: `41a4626844da9216b031308d4423045c765fe4231d99862d85d7d74509d37703`.
- [Playpen Sans](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/playpensans/PlaypenSans%5Bwght%5D.ttf), [license](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/playpensans/OFL.txt). SHA-256: `e083f9da8be210e08ea982ab510894641aab37590e70620d6db73f8de881ec4a`.
- [Orbitron](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/orbitron/Orbitron%5Bwght%5D.ttf), [license](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/orbitron/OFL.txt). SHA-256: `f42db2dd16e642258e35782916eceb1dcdbea06fb958d77ad71dc5963587e8fd`.
- [New Rocker](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/newrocker/NewRocker-Regular.ttf), [license](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/newrocker/OFL.txt). SHA-256: `28a499ce5b8bcf0652f8b2e25135183eaddf0c3c8c3e251259b075c71d1ef14c`.
- [Kalam](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/kalam/Kalam-Regular.ttf), [license](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/kalam/OFL.txt). SHA-256: `57cecb63d4608019371954274ae1d8c397764debd5b19d4a33c1efa4dc923c0b`.

The 14 display-name faces total **3,493,132 bytes (3.33 MiB)**, asserted below 4 MiB by `cargo test -p ui name_fonts`.

## Icons

`icons/atlas.png` (38,534 bytes, 512×320 RGBA) bundles 37 [Phosphor Icons](https://github.com/phosphor-icons/core) 2.1.1 glyphs under the MIT license; see [icons/README.md](icons/README.md) for the pinned sources, hashes and the `resvg` regeneration command.

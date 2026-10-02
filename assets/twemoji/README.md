# Twemoji artwork

Twemoji graphics by Twitter, Inc. and other contributors, maintained by the
[jdecked/twemoji project](https://github.com/jdecked/twemoji), are licensed under
[Creative Commons Attribution 4.0 International](https://creativecommons.org/licenses/by/4.0/).
The complete license is in [LICENSE-GRAPHICS](LICENSE-GRAPHICS).

Source: [v17.0.3](https://github.com/jdecked/twemoji/releases/tag/v17.0.3), commit
`b6b55fef1e8636b540a6d016a4729ca8cdf2e60b`, verified September 10, 2026.
All 4,009 upstream `assets/72x72/*.png` images are included. Changes: resized with
Lanczos to 30×30 pixels and arranged into a transparent PNG atlas with one pixel
of padding on each edge of every 32×32 cell, then losslessly recompressed with
`oxipng -o max --strip all` (pixels unchanged). The same 4,009 upstream SVGs are retained without modification, individually zstd-compressed for scalable jumbo artwork. No JavaScript runtime is included.

- `atlas.png`: RGBA, 2,048×2,016 pixels, 64 columns, 63 rows; row-major cells.
  Compressed: 5,225,108 bytes; decoded: 16,515,072 bytes (15.75 MiB).
- `vectors.bin`: 4,244,356 bytes; 4,010 little-endian u32 offsets followed by
  individually compressed SVG frames in atlas cell order, each with a 64 KiB
  maximum zstd window and an exact content-size declaration. Each source expands to
  at most 64 KiB. The media worker rasterizes 64/128/256-pixel renditions off-thread;
  inline-sized glyphs retain the atlas.
- Vector SHA-256: `a8c292d6ed7954b6e7f3df6a7fabbedeedb039b1cdd0145efe889e693589f4c0`
- `index.tsv`: UTF-8 Unicode sequence, tab, zero-based cell index; one entry per
  line, sorted by Unicode sequence after removing emoji presentation selectors
  (U+FE0F). Cell indices retain the original upstream sequence order. The renderer
  removes the same selectors when looking up user text, including ZWJ sequences.
- Archive SHA-256: `705d79de1460e5e775f362f0d0f01fbe3ef8d65bf4648c490e4649704584f747`
- Atlas SHA-256: `cd71ddd3d37536b562da9a73a8a19bbce895b4a48358f8946455eed2176c79dc`
- Index SHA-256: `48cc625700dd22dd15134b181f5213b4a467d128bc19967220ae34c0972600fc`

Regenerate from the repository root (Pillow and the `zstd` command are development dependencies only):

```sh
python3 -m venv /tmp/serein-twemoji-venv
/tmp/serein-twemoji-venv/bin/pip install Pillow==11.3.0
/tmp/serein-twemoji-venv/bin/python tools/generate-twemoji.py
```

The generator downloads the commit-pinned archive and verifies its hash before
reading it. Pass `--archive /path/to/archive.tar.gz` to rebuild offline. Its
assertions check the asset count, unique sequences, and source dimensions.
PNG compression bytes can vary with the platform's zlib version; index entries
and decoded pixels are deterministic.

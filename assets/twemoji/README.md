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
`oxipng -o max --strip all` (pixels unchanged). No JavaScript runtime is included.

- `atlas.png`: RGBA, 2,048×2,016 pixels, 64 columns, 63 rows; row-major cells.
  Compressed: 5,225,108 bytes; decoded: 16,515,072 bytes (15.75 MiB).
- `index.tsv`: UTF-8 Unicode sequence, tab, zero-based cell index; one entry per
  line, sorted by Unicode sequence after removing emoji presentation selectors
  (U+FE0F). Cell indices retain the original upstream sequence order. The renderer
  removes the same selectors when looking up user text, including ZWJ sequences.
- Archive SHA-256: `705d79de1460e5e775f362f0d0f01fbe3ef8d65bf4648c490e4649704584f747`
- Atlas SHA-256: `9a437963ac9bdd909e297e74021a7797d5139a78d8b2ac104409dcda6ea8cfbe`
- Index SHA-256: `48cc625700dd22dd15134b181f5213b4a467d128bc19967220ae34c0972600fc`

Regenerate from the repository root (Pillow is a development dependency only):

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

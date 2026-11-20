# Twemoji artwork

Twemoji graphics by Twitter, Inc. and other contributors, maintained by the
[jdecked/twemoji project](https://github.com/jdecked/twemoji), are licensed under
[Creative Commons Attribution 4.0 International](https://creativecommons.org/licenses/by/4.0/).
The complete license is in [LICENSE-GRAPHICS](LICENSE-GRAPHICS).

Source: [v17.0.3](https://github.com/jdecked/twemoji/releases/tag/v17.0.3), commit
`b6b55fef1e8636b540a6d016a4729ca8cdf2e60b`, verified September 10, 2026.
All 4,009 upstream SVGs (`assets/svg/*.svg`, ordered by the matching
`assets/72x72/*.png` names) are included without modification, individually
zstd-compressed. Serein rasterizes each one off-thread when first drawn, at the pixel size
it is drawn. No JavaScript runtime and no pre-rendered images are included.

- `vectors.bin`: 4,244,356 bytes; 4,010 little-endian u32 offsets followed by
  individually compressed SVG frames in cell order, each with a 64 KiB
  maximum zstd window and an exact content-size declaration. Each source expands to
  at most 64 KiB. Renditions of 16 to 256 pixels are rasterized off-thread.
- Vector SHA-256: `a8c292d6ed7954b6e7f3df6a7fabbedeedb039b1cdd0145efe889e693589f4c0`
- `index.tsv`: UTF-8 Unicode sequence, tab, zero-based cell index; one entry per
  line, sorted by Unicode sequence after removing emoji presentation selectors
  (U+FE0F). Cell indices retain the original upstream sequence order. The renderer
  removes the same selectors when looking up user text, including ZWJ sequences.
- Archive SHA-256: `705d79de1460e5e775f362f0d0f01fbe3ef8d65bf4648c490e4649704584f747`
- Index SHA-256: `48cc625700dd22dd15134b181f5213b4a467d128bc19967220ae34c0972600fc`

Regenerate from the repository root (the `zstd` command is a development dependency only):

```sh
python3 tools/generate-twemoji.py
```

The generator downloads the commit-pinned archive and verifies its hash before
reading it. Pass `--archive /path/to/archive.tar.gz` to rebuild offline. Its
assertions check the asset count, unique sequences, and source dimensions.
PNG compression bytes can vary with the platform's zlib version; index entries
and decoded pixels are deterministic.

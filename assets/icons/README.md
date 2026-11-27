# Interface icons

[Phosphor Icons](https://phosphoricons.com) by Helena Zhang and Tobias Fried, Copyright (c)
2023 Phosphor Icons, licensed under the [MIT License](LICENSE). Source: npm package
`@phosphor-icons/core` version 2.1.1 (repository https://github.com/phosphor-icons/core),
fetched through the jsDelivr npm mirror on September 10, 2026. The unmodified license is
`LICENSE` and is staged in both packages as `licenses/Phosphor-Icons-MIT.txt`.

## SVG-first rendering and PNG fallback

`svg/<canonical-name>.svg` contains one standalone glyph per canonical name in
`index.tsv`. The build automatically includes SVGs by canonical filename; replacing or adding
an SVG for an existing icon does not require a Rust asset list change. New canonical icons
still need their runtime mapping and fallback cell. Missing or malformed SVGs fall back to the
corresponding cell in `atlas.png`. The application tints glyphs at draw time; no icon font or
JavaScript is required. Upstream artwork is white so UI tint supplies its color; custom SVG
colors are preserved and multiplied by that tint.

Each SVG has `width="64"`, `height="64"` and `viewBox="0 0 64 64"`. Phosphor
`assets/fill/*.svg` and `assets/bold/*.svg` sources are scaled into a 56×56-unit glyph area,
retaining the atlas's exact 4-unit padding. `headphones-slash` is derived from
`headphones-fill.svg` with a self-contained diagonal knockout mask and a 16-source-unit
round-capped stroke, matching the style of Phosphor's own `*-slash` icons. Full regeneration
also rasterizes the same artwork with `resvg` 0.45.1 into the transparent PNG fallback atlas
with 64×64 cells (8 columns, 14 rows).

At startup, icon installation rasterizes and caches five fixed raster layers with 16, 24, 32,
64 and 128-pixel cells rather than parsing SVGs or rasterizing individual glyphs during UI
rendering. Their total RGBA texture payload is 10,006,528 bytes per egui context, excluding
renderer overhead. Source files are bounded to 256 KiB each and 1 MiB total at build time;
external SVG image references are disabled. The PNG atlas remains the fallback source; SVG
regeneration does not require changing its pixels or cell indices.

The custom `svg/headphones.svg` retains its supplied path, fitted into a centered 49-unit area
to match the original Phosphor headphones' visual size without clipping. The deafened
`svg/headphones-slash.svg` uses the same shape with a round-capped diagonal slash and a transparent
knockout gap. Both icons use white artwork so the caller supplies the theme's deafened or neutral
tint without multiplying it by a hardcoded color.

Ten brand marks Phosphor does not ship (PlayStation, Battle.net, Epic Games, League of Legends,
Riot Games, Bungie, Roblox, Crunchyroll, eBay, Bluesky) come from
[Simple Icons](https://simpleicons.org) npm package `simple-icons` version 16.30.0
(repository https://github.com/simple-icons/simple-icons), released under
[CC0 1.0](LICENSE-SIMPLE-ICONS) and fetched through the same mirror on September 11, 2026.
Their 24-unit glyphs fill the whole view box, so they are drawn at 80 % of the 56-unit glyph area,
centered inside the same padded cell
to match Phosphor's visual weight. Brand marks remain trademarks of their owners; Simple Icons'
legal disclaimer applies. The license file is staged in both packages as
`licenses/Simple-Icons-CC0.txt`.

One repository-drawn glyph, `thread.svg` (four slanted round-capped bars on the same 256-unit
grid), marks threads; it is normalized with the Phosphor set and carries no upstream license.
The source remains `assets/icons/thread.svg`; the generated standalone is `svg/thread.svg`.

`monitor-x.svg` marks stopping this device's screen share. It is Phosphor `monitor-fill.svg`
(MIT, same pinned 2.1.1 package, fetched November 10, 2026) with a 16-unit round-capped X masked
out of the screen where `monitor-arrow-up` draws its arrow, so it pairs with the share glyph.

- `atlas.png`: 512×896 RGBA, 27,326 bytes after lossless `oxipng -o max --strip all`.
  SHA-256 `4640a87f8aedbe48af687404ac8f51c63cd7f1d1a782e46bdcf5bd98a07fa929`.
- `index.tsv`: icon name, tab, zero-based cell; SHA-256
  `06cabde3334f728e164e18f0057b47a0e6c1db1657ff677fa6bca157f3fb824a`.

## Regeneration

Every upstream SVG's SHA-256 is pinned in `tools/generate-icons.py`, which refuses to build
from mismatching files. Downloads use the version-pinned packages at `cdn.jsdelivr.net`.
Regenerate only `svg/*.svg` from the repository root, without resvg and without writing the
PNG atlas, index or license files:

```sh
python3 tools/generate-icons.py --svg-only
```

Regeneration restores upstream-derived artwork and overwrites custom files in `svg/`; preserve
custom replacements (including the headphones) separately before running it.

Normal invocation generates both the standalone SVGs and the PNG atlas, index and licenses:

```sh
cargo install resvg --version 0.45.1 --root /tmp/resvg-tool
python3 tools/generate-icons.py --resvg /tmp/resvg-tool/bin/resvg
```

PNG compression bytes can vary with the resvg/png versions; decoded pixels and cell indices
are deterministic. `cargo test -p ui --lib icons::tests` checks bundled SVG parsing and visible
artwork, cell replacement, preserved colors/transparency, and missing/malformed SVG fallback.

The folder and open-folder glyphs are unmodified Phosphor `folder-fill.svg` and
`folder-open-fill.svg`, fetched from the same pinned 2.1.1 package on September 11, 2026.

The media-viewer caret and download glyphs are unmodified Phosphor `caret-left-bold.svg` and
`download-simple-bold.svg`, fetched from the same pinned 2.1.1 package on September 11, 2026.

The screen-share fullscreen glyph is unmodified Phosphor `corners-out-bold.svg`, fetched from the
same pinned 2.1.1 package on September 27, 2026.

The text context-menu glyphs are unmodified Phosphor `scissors-bold.svg`, `clipboard-text-bold.svg`
and `selection-all-bold.svg`, fetched from the same pinned 2.1.1 package on November 6, 2026.

The `serein-mark` cell is our own artwork, not an upstream icon: it is the solid silhouette of
the Serein chat-wave from `assets/brand/serein-mark.svg`, trimmed to its bounding box so it
matches Phosphor's glyph weight in the cell. It replaced the Discord brand mark that previously
occupied cell 58; no third-party application logo is bundled any more. `tools/generate-icons.py`
pins its SHA-256 like every upstream source and reads it from the repository rather than a
package mirror. Its source viewBox offset is removed before fitting it into the same padded
56-unit glyph area in both the standalone SVG and the fallback atlas.

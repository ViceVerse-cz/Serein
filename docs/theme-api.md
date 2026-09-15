# Theme API

A `.serein-extension` theme package contains a version 1 manifest with
`"kind": "theme"`, empty capabilities/actions, and a `theme` object. There is no
Wasm module. Import the package from Settings > Themes to try it locally.
The existing packages in [`extensions`](../extensions) are complete examples.

Theme package IDs are normalized to ASCII lowercase when parsed, so an imported
`Golden-Theme` uses the same identity as `golden-theme`. Other ID restrictions
(including path separators, non-ASCII characters and reserved device names) still
apply. Plugin IDs and reviewed catalog manifests remain strictly lowercase.

The bundled Themes page also includes these MIT-licensed presets by **a1.lol**:
Golden Theme, BlackTheme (the supplied Katana package), Obsidian Theme and Teal Theme.
Their supplied palettes and control metrics are preserved. Package IDs are normalized
to lowercase, and source links point to this repository, which contains the packages.
BlackTheme supplies dark colors only; light mode inherits the built-in palette while
keeping its shared control metrics. The other three supply both light and dark colors.
Themes remain opt-in and use the existing preview, install, selection and reset flow.
The existing limit of eight installed themes is unchanged; remove an installed theme
before adding another if that limit is reached.

The `light` and `dark` objects each accept `colors` and an optional `backdrop`.
Color values are `#RRGGBB` or `#RRGGBBAA`. Supported color names are:

| Area | Tokens |
| --- | --- |
| Surfaces | `base`, `sidebar`, `chat`, `raised`, `hover`, `selected`, `border` |
| Text | `text_strong`, `text`, `muted`, `link` |
| Actions and states | `accent`, `accent_text`, `positive`, `warning`, `danger` |
| Mentions | `mention_bg`, `mention_text` |

`backdrop` is a two-color array for the existing top-to-bottom background
gradient. Omitted colors fall back to the selected built-in appearance; omitting
`backdrop` uses flat surfaces. The user's custom accent takes precedence.
Small controls and popouts composite translucent surfaces into opaque colors.

The optional `style` object customizes shared native controls in both appearances.
Every field is optional; omitted fields use the defaults below. Distances and
font sizes are whole logical pixels, before the user's display scale.

| Field | Default | Allowed range |
| --- | --- | --- |
| `body_size` | 13 | 10–28 |
| `heading_size` | 18 | 12–40 |
| `button_size` | 13 | 10–28 |
| `small_size` | 11 | 10–28 |
| `monospace_size` | 13 | 10–28 |
| `item_spacing` | [7, 7] | Each axis 0–24 |
| `button_padding` | [11, 5] | Each axis 0–24 |
| `control_height` | 29 | 24–56 |
| `widget_radius` | 8 | 0–24 |
| `window_radius` | 12 | 0–24 |
| `menu_radius` | 12 | 0–24 |

For example, this `theme` value provides a flatter, roomier appearance:

```json
{
  "light": {"colors": {"accent": "#087F8C", "accent_text": "#FFFFFF"}},
  "dark": {"colors": {"chat": "#15252A", "accent": "#55CBD7", "accent_text": "#15252A"}},
  "style": {
    "body_size": 16,
    "button_padding": [16, 8],
    "control_height": 36,
    "widget_radius": 2,
    "window_radius": 4,
    "menu_radius": 4
  }
}
```

Unknown fields and out-of-range metrics are rejected before installation.
Existing color-only packages continue to work. Disabling or resetting a theme
restores built-in control metrics as well as colors; `Ctrl+Shift+F12` is the
emergency reset shortcut.

These metrics affect controls that inherit the shared native style. Custom
painted elements, explicit text sizes, fixed-height rows and per-widget padding
or radius overrides retain their own geometry. Shop thumbnails preview palette
colors, not all control metrics. Themes cannot rearrange application panels,
inject CSS/scripts, load fonts or images, fetch URLs, or change message data.

Run the offline validation/application example with:

```sh
cargo run --locked -p ui --example theme_api
```

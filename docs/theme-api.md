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

The `light` and `dark` objects each accept `colors`, an optional `backdrop`,
and optional `background` image settings.
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
| `body_size` | 15 | 10–28 |
| `heading_size` | 20 | 12–40 |
| `button_size` | 14 | 10–28 |
| `small_size` | 12 | 10–28 |
| `monospace_size` | 14 | 10–28 |
| `item_spacing` | [8, 8] | Each axis 0–24 |
| `button_padding` | [12, 6] | Each axis 0–24 |
| `control_height` | 32 | 24–56 |
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
inject CSS/scripts, load fonts, fetch image URLs, or change message data.

Run the offline validation/application example with:

```sh
cargo run --locked -p ui --example theme_api
```

## Theme maker and embedded backgrounds

Settings > Themes > Create theme opens the editor inside the Themes settings page.
Background and the main colors appear together on one page. Preview and Save stay
above the scrolling controls; additional colors, typography, gradients and sharing
details expand when needed. Choosing Save reveals missing theme details.
The selected image has its own thumbnail. Duplicate and edit
creates a new unreviewed identity from an installed theme. Colors, alpha, gradients,
typography, spacing and corners use the fields and bounds above. Preview in app
temporarily applies the draft and closes Settings to show the normal conversation
view, including the current account's loaded conversations. A persistent
Back to theme editor button restores the saved appearance and reopens the same
draft. Opening Settings again also ends preview; edits remain available. Preview
does not save changes or send messages. Save and apply uses the existing
installed-theme store, while Export writes a portable `.serein-extension` package.
Local saves can replace only a theme previously created by the editor; imported
or reviewed packages must be duplicated first. The eight-theme limit still applies.
A local theme may leave the manifest `source` empty. Catalog entries still require a
credential-free HTTPS source repository. Complete source, author and licensing
metadata before sharing a package, and include attribution required by image licenses.

A theme package may contain one `background_image` field: a JSON byte array holding
a PNG or JPEG, at most 2 MiB compressed. There are no paths or remote image URLs in
the package. Both palettes share this image and can set different image settings:

```json
"background": { "opacity": 25, "fit": "cover", "target": "chat" }
```

Opacity is an integer from 0 to 100. Fit is `cover` (center crop) or `contain`
(centered whole image); defaults are 25 and `cover`. Target is `chat` for the message
area or `window` for the whole-window backdrop. Missing targets retain `window`
for existing packages; newly chosen editor images default to Message area.
A missing setting inherits the
underlying image settings. A supplied background object replaces those settings
when an appearance plugin overlays the selected theme. Plugins cannot supply bytes,
open an image file, or fetch an image URL.

Images are decoded on the extension worker, with at most 4,096 pixels per edge,
4,000,000 pixels, and 32 MiB decoder allocation. Only a static decoded image is used.
Invalid images fail before installation or export. The 16 MiB serialized package
limit includes the embedded byte array. Older packages remain valid and keep their
appearance; older clients may reject packages with the new background fields.

Message-area images paint above the chat surface and below messages, clipped to
the message area; they are visible without changing surface alpha. The message
input and channel header keep their normal surfaces. Whole-window images paint
after the base/gradient and before main surfaces; use sidebar/chat color alpha to
reveal those images. Popouts and small
controls keep their readable composited surfaces. Opacity changes reuse the texture;
changing, removing or resetting the background releases the previous texture.
The custom accent preference still overrides theme accents, and Ctrl+Shift+F12
remains available during editing and app preview.

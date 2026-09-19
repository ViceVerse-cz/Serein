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
| `transparency_blur` | `true` (requires Appearance opt-in) | `true` or `false` |
| `transparency` | user Appearance setting | 0–100 |
| `blur` | user Appearance setting | 0–100 |
| `transparent_all` | user Appearance setting | `true` or `false` |
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
emergency reset shortcut. Reset keeps installed themes available for re-selection;
Disable removes the selected package and its local data.

The Appearance **Transparency & blur** switch is the device-wide opt-in and requires
restarting Serein after enabling or disabling it. Disabled launches use an opaque
native window and GPU surface, with no blur or transparency compositor requests.
Themes cannot enable window effects while this switch is off. Once enabled, the
optional theme `transparency_blur` value can disable effects for that theme;
omitting it permits effects. Theme percentages override the Appearance defaults.
`transparency` controls how much desktop shows through the conversation;
`transparent_all` extends it to sidebars, the server rail, headers and composer.
These values and theme overrides update live within an enabled session.
`blur` at zero disables native compositor blur; nonzero values request it, but the
compositor chooses the exact radius. Systems without native blur keep translucency.
Setting transparency to zero disables blur and restores the native opaque-window
hint where supported; only restarting with the Appearance switch off releases the
alpha-capable GPU surface. X11 cannot change its native hint after window creation.
The synthetic native preview can opt in without saved settings using
`cargo run --locked -p serein --features demo -- --demo --demo-transparency`.

These metrics affect controls that inherit the shared native style. Custom
painted elements, explicit text sizes, fixed-height rows and per-widget padding
or radius overrides retain their own geometry. Shop thumbnails preview palette
colors with omitted values from the built-in preset, not the currently selected
theme or all control metrics. Themes cannot rearrange application panels,
inject CSS/scripts, load fonts, fetch image URLs, or change message data.

Run the offline validation/application example with:

```sh
cargo run --locked -p ui --example theme_api
```

## Theme maker and embedded backgrounds

Settings > Themes > Create theme opens the editor inside the Themes settings page.
Basics, Background, Colors and Advanced tabs keep required identity fields visible
and related controls together. Preview and Save stay above the scrolling controls.
Background shows a clickable synthetic app map: select a top bar, list or message
area to edit that section's opacity. The image is shared, while colors and surface
opacity can differ between Dark and Light. Choosing Save opens the tab with the
first invalid field and shows a nearby error.
The selected image has its own thumbnail. Customize on a bundled or installed theme
creates a new unreviewed identity; bundled originals do not require installation first.
Local themes retain their Edit theme action and save to the same identity.
Installed inactive themes show Use theme, which switches and persists the active
appearance without reinstalling. A filled Active badge identifies the current theme;
Disable remains a separate action that removes the installed theme and its data.
Clicking a bundled or installed theme thumbnail temporarily previews it on the normal app.
Back to themes restores the saved appearance and discards the temporary preview draft;
Customize restores the saved appearance and opens that draft in the editor.
Colors, alpha, gradients,
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

An optional, separate `cover_image` byte array holds a PNG or JPEG for the theme
card in Settings > Themes. It is also capped at 2 MiB, uses the same static-image
decode limits, and shrinks to at most 640 x 360 for the card. The card center-crops
the image to 16:9; removing it restores the automatic palette preview. It has no
effect on the conversation background. Local themes made in the editor, including
copies of installed themes, can be edited and saved with their existing ID. Imported
and bundled themes must be duplicated before saving, so their package is preserved.

```json
"background": {
  "opacity": 100, "fit": "cover", "target": "window",
  "sections": {
    "top_bar": 85, "server_list": 85, "channel_list": 85,
    "message_list": 75, "member_list": 85, "composer": 90
  }
}
```

Opacity is an integer from 0 to 100. Fit is `cover` (center crop) or `contain`
(centered whole image); defaults are 25 and `cover`. Target is `chat` for the message
area or `window` for the whole-window backdrop. Missing targets retain `window`
for existing packages; newly chosen editor images default to the whole window.
The optional `sections` object sets independent surface coverage, from 0 (clear)
to 100 (solid), over that one image. It covers both title/channel bars, the server
rail, the left DM/channel list, message list, right member/search list and the
message input area. These percentages affect surface fills, not their text or
controls. Missing section fields use the defaults shown above. Older packages
without `sections` retain their previous chat/window behavior.
A missing setting inherits the
underlying image settings. A supplied background object replaces those settings
when an appearance plugin overlays the selected theme. Plugins cannot supply bytes,
open an image file, or fetch an image URL.

Images are decoded on the extension worker, with at most 4,096 pixels per edge,
4,000,000 pixels, and 32 MiB decoder allocation. Only a static decoded image is used.
Invalid images fail before installation or export. The 16 MiB serialized package
limit includes both embedded byte arrays. Older packages remain valid and keep their
appearance; older clients may reject packages with the new background fields.

Legacy message-area images paint above the chat surface and below messages, clipped
to the message area. Whole-window images paint after the base/gradient and before
main surfaces. With `sections`, each main surface gets its own coverage, so changing
one section does not change its neighbors. Popouts and small controls keep their
readable composited surfaces. Opacity changes reuse the texture;
changing, removing or resetting the background releases the previous texture.
The custom accent preference still overrides theme accents, and Ctrl+Shift+F12
remains available during editing and app preview.

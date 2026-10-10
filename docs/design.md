# Serein interface direction

Serein follows the familiar three-column messaging layout and density of a modern desktop chat
client, but it is not a visual clone: it uses its own cool blue-grey neutrals, the Serein azure
accent, softer corner radii and its own server-rail selection language. The palette, typography
and spacing live in `crates/ui/src/design.rs`; every view resolves colours through
`design::palette(ui)`.

## Theme tokens and presets

The `Palette` carries surface roles: `base` (title strip and server rail), `sidebar`
(channel and member lists), `chat`, `raised` (composer, cards, search field, popovers), `hover`,
`selected`, `border`, `text_strong`/`text`/`muted`, `link`, `accent` (Serein azure `#1a72e8`),
presence
colours, mention colours and an optional two-stop `backdrop` gradient. `canvas` and `surface`
remain as aliases of `chat` and `sidebar` for older call sites.

A process-wide `Variant` recolours the whole application on top of egui's light/dark preference:

| Preset | Surfaces |
|---|---|
| Serein | House neutral greys: dark (`#0e0e10` / `#141416` / `#18181b` / `#202023`) or light (`#e4e4e7` / `#f2f2f4` / white), following System/Light/Dark |
| Eclipse | Deep black surfaces for OLED displays |
| Slate | Lighter blue-grey surfaces (`#1b1f2a` / `#262b38` / `#2c3140`) |
| Nightfall, Ember, Verdant, Afterglow | Gradient backdrop painted under translucent dark surfaces |

Gradient presets paint a full-window mesh in the background layer each frame and use
translucent panel fills; they always use dark text. Presets are chosen from the account card's
settings menu (swatch row) and persist in the application-wide SQLite `theme_variant` row next
to the light/dark appearance; unknown keys fall back to Serein. The keys written by earlier
builds (`onyx`, `ash`, `midnight-blurple`, `crimson-moon`, `forest`, `sunset`) still resolve to
their renamed presets, so a stored preference survives the rename. `--demo --demo-theme=<key>` and
`--demo-light` open fixtures in a preset for screenshots.

Message cards use a thin tint only on the conversation surface when chat is translucent.
Search previews, profile code blocks, component modals and opaque component containers keep
their normal card/code fills. Pending upload cards and their preview wells retain their
palette surfaces in both message and forum composers, independently of chat transparency.

## Brand mark and server rail

The application mark is the Serein chat-wave (`assets/brand/`), not a third-party logo. Its
silhouette is rasterized into the shared icon atlas as `serein-mark` and painted wherever the
client identifies itself: the loading screen, the sign-in card and its button, the login header
and the Direct Messages tile at the top of the server rail.

Server tiles keep one constant rounded-square silhouette instead of morphing between a circle and
a squircle. Selection and unread state are shown by `notifications::rail_indicator`: a 3px accent
underline below the tile (wide when selected, narrower on hover), a short neutral tick for unread,
and a 2px accent ring around the selected tile. Controls use an 8px widget radius and 12px
window/menu radius, softer than the 4px/8px pair the layout started from.

## Typography

Inter (Regular, Medium, SemiBold; SIL OFL 1.1) leads proportional text; egui's default faces and
the bundled Noto CJK/Arabic fallbacks follow in every family. egui has no synthetic bold, so
`design::semibold`/`design::medium` select the heavier families for author names, headings,
channel names and uppercase 12px eyebrows. Body is 15px, small 12px. Until `fonts::install`
marks a context, the weight families resolve to the default face so headless tests never
reference an unknown family.

Appearance → Typography can import one TTF or OTF file up to 8 MiB, apply it immediately,
or reset to Inter. The selected face leads all proportional families; variable fonts use
400/500/600 weights while static fonts retain their supplied weight. Code stays monospace,
and bundled/system missing-glyph fallbacks remain available. A bounded local copy survives
restart and logout; import and persistence failures leave the current font in use.

Text is rasterized by egui on the CPU as grayscale coverage, not by DirectWrite or Core Text.
That cannot reproduce ClearType. `design::apply` turns TrueType hinting off and sub-pixel
binning on. Dark mode remaps coverage with `FontColorTransferFunction::Gamma(0.5)`. Light
mode leaves the transfer function off. Inter faces set `FontTweak.hinting` to `Some(false)`.
The bundled faces remain upstream's hinted TrueType builds. See `assets/README.md`.

## Layout

- 36px title strip (`base`): hidden native title bar on macOS with traffic lights inline, centred
  context title, session status text and an OFFLINE PREVIEW / EXPERIMENTAL pill. Eframe centres the
  native macOS buttons in the active title strip every frame, including after resizing or zooming;
  sign-in and login use their taller headers. Windows and macOS General
  settings can hide the app strip and use native window decorations instead. Linux always omits
  the app strip and defaults to system decorations, with the Adwaita Wayland fallback on GNOME.
  General → Window can hide Linux decorations immediately for tiling window managers; the
  device preference also applies at startup.
- 72px server rail (`base`): 48px home button and server icons (circle, rounded square when
  hovered/selected), white edge pill (8px unread, 20px hover, 40px selected), red mention badges.
- The lists and conversation share one rounded surface beside the rail. Channel sidebar
  (240px default, resizable): 48px header with the server name, category eyebrows with chevrons,
  32px rows with `#`/speaker/forum/thread glyphs, `selected`/`hover` fills, unread edge pill,
  mention badge; DM rows are 44px with 32px avatars. Forum rows open their post archive.
  Account card at the bottom (`raised`, avatar with presence dot, name, status, settings gear).
- Conversation header (48px): channel glyph or DM avatar, semibold name, then icon buttons
  (reload, threads/archive, pins, member list toggle), a 144px search field and DM call/voice
  controls. A thin notice strip appears only for loading/stale/archive/history states.
- Timeline: 16px gutters, 40px avatars, content at 72px, medium-weight author names, 12px muted
  timestamps, `hover` row highlight, date dividers with a centred label, red "New messages"
  divider, floating hover toolbar (react, reply, edit, more) overlapping the row above.
- Composer: rounded `raised` bar with attach (+), placeholder `Message #channel`, emoji picker
  and send icons; a character counter appears within 200 characters of the limit.
- Member list (240px): ONLINE/OFFLINE eyebrows with counts; group DMs show a localized
  Members — N heading including the current account, updated with participant changes. 42px rows with
  presence dots, custom status and hover fill; opens a Members window on narrow layouts.

Confirmed empty guild text, announcement and thread histories show a welcome above the
composer: a circular channel-kind icon, a wrapped semibold channel heading and a short
description, using the active palette. Loading, unavailable, incomplete and historical
pages keep their existing status treatment; pending messages suppress the welcome.
`--features demo -- --demo --demo-empty-channel` previews this state offline;
`--demo-empty-channel-long` exercises a long Unicode name and `--demo-light` selects light mode.

Channel references and bare Discord message links share one inline pill. The leading icon
distinguishes text, announcement, voice, thread, forum, forum post, direct and group
conversations; message links append a chevron and message glyph, or the post name for forum
posts, and links to another joined server lead with its icon and name. Pills wrap like text,
keep one hit region per row and one keyboard stop, and expose a localized accessible label.
Destination names come only from metadata the session may view and read; hidden or unknown
targets show `unknown-channel` or `unknown-conversation`. Untrusted names are sanitized and
bounded before layout. Masked links retain their text, inline/fenced code stays literal, and
hidden spoilers remain concealed. Drag selection copies a link pill's original URL, and two
adjacent copies of the same URL stay separate. Click, Enter and Space activate native
navigation; the timeline centers and briefly highlights the exact target.
`--features demo -- --demo --demo-message-links` exercises these states with an offline
fixture; `--demo-light` selects the light appearance.

Icons are [Phosphor Icons](https://phosphoricons.com) 2.1.1 (MIT) in the fill/bold weights,
rasterized once into `assets/icons/atlas.png` (37 white glyphs in 64px cells) and tinted at
draw time by `crates/ui/src/icons.rs`; there is no icon font. Provenance and the regeneration
command are in `assets/icons/README.md`. The profile popout keeps its 300px Discord-style card.

Shortcut capture also accepts egui's browser, media and system keys, including Caps Lock,
Pause and the macOS Fn key, when the OS delivers them to the focused window. These bindings
persist through the existing device settings. Global availability remains limited by the
native shortcut adapter; unsupported global bindings continue to work while Serein is focused.

Voice follows Discord's call screens: a black stage with 80px participant avatars (DM calls,
above the conversation) or 16:9 tiles with name badges (guild channels), a bottom control bar
of dark pills (mute with settings chevron, camera, screen share, activities, soundboard, more)
and a red hang-up button, a green "In a call" badge in the header, mute/deafen toggles in the
account card, and a "Voice Connected" panel above it while connected. The soundboard pill
opens a popup of 146px sound buttons (emoji, elided name) over a volume slider; it is shown
disabled outside server voice channels. Serein has no activities.

## Verification notes

Native macOS captures at 1120×760 in Serein dark, Eclipse, Slate, Nightfall and light were
inspected on September 10, 2026 with the offline fixtures (`--demo`, `--demo-chat`,
`--demo-notifications`, `--demo-voice`). Keyboard reachability of channel rows, the forum row,
toolbar actions and members is covered by headless egui tests. Screen-reader/IME behaviour and
Windows/Linux rendering (including the inline title bar, which is macOS-only) remain unverified.
Palette contrast is asserted by a unit test for every opaque preset: body text ≥ 7:1 on `chat`,
muted text ≥ 4.5:1 on `sidebar`, accent text ≥ 4.5:1 on `accent`. Gradient presets are not
contrast-certified because their surfaces are translucent.

The empty-channel welcome was inspected natively on Ubuntu 26.04.1 at 1120×760
and at 760×520 with a long Unicode name in light/dark mode. The committed pair
uses an isolated Xvfb display; the offline composer was also exercised by typing
and pressing Enter, which replaced the welcome with the synthetic message.

Server Notification Settings are available from the server dropdown and server
icon context menu, independently of administrative Server Settings. The shared
modal uses four radio choices, mute and mention-suppression switches, and an
explicit Save changes action. The content scrolls in short/narrow viewports while
the footer remains accessible. `--demo --demo-chat --demo-server-notifications`
with `--features demo` opens the synthetic editor without contacting Discord.

Poll cards follow the desktop layout shown in Discord's
[Polls FAQ](https://support.discord.com/hc/en-us/articles/22163184112407-Polls-FAQ):
a question above stacked, left-aligned answer rows, right-edge selection circles
and checkmarks, separate vote counts and percentages, and a compact vote/expiry
footer. The card spans up to 472px including padding. Long answers wrap while
their selection markers stay aligned; narrow result rows put tallies beneath the
answer. Colors continue to resolve from the active Serein palette.

Open poll creation from Create a poll in the composer's `+` menu.
The creation dialog uses uppercase field labels, inset question/answer fields,
answer removal icons, a duration row and a separate footer with multiple-choice
and Post controls. Its content scrolls while the footer remains available. Escape,
the close icon and a backdrop click dismiss an idle editor; a pending submission
keeps it open. Unknown results and expired but unfinalized polls retain explicit
status text. `--features demo -- --demo --demo-polls` previews the fixture offline.

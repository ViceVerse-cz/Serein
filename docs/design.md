# Serein interface direction

Serein mirrors the real Discord desktop client's layout and density (the 2025 refresh) while
remaining a native egui application. The palette, typography and spacing live in
`crates/ui/src/design.rs`; every view resolves colours through `design::palette(ui)`.

## Theme tokens and presets

The `Palette` carries Discord-style roles: `base` (title strip and server rail), `sidebar`
(channel and member lists), `chat`, `raised` (composer, cards, search field, popovers), `hover`,
`selected`, `border`, `text_strong`/`text`/`muted`, `link`, `accent` (blurple), presence
colours, mention colours and an optional two-stop `backdrop` gradient. `canvas` and `surface`
remain as aliases of `chat` and `sidebar` for older call sites.

A process-wide `Variant` recolours the whole application on top of egui's light/dark preference:

| Preset | Surfaces |
|---|---|
| Default | Discord refresh dark (`#121214` / `#1a1a1e` / `#222327`) or light (`#e3e5e8` / `#f2f3f5` / white), following System/Light/Dark |
| Onyx | Deep black surfaces for OLED displays |
| Ash | Classic grey Discord surfaces (`#1e1f22` / `#2b2d31` / `#313338`) |
| Midnight Blurple, Crimson Moon, Forest, Sunset | Gradient backdrop painted under translucent dark surfaces |

Gradient presets paint a full-window mesh in the background layer each frame and use
translucent panel fills; they always use dark text. Presets are chosen from the account card's
settings menu (swatch row) and persist in the application-wide SQLite `theme_variant` row next
to the light/dark appearance; unknown keys fall back to Default. `--demo --demo-theme=<key>` and
`--demo-light` open fixtures in a preset for screenshots.

## Typography

Inter (Regular, Medium, SemiBold; SIL OFL 1.1) leads proportional text; egui's default faces and
the bundled Noto CJK/Arabic fallbacks follow in every family. egui has no synthetic bold, so
`design::semibold`/`design::medium` select the heavier families for author names, headings,
channel names and uppercase 12px eyebrows. Body is 15px, small 12px. Until `fonts::install`
marks a context, the weight families resolve to the default face so headless tests never
reference an unknown family.

## Layout

- 36px title strip (`base`): hidden native title bar on macOS with traffic lights inline, centred
  context title, session status text and an OFFLINE PREVIEW / EXPERIMENTAL pill.
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
- Member list (240px): ONLINE/OFFLINE eyebrows with counts (DMs show MEMBERS), 42px rows with
  presence dots, custom status and hover fill; opens a Members window on narrow layouts.

Confirmed empty guild text, announcement and thread histories show a welcome above the
composer: a circular channel-kind icon, a wrapped semibold channel heading and a short
description, using the active palette. Loading, unavailable, incomplete and historical
pages keep their existing status treatment; pending messages suppress the welcome.
`--features demo -- --demo --demo-empty-channel` previews this state offline;
`--demo-empty-channel-long` exercises a long Unicode name and `--demo-light` selects light mode.

Icons are [Phosphor Icons](https://phosphoricons.com) 2.1.1 (MIT) in the fill/bold weights,
rasterized once into `assets/icons/atlas.png` (37 white glyphs in 64px cells) and tinted at
draw time by `crates/ui/src/icons.rs`; there is no icon font. Provenance and the regeneration
command are in `assets/icons/README.md`. The profile popout keeps its 300px Discord-style card.

Voice follows Discord's call screens: a black stage with 80px participant avatars (DM calls,
above the conversation) or 16:9 tiles with name badges (guild channels), a bottom control bar
of dark pills (mute with settings chevron, camera, screen share, activities, soundboard, more)
and a red hang-up button, a green "In a call" badge in the header, mute/deafen toggles in the
account card, and a "Voice Connected" panel above it while connected. Camera, screen share,
activities and soundboard are shown disabled: Serein has no such features.

## Verification notes

Native macOS captures at 1120×760 in Default dark, Onyx, Ash, Midnight Blurple and light were
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

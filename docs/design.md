# Serein interface direction

## Server and personal channel controls — October 23, 2026

Browse Channels is the recovery route for a customized channel list, including an empty list.
Its search, category groups and All / Shown / Hidden filters distinguish personal selection
from muting and missing permissions. Show All Channels preserves individual choices. Channel,
category and forum/thread menus reuse notification, favorite and read controls; hiding alone
changes none of those settings. Account favorites and category collapse synchronize through
account preferences, while device collections remain visibly separate and import explicitly.
Selected channels, active voice and unsuppressed mentions remain discoverable without rewriting
the saved selection.

Server Settings groups searchable destinations under Server, People, Moderation, Community,
Apps and Expressions. Narrow windows use the compact navigation picker. Existing editors
retain their Save/Reset, rejected drafts and explicit reload/reapply workflow. Permissions
control available pages and actions; limited moderators can open Mod View from member,
profile, message or voice contexts without needing server-wide member search.

Mod View stays beside the conversation with identity, roles, join information, timeout and
service-supplied safety details. Recent messages and audit activity load independently and
never replace the timeline or mark it read. Audit activity labels its actual loaded window.
Actions share current hierarchy/permission checks and explicit confirmations. Bulk selection
means the displayed members selected by the user, with a count, target review and history
retention choice. Per-member results distinguish a submitted request, confirmed completion
and an unknown result; Refresh checks state, and Done retires the view.

Protected actions open a local verification dialog for the methods Discord advertises.
Cancel and expiry retire that operation; an SMS is sent only from Send code. Unsupported
passkey-only verification offers the official client. No input is reused for unrelated actions.
Safety Setup shows active service restrictions and their expiry, with explicit resume controls.
Application decisions and interview creation are separate actions. Integrations exposes
application-wide and command overrides with conflict recovery. View Server As Role has a
persistent read-only banner and exit control; it changes only the preview, with voice and
member-specific simulation limits stated. Loading, unavailable and retry states stay within
the owning panel, and hidden panels do not keep page-specific polling or repaint timers.

## Conversation navigation and local time

Mention badges remain visible on pinged channels and posts, including through collapsed
categories and Hide muted channels. Parent badges include child-thread mentions; server
totals count each conversation once. The selected thread comes first, followed by pinged
threads and up to four other recent threads. Forum list and gallery cards reserve title
space for mention counts.

Member rows show the highest-ranked assigned role icon independently of role color.
Composer suggestions separate Members, Roles and permitted special mentions. Nicknames
remain primary; a differing service-provided username distinguishes members in the picker.

Reply previews use service-provided summaries or bounded visible-only reads, independently
of the loaded history page. A jump keeps the current view until its target page succeeds;
Back restores the preceding reading position. Deleted, inaccessible and failed originals
have distinct outcomes, with Retry for temporary preview failures.

Language & Time keeps Auto, 12-hour and 24-hour clock styles and adds System or Custom
timezone. Custom accepts named IANA regions, with daylight-saving rules for each instant,
or fixed UTC offsets. These device-local preferences update absolute timestamps and date
grouping together while preserving the reading anchor. Relative durations and explicitly
labelled UTC inputs retain their meaning.

Discrete wheel motion has a 150 ms completion deadline after the last input; any remaining
movement lands on the next available frame. Precision scrolling uses the operating system's
movement directly. Input reversal and another interaction
retire old motion, and a gesture cannot continue in a different conversation or overlay.

Serein follows the familiar three-column messaging layout and density of a modern desktop chat
client, but it is not a visual clone: it uses its own cool blue-grey neutrals, the Serein azure
accent, softer corner radii and its own server-rail selection language. The palette, typography
and spacing live in `crates/ui/src/design.rs`; every view resolves colours through
`design::palette(ui)`.

## Signal Desk

The desktop uses cool, stepped surfaces to separate navigation, the current workspace, and its
controls. The existing Signal N and azure identify the app, the current destination, and primary
actions. Selected rows use a full outline and tint; keyboard focus has a separate, stronger
outline. Unread, mention, warning, and call states keep their own labels, icons, or badges.
Sign-in, Friends, conversations, settings, dialogs, and the dark call stage share this hierarchy.

Switching conversations briefly extends the accent line under the destination header, then settles
it at the same width every time. Only paint changes: focus, text, media, and the timeline stay put.
There is no entry animation on first paint, and the transition stops on focus loss. Accessibility →
Motion and scrolling → Reduce interface motion uses the settled treatment and skips chrome fades.
This is a device setting, independent of artwork playback and scrolling.

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
| Serein | Signal Desk surfaces: dark (`#0c111a` / `#121a27` / `#171f2c` / `#202c3d`) or light (`#e4ebf6` / `#f0f4fa` / white), following System/Light/Dark |
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

The application mark is NuSerein’s Signal N (`assets/brand/`): an N and waveform in a speech
bubble. The full-color icon appears on the loading and sign-in screens, login header and updates
card. A simplified, tintable version in the shared `serein-mark` atlas cell serves buttons and
the Direct Messages tile at the top of the server rail.

Server tiles keep one constant rounded-square silhouette instead of morphing between a circle and
a squircle. Selection and unread state are shown by `notifications::rail_indicator`: a 3px accent
underline below the tile (wide when selected, narrower on hover), a short neutral tick for unread,
and a 2px accent ring around the selected tile. Controls use an 8px widget radius and 12px
window/menu radius, softer than the 4px/8px pair the layout started from.

## Typography

Inter (Regular, Medium, SemiBold; SIL OFL 1.1) leads proportional text; egui's default faces and
the bundled Noto CJK/Arabic fallbacks follow in every family. egui has no synthetic bold, so
`design::semibold`/`design::medium` select the heavier families for author names, headings,
unread/selected channel names and 12px eyebrows. Category names retain their original case;
read channel names use the regular face. Body is 15px, small 12px. Until `fonts::install`
marks a context, the weight families resolve to the default face so headless tests never
reference an unknown family.

Dark-mode coverage compensation is deliberately light so regular text keeps its intended weight.

Appearance → Typography offers a searchable installed-font picker and a reset to Inter.
The selected TTF or OTF face is validated within a 32 MiB limit and applied immediately.
It leads all proportional families; variable fonts use
400/500/600 weights while static fonts retain their supplied weight. Code stays monospace,
and bundled/system missing-glyph fallbacks remain available. A bounded local copy survives
restart and logout; import and persistence failures leave the current font in use.

Text is rasterized by egui on the CPU as grayscale coverage, not by DirectWrite or Core Text.
That cannot reproduce ClearType. `design::apply` turns TrueType hinting off and sub-pixel
binning on. Dark mode remaps coverage with `FontColorTransferFunction::Gamma(0.8)`. Light
mode leaves the transfer function off. Inter faces set `FontTweak.hinting` to `Some(false)`.
The bundled faces remain upstream's hinted TrueType builds. See `assets/README.md`.

## Responsive controls

Settings headings and descriptions wrap within a reserved Close column. Segmented choices
fit their track at narrow widths, with elided labels available on hover. Add Friend keeps
its navigation fixed while the form scrolls in short windows.

The conversation header collapses its closed search field to a keyboard-accessible icon
in narrow panes. Reply/edit context reserves the action buttons first and elides long names
or status text; hover reveals the full context. Message actions stay inside the timeline
viewport when the start of a tall message scrolls away, without changing its cached height.

## Layout

The sidebar's Inbox button opens one native dialog with Unread, Mentions and Bookmarks
tabs. Filters apply to known accessible conversations and can include muted conversations
or restrict results to the current server/direct-message area. Opening the dialog alone
never acknowledges messages. Read, dismiss, save and reminder changes are explicit;
Write errors preserve the current page unless access or service updates invalidate it;
no write retries happen automatically. Spoiler text
stays concealed, message jumps retain drafts, and Escape releases the session-only page.
Standard reminder intervals and a bounded custom UTC date/time are available where the
account's service accepts saved messages. This is not a local reminder scheduler.

Blocked and ignored message bodies use a native Show/Hide control before the author,
text and media. The same rule applies to thread starters, search/pins, Inbox and forward
previews; loaded reply previews name the restriction. Revealing a body is local to that
view and expires when the content or relationship changes. Friends exposes New Group
DM; group management exposes member refresh, invitations, confirmed owner transfers
and owner-only removals. Threads provide
public/private creation and a member manager; existing forum posts provide a bounded
tag editor. Announcement channels expose Follow with destination selection, and eligible
messages expose confirmed Publish. All of these keep the ordinary conversation draft.

- 36px title strip (`base`): hidden native title bar on macOS with traffic lights inline, centred
  context title, session status text and an OFFLINE PREVIEW / EXPERIMENTAL pill. Eframe centres the
  native macOS buttons in the active title strip every frame, including after resizing or zooming;
  sign-in and login use their taller headers. Windows renders native-fidelity caption buttons
  (minimize, maximize/restore, close) flush in the top-right corner, and resize hit testing excludes
  the caption area so corner clicks hit Close. Windows and macOS General settings can hide the app strip
  and use native window decorations instead. Linux always omits the app strip and defaults to system
  decorations, with the Adwaita Wayland fallback on GNOME. General → Window can hide Linux decorations
  immediately for tiling window managers; the device preference also applies at startup.
- 72px server rail (`base`): 48px home button and server icons (circle, rounded square when
  hovered/selected), white edge pill (8px unread, 20px hover, 40px selected), red mention badges.
- The lists and conversation share one rounded surface beside the rail. Channel sidebar
  (240px default, resizable): 48px header with the server name, category eyebrows with chevrons,
  32px rows with `#`/speaker/forum/thread glyphs, `selected`/`hover` fills, unread edge pill,
  mention badge; DM rows are 44px with 32px avatars. Forum rows open their post archive.
  Account card at the bottom (`raised`, avatar with presence dot, name, status, settings gear).
  During a call, hovering the connection icon/title shows ping and voice-server details.
  Clicking or keyboard-activating it opens connection details and the opt-in Voice Debug view;
  technical counters stay inside that view. Missing measurements show Unavailable.
- Conversation header (48px): channel glyph or DM avatar, semibold name, then icon buttons
  (reload, threads/archive, pins, member list toggle), a 144px search field and DM call/voice
  controls. A thin notice strip appears only for loading/stale/archive/history states.
- Timeline: 16px gutters, 40px avatars, content at 72px, medium-weight author names, 12px muted
  timestamps, inline edited labels that wrap with the body, `hover` row highlight, date dividers
  with a centred label, red "New messages"
  divider, floating hover toolbar (react, reply, edit, more) overlapping the row above.
- Composer: rounded `raised` bar with attach (+), placeholder `Message #channel`, emoji picker
  and send icons; a character counter appears within 200 characters of the limit.
  Typing `@game` offers a game-search option. Selecting it leaves an `@game` filter chip
  and opens a scrollable catalog above the composer; names match case-insensitively by
  prefix and substring, including multiword names. Click, Tab or Enter inserts a game
  name/icon chip while retaining the application reference in the editor's text model.
  Arrow keys select results and Escape dismisses the picker. Loading, no matches and an
  unavailable catalog have explicit states; Retry (or Enter in the failed picker) retries
  a failed lookup. Empty or loading results leave Enter available to send the draft and
  Tab available for normal focus navigation. The catalog also resolves game references
  in messages and reply previews. Custom emojis render as artwork in reply previews as well as
  message bodies; the same visibility and bounded image cache rules apply. Draft mention pills
  resolve current server nicknames consistently with the picker and received messages, while the
  editor retains the original user-ID tokens for selection, copying, undo and sending.
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

Voice & Video places video backend and codec controls after audio, keybinds and
camera controls, at the bottom of the page.

Social link cards place post photos below the text, including portrait photos and related
thumbnail galleries; explicit bot thumbnails retain their side placement. Provider labels
use the small muted style. Embedded video with unknown dimensions reserves a normal
16:9 stage, and pictures keep their aspect ratio. Direct media files use the
native player, including extensionless proxy/offload endpoints. YouTube posters and Play buttons
open its isolated provider player inside the message card on Windows/macOS. Enlarge and fullscreen are
explicit; Close, Retry and confirmed Open original remain available. Linux labels its separate
Serein player action explicitly. Native surfaces follow the card's scroll clip without resizing
its content, and hide behind client menus/dialogs. Offscreen playback pauses at its current
position; returning resumes only a clip that was playing. Source/session/privacy changes retire
playback. Unsupported HTML players show an external-link action rather than an in-app Play icon.
`profile_preview --demo --page=embeds --media-embeds` exercises synthetic social/video cards;
`--provider-embed` selects the YouTube fixture without contacting a provider.

Shortcut capture also accepts egui's browser, media and system keys, including Caps Lock,
Pause and the macOS Fn key, when the OS delivers them to the focused window. These bindings
persist through the existing device settings. Global availability remains limited by the
native shortcut adapter; unsupported global bindings continue to work while Serein is focused.

Voice follows Discord's call screens: a black stage with 80px participant avatars (DM calls,
above the conversation) or 16:9 tiles with name badges (guild channels), a bottom control bar
of dark pills (mute with settings chevron, camera, screen share, activities, soundboard, more)
and a red hang-up button, a green "In a call" badge in the header, mute/deafen toggles in the
account card, and a "Voice Connected" panel above it while connected. Camera and screen
sharing depend on native capture and negotiated codec support; offline previews keep
capture inert. Activities and soundboard remain disabled where unsupported.
The soundboard pill opens a popup of bounded sound buttons and a volume slider; it is
disabled outside server voice channels.

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
explicit Save changes action. Muting offers the standard 15-minute, 1-hour,
3-hour, 8-hour, 24-hour and indefinite durations. A searchable channel/category
chooser exposes notification overrides without requiring administrative permissions;
choosing a source alone never changes its settings. Override choices apply explicitly,
while the server-level draft uses Save changes. The content scrolls in short/narrow viewports while
the footer remains accessible. `--demo --demo-chat --demo-server-notifications`
with `--features demo` opens the synthetic editor without contacting Discord.

Server, category, channel, DM and group-DM menus share the mute-duration choices.
Category mutes apply to descendant channels and threads; timed expiry refreshes
hidden channels and unread indicators even without another Gateway event. Collapsed
categories retain the selected conversation, unmuted unread channels and mentions.
Collapse all / Expand all affects only the current server. Category and folder
Mark as Read actions acknowledge their own readable conversations without navigating.
Alt+Shift+Down/Up moves between unread conversations, skipping muted conversations
unless they contain mentions; Ctrl+Shift+U (Command+Shift+U on macOS) opens the
current unread boundary. These shortcuts are rebindable and respect modal, IME
and key-capture input. Notification clicks open the referenced message through the
normal permission checks and retain existing composer drafts.

The personal notification controls were inspected in the native Linux `profile_preview`
fixture (`--demo --interactive --page=channel-links`), with the installed Inter fonts
and WGPU on an isolated Xvfb display at 1× scale. The matched dark pair in
`docs/pr-evidence/user-controls-qol/` uses 1120×760; the before image is `f9e39f7`
with the demo-only pending-upload tuple repair required to build that fixture.
Native inspection also covered duration selection with Arrow Down/Enter, picker
closure, and scrolling with the footer visible at 760×520 in light and dark mode.
The fixture has no service adapters. Synthetic UI/HTTP regressions cover writes;
live synchronization and OS notification activation on Windows, macOS and Linux remain unverified.

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


Application context commands use the existing user/message context menu's Apps submenu.
Command arguments with autocomplete retain text input and add a keyboard-accessible choices
menu with loading/error/retry states. Suggestions update after a short typing pause and
never submit the command. The ordinary composer preserves its draft and explicit Run action.

Native server settings include AutoMod and Moderation pages. AutoMod edits use typed trigger
and action controls, bounded line lists and explicit save/discard/delete confirmation.
Moderation presents a known-member chooser or explicit lookup, timeout duration, history
retention and a reason in its final confirmation. Role hierarchy and current permissions
control availability. Failed writes keep input; refresh and reapply are explicit after an
unknown outcome. These pages use the existing settings navigation, focus and scrolling.

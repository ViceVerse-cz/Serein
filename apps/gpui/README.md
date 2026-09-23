# Serein GPUI experiment

Run from this worktree:

```sh
cargo run --locked -p serein-gpui -- --demo
```

The offline preview never opens the credential store or connects to Discord. It uses the
existing synthetic fixtures, including channel switching, local message sending and the shared
synthetic member list. Screenshot states: `--demo-channel=21`, `--demo-dm`, `--demo-reply`,
`--demo-hover` and `--demo-sign-in`.

To use your existing account yourself:

```sh
cargo run --locked -p serein-gpui
```

The experiment restores the exact same `cz.viceverse.serein` / `discord-session` OS credential
entry as the main desktop app. No token copying or separate login should be needed when that
entry is valid and the OS permits access. macOS treats this as a new executable and may show a
keychain prompt; the lookup waits up to 60 seconds for it. **Continue with Discord** (after
confirming account ownership) stops the lookup and opens the same ephemeral hosted login as the
main app, attached to the GPUI window on macOS/Windows. New credentials are saved only after
REST authentication and a matching READY. Linux currently restores saved logins; use the main
Serein app for hosted login there. No live account validation was performed.

The app installs a native Serein/Edit/Window menu. On macOS, WKWebView receives Cut, Copy,
Paste and Select All only through the Edit menu's responder-chain actions; the first version had
no menu, so pasting an email or password into the hosted Discord page did nothing.

## What it shows

The layout follows the egui app's metrics and `ui::design` palette with the bundled Inter
faces and Phosphor icons (see `assets/icons/README.md`):

- Server rail with the Serein home tile, initials tiles, the selection/unread pill and tooltips.
- Channel list with collapsible categories, channel-type icons, up to three threads per channel,
  unread pills and mention badges; DMs with avatars, presence and group size. Voice and forum
  channels are listed but open only in the main app.
- Timeline with the egui grouping rule (same author, five minutes, same day), date and
  "New messages" dividers, role-coloured names, BOT/APP badges, hover timestamps, highlighted
  mentions of you, reply previews, forwarded markers, "(edited)", attachment cards, embed cards
  and reactions. Message text uses the main app's bounded Markdown parser
  (`ui::Formatted::spans`): bold, italic, underline, strike, inline and fenced code, quotes,
  headings, subtext, spoilers (click to reveal), links (opened only after a native confirmation),
  resolved `@user`/`@role`/`#channel` mentions and `<t:…>` timestamps.
- Hover toolbar with Reply (reply cap above the composer) and Copy text; clicking a reaction
  toggles it through the existing reducer, which requires a live, fully synced session.
- Composer with a "Message #channel" placeholder, native IME and clipboard, and a send button.
- Member list grouped by hoisted role, online and offline, with presence dots and statuses.
- Older history loads automatically near the top; the newest visible message is acknowledged
  while the window is active, like the main app.
- Problems are transient notices that expire after five seconds, not status lines.

Images, avatars and custom emoji are not downloaded: avatars use the main app's initials
fallback and image attachments appear as file cards with a confirmed "open in browser" action.
Media, voice controls, editing, search, pins, account switching and settings stay in the main
app. Appearance, theme, accent, the desktop-notification opt-in, and per-account drafts and
collapsed categories are remembered in the experiment's own bounded SQLite file
(`serein-gpui/store.sqlite3` in the platform data directory), written by one worker thread;
logging out removes that account's drafts. It never opens or writes the main app's
`serein/client.sqlite3`, and `--demo` touches no disk. There is no logout/credential-deletion control in this prototype.

## GPUI version

GPUI comes from Zed's `main` branch at commit `96e95edac45b0e0696179b8139aa521cf48db392`
(September 22, 2026), pinned by `rev` in `Cargo.toml` and the lockfile, together with the
split `gpui_platform` crate. Update it deliberately by changing both `rev` values and running
`cargo update -p gpui -p gpui_platform`. macOS builds require the Xcode Metal Toolchain. The
wasm-only `gpui_web` crate is replaced by an empty stand-in (`vendor/gpui_web`) because its
exact `unicode-properties` pin conflicts with egui. The composer adapts GPUI's Apache-2.0 native
input example. `discord-voice` remains an unconditional dependency.

The default `serein` executable remains the egui app. `!fast` verification uses only the offline
debug run; no release package, benchmark, full test suite, screenshots, commit, or push.

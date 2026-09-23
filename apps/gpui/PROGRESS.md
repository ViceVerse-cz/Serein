# GPUI frontend — parity progress

Testing-only experiment; this file tracks what the GPUI frontend (`serein-gpui`) matches from the
egui app and what is still missing. Run it with `cargo run -p serein-gpui -- --demo`.

Legend: **[x]** done · **[~]** partial · **[ ]** to do

## Platform and plumbing

- [x] GPUI from Zed `main` at a pinned commit (`gpui` + `gpui_platform`), `vendor/gpui_web` stand-in
- [x] Bundled Inter fonts, Phosphor SVG icons, `ui::design` palette
- [x] Transparent title bar with drag, double-click zoom and `OFFLINE PREVIEW` chip
- [x] Native Serein/Edit/Window menus (Cut/Copy/Paste reach the hosted login webview)
- [x] Wake-on-event backend polling (no fixed 60 Hz timer), bounded event batches
- [x] Member-list gateway subscription
- [x] Transient notices instead of status lines
- [x] Typing events forwarded from the gateway (separate droppable 8-slot queue)
- [~] Image downloads with a bounded cache (`images.rs`: 256 items / 32 MiB LRU, 6 jobs, CDN
      allow-list, fingerprint UA, no redirects): avatars, guild icons and inline image attachments.
      Not yet: embed images/thumbnails, custom emoji, animated images, spoiler images, ThumbHash
      placeholders. Untested against the live CDN.
- [ ] Persisted drafts/settings (main app uses SQLite; the experiment keeps them in memory)
- [ ] Linux hosted login (GTK webview) — Linux restores saved logins only

## Sign-in

- [x] Saved-login restore from the shared OS credential entry (60 s keychain prompt window)
- [x] Consent checkbox, "Continue with Discord", hosted login with a native header and Cancel
- [x] Clipboard in the hosted login (Edit menu fix)
- [ ] Saved-account roster / account switching
- [ ] Session-token disclosure and "Forget saved login"

## Navigation

- [x] Server rail: home tile, initials tiles, selection/hover/unread pill, tooltips
- [ ] Rail mention badges and unread-DM avatars under the home tile
- [ ] Server folders
- [x] Channel list: categories (collapsible), channel-type icons, threads (max 3), unread pills,
      mention badges
- [x] DM list: avatars, presence, group size
- [ ] Friends page on the home tile
- [ ] Channel/server context menus (mark as read, mute, copy link)
- [ ] Voice channels (listed only), forum posts view

## Timeline

- [x] Grouping rule, date and "New messages" dividers, hover timestamps
- [x] Role-coloured names, BOT/APP badges, mention highlight, reply previews, forwarded marker
- [x] Markdown via `ui::Formatted::spans` (code blocks, quotes, headings, spoilers, links, mentions,
      timestamps)
- [x] Attachment cards, embed cards (text), reactions (toggle when live)
- [x] Auto-load older history near the top; mark read at the bottom
- [x] Typing indicator below the composer (shared `ui::typing_segments` wording)
- [x] Edit (inline, Enter/Escape), delete (native confirmation) and pin from the hover toolbar;
      Up in an empty composer edits your last message; Escape cancels a reply
- [x] Jump to present when scrolled up
- [~] Image attachments inline (sized from metadata, click opens after confirmation); video,
      embed images/thumbnails and custom emoji images still to do
- [x] Syntax highlighting in code blocks (parser segments + `ui::design::code_colors_for`), Copy
- [ ] Message components (buttons/selects), polls, stickers
- [ ] Reaction picker / add reaction, reaction user list
- [~] Profile card on author-name click from in-memory data (name, username, status, roles,
      Mention); no profile fetch, bio or mutual servers yet

## Composer

- [x] "Message #channel" placeholder, IME, clipboard, send button, reply cap
- [ ] Attachments / uploads
- [x] `@person` / `#channel` autocomplete (Up/Down, Enter, Escape, click)
- [ ] Emoji picker and `:shortcode:` suggestions, slash commands
- [ ] Multi-line growth limits matching the main app

## People and settings

- [x] Member list: gateway groups, thread/DM grouping, presence, role colours, statuses
- [ ] Lazy member-list paging beyond the first window
- [x] Theme variant, Dark/Light/System and accent presets from the user-panel gear (in memory)
- [ ] Settings window (notifications, privacy, voice)

## Not planned in this experiment

Voice/video calls, screen share, extensions, server administration and the updater stay in the
main egui app.

## Log

- 2026-09-23: typing indicator, inline edit/delete/pin, jump to present, syntax-highlighted code
  blocks, profile card, mention/channel autocomplete, bounded image cache, theme settings.

- 2026-09-23: rebased onto `main` (`f420f1f5`); chat design port, login clipboard fix, Zed `main`
  GPUI, wake-on-event polling, shared `test_support::demo_members` fixture.

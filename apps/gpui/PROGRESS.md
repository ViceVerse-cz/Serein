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
- [ ] Typing events forwarded from the gateway
- [ ] Image downloads (avatars, guild icons, attachments, embeds) with a bounded cache
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
- [ ] Typing indicator above the composer
- [ ] Edit, delete and pin own messages from the hover toolbar; Up to edit last; Escape cancels
- [ ] Jump to present when scrolled up
- [ ] Image/video attachments inline, embed images/thumbnails, custom emoji images
- [ ] Syntax highlighting in code blocks (the parser already computes segments)
- [ ] Message components (buttons/selects), polls, stickers
- [ ] Reaction picker / add reaction, reaction user list
- [ ] Profile popout on avatar/name click

## Composer

- [x] "Message #channel" placeholder, IME, clipboard, send button, reply cap
- [ ] Attachments / uploads
- [ ] Emoji picker, mention autocomplete, slash commands
- [ ] Multi-line growth limits matching the main app

## People and settings

- [x] Member list: gateway groups, thread/DM grouping, presence, role colours, statuses
- [ ] Lazy member-list paging beyond the first window
- [ ] Theme variant and light/dark selection
- [ ] Settings window (notifications, privacy, voice)

## Not planned in this experiment

Voice/video calls, screen share, extensions, server administration and the updater stay in the
main egui app.

## Log

- 2026-09-23: rebased onto `main` (`f420f1f5`); chat design port, login clipboard fix, Zed `main`
  GPUI, wake-on-event polling, shared `test_support::demo_members` fixture.

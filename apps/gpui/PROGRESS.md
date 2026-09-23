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
      Embed images/thumbnails through Discord's `images-ext` proxy (same rules as the main app).
      Custom emoji images in emoji-only messages and reactions (inline ones read `:name:`).
      Not yet: animated images, spoiler images, ThumbHash placeholders. Untested against the live CDN.
- [ ] Persisted drafts/settings (main app uses SQLite; the experiment keeps them in memory)
- [ ] Linux hosted login (GTK webview) — Linux restores saved logins only

## Sign-in

- [x] Saved-login restore from the shared OS credential entry (60 s keychain prompt window)
- [x] Consent checkbox, "Continue with Discord", hosted login with a native header and Cancel
- [x] Clipboard in the hosted login (Edit menu fix)
- [ ] Saved-account roster / account switching
- [x] Log out from the settings popover (confirmation; removes the shared saved login, which
      also signs out the main app)
- [ ] Session-token disclosure

## Navigation

- [x] Server rail: home tile, initials tiles, selection/hover/unread pill, tooltips
- [x] Rail mention badges (99+ cap, ringed, per server/folder via `lights_guild_rail` and
      `mention_count`), home tile request badge (`home_request_parts`, as egui), unread-DM
      48 px avatars under the home tile (`unread_directs`, max 15) with pill and count
- [~] Server folders from `state.guild_folders` (loaded with `load_guild_folders`): collapsed
      2×2 mosaic in the folder colour, expand/collapse (session-only), tinted plate, summed
      badge. Not yet: drag-and-drop reordering, folder name/colour editor, folder menu
- [x] Channel list: categories (collapsible), channel-type icons, threads (max 3), unread pills,
      mention badges
- [x] DM list: avatars, presence, group size
- [~] Friends page from a "Friends" row atop the DM list: Online / All / Pending tabs, presence,
      Message (existing DM only), accept/ignore/cancel via `resolve_friend_request`. Not yet:
      Add Friend, Blocked & Ignored, search, per-friend menu, opening a new DM; the home tile
      still opens the latest DM rather than Friends
- [~] Right-click menus: channel rows and rail DMs (Mark As Read, Copy Link, Copy Channel ID),
      server tiles (Mark As Read, Copy Server ID); Escape/outside click closes. Not yet: mute,
      notification settings, invite, category and folder menus
- [ ] Voice channels (listed only)
- [~] Forum/media channels (`forum.rs`, `--demo-forum`): post cards (unread dot, latest author
      and excerpt from `post_summary`, reply count, "(N New)", last activity), sort by activity or
      creation, "Load more posts"/Retry via `request_forum_posts`, visible-card summaries via
      `request_post_summaries`; a card opens the post as a thread; forum rows light from
      `forum_unread`. Not yet: tags (not in the model), post search, New Post, archived posts,
      author member lookup for role colours, post context menu
- [x] ⌘K / Ctrl+K quick switcher (`ui::switcher_choices` ranking, arrows/Enter/Escape)

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
- [x] "N new messages · Jump to unread" banner while the unread boundary is above the view
- [x] Channel search (header pill, Enter/Escape, results panel, load more, jump to hit) and
      pinned-messages panel via `client_core::search`; no search filters UI (`from:`, `has:`) yet
- [~] Image attachments inline (sized from metadata, click opens after confirmation), embed
      images and thumbnails; video and custom emoji images still to do
- [x] Syntax highlighting in code blocks (parser segments + `ui::design::code_colors_for`), Copy
- [~] Message components: action rows, buttons (styles, link with confirmation), string selects,
      text displays (Markdown), sections, separators, containers via `prepare_component`. Not yet:
      user/role/channel selects, media galleries, files, modal forms; polls and stickers
- [x] Add reaction from the hover toolbar (smiley button, same emoji picker, `prepare_reaction`)
- [ ] Reaction user list
- [x] Profile card on author-name click: local data immediately, then the fetched profile (bio,
      pronouns, server roles) via `request_profile`; account age; Mention. No badges, banner
      image, connections or mutual servers yet

## Composer

- [x] "Message #channel" placeholder, IME, clipboard, send button, reply cap
- [x] Formatting shortcuts (⌘/Ctrl B, I, U, E, ⇧X, ⇧C, ⇧P wrap or unwrap the selection)
- [~] Attachments / uploads (`uploads.rs`): "+" opens the native multi-file panel, files dropped
      on the conversation are added, removable cards (name, size) above the input; Enter sends
      them with the draft via `prepare_send_with_attachments` and `discord_api` `upload_messages`
      on its own backend lane (1 queued, progress watch, cancel), progress line with Cancel,
      failures as notices. Same limits as egui (10 files, 500 MB total, `Source::inspect` name and
      regular-file checks, metadata read on a worker thread). `--demo` fakes the result offline;
      `--demo-attachments` / `--demo-upload-progress` for screenshots. Not yet: thumbnails, paste
      image, spoiler/description, per-account size limits, forum-post files. Untested live
- [x] `@person` / `#channel` autocomplete (Up/Down, Enter, Escape, click)
- [~] Emoji picker from the composer smiley button (`emoji.rs`): search, category tabs, bundled
      Unicode grid (system emoji font, skin-tone variants hidden) and the current server's usable
      emoji listed by name; click appends, Shift-click keeps it open. No custom emoji images,
      skin-tone selector, frequently-used row, stickers or GIFs yet
- [x] `:shortcode` suggestions (`:` + 2 characters, 8 matches, Unicode from `ui::emoji::unicode`
      plus current-server emoji inserted as `<:name:id>` / `<a:name:id>`)
- [ ] Slash commands
- [ ] Multi-line growth limits matching the main app

## People and settings

- [x] Member list: gateway groups, thread/DM grouping, presence, role colours, statuses
- [x] Lazy member-list paging: a `uniform_list` of 42 px rows over the list `total` (headers
      as rows, placeholder rows until a chunk arrives), `member_slot` + window lookup, visible
      range sent through `focus_member_ranges` as `Command::Members`; small lists unchanged.
      Untested against a live large guild
- [x] Theme variant, Dark/Light/System and accent presets from the user-panel gear (in memory)
- [x] Desktop notifications opt-in (settings popover): mentions/DMs from the reducer's filtered
      queue while the window is inactive or elsewhere; clicking opens the channel. No sounds yet
- [ ] Settings window (notifications, privacy, voice)

## Not planned in this experiment

Voice/video calls, screen share, extensions, server administration and the updater stay in the
main egui app.

## Log

- 2026-09-23: attachments/uploads (picker, drop, cards, progress, cancel), forum view, lazy
  member paging, ⌘K switcher, formatting shortcuts, desktop notifications, log out, fetched
  profiles, custom emoji images.

- 2026-09-23: rail badges, unread-DM avatars, server folders, right-click menus, Friends page,
  emoji suggestions/picker/add-reaction, search and pins panel, message components, embed
  images, unread banner, clickable reply previews.

- 2026-09-23: typing indicator, inline edit/delete/pin, jump to present, syntax-highlighted code
  blocks, profile card, mention/channel autocomplete, bounded image cache, theme settings.

- 2026-09-23: rebased onto `main` (`f420f1f5`); chat design port, login clipboard fix, Zed `main`
  GPUI, wake-on-event polling, shared `test_support::demo_members` fixture.

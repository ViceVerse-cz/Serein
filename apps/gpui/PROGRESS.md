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
- [x] Persisted drafts/settings: appearance, theme, accent, notification opt-in, per-account
      drafts and collapsed categories in the experiment's own `serein-gpui/store.sqlite3`
      (never the main app's store; `--demo` writes nothing). Window size is not remembered yet
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
      mention badges; muted channels/DMs dimmed without the unread pill (`channel_access`, as
      egui `channel_marks`); "Hide Muted Channels" drops muted rows and emptied categories
      (`--demo-hide-muted`). Muted servers keep their rail pill dark via `lights_guild_rail`
- [x] DM list: avatars, presence, group size
- [~] Friends page from a "Friends" row atop the DM list: Online / All / Pending / Add Friend
      tabs, presence, Message (opens a new DM through `open_friend_dm` when none exists),
      accept/ignore/cancel via `resolve_friend_request`, Add Friend username field via
      `add_friend` (`--demo-add-friend`), right-click friend menu (`--demo-friend-menu`). Not
      yet: Blocked & Ignored, search, CAPTCHA-challenged requests (cancelled with a notice); the
      home tile still opens the latest DM rather than Friends
- [~] Right-click menus (`nav_menu.rs`; submenus open as a page with Back, Escape/outside click
      closes, Left/Backspace goes back; results as transient notices):
      - channels: Mark As Read, Copy Link, Unmute, Mute Channel (15 min/1 h/3 h/8 h/24 h/until
        turned back on), Notification Settings (All / @mentions / Nothing / Use Category or
        Server Default), Copy Channel ID (`--demo-channel-menu`, `--demo-mute-menu`,
        `--demo-notification-menu`)
      - categories: Mark As Read (each unread channel in turn), Collapse/Expand, Collapse/Expand
        All, Mute Category, Notification Settings, Copy Category ID (`--demo-category-menu`)
      - DMs and groups: Mark As Read, Mute/Unmute Conversation, Close DM, Leave Group (native
        confirmation), Copy Link/ID (`--demo-dm-menu`, `--demo-group-menu`)
      - servers: Mark As Read, Hide Muted Channels, Leave Server (native confirmation, disabled
        with the reducer's reason as tooltip), Copy Server ID (`--demo-server-menu`)
      - friends: Message, Remove Friend and Block (both confirmed), Unblock, Copy User ID
      Not yet: Mute Server and server-level notification settings/suppress @everyone/roles (the
      reducer only reads guild settings; it has no write command), Privacy Settings, invite,
      folder and thread/post menus, mute-until time in the menu (no public getter)
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
      images and thumbnails; videos as sized cards with a play button (open in the browser after
      confirmation; no inline playback)
- [x] Syntax highlighting in code blocks (parser segments + `ui::design::code_colors_for`), Copy
- [~] Message components: action rows, buttons (styles, link with confirmation), string selects,
      text displays (Markdown), sections, separators, containers via `prepare_component`. Not yet:
      user/role/channel selects, media galleries, files, modal forms; polls and stickers
- [x] Add reaction from the hover toolbar (smiley button, same emoji picker, `prepare_reaction`)
- [x] Reaction user list (right-click a reaction; paged via `request_reaction_users`); the
      offline demo toggles reactions in synthetic RAM like the main app
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
- [~] Slash commands (`slash.rs`): `/` opens a picker fed by `request_application_commands`
      (grouped by application, name/options/description, 8-row window, Up/Down/Enter/Escape
      and click); a chosen command shows option chips above the composer (text/integer/number
      inputs, choice/boolean/user/channel/role/mentionable choosers, required `*`, red ring and
      `CommandOption::problem` help line); Enter runs it through `prepare_application_command`,
      errors become notices, interactions time out via `expire_interaction`. Private replies
      render above the composer with "Only you can see this · Dismiss message". `--demo` uses
      a synthetic catalog and replies (`--demo-slash`, `--demo-slash-options`,
      `--demo-slash-reply`). Not yet: live interaction session plumbing in `backend.rs`,
      private replies inside the timeline list, built-in commands (`/shrug`…), autocomplete
      options, attachment options, app icons, modals. Untested live
- [ ] Multi-line growth limits matching the main app

## People and settings

- [x] Member list: gateway groups, thread/DM grouping, presence, role colours, statuses
- [x] Lazy member-list paging: a `uniform_list` of 42 px rows over the list `total` (headers
      as rows, placeholder rows until a chunk arrives), `member_slot` + window lookup, visible
      range sent through `focus_member_ranges` as `Command::Members`; small lists unchanged.
      Untested against a live large guild
- [x] Theme variant, Dark/Light/System and accent presets from the user-panel gear (remembered locally)
- [x] Desktop notifications opt-in (settings popover): mentions/DMs from the reducer's filtered
      queue while the window is inactive or elsewhere; clicking opens the channel. No sounds yet
- [ ] Settings window (notifications, privacy, voice)

## Not planned in this experiment

Voice/video calls, screen share, extensions, server administration and the updater stay in the
main egui app.

## Log

- 2026-09-23: local persistence (own `serein-gpui/store.sqlite3`: theme, notifications opt-in,
  drafts, collapsed categories), slash commands with option chips and private replies, mute and
  notification-level menus, category/DM/group/friend menus, Add Friend, reaction user lists,
  video cards, gateway interaction sessions for live component/command submits.

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

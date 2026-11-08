# Equicord UI Tools (native companion)

This directory is documentation for a **native** Serein adaptation of a batch of small
Equicord workflows. It deliberately contains no `manifest.json` and no packaged Wasm: the
behavior belongs to the desktop interface and its already-bounded session state, not to a
sandboxed extension. The host builds and packages extensions centrally; nothing here should
be added to the catalog.

Upstream: `Equicord/Equicord` at commit
`200dba124066146a8f043de7fc8b05e3dc5e68e`. The upstream sources are JavaScript/TSX written
against Discord's Webpack modules. Serein does not execute them, does not inject code, and
does not claim runtime parity or Discord interoperability. Only the narrow, local, appearance
and composer behavior described below was reimplemented in Rust.

## Implemented in this wave

| Upstream workflow | Native surface | Bounds |
| --- | --- | --- |
| `src/plugins/previewMessage` | Composer **Attach → Preview message** card above the input | Current draft capped to the channel's send limit (2,000 default) and one 160px scroll area; at most the existing 10 staged attachments are named; no upload, network or send. |
| `src/plugins/sortFriendRequests` | Pending friend-request sort and optional receipt date | Uses only relationship rows the gateway already sent (`since`), at most `MAX_RELATIONSHIPS` (10,000) and the existing request byte cap; local date formatting, no fetch. |
| `src/plugins/serverListIndicators` | Compact rail tile between Home's direct messages and the server list | Counts only loaded `friends()`, loaded presences and loaded guilds; the count is cached until generation, relationship view, presence epoch, connection or guild count changes. No presence or guild fetch. |
| `src/plugins/startupTimings` | Read-only **Settings → Updates → Support & diagnostics** rows | At most 8 phases recorded from process launch to first frame; milliseconds only, no account/channel content, no persistence and no upload. |

The diagnostic phase recorder starts once per process, stops at `First frame`, and is not a
replacement for the opt-in diagnostic log.

## Audited as already covered (no edit)

| Upstream workflow | Existing native behavior |
| --- | --- |
| `src/plugins/quickMention` | Message menu inserts a validated `<@id>` token into the draft with send-permission checks (`apply_pending_mention`, covered by an inline test). Never sends. |
| `src/plugins/quickReply` | `ReplyEarlier` / `ReplyLater` and `EditLastMessage` keybinds navigate loaded, supported, non-ephemeral messages and skip blocked/ignored authors. Opening a reply/edit never sends. |
| `src/plugins/replyTimestamp` | `presentation::reply_timestamp` renders the already-resolved referenced message's local clock/calendar in the reply preview. No linked-message fetch. |
| `src/plugins/revealAllSpoilers` | Ctrl-click reveals one message's spoilers; Ctrl+Shift-click reveals loaded visible spoilers through `presentation::request_reveal_all` / `take_reveal_all`. Media spoiler controls remain separate. |
| `src/plugins/roleColorEverywhere` | Loaded member/role colors are applied to authors, replies, mentions, member/voice/reaction rows in `presentation::role_colors` and the existing name-style rendering. No guild-wide metadata fetch. |
| `src/plugins/showTimeoutDuration` | `presentation::timeout_remaining` shows remaining time only when the matching moderation member record is already loaded; nothing is inferred. |
| `src/plugins/stickerPaste` | The existing picker returns `Pick::Sticker`, the host stages it in `staged_sticker`, and only an explicit Send consumes it. Selecting a sticker never sends. |

Re-skinning these as new packages would inflate directory counts without adding behavior.

## Omissions / deliberate deviations

- `previewMessage`: the draft is rendered as bounded plain text, not Discord's rich markdown,
  embeds or image thumbnails. Attachment names and sizes are listed; no local object URLs,
  image decoding or fake message insertion into the timeline. No preview while editing.
- `sortFriendRequests`: receipt dates come from the relationship `since` field, which is
  **restricted/unofficial**. Rows without it keep alphabetical order after dated rows. The
  upstream `showDates` preference is not added; a date is shown whenever the service supplied
  one, because Serein has no per-plugin preference file here.
- `serverListIndicators`: upstream's three-mode setting (friends / servers / both) is not
  added; both counts appear together. Pending guild join requests and unloaded presences are
  not counted or guessed. The tile is not interactive.
- `startupTimings`: phases are process-relative host timestamps, not Discord's performance
  entries. No GPU-driver, font-cache or per-module breakdown, and no manual refresh.
- None of these workflows include arbitrary JS, Webpack/system injection, credential or
  session access, automatic sends, account actions or microphone use.

## Host integration

Native changes are in the existing crates; the parent must compile and review them
sequentially. No new crate, dependency, catalog pin or packaged Wasm is required.

1. `crates/discord-protocol/src/relationships.rs`: `requests_since()` projection and parsing
   assertion.
2. `crates/discord-gateway/src/lib.rs`: emits `RequestsSince` from READY and relationship
   updates.
3. `crates/client-core/src/user_actions.rs`: bounded request-date map, `RequestsSince`
   reducer, `friend_request_since()` accessor, pruning on removal and logout.
4. `crates/ui/src/friends.rs`: request sorting and date label.
5. `crates/ui/src/guild_folders.rs`, `notifications.rs`: rail indicator and cached counts.
6. `crates/ui/src/lib.rs`: composer preview state/card and startup phase recorder.
7. `crates/ui/src/updates.rs`: read-only startup timing rows.
8. `apps/desktop/src/main.rs`: sets the launch instant and records the first frame.
9. `crates/ui/locales/en-US/main.ftl`: English strings; other locales fall back to English.

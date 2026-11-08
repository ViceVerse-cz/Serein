# Wave notes — more-quick-reactions-tools

This directory and commit contain **one** new plugin package,
`more-quick-reactions-tools`, plus the audit below for the eleven upstream paths assigned
to this worktree. The parent owns every central file; this worktree did not edit
`extensions/Cargo.toml`, `extensions/Cargo.lock`, `extensions/catalog.json`,
`extensions/equicord-workflows.json`, the debug starter registry, root docs or generated
catalog pins.

## Parent integration steps

1. Add `plugins/more-quick-reactions-tools` to the `members` list in
   `extensions/Cargo.toml`. This worktree intentionally did not change that shared file.
2. Build with the pinned toolchain and lockfile:
   `cargo build --locked --release --target wasm32-unknown-unknown` from `extensions/`.
3. Package with the shared license section, following the existing Equicord-derived
   packages:
   `python pack.py plugins/more-quick-reactions-tools/manifest.json target/wasm32-unknown-unknown/release/more_quick_reactions_tools.wasm plugins/packages/more-quick-reactions-tools.serein-extension --license ../licenses/Equicord-GPL-3.0.txt`
   (run from `extensions/`; confirm the current `pack.py` flags rather than copying this
   command blindly).
4. Regenerate the catalog/pin through the normal post-merge workflow and, if this wave uses
   the debug starter path, register the reviewed package there. No package bytes are
   committed here.
5. Run the normal sequential checks the parent reserved: `cargo xtask check`, focused
   `cargo test -p more-quick-reactions-tools`, Clippy, catalog validation and package/sandbox
   checks. This agent ran **no** Cargo, rustc, Clippy, build, package, test or screenshot
   command. Only `rustfmt --edition 2024` was applied to the new `src/lib.rs`.
6. If the parent maps this package in `extensions/equicord-workflows.json`, record only the
   implemented subset below. Do not label it upstream JavaScript parity or live Discord
   interoperability.

## Implemented subset

| Upstream path | Result in this worktree |
| --- | --- |
| `src/plugins/moreQuickReactions` | **Implemented as a bounded subset** in `more-quick-reactions-tools`. Explicit 0–42 reaction list, 20 loaded-message choices, one Apply-reviewed `SetReaction`. Hover bar, rows/columns, compact/scroll, picker integration and emoji-frecency source are omitted. |

## Assignment audit

| Upstream path | Disposition | Reason / required owner |
| --- | --- | --- |
| `src/plugins/gifPaste` | Already native — no edit | Serein already has the `gif_insert` preference and `Pick::Insert` path that inserts the validated GIF provider URL plus a space instead of sending (`crates/ui/src/lib.rs`). The inventory already records this as native. Re-skinning it as a plugin would duplicate working behavior. |
| `src/plugins/hideAttachments` | Blocked — native renderer change | Needs per-message gating of attachments, embeds, stickers and component accessories plus a persistent hidden-ID set consulted by `crates/ui/src/timeline.rs`/`attachments.rs` and local storage. The extension SDK has no message-render hook or hidden-media effect; a plugin-only implementation would store IDs that nothing reads. |
| `src/plugins/imageLink` | Covered by existing native preference — no edit | The upstream patch only stops Discord hiding a message consisting solely of an image link. Serein already exposes the inverse through `ReadingPreferences.hide_media_links` and the existing `standalone_media_links` render condition. A plugin toggle would duplicate the native setting and add an Apply step for no new behavior. |
| `src/plugins/ircColors` | Blocked — native renderer change | Requires overriding author and member-list name colours with a deterministic hash at paint time. The SDK exposes no per-user colour or name-style hook; its `Appearance` capability is declarative theme data, not message/member rendering. |
| `src/plugins/jumpToTop` | Blocked — native history/header change | Upstream adds a channel-header button and jumps with an unofficial `messageId = channelId` call to load the channel start. Sandboxed plugins cannot add header controls or fetch a history page. Offering a jump to the earliest *already loaded* row would not implement the workflow and was not faked. |
| `src/plugins/mentionAvatars` | Blocked — native renderer change | Needs 16 px avatars and role icons injected into user and role mention spans. The SDK has no mention-render hook and no image handle for inline mention accessories. |
| `src/plugins/messageClickActions` | Blocked — native input change | Needs configurable single/double/triple pointer timing and dispatch of edit/reply/delete/quote/react in the timeline. Extension actions receive an explicit menu invocation, not pointer gesture or timing data. |
| `src/plugins/messageLatency` | Blocked — missing host data | Upstream compares the message snowflake timestamp with the message `nonce` and checks `author.bot`. `MessageSnapshot` exposes only id, author, content, attachment count and edited; it has no nonce, timestamp or bot flag. Deriving latency from a snowflake alone would be fabricated. |
| `src/plugins/mutualGroupDMs` | Blocked — missing host data/surface | Needs the recipients of every cached private channel and a profile side-panel/DM-list injection point. `ChannelDirectorySnapshot` has no recipients and `ChannelDetailsSnapshot` covers only the selected channel; there is no profile-list extension surface. Filtering by name would not truthfully identify shared groups. |
| `src/plugins/noBlockedMessages` | Blocked — native renderer/stream change | Needs filtering of blocked/ignored grouped rows, reply-to-suppressed handling, thread-starter attribution and trailing-divider cleanup. The SDK cannot remove rows from the timeline stream. Native notifications already suppress blocked/ignored authors, so only the timeline stream remains unimplemented. |

## Safety statement

No upstream JavaScript is executed. This package has no Webpack/system injection, raw
Gateway or store access, credential/session extraction, generic network/filesystem/OAuth
API, automatic send/account/microphone action, unbounded cache/queue/log or live Discord
call. Composer and renderer behavior from the existing translator/history/media/
notification work is unchanged, and the durable MessageLogger was not touched.

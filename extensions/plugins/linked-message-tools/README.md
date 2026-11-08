# Linked Message Tools

A bounded Serein adaptation of Equicord's **MessageLinkEmbeds**. Upstream renders an inline
accessory that fetches the linked message and draws its full content. This port deliberately
does not do that: it only previews messages that the host has already loaded and exposes one
explicit navigation proposal.

Upstream source at commit `200dba124066146a8f043fde7fc8b05e3dc5e68e`:
`src/plugins/messageLinkEmbeds/index.tsx`.

## What the message action does

**Preview linked messages** scans the selected message text for exact Discord message links of
the form `https://discord.com/channels/<guild-or-@me>/<channel>/<message>` (also
`discordapp.com` and one subdomain such as `canary.`). It:

- ignores links wrapped in angle brackets, the client's suppressed-link form;
- ignores duplicates and considers at most **5 distinct targets**;
- accepts only 17-20 digit nonzero channel and message IDs;
- never follows a link, downloads anything, reads another channel, or renders media;
- shows a short excerpt only when the linked message is already in the granted loaded
  timeline for the selected chat.

For loaded targets the panel offers one **Open selected linked message** action. That panel
action revalidates the target against the same loaded timeline and proposes exactly one
`jump_to_message` navigation effect, which the host applies only after the user confirms it.
Messages outside the loaded window are listed by ID and are never fabricated or fetched.

## Capabilities and bounds

| Item | Bound |
| --- | --- |
| Capabilities | `selected_message`, `timeline`, `navigation` |
| Scanned text | 8 KiB; larger input is rejected |
| Links reviewed | 5 distinct, 256-char search query limit inherited from the host |
| Excerpt | 160 UTF-8 bytes, control characters replaced |
| Select option | 128 UTF-8 bytes |
| Host effects | at most one `jump_to_message` after Apply |

The plugin has no storage, no timers, no logs, no network, no clipboard access and no automatic
action. It does not inspect attachments, embeds, media or other users' private payloads beyond
the already-granted timeline excerpt.

## Upstream omissions

- No inline accessory, custom embed rendering, images or automod embed layout.
- No fetch of unloaded linked messages and no channel/guild permission lookup. Only the loaded
  current-chat timeline is used, so links to other channels are listed without content.
- No blacklist/whitelist ID list, automod embed preference, cache or clear-cache button.
- No nesting-depth traversal of linked-message links; only the selected message's text is read.

These omissions are intentional. Fetching arbitrary linked messages or embedding remote media
would need a new narrowly scoped host capability and was not added here.

## Parent integration

This directory does not include packaged Wasm. The parent host must:

1. Add `plugins/linked-message-tools` to the `members` list in `extensions/Cargo.toml`.
2. Build `linked_message_tools.wasm` for `wasm32-unknown-unknown` with the pinned toolchain.
3. Package it with `python pack.py plugins/linked-message-tools/manifest.json ...` and
   `--license ../licenses/Equicord-GPL-3.0.txt` from `extensions/`.
4. Review the built artifact, pin it in the central catalog and update the parent's
   `equicord-workflows.json` implementation note.
5. Verify that the host message-action surface supplies `app.timeline` and clears
   `selected_message` on panel actions as documented in `extensions/sdk`.

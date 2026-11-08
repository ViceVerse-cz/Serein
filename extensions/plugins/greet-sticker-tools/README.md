# Greet Sticker Tools

A bounded Serein adaptation of Equicord's **GreetStickerPicker**. Upstream patches the welcome
CTA, holds Discord's private welcome-sticker catalog and can send one greet or up to three
stickers in a multi-greet. This port uses only stickers already loaded on the selected message
and proposes a single host-reviewed sticker send.

Upstream source at commit `200dba124066146a8f043fde7fc8b05e3dc5e68e`:
`src/plugins/greetStickerPicker/index.tsx`.

## What the message action does

**Choose a sticker to re-send** reads the granted loaded `message_content` snapshot for the
selected message. If it has loaded stickers, the panel offers one select box and a review
button. **Review sticker send** revalidates that the exact channel, message and sticker triple
still exists in the same loaded snapshot, then proposes one `send_sticker` host action.

Nothing is sent before the user's Apply. There is no automatic send, no text, no mention, no
attachment, no reaction and no microphone or account action.

## Capabilities and bounds

| Item | Bound |
| --- | --- |
| Capabilities | `selected_message`, `message_content`, `message_send` |
| Messages read | the granted loaded rich-message snapshot only |
| Stickers offered | at most the SDK's 3 loaded stickers on the selected message |
| Sticker name | 40 UTF-8 bytes, control characters and `:`/`|` replaced |
| Select option | 128 UTF-8 bytes |
| Host effects | at most one `send_sticker` after Apply |

IDs must be canonical nonzero decimal snowflakes. The same channel/message/sticker triple is
checked again in the panel action, so a stale or tampered option cannot send another loaded
chat's sticker.

## Upstream omissions

- No fixed welcome/greet sticker catalog and no "greet mode" or "unholy multi-greet" state.
- No welcome-CTA right-click patch, no React/Webpack access and no context-menu injection.
- No multi-sticker send in one action; the host applies one sticker per proposal.
- No `sendGreetMessage` API; the proposal uses the ordinary `send_sticker` action.

A fixed catalog would require a new native sticker-catalog capability and a stricter greet-row
eligibility surface. Neither was added.

## Parent integration

This directory does not include packaged Wasm. The parent host must:

1. Add `plugins/greet-sticker-tools` to the `members` list in `extensions/Cargo.toml`.
2. Build `greet_sticker_tools.wasm` for `wasm32-unknown-unknown` with the pinned toolchain.
3. Package it with `python pack.py plugins/greet-sticker-tools/manifest.json ...` and
   `--license ../licenses/Equicord-GPL-3.0.txt` from `extensions/`.
4. Review the built artifact and pin it in the central catalog.
5. Confirm the host still supplies `app.message_content` for granted `message_content`
   actions and still clears `selected_message`/`selected_message_ref` on panel actions. The
   `send` action deliberately depends on the snapshot, not on the cleared selection.

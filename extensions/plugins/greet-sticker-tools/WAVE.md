# WAVE notes — Greet Sticker Tools

Assigned upstream path: `src/plugins/greetStickerPicker`.

Implemented safe subset: re-send one sticker that is already loaded on the selected message,
using a single Apply-reviewed `send_sticker` host action. No fixed greet catalog, multi-greet
or automatic send.

## Parent steps

- Add `plugins/greet-sticker-tools` to `extensions/Cargo.toml` workspace members.
- Build and package centrally; do not commit hand-built Wasm in this branch.
- Add the packaged entry and implementation note to `extensions/catalog.json` and
  `extensions/equicord-workflows.json` in the parent integration commit.
- The host must keep supplying `app.message_content` for granted `message_content` actions and
  clearing the selected message on panel re-invocation; the `send` action revalidates the
  encoded channel/message/sticker triple against that snapshot.

## Deliberate limits

- No welcome-CTA patch, private sticker catalog, greet mode, multi-greet, DOM/React hook or
  automatic send.
- One sticker per confirmed proposal.

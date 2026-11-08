# Plus Reacts (loaded timeline)

A safe, bounded Serein composer tool that proposes adding **your** reaction to
an earlier message in the already-loaded timeline. The number of leading `+`
signs chooses how far back to go.

This is an independent Rust adaptation of Equicord's `betterPlusReacts`
workflow at commit `200dba124066146a8f043de7fc8b05e3dc5e68e`
([`src/equicordplugins/betterPlusReacts/index.ts`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043de7fc8b05e3dc5e68e/src/equicordplugins/betterPlusReacts)).
Upstream is GPL-3.0-or-later; this port keeps the same license. See
<https://www.gnu.org/licenses/gpl-3.0.html>.

## Usage

Type one of these into the draft, then choose **Plus Reacts** from the composer
tools menu:

| Draft | Target |
| --- | --- |
| `+🎉` | the newest loaded message |
| `++🎉` | one message before the newest loaded message |
| `+++🎉` | two messages before the newest loaded message |
| `+party:123456789012345678` | a custom emoji by `name:id` |
| `+<:party:123456789012345678>` | the same custom emoji in Discord markup |
| `+<a:party:123456789012345678>` | an animated custom emoji, normalized to `name:id` |

One plus is the newest loaded row, two pluses is the next older row, and so on
up to **50** plus signs. This is a documented bounded interpretation of
upstream's message-store index math; it only walks the host-provided snapshot.

The result dialog shows the proposed target, author, text preview and emoji.
**Apply** adds only your reaction. The plugin never sends a message, never
changes the draft, and never reacts automatically.

## Bounds and safety

- At most **50** leading plus signs; the host timeline is at most 50 messages.
- The composer draft must be at most **512 bytes** before parsing.
- The reaction emoji is at most **128 bytes**, matching the host validator.
- Text previews are sanitized of control characters and truncated to **160
  bytes** at a UTF-8 boundary.
- Exactly **one** `AppAction::SetReaction { add: true }` host effect is proposed.
  The host revalidates the current conversation, message ID and emoji at Apply.
- Only the currently loaded timeline is inspected. If it is truncated, the
  panel says so; older history is never fetched, requested or guessed.
- No storage, no network, no filesystem, no Webpack/Discord module patching, no
  raw message store, no arbitrary JavaScript and no automatic action.
- Custom emoji input requires a 2-32 character ASCII name and a nonzero numeric
  ID. Shortcode text such as `:smile:` is rejected instead of being sent as
  literal text.

## Deviations from upstream

- Upstream patches Discord's composer/autocomplete and reads
  `MessageStore.getMessages(channelId)`. This port is an explicit composer tool
  over the bounded SDK `timeline` snapshot.
- Upstream can target a store index outside the visible window. This port caps
  the lookback at the loaded snapshot and explains when there are not enough
  loaded messages.
- Upstream interprets `:emoji:` shortcodes in the composer. This port requires
  the actual Unicode emoji or a custom `name:id` value, because the host does
  not resolve arbitrary shortcodes.
- The draft is not rewritten or cleared; the reaction is proposed separately
  after the user chooses the tool.

## Files

- `Cargo.toml` — standalone crate using the workspace SDK dependency.
- `manifest.json` — capabilities `composer`, `timeline`, `reactions_control`
  and the `plus-react` composer action.
- `src/lib.rs` — parser, bounds, handler and focused tests.
- `WAVE.md` — audit of the assigned upstream paths and parent integration steps.

Focused tests (run by the parent, not by this source-only change):

```sh
cargo test --manifest-path extensions/plugins/plus-reacts-tools/Cargo.toml
```

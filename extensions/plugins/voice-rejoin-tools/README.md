# Voice Rejoin Tools

A bounded Serein panel that remembers **one** last voice channel and proposes
rejoining it only after an explicit **Apply**. It never reconnects in the
background, never rings a call and never joins with an unmuted microphone.

## Provenance

This plugin adapts the behavior of Equicord's
[`src/equicordplugins/voiceRejoin/index.tsx`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e/src/equicordplugins/voiceRejoin)
by Vendicated contributors at upstream commit
`200dba124066146a8f043fde7fc8b05e3dc5e68e`, licensed
**GPL-3.0-or-later**. This port is distributed under the same license. See the
header in `src/lib.rs` and <https://www.gnu.org/licenses/gpl-3.0.html>.

## What is implemented

The plugin declares five actions:

- **`observe`** (`app_event`) receives Voice changes. When the snapshot already
  supplies a valid active call channel ID, it stores that single ID. A
  disconnect, a participant change that keeps the same channel, a malformed ID,
  another event kind, or already-corrupt storage causes no write. This action
  never shows a panel or returns an effect.
- **`show`** (`panel`) reads the saved ID and the current app snapshot. It
  resolves a name from the bounded loaded channel directory when available,
  shows the current call, and offers only the applicable actions.
- **`save-current`** (`panel`) explicitly stores the current call channel. This
  is also the deliberate way to repair corrupt saved data.
- **`forget`** (`panel`) writes the valid empty value `{}`. It works for a
  valid or corrupt saved value and is a no-op when nothing is saved.
- **`rejoin`** (`panel`) returns exactly one
  `HostEffect::AppAction::JoinVoice { ring: false, muted: true, deafened: false }`.
  Serein displays an Apply proposal and performs it only after the user
  confirms. The panel states that joining is muted, does not ring and does not
  retry; the user unmutes manually after joining.

Rejoin is rejected with no effect when:

- no channel is saved, or the saved value is corrupt;
- the app snapshot has no voice state;
- the user is already in the saved channel;
- the user is in a different call (this port never switches an active call);
- the loaded channel directory is complete and does not contain the saved
  channel;
- the loaded channel directory is complete and reports a non-call channel kind
  (`1`, `2`, `3` and `13` are accepted).

A **truncated** channel directory is treated as partial. The plugin does not
claim the saved channel is missing and still proposes the rejoin; Apply and the
host revalidation remain authoritative.

## Bounds

- One saved channel ID, at most 20 decimal bytes and nonzero.
- Storage is a strict object `{"channel_id":"<id>"}` or the valid empty `{}`,
  at most 512 bytes. Unknown fields, wrong types, empty IDs, non-decimal IDs and
  malformed JSON are corrupt and are preserved until an explicit Save/Forget.
- One host effect per response; the proposal is at most one `JoinVoice`.
- The complete panel is kept at or below 4 KiB. Names are control-stripped and
  truncated at 128 UTF-8 bytes.
- No ring, no retry, no automatic join, no timer, no background task, no
  microphone access and no raw gateway/store access.

## Deliberate deviations from upstream

Upstream `voiceRejoin` dispatches `VOICE_CHANNEL_SELECT` automatically after a
configurable delay, may retry after restart, uses a 5-120 second timeout window,
and can check whether other users remain in the saved call. This port does not
reproduce those behaviors:

- **No automatic rejoin.** The only join path is the Apply-gated proposal.
- **No time window.** App snapshots provide no clock or timestamp, and the
  plugin has no host tick input carrying app data. The saved channel therefore
  remains until replaced or forgotten; the panel says so.
- **No empty/ended-call check.** The snapshot exposes participants only for the
  current call, not for the saved non-current channel. The plugin does not
  pretend to know that roster.
- **No DM/server mode setting.** Only one saved channel is stored, so the
  distinction is unnecessary.
- **Muted join.** The port always proposes `muted: true` so a rejoin cannot
  start transmitting the microphone without a later explicit unmute.
- **No ring.** `ring` is always false.

## Files

- `Cargo.toml` — standalone crate using the local `extensions/sdk` path.
- `manifest.json` — capabilities `app_events`, `voice_state`,
  `channel_directory`, `voice_connect`, `storage` and the five actions above.
- `src/lib.rs` — handler, strict storage parser, bounds and focused tests.
- `WAVE.md` — audit of every assigned upstream path and parent integration
  steps.

## Tests

Parent runs the workspace checks. The crate contains focused inline tests for:

- remembering only a changed valid active channel;
- ignoring disconnects and other event kinds;
- preserving corrupt storage through implicit observation;
- resolving a loaded channel label and current-call note;
- exactly one muted, no-ring `JoinVoice` proposal;
- refusing a missing save, an active same/other call and unavailable voice state;
- rejecting a missing or non-voice destination only with a complete directory;
- still proposing with a truncated directory;
- explicit repair/forget and bounded, control-free long labels.

No tests were run in this worktree by instruction; the parent executes the
build, Clippy and test suite.

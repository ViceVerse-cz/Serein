# Message history

A bounded, persistent native follow-up to Equicord's **MessageLogger** and
**MessageLoggerEnhanced** plugins. Upstream source is pinned to Equicord/Equicord commit
[`200dba124066146a8f043de7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043de7fc8b05e3dc5e68e):
`src/plugins/messageLogger` and `src/equicordplugins/messageLoggerEnhanced`. This port keeps the
upstream GPL notice and is licensed `GPL-3.0-or-later`.

## What works

- The package declares only `message_events`; it does **not** request `storage`. Its
  `enable-history` activation action explicitly offers persistent local logging, while `observe`
  acknowledges live events without returning panels or retained text.
- Installing and enabling is not sufficient by itself: the user must grant `message_events` and
  activate the exact reviewed `message-history` package. Imported or unreviewed lookalikes cannot
  enable the native retention path. Retained bodies never return to Wasm or plugin storage.
- The native store records validated pages and live create/update/delete/bulk-delete events from
  all subsequently loaded, readable conversations, including background conversations. Channel
  switches, timeline eviction, normal account switches and ordinary disconnects do not erase it.
  No account-wide REST harvest, search, invisible account-history recovery or extra history request
  is introduced by logging.
- Bodies and versions are bounded at 16 KiB and eight changes per message. Defaults are 30 days,
  10,000 messages and 32 MiB of metadata/versions per account. Retention presets allow 1–365 days
  and 1,000–50,000 rows; metadata has a hard 4–64 MiB policy bound. Old rows and versions evict
  first. The SQLite database has a 128 MiB global page ceiling.
- A deletion preserves an already observed body and marks attachment metadata deleted. An unknown
  delete can retain only a bounded `(channel, message)` tombstone, never an author or content, so
  late pages cannot recreate deleted-before-load content. Unknown deletes do not emit alerts.
- The native viewer pages at most 100 rows with edit/delete/channel/author filters and a bounded
  500-row / 2 MiB UI cache. It shows before/after versions and attachment metadata. Deleted targets
  open their local retained record, not a fabricated Discord search or timeline row.
- User/channel ignore lists (512 IDs each), optional edit/delete ghost alerts, and bounded text/JSON
  export are available. Ghost alerts are body-free, respect access/mute/DND/notification policy,
  coalesce to one per channel per 30 seconds and six per account per minute, and open native targets.
- Export is explicit and account-scoped, includes retained versions/deleted bodies, and stops at
  50,000 rows or 256 MiB. It writes no signed URLs, local media paths, tokens, cookies or Gateway
  payloads. Export uses a temporary file and publishes only after complete success.
- Supported attachment bytes are optional per type. Images/audio/video default to metadata only;
  limits are 8/16/32 MiB per file and 256 MiB per account. Downloads validate the exact Discord CDN
  object and content type, use a bounded worker, and never persist signed URLs. Other types and
  generic URLs are not admitted.

## Lifecycle and privacy

- Persistent policy is restored before package activation can start capture. A first activation
  may create an enabled policy; a saved explicit opt-out is never silently re-enabled.
- Disable/Clear deletes retained messages, versions, tombstones and attachment rows and saves
  capture disabled while preserving bounded retention/filter choices. A new explicit Apply is
  required to enable capture again.
- Access-loss purges affect only the removed channel. Logout and Forget saved account remove the
  database history and its policy. Media downloads are cancelled before account-file cleanup;
  ordinary switches and disconnects keep durable history. Queue generations prevent late writes
  from restoring a cleared or different account's rows.
- The viewer's in-memory cache and core's volatile observation budget are separate from persistence.
  Timeline eviction is not durable eviction and cannot imply that the underlying log disappeared.
- No raw history, attachment bytes or export contents are exposed to extensions. The rendering and
  audio callbacks do not perform SQLite or attachment-network work.

## Safe differences from upstream

Equicord's base MessageLogger is session-only; MessageLoggerEnhanced supplies persistence. This
port deliberately does not copy upstream's unaccounted files, unbounded export, account-mixed
storage, arbitrary attachment fetches or incomplete orphan cleanup. It also does not implement
import or claim to reconstruct deleted-before-load messages. Full parity with all upstream UI
hooks is not claimed, and no live Discord interoperability has been established by offline tests.

## Build and registration

The package source builds against the shared SDK. Repackage with its full GPL license section:

```sh
cargo build --locked --release --target wasm32-unknown-unknown -p message-history
python pack.py plugins/message-history/manifest.json target/wasm32-unknown-unknown/release/message_history.wasm plugins/packages/message-history.serein-extension --license licenses/Equicord-GPL-3.0.txt
```

The committed package must be rebuilt from this reviewed source after changes. Catalog pinning is
handled by the repository's post-merge generation workflow, not by manually changing a live pin.

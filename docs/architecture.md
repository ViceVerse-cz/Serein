# Architecture

Serein is a desktop protocol client for Discord, with no project messaging service. The final owner revisions permit an authentication-only webview, saved login, and local caches.

Incoming typing has its own eight-slot lossy inbox, separate from reliable message events.
The producer filters the selected channel and limits duplicate/burst wakeups to eight per two
seconds. The consumer collects reliable events first, then typing, and applies typing before
reliable events so queued messages and access changes retire older indicators. Core state is
eight fixed identity/deadline slots, scoped to fresh readable text history, with no persistence.
The composer schedules one visible expiry deadline and performs no animation or identity fetch.

- `model`: IDs preserved as strings on the wire / u64 in memory, typed entities and absent/null/value patches. No GUI, filesystem or networking.
- `discord-protocol`: bounded wire decoding and Discord DTOs, independent of rendering.
- `client-core`: single UI-thread state owner, generation-tagged events, composer/send lifecycle, navigation and freshness. Network callbacks never mutate it directly.
- `session-cache`: one active 500-message / 4 MiB timeline, bounded patches/tombstones. Inactive history goes to `local-store`; no unbounded RAM cache per channel.
- `discord-api`: fixed Discord origin, verified TLS, no redirects, cookies or proxy discovery, four concurrent REST permits, bounded response bodies and conservative shared service cooldown.
- `discord-gateway`: independent Tokio task; JSON without compression, Hello/ACK/heartbeats, Identify/Resume, bounded initial login retries and persistent reconnection with capped backoff after READY. UI queue overload stops the connection instead of silently losing message mutations.
- `local-store`: SQLite transactions, global disk ceilings, account-isolated messages/drafts. A dedicated worker serializes operations away from rendering.
- `platform`: OS credential store, temporary Wry login webview and native file-save dialog. No system profile/token extraction. Credential operations are serialized by a dedicated worker to order saves before logout deletion.
- `ui`: egui panels, viewport virtualization and multiline composition; typed commands only. No network requests or tokens.
- `apps/desktop`: wiring, cancellation, bounded queues, native login lifecycle, cache hydration and eframe options. `test-support` is explicitly synthetic; `xtask` and `replay-bench` are development tools.

No generic plugin system, bot SDK or external database is introduced. The `discord-voice` crate provides native DM and guild media in every desktop build.

Loaded threads reuse channel models, selection and history. READY and thread dispatches supply validated guild/parent metadata; the core reconciles scoped active snapshots atomically before removing missing active threads. Removed selected conversations cancel pending history/search and invalidate visible/cache state while preserving drafts. The sidebar builds a bounded two-level parent/thread index with root fallback for malformed parents. Explicit archive browsing shares search/pins' single replaceable read worker and admits at most one transient navigation entry on Open. Active snapshot omission preserves that archived entry; matching Gateway metadata can adopt it. No background directory task or subscription expansion is introduced; scope and limits remain explicit.

The storage worker admits at most 16 operations within a 16 MiB estimated allocation budget; history payloads retain the active 4 MiB window limit. Its 16-result queue has a separate 16 MiB budget, including results being processed by the UI. A worker can hold one additional completed result while waiting for byte capacity. Admission failures retain the existing visible unsaved/cleanup handling. Authenticated REST has four permits and a shared cooldown; the cooldown mutex protects admission state rather than the lifetime of a slow response. Gateway control runs independently, and newer navigation cancels old history work. One separate profile task is cancelled by a replacement profile request, closing the card or connection teardown. Profile responses match the session generation and selected user/guild/request before entering the single 64 KiB RAM view. A separate 16-slot serial message-write worker keeps slow REST writes from blocking call controls. Session generations reject old outcomes. Credential queues are four small operations and serialize read/save/delete. UI snapshots do not clone the account per frame.

Incremental history saves reuse their loaded channel rows and prepared write statement.
A connection-local global byte total is adjusted from indexed channel sums inside the write
transaction. SQLite total-changes and data-version counters invalidate it after other writes;
rollback never installs a new total. No persistent counter or schema migration is introduced.
Signed uploads reuse a separate credential-free HTTP client with their existing timeouts and
single-attempt policy; REST bodies reserve bounded capacity from a validated Content-Length.

The native message renderer uses pulldown-cmark without its HTML or command-line features. Parsing is capped at 8192 UTF-8 bytes / 128 lines, 512 parser events and 16 nesting levels; complexity overflow falls back to bounded literal text. A 512-entry / 1 MiB estimated source-and-span MRU serves visible messages. HTML remains inert text and Markdown images are placeholders. HTTP(S) links require an explicit destination confirmation; nothing fetches them for previews. Up to 32 inline spoiler regions use a fixed reveal mask; concealed spans never enter text selection, accessibility labels, link/reference actions or emoji rendering. Spoiler media has a separate explicit reveal. Consent is invalidated when the original text or media changes. Complexity fallback conservatively conceals bounded source containing spoiler markers. Spoiler consent copies are bounded by the active message window. Native height caches include message content/metadata, width, body font size and display scale.

Bundled OFL Noto CJK/Arabic fallbacks add 16.51 MiB raw font data; there is no runtime download or system-font scan. See assets/README.md for provenance, regional forms and shaping limitations.

History responses/errors must match the active request and connection state. Recent reload replaces the retained view while preserving mutations observed during the request; back-pagination validates page channel, ID boundary, duplicates and cardinality. At capacity, an older window retains its reading position instead of evicting its anchor for new messages; Reload returns to latest. Successful Resume triggers active-page revalidation. Bulk deletions occupy one bounded event instead of flooding the UI queue.

Known limits: one RAM channel window, a CommonMark subset rather than Discord Markdown parity, conservative shared rate cooldown, six initial connection attempts (established sessions keep retrying outages with a delay capped at 32 seconds plus jitter), and full-window cache writes rather than per-message SQL updates. These are implementation limits, not claims of full milestone completion.

The active People pane retains at most 100 service list positions and 128 KiB of member metadata. DM participants are retained with navigation; guild member subscriptions are on demand and remain unofficial/live-unverified. Member events are tagged with the active request and matched to the service list identity; closing or navigating the pane releases its working set.

Avatar metadata travels through user/message DTOs and SQLite alongside embeds, attachments, mentioned users and unsupported-content presence bits. A credential-free worker constructs Discord CDN image URLs from validated IDs/hashes and validates service-proxied message image URLs, disables redirects/proxy discovery, and does disk I/O and bounded decoding away from the render thread. The UI requests visible images, deduplicates attempts, and owns a 256-entry / 64 MiB texture cache plus at most four animations / 16 MiB of retained animation pixels. Four downloads can overlap; decoding stays serial. Account cache directories can retain 1 GiB / 4096 encoded images. See storage-policy.md for exact queue/decoder limits and cleanup ordering.

The image viewer is a full-window egui modal sharing the bounded texture cache; its larger image is reduced to a longest edge of 2048 pixels. Explicit Download uses a separate credential-free client and a single worker, after a native destination choice. Only the selected CDN attachment original is streamed; no message filename becomes a path without native user selection. Progress is coalesced, transfer bytes are bounded at 100 MiB, and filesystem writes/flush/cleanup run off the render thread. A sibling partial is published only on complete success; cancellation and ordinary shutdown wait for cleanup. Native Cocoa/Windows dialogs and the Linux FileChooser portal are provided by rfd; Linux avoids a second GTK owner competing with the login webview. Download guarantees and limitations are strictly enforced.

## Native DM and server voice

`client-core::voice` owns one active call and one incoming indicator, with generation/request matching and no automatic answer/rejoin. Existing kind-1 DMs with one recipient, kind-3 group DMs with fewer than 64 recipients, and guild voice channels are eligible. `discord-protocol` decodes CALL_* and voice state/server dispatches; `discord-gateway` sends the unofficial normal-user opcodes 13/4. Leaving retains a barrier until the owner's null voice state or CALL_DELETE acknowledges departure; after ten seconds it reports a timeout and continues rejecting another join until acknowledgment or reconnect. An old departure cannot become the next call's negotiation event.

The desktop consumes redacted, zeroizing voice credentials before core reduction. It pairs the owner's session with a channel-scoped voice server/token, bounded by a 30-second allocation deadline. Outgoing ringing is a separate, once-per-request REST command after transport allocation is confirmed, before waiting for the peer-dependent DAVE group. Answer never rings. Failed/ambiguous ringing writes are not replayed. Slow REST cannot block native mute or hangup; call control and notice queues each hold eight items.

`discord-voice` owns the voice WebSocket, public-address UDP discovery, RTP transport authentication, DAVE/OpenMLS state, Opus and native CPAL audio. TLS/origin validation and required DAVE readiness gate media; there is no encryption downgrade. Audio callbacks use preallocated rings, while codecs/networking and device work run off the render thread. Capture/playback are enabled only after required encryption, and stop on hangup, session failure or account change. The desktop polls media lifecycle from eframe logic as well as after UI gestures, so teardown and focused push-to-talk do not depend on a visible repaint.

Device choices, mute/deafen and focused V push-to-talk remain session-local. Voice WebSocket resumption is bounded and preserves the current call's cryptographic state; rejected resumption and main Gateway disconnect require deliberate rejoin. Group calls share the bounded media engine, AEC, camera and screen-sharing paths; recording is not included. Exact media limits and offline evidence are in [the adapter README](../crates/discord-voice/README.md); [the live gate](voice.md) is still blocked.

Conversation search owns one page of at most 25 ID/author/excerpt records, capped at 64 KiB, alongside the existing 500-message / 4 MiB timeline. Queries are capped at 256 characters / 1024 UTF-8 bytes. The decoder caps the HTTP body at 512 KiB, 25 result groups and 5 context records per group; only the matching excerpt (256 characters) is retained. Snippets are plain, inert text with conservative whole-snippet spoiler concealment and no media loads. One cancellable read task shares the four REST permits. Opening a result replaces the active window with revalidated history rather than merging index snapshots into the message cache.


Application components reuse bounded model/protocol records and the serial REST write
worker. The connection adapter intercepts the zeroized Gateway session handoff; the
UI never receives it. Core state authorizes explicit component selections, validates
modal inputs against the received schema, correlates nonce-tagged outcomes and retires
stale/session-changed forms. Private replies bypass persistent timeline state. Native
file dialogs stage modal uploads separately from composer attachments, through the
existing upload worker. No bot credentials, backend or embedded messaging webview is
introduced. Normal-account interaction behavior remains unofficial and live-unverified.

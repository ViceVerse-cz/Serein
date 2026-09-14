# Local storage policy and audit

## Large account startup (September 14, 2026)

Account navigation supports 131,072 guild/channel entries within 128 MiB of estimated
navigation and permission storage. The permission mirror has a 64 MiB sub-budget,
131,072 aggregate roles and 1,048,576 aggregate overwrites; per-object role/overwrite
validation remains unchanged. Permission decisions still cache at most 4,000 entries.
Incoming read-state snapshot vectors use at most 131,072 entries / 16 MiB; retained
read maps are bounded by account channels, with the existing separate activity/alert budgets.
Notification preferences
use bounded account collections / 32 MiB. These are on-demand ceilings, not reservations
or an RSS guarantee. This section supersedes the older account snapshot limits below.

The reliable FIFO retains its 4,008-item capacity; ordinary events share a 32 MiB budget
and a 4 MiB per-event limit. One startup snapshot may use a separate memory reservation,
capped at 128 MiB including its metadata, within that same FIFO; consuming or dropping
it releases the reservation. Concurrent startup snapshots are rejected. Startup buffers,
the prior account state during replacement, decoding, maps and runtime/allocator overhead
can add to peak memory. Gateway compression still caps input and output at 64 MiB each.

Rejected optional READY metadata is discarded by section. Only fixed feature-warning flags
remain; invalid read state stays unknown, and missing settings or DND suppress alerts.
Fresh READY/logout release stale optional state. Valid authoritative updates can restore
features. No payload logs, account database changes, background directory fetching, or
new persistent caches are added.

Cross-server emoji (September 14): browsing and provenance reuse the existing bounded joined
guild catalogs and avatar cache. Picker search retains at most 1,000 borrowed catalog pairs;
autocomplete retains at most 256 ranked suggestions, with custom names capped at 32 ASCII bytes
and source labels at 120 Unicode scalars. Only visible cells request artwork. No new catalog,
network endpoint, persistent metadata, background job, or storage migration is introduced.
Emoji information cards resolve names and source servers on demand from the loaded catalogs;
unknown/deleted source metadata remains explicitly unknown.

Notification sounds (September 13): three owner-supplied MP3 tracks are embedded
in the executable (106,608 bytes total), with no runtime files or downloads.
The existing single lazy worker and one-slot fixed-size request queue decode one
track at a time outside rendering/audio callbacks. Each asset is capped at
128 KiB encoded, 48 kHz stereo and five seconds decoded (PCM vector capacity
less than 4 MiB). Conversion retains at most five seconds of stereo f32 at the output
device rate, capped at 192 kHz / 7,680,000 bytes, alongside source PCM during
conversion. Decoder/device allocations are separate. Playback buffers are
released after each cue; cancellation silences the callback and is checked by
the worker every 20 ms. No notification-audio cache or storage migration is added.

Explicit media clipboard copies (September 13) reuse the bounded attachment
download worker. One original video, at most 100 MiB, remains in a randomized
`serein-clipboard-*` OS temporary directory while its file clipboard entry is
usable. The next media copy, logout, or normal exit releases it; pasting requires
the app to remain open. Image staging files are removed after decoding. All file
work and cleanup run outside rendering. Forced termination or filesystem failures
can leave a temporary file; cleanup errors are visible. Clipboard contents belong
to the OS and may also be retained by clipboard managers. No cache schema changes.

Forwarded messages (schema 16): one checked, default-false `forwarded` column marks
the immutable snapshot body. Text, embeds and attachments reuse existing bounded
message storage; source channels/messages are never fetched. Existing rows retain
their content and default to ordinary messages until refreshed. Schema-15 binaries
cannot reopen this upgraded cache.

Channel shortcuts (September 12, schema 15): favorites and pins are device-local,
account-isolated SQLite preferences. Both lists together contain at most 256 IDs,
with at most 4 KiB retained vector storage and an 8 KiB serialized record. Loading
and saving run on the existing bounded cache worker; corrupt/oversized records and
save failures are shown. Shortcuts survive restart and are removed on account
logout. They do not sync to Discord. Channel edit drafts, authoritative settings,
pending actions and locally observed mute expiry times remain bounded session RAM.
Each category/channel settings snapshot retains at most 1000 overwrites (48 KiB),
a 400-byte name and a 16 KiB topic, under 64 KiB total heap allocation. The UI holds
one before/after pair; core holds one loaded snapshot and one pending edit, with
one additional before/after pair in the existing command lane. Settings are read
and written on demand and never persisted locally. Picker suggestions reuse the
loaded member window and role mirror; searches are capped at 64 characters and
explicit member IDs at 20 characters.
Shortcut restore retains one request when the storage queue is full, then admits it
when worker completions free space. Only one load can be in flight; account/session
reset clears its flags. Actual storage failures stop automatic retries and retain
the explicit Retry shortcuts action. Queue pressure alone no longer reports a
failed restore. Queue, payload and database bounds are unchanged.

Community extensions (September 12): packages and grants are stored under the
application data directory's `extensions` subtree. Plugin data/grants are
account-isolated; declarative themes are device preferences. Up to eight installed
themes remain available in Colour preset; switching only changes the bounded
64-byte active theme identifier and retains the packages. Installation,
validation, invocation storage and removal run on a bounded background worker.
Disabling removes Serein's package and extension data, and failed cleanup is
reported and retried. Logout removes the account's plugin data. Imported
original files and source repositories are never deleted. No credentials belong
in plugin storage; it is not encrypted. Only bounded metadata may remain after
successful disable. Shop descriptions and optional preview URLs/hashes belong to
bounded catalog metadata (1 MiB / 256 entries). Visible cards may fetch a PNG/JPEG
preview using credential-free validated public HTTPS; bytes are hash/size checked,
limited to 256 KiB compressed, decoded off-thread within 4,194,304 source pixels,
and reduced to at most 640 x 360. Eight UI thumbnails cost at most 7,372,800 RGBA
bytes plus GPU/framework overhead. Images stay in memory only; no preview disk
cache or background refresh is added. Existing single-worker/four-job bounds and
session cancellation apply. See `extensions.md` for the creator and permission model.

Schema 13 adds a constrained webhook boolean to cached message authors. It comes
from the service message webhook_id and follows the existing bounded author data
through message, reply, and profile views. Legacy rows default to unknown (false)
until refreshed; no name, bot flag, or profile error is used to infer a webhook.
The migration preserves existing messages, settings, and drafts.

Group conversation actions (September 11): names and icon hashes are session
navigation metadata, with icon capacity included in channel/event byte counts.
The editor retains one name (100 characters), one bounded PNG data URI, and one
256x256 RGBA preview. A single native picker/worker admits a regular file up to
8 MiB, decodes outside rendering with 4096x4096 and 64 MiB decoder limits, then
center-crops and downsizes to at most 256 pixels. Encoded PNG is capped at
256 KiB (data URI at 349,550 bytes). Selecting a file never uploads it; Save
queues one bounded group write. Pending core state and completion events do not
retain the image payload. Closing the editor releases its draft/preview; stale
picker results are discarded by session, channel and editor revision.
Source paths and chosen image bytes are not persisted or logged. Confirmed CDN
icons use the existing bounded avatar cache; no new database or cache schema is
introduced. Confirmed leave invalidates cached channel history through the
existing access-removal path and keeps the user's text draft.

User context actions (September 11): close-DM, block and notification-mute preferences are
written directly to Discord through the existing authenticated transport, never a new local
settings file. Relationship state keeps at most 4000 fixed ID/bool entries with a 128 KiB
estimated allocation ceiling; only one fixed-size action may be pending. DM mute state reuses
the existing bounded notification overrides. Logout clears both with other account RAM state.
Closing a DM preserves its local draft. Accepted navigation removal uses the existing account
history invalidation path, which can clear other cached history but preserves drafts. The
standalone offline fixture changes synthetic RAM only and issues no service or storage writes.

Current reply metadata schema is 10. It adds one constrained reply_deleted boolean to each
bounded message row; legacy rows default to unknown (false). Only an explicit service-null
reference on a valid same-channel reply/context-menu message establishes deletion. Nested
referenced bodies are discarded. The migration retains schema-9 message kinds/content markers,
reading preferences and drafts in the existing transaction. Saved/loaded markers require a
positive earlier target and message kind 19 or 23; malformed cache markers are rejected.
Accepted deletion effects use the existing cache epoch and deletion queue, so pre-deletion
loads/saves cannot restore the body. Queue saturation falls back to existing history clearing,
preserving drafts. Older binaries limited to schema 9 cannot reopen the upgraded cache.

The September 10 PR integration uses schema 9 to combine system-message `message_kind`,
unsupported-content `extra_content` and the reading/layout singleton. Both independent
schema-7 layouts and schema 8 are migrated by detecting the actual message columns. Existing
rows, drafts and settings are retained; older binaries with a lower schema ceiling cannot
reopen the upgraded cache. No unpublished reply-navigation metadata is included.

Reading/layout settings use one application-wide schema 8 SQLite singleton: integer display
scale 80..150 percent, sidebar width 190..360 logical points, and wide-layout People visibility.
Missing row means 100 percent / 236 points / visible. Reset removes just this override in an
atomic statement; neither theme nor account drafts/history are reset. Logout retains these
non-account settings. Startup reads them on the existing worker; delayed results never override
an explicit user choice. No account identifiers or message content enter this record.

Interactive changes coalesce for 300 ms into at most one queued write and one fixed-size latest
value. A full or failed worker reports an unsaved change without an automatic retry loop;
Retry saving is deliberate. Closing with pending/failed writes prompts before discarding.
In-app preview edits are not saved; a write already requested outside preview still completes.
The standalone --demo does not start the SQLite worker. Category collapse, narrow People overlays and outer window geometry remain session-local.
Notification opt-in, hidden-channel visibility, primary RGB color, audio devices (up to 1,024 bytes each),
noise suppression, push-to-talk and gain are saved in the device-wide `app_preferences`
SQLite singleton (16 KiB maximum), using the existing background worker. These survive
restart/logout; demo controls never read or write them. Save failures remain visible.

Recently visited conversations now keep at most two dormant RAM timelines in the current
account session, moved rather than cloned. Only readable Fresh ordinary text windows are parked;
search-target ranges and transient archived threads are excluded. Promotion rechecks identity
and read permission, shows a Loading preview, and always requests a fresh recent service page.
This is a preview cache, not saved historical scroll position. It avoids a SQLite read on a hit.
Unknown/deleted-only row handling retains the same guards as the active window.

Active plus dormant timelines share 1,475 rows and 16 MiB minus 66 KiB of estimated allocations,
reserving 25 rows and 66 KiB for the single search/pins page and query/view metadata. Estimation
includes retained payloads, pending patches, mutation/deletion sets, container storage and a
B-tree slack allowance; it is not an allocator or RSS measurement. Each individual timeline keeps
its existing 500-row / 4 MiB payload limit. Oldest whole dormant windows are evicted when needed,
without touching drafts or pending sends. Incoming mutations evict the affected dormant channel;
permission/identity changes prune it, and session replacement/resync/logout removes all dormant
history. Clear cached history removes dormant RAM and disk history while preserving the active
displayed conversation. No new disk data, database schema, worker or service request is added.

Loaded messages deleted by the service leave only their ID as a session-local reading row.
Author, body, attachment and embed metadata are dropped from the timeline and its formatted /
revealed-content views. An untouched open editor closes; text the user modified remains an
unsent edit with Copy/Cancel and no Save. Old editor undo snapshots are cleared immediately,
including when the user is viewing a different channel. Unknown deletion IDs never create visible rows.
Live and deleted rows share the 500-row / 4 MiB estimated storage ceiling; deletion guards
retain the existing 1,024-ID bound. Placeholders are not written to SQLite.

Gateway deletion removes matching account/channel/message rows in a bounded SQLite transaction,
including inactive and loading conversations. Cache history operations carry a session epoch;
deletion invalidates older queued snapshots and returned pages. If deletion cannot be queued,
history reuse pauses until account-scoped cleanup finishes. Cleanup retains at most 16 pending
account IDs in addition to the existing 16-command / 16-result worker queues, preserving drafts
and unrelated accounts. Storage failure or cleanup-scope overflow disables history caching for
the rest of the session and reports that content may remain on disk. This cannot guarantee
removal after filesystem failure or forced termination; no forensic-erasure claim is made.
No schema change or persistent deletion journal is introduced.

Schema 7 adds one integer extra_content column (0..31) for presence of polls, sticker_items,
legacy stickers, component arrays and the Components V2 flag. RAM uses five booleans; partial
updates preserve each source independently. Poll answers, sticker data, component payloads and
their URLs are not retained. Existing cached rows default to no known markers until normal
service revalidation because older builds discarded that metadata. Account isolation, existing
database/cache limits and logout deletion remain unchanged; unsupported content is not rendered
or executed from SQLite. Invalid stored marker bits reject the cached page.

Microphone gain and speaker volume are saved bounded integer percentages (0..=200),
initially 100. System mixer settings are unchanged. Two callback atomics hold active levels;
no audio is retained. Preview levels remain session-only.

Loaded thread navigation shares the 4,000-entry account navigation and 4 MiB normalized navigation budgets. Incoming thread syncs additionally cap combined parent/thread entries at 4,000 and normalized snapshot metadata at 2 MiB; wire JSON remains capped at 4 MiB. Removed-member arrays are capped at 4,000 and are discarded after checking the owner. Navigation/member lists are session-only; selected thread messages/drafts reuse existing account history/draft storage. Actual accepted thread removals enqueue the existing account-wide history clear, preserving drafts; ignored, empty-scope and rename-only events do not clear disk history. This coarse invalidation trades refetch cost for simpler deletion, without new tables or workers.

The owner explicitly withdrew the no-storage policy on 2026-09-09. Local files, SQLite, saved drafts, settings and caches are permitted. The implementation persists **history with embeds, attachments and mentioned users; avatar/server-icon/banner/preview images; drafts; appearance; reading/layout preferences; and the login token**. Outer window geometry remains session-local.

| Data | Location / bound | Removal |
|---|---|---|
| Discord token | OS credential store, service `org.serein.desktop`, account `discord-session`; at most 2048 bytes | Explicit logout / Forget saved login; invalid-token expiry also requests deletion |
| History and drafts | `dirs::data_local_dir()/serein/client.sqlite3` | Clear cached history also clears service images and keeps drafts; logout clears the authenticated account’s history and drafts |
| Messages | 500 per window, at most 20 stored channel windows globally, 48 MiB estimated text/metadata; SQLite main database capped at 64 MiB | Oldest touched channel evicted transactionally |
| Avatar, server-icon, profile-banner and message-preview PNGs | Account subdirectory beneath `dirs::data_local_dir()/serein/avatars`; 1 GiB / 4096 files per account, 90 days since last use, at most 2 MiB per preview (512 KiB for icons/avatars) | Clear cache or account logout; versioned avatar/icon/banner keys and hashed media-source keys separate changed images |
| Selected profile metadata | One session-memory record, at most 64 KiB; profile response body at most 256 KiB | Closing/changing the profile, session reset or logout; no SQLite profile table |
| Explicit attachment downloads | User-selected destination, 1 byte through 100 MiB per original file; one active dialog/transfer; randomized sibling partial while writing | Cancel/error removes the partial when possible; completed downloads remain user-owned outside cache cleanup |
| Selected upload source | One session-only path (4096 encoded bytes), filename (256 UTF-8 bytes) and size/modified metadata; file at most 20,000,000 bytes, read in 64 KiB chunks | Removal, send completion/failure, cancellation or session teardown; source is never copied to a recovery/cache file or deleted |
| Drafts | 64 globally, at most 2 MiB content; each draft at most 8192 UTF-8 bytes | Clear draft, confirmed send, or account logout |
| Appearance | One application-wide SQLite row: Light or Dark; absent means System | Select System to remove the override; retained across account logout |
| Reading/layout | One application-wide SQLite row with three bounded scalar fields | Reset reading and layout removes only this override; retained across account logout |
| Theme preset | One application-wide SQLite row (`theme_variant`, ≤32-byte key such as `onyx`); absent means Default | Select Default to remove it; unknown keys are ignored; retained across account logout |
| SQLite working files | DELETE journal mode, in-memory temporary tables, 2 MiB page cache; transaction journal may temporarily add disk usage | SQLite transaction completion; normal SQLite crash recovery |
| Voice credentials, DAVE identities/keys and PCM/Opus audio | Session memory only; one call, bounded media queues; no recording or audio cache | Hangup, failure, logout and application teardown; no forensic-erasure claim |
| Audio devices and push-to-talk preferences | Session memory only | Application exit / UI reset; not saved in SQLite |
| Authentication page | Wry incognito on Windows/macOS; ephemeral WebKit6 NetworkSession on Linux, destroyed on token handoff/cancel/timeout | Platform engine teardown; OS artifacts not promised erased |

Typical database directories: macOS `~/Library/Application Support/serein`, Windows `%LOCALAPPDATA%/serein`, Linux `$XDG_DATA_HOME/serein` or `~/.local/share/serein`. The Unix directory is private (0700). Database contents are **not encrypted by Serein**. OS token protection does not encrypt history, backups or drafts.

The app writes no background log, analytics, crash upload, saved password, MFA ticket, or plaintext credential file. A separate credential-free CDN downloader loads visible avatars, server icons, profile banners and validated service-proxied message images. Build outputs, this documentation, synthetic test databases and package files are development artifacts.

SQLite work is serialized on a worker. Normal startup opens the database to load appearance and reading/layout preferences before authentication; `--demo` does not open the database, credential store or network. Schema version 8 adds the bounded reading/layout singleton while preserving author metadata, embeds/suppression, attachments, mentioned users and unsupported-content presence bits; older history and drafts remain readable. Embed and attachment JSON are each capped at 256 KiB per message; mentioned users are capped at 100 entries and 128 KiB JSON. All three contribute to eviction accounting. Signed original/preview URLs and mention names/avatar hashes may be retained in unencrypted cached message metadata. Draft save status is visible; a full queue or disk failure is reported and must not be described as saved. An interrupted send may leave a saved draft for content Discord already accepted: recovered text never automatically sends. Normal close waits for queued store work if necessary; logout orders one transactional account deletion after earlier writes. Deletion is not a forensic erasure guarantee.

Saved recovery text prefers the current nonempty draft, otherwise the most recent unresolved send in that channel. Only a matching own-author/channel/nonce confirmation updates this recovery record. This is one recovery draft per channel, not a durable multi-message outbox; additional unresolved sends remain in RAM and the close prompt warns before discarding them. Incoming hidden-channel events do not rewrite the active cache; accepted active-view changes are coalesced into at most one snapshot per event-drain pass.

Source checks verify disabled eframe persistence, disabled REST cookies, one renderer, and an ephemeral webview request. egui-wgpu 0.36.2 creates its render pipeline with `cache: None`. Avatar decoding and disk I/O run on a dedicated worker; egui receives bounded decoded results. These are chosen implementation settings, **not a renewed no-storage requirement**.

Offline SQLite tests exercise real temporary-file reopen, schema upgrade, appearance reset, read-only failure, draft capacity rollback, account isolation, 20-channel eviction and atomic logout rollback/deletion. They remove their synthetic test files. OS credential-store and webview write tracing remain unverified. Windows WebView2 may create user-data/runtime artifacts even for InPrivate mode; this must be measured, and old WebView2 runtimes that ignore incognito must not be claimed ephemeral. No zero-byte storage claim is made.

A process-write trace was attempted with `sudo -n fs_usage -w -f filesys -t 3 <synthetic-app-pid>`; the OS returned “a password is required.” No trace was obtained. The account/cache code and synthetic SQLite files were tested, but actual process-write behavior is not certified.

Current image limits include the GIF and larger-viewer features added after September 10.
One worker decodes serially while up to four credential-free downloads overlap, with
128 bounded keys waiting and two decoded results queued. Ordinary encoded bodies are
capped at 2 MiB, animation bodies at 8 MiB and larger-viewer bodies at 16 MiB; four large
downloads can therefore hold 64 MiB of encoded payload, separately from decoder memory.
The completed encoded source is released before waiting to deliver its decoded result.
Avatar/icon decoding accepts at most 512 KiB encoded, 256×256 source, 1 MiB decoder
allocations and 128×128 output. Previews/banners use 1024×1024 source, 8 MiB decoder
allocations and a 512-pixel output edge. Larger-viewer images allow 4096×4096 source,
96 MiB decoder allocations and a 2048-pixel output edge (16 MiB RGBA per image).
GIF/WebP animations retain at most 80 frames with a 160-pixel edge, about 8 MiB per clip.
Two queued large stills can retain 32 MiB of decoded pixels; active decoding, image
conversion and framework/driver allocations are additional. Shared textures are bounded
by 256 entries / 64 MiB, with a separate four-animation / 16 MiB retained-pixel budget.
These are component ceilings, not measured whole-process RSS. Disk eviction retains only
32 candidate paths at a time. Worker completion fences replacement and deletion, so
logout/clear cannot race an older worker's writes. Picture-cache failures appear in
local-storage status. Disk cache contents are unencrypted. Category collapse remains
session-local.

Storage commands and results each retain their 16-item limit and have separate 16 MiB
estimated allocation budgets. Reservations include vector/string capacity and metadata
allowances and are released when work/results are consumed or dropped. Byte exhaustion
rejects command admission through the existing unsaved/cleanup handling; the storage worker
waits for result capacity without dropping completions. One completed result awaiting
admission and the SQLite working set are additional. History payloads retain the 500-row /
4 MiB limit. A connection-local byte total avoids rescanning all history on each save;
SQLite write/version counters invalidate it after other writes. The disk schema and
transactional eviction limits are unchanged.

Each remote-video decoder queue admits at most 64 access units and 16 MiB of allocated
encoded capacity, including the access unit being decoded. Exhaustion uses the existing
frame-drop/keyframe-recovery path. At most 16 reassemblers retain partial access units
of 2 MiB + 64 KiB each; discarded partials release capacities above 256 KiB. Decoder,
reference-picture, hardware and displayed-frame memory are additional. The synchronous
software-video sink borrows the reusable RGBA buffer, avoiding a full-frame clone before
conversion to UI pixels. No live media was used to establish these implementation bounds.

Voice introduces no application audio files, recordings or voice-key store. Device preferences are saved locally as described above. Voice tokens/session IDs use redacted, zeroizing buffers and never enter SQLite or diagnostics; DAVE identities are regenerated for a new call. Eight-frame PCM queues, bounded Opus packets and one bounded decoder/jitter/PCM working set per remote speaker (up to 63) are transient media allocations, not disk caches. Guild voice rosters are session-only with 4,096-entry and 1 MiB budgets; they are never persisted. Upstream cryptographic tracing is compiled out. Audio-device shutdown is fenced before another device session starts. Synthetic crypto, transport and device-free capture-gate tests passed; actual audio-driver/permission artifacts and process writes during a physical call have not been traced. OS microphone permissions and driver behavior are outside Serein's cache-clearing guarantee.

The optional READY voice-user lookup and per-snapshot/passive-update member lookup each admit
at most 4,096 unique nonzero users / 1 MiB of estimated model storage, checking before insertion.
Duplicate or excess metadata is discarded rather than ending login. These estimates use the
existing user/member storage accounting and exclude BTreeMap node overhead. Missing identities
use the existing participant-ID fallback. READY users are cleared after READY_SUPPLEMENTAL,
disconnect or session reset; member lookups are temporary. Disconnected states and channels not
admitted for voice do not consume the actual roster's count budget. The 4 MiB wire bound and
the actual roster's 4,096-entry / 1 MiB bounds are unchanged; no new disk storage is introduced.

Image attachment metadata remains bounded by 10 attachments / 64 KiB retained metadata and 256 KiB JSON per message, including original/proxy signed URLs. It counts toward existing window, pending-patch and database budgets. Decoded pixels reuse the shared media worker/cache; spoiler attachments are not requested before explicit reveal. Profile metadata (bio, pronouns, badges, connections and mutual-server summaries) stays in the single bounded RAM view. Profile and server-specific banner/avatar pixels may remain in the shared account image cache after closing the profile; cache clear/logout removes them under the same policy.

Explicit Download creates an original attachment file only at the user-selected location. Suggested filenames are sanitized; downloads never reinterpret message filenames as destination paths, follow redirects, or send credentials to the CDN. Existing regular files are replaced only after native Save confirmation and a complete, flushed transfer. A new destination is published without overwriting a file created meanwhile. The one worker closes/removes its sibling partial on cancellation or failure; cleanup failures are visible. Forced termination or a filesystem error can leave a `.serein-*.partial` sibling, and filesystems without hard links cannot use the atomic new-file publication path. Normal close waits for the active worker; a cancelled native dialog must still be dismissed. Downloads are explicit user files, not account cache entries, and survive logout/cache clearing. Limits and transfer bounds are strictly enforced.

Conversation search queries and result snippets are session-only, limited to one 25-result / 64 KiB page and a 256-character query. Neither is written to SQLite or diagnostics. Opening a result uses normal bounded history retrieval, whose revalidated messages can enter the existing account cache.

Archived-thread pages share the same exclusive read/result slot with search and pins. At most 25 channel summaries / 64 KiB are retained from a response capped at 512 KiB; member payloads are ignored. Request/next cursors are fixed-size timestamps or IDs. Pages and cursors are not persisted. Opening admits one transient channel within existing account item/byte navigation limits, then uses ordinary bounded history caching. Leaving retires transient navigation, not saved drafts or cached history; explicit revocation still invalidates inaccessible content. No archive directory cache, background paging or added worker queue exists.

Pinned-message summaries share search's single session-only 25-item / 64 KiB result slot and 512 KiB response limit. Manual older-page navigation replaces that slot instead of accumulating results; two optional fixed-size timestamp cursors track the request and next page. Failed older requests can be retried deliberately, with no background retry. Opening a result uses ordinary bounded history caching; pin snapshots/cursors are not written to SQLite. PR screenshots are synthetic development evidence and are excluded from packaged documentation.

Uploads do not persist local source paths, signed staging targets or file bytes. Pending filename/size labels remain bounded session metadata; existing recovery drafts retain only composed text, so retrying an attachment requires selecting the source again. Files are opened for reading and checked for observable size/modification changes; this is not an immutable snapshot guarantee. Cancellation stops the local job, but bytes already uploaded to Discord staging may remain there without a created message; no remote cleanup or retention guarantee is claimed. Completed messages and their returned attachment metadata can enter the existing bounded history cache. The OS file picker may retain OS-managed recent-location history. No new application log or hidden upload recovery store is introduced.

Twemoji artwork is public bundled data, not an account cache: one 6,002,931-byte PNG
and a fixed 4,009-entry Unicode index are embedded in the executable. Startup decodes
one 2,048×2,016 RGBA atlas (15.75 MiB) before the first render callback; the GPU texture
has the same pixel payload, with driver overhead additional. Decode/conversion/upload
can temporarily hold multiple copies. The context retains the single atlas until exit,
including across logout; there are no emoji downloads, disk writes, or growing texture
queues. Unknown sequences and explicit text-presentation selectors remain font text.

Custom server emoji catalogs live only in session navigation memory: at most 1,000 entries
and 256 KiB allocated data per server, including names and role lists, within the shared
4 MiB navigation budget. Reconnect READY replaces catalogs; full emoji-update events replace
a server catalog; deletion clears it and old session generations cannot repopulate it.
No catalog table or schema migration is added. Custom PNG previews reuse the existing
account-isolated image worker/disk cache and its 256-texture / 64 MiB GPU working set, request
and retry limits, 512 KiB icon decode input and 128×128 decoded dimension cap. They use
validated numeric `emoji-ID` keys and credential-free Discord CDN PNG requests (64px static
preview), never arbitrary URLs or automatic animation. Cache clear/logout follows the existing
image-cache policy. Synthetic IDs 9001/9002 only get local generated images in `--demo`.
The standard picker palette has 3,953 fixed named entries and renders only viewport rows;
search input is capped at 64 characters. Picker insertion honors character and total draft
capacity limits and never sends a message on selection.


Channel obfuscation and accepted READY removals invalidate inaccessible history using the existing
account-wide ClearHistory operation; readable-to-unsupported channel changes count as removal.
This deliberately trades a broader history refetch for no new per-channel deletion API. Cache
hydration requires the current request, a pending empty loading view, a loaded text channel and
matching message channel IDs. Late disk/HTTP responses cannot refill a revoked view. Drafts remain
available for recovery. This does not erase explicit downloaded files or promise deletion of
already requested image pixels, OS artifacts or remote attachment staging data. No schema,
persistent visibility list, new worker queue or credential storage is introduced.

The permission mirror is session-only and never enters SQLite, credentials or diagnostics.
It retains at most 4,000 guild/channel records, 16,384 guild roles and 32,768 overwrites;
per guild/member role lists stop at 512, per-channel wire overwrites at 1,000. Other members'
overwrite entries are validated then discarded; all role overwrite entries remain so later
self-role changes can be calculated. A 2 MiB estimated allocation budget includes reserved
space for at most 4,000 cached decisions. Updates clone the bounded metadata for atomic
validation; that temporary copy is additional peak memory. These estimates are not process
RSS. Decisions expire at timeout boundaries, are recomputed after clock rollback and are
cleared on metadata updates. Logout/READY replace the session mirror.

Current VIEW and READ_MESSAGE_HISTORY are required for HTTP/cache admission and history
persistence. Read-access loss, including thread parent/type changes, queues the existing
account-wide ClearHistory operation. Sending-only permission changes do not erase cached
history. VIEW-only live messages stay in the bounded RAM timeline without being persisted.
Existing recovery drafts and explicitly downloaded files keep their documented lifecycle.
No schema change or external runtime dependency is added; UI tests reuse the existing
workspace test-support crate through a dev-dependency.

Notification/read activity and remote notification preferences remain bounded session RAM only;
the local notification opt-in is saved in `app_preferences`. The OS receives generic
fixed text only after explicit opt-in and may keep its own notification/permission history.
Logout invalidates queued work and requests dismissal; this does not erase OS records.
See notification limits and platform behavior. Composer artwork uses
the existing Twemoji atlas and custom-image cache; saved drafts keep their original wire
text, with no extra rendered-token storage.


Loaded People presence is session-only within the existing 100-row / 128-KiB member mirror.
The Gateway additionally holds at most 100 pending IDs and complete normalized presence values
(each at most seven status bytes plus 128 characters / 512 bytes of custom text), plus bounded
BTreeMap node overhead. An emitted compact batch is limited to 100 entries / 64 KiB including allocated vector/string capacity and uses the existing
bounded event queue. Wire decoding keeps the existing 4-MiB cap, consumes at most 16 activities
one at a time (state <=4096 bytes, emoji name <=128 bytes), and drops unrelated fields.
Presence growth is checked against the member budget before mutation. Full snapshots and
subscription/session invalidation clear pending status batches. No presence, activity or client
device history is saved to SQLite, logs or diagnostics; status-only changes do not persist chat
history or invalidate timeline layout. These are component bounds, not process RSS measurements.


The conversation switcher retains only its open-state flags, focused control ID and a query of
at most 128 characters / 512 UTF-8 bytes. Each open frame builds at most 20 labels from bounded
channel/guild names; each field is limited to 128 characters. Matching normalizes one eligible
channel's bounded names and at most 64 known DM recipient names at a time, then drops them.
It reuses the existing navigation limits and permission cache, with no persistent query history,
search index, directory fetch or new worker/queue. Closing clears the query; logout resets the UI.

## September 10: message type retention

Schema 7 adds one checked integer `message_kind` (0..255) per cached message, with no new payload or cache. Existing unsupported rows migrate to the unknown sentinel 255; ordinary rows use 0. Reloaded history supplies the actual type. Existing account/item/byte/page limits remain in force. Migration and reopen/roundtrip tests cover retained rows and invalid values. Builds limited to schema 6 cannot reopen this cache. PR #28 independently uses schema 7 for content markers; merge both column-detected migrations and both save/load fields when integrating these branches.


### Opt-in synchronization and compatibility diagnostics

Voice performance diagnostics (`SEREIN_VOICE_DIAGNOSTICS=1`) are also off by default.
They retain at most eight fixed-size numeric reports in a worker queue (under 2 KiB),
plus one report per producer and one being written. One background writer formats
reports and caps attempted stderr output at 128 reports AND 64 KiB per process,
across calls. Queue overflow drops diagnostics without delaying media. A blocked
stderr can stall only that single diagnostic writer. No files, identifiers, device
names, payloads, audio, keys or telemetry are produced. Explicit shell redirection
is owner-managed; unrelated output and appended runs are outside these limits.
See [voice CPU diagnostics](voice.md#investigating-high-cpu-during-a-call) for usage.

`SEREIN_MEMBER_DIAGNOSTICS=1` enables fixed-label member synchronization diagnostics;
`SEREIN_GATEWAY_DIAGNOSTICS=1` enables Gateway compatibility diagnostics. Both are off by default.
Each enabled scope has a hard budget of 64 attempted records AND 8 KiB of formatted UTF-8
output per Gateway run, shared across reconnect attempts. The desktop adds at most one
fixed-label terminal session-failure line per enabled scope (less than 256 bytes). Failed or
partial writes consume the attempted record's budget; a closed stderr never fails the session.
No diagnostic strings, raw events, queue, archive, database entries or telemetry are retained.

Member labels distinguish subscription/cancellation, empty or populated SYNC, identity mismatch,
decode/range/capacity failure and timeout. Gateway labels distinguish an unsupported dispatch,
a missing/empty dispatch name, and an unsupported opcode that stops the connection. Unsupported
dispatches continue to be ignored and grant no capabilities. The diagnostic deliberately cannot
identify the exact unknown service event: even its received name is excluded, along with payloads,
credentials, account/channel/list IDs, usernames, message content and signed URLs.

Output goes only to stderr. Serein creates no log file; explicit shell redirection is owner-managed
and can also capture unrelated framework output, to which these limits do not apply. Repeated
application runs appended to one external file are not bounded by a single Gateway-run budget.
Windows GUI builds may have no inherited stderr; launch with deliberate stderr redirection to
capture diagnostics. Do not use a real owner session in default tests. Synthetic tests check
redaction, UTF-8 byte/line limits, disabled and broken-output cases, and message delivery plus
heartbeat sequencing after unsupported dispatches. No live interoperability claim follows.

### Existing DM call presence

Session memory keeps at most 64 ongoing one-to-one DM channel IDs (512 bytes of ID storage,
plus the Vec header), independently of the active local media session and incoming ringing.
No voice secrets, participant payloads, audio, or new disk entries are retained for this list.
Duplicate updates reuse an entry; at capacity, the oldest entry is evicted. Opening a DM
requests its call state again through the bounded existing command/signaling queues. There
is no background polling or all-DM subscription. Deletion/unavailability or channel removal
clears the matching entry; fresh READY, resync and logout clear the list. A resumable
disconnect retains it for replay, with Join disabled until Gateway connectivity returns.



The reliable Gateway/HTTP-to-UI queue accepts one bounded navigation refresh burst
(up to MAX_NAV + EVENT_SLOTS = 4,008 items) within the existing 32 MiB aggregate estimated
envelope/permit byte budget. Each event remains capped at 4 MiB. Receiving or dropping an
envelope releases its permits; full/oversized admission still fails visibly. The UI drains
eight reliable events per frame and repaints only while work remains. This fixes normal
GUILD_CREATE channel fanout exceeding the old eight-item queue; it does not enlarge the
byte budget or silently discard reliable events. These are component limits, not RSS.

Member role display reuses session-only permission metadata: at most 512 roles per guild,
16,384 across the mirror, within its existing byte budget. Each name retains at most 100
non-control characters; owned string capacities count toward both event and mirror budgets.
Member rows retain at most 512 role IDs, bounded while decoding and checked at state admission;
their vector capacities count toward the existing active-pane byte bound. No role directory,
new cache, persistent schema, or network endpoint is introduced.

Unread/forward navigation reuses the cancellable history worker, 50-message response limit,
500-row/4-MiB active timeline and existing global resident budget. It replaces the selected
window, preserving bounded deletion/reconciliation metadata and drafts. Three fixed-size
cursor fields and a full-page flag are session-only. Forward-target pages are not restored
from the SQLite latest-page cache or parked in the resident recent-window cache. Accepted
message metadata remains subject to ordinary account history persistence; no new cache,
queue, directory fetch or background pagination is introduced.


Role/everyone/silent notification fields are session-only message-arrival metadata. Up to 100
unique positive role IDs and 800 retained role-vector bytes are admitted per message, charged
by allocated capacity in Message/Event/timeline budgets. SQLite restoration supplies empty/false
notification fields; reading history never generates an alert. No schema change is needed.
Queued notifications retain role IDs and direct/everyone provenance to recheck delivery, within
the existing 32-item/16-KiB ceiling including unused deque slots. Observed badge records remain
4096 fixed entries/128 KiB, without retaining role arrays. No additional cache or queue exists.


### Received activity metadata (September 10, 2026)

Rich presence stays in RAM: up to four activities per user, each name/details/state field
128 characters / 512 bytes. Custom status retains its separate 128-character / 512-byte limit.
Active member rows share the existing 128 KiB pane budget. Known DM recipients additionally
use a FIFO cache of at most 256 records / 512 KiB including vector storage and owned strings.
Gateway updates coalesce for 100 ms, at most 100 users / 128 KiB per batch. Initial friends are
filtered to the first 256 known DM recipient IDs and emitted in bounded batches. Unknown users
are never retained in the core cache. Disconnect hides cached DM presence until successful
resume; fresh READY, resync, failure and logout discard it. Removed recipients are pruned.
One optional artwork reference per activity is retained: two asset IDs, one application ID, or
a Discord media-proxy path of at most 1,024 bytes. Its enum/vector storage and allocated path
capacity count toward the same presence budgets. Secrets, buttons and raw event payloads are
discarded. Images needed by the visible profile share the existing credential-free image worker,
256-texture / 64-MiB texture cache and account-isolated 1-GiB / 4,096-file / 90-day disk cache.
Application-icon metadata responses are capped at 64 KiB and discarded after validating the
application ID and icon hash; only the decoded-valid PNG enters the disk cache. Application icons
cached by application ID can remain stale until normal cache expiry or clear-cache. Proxy keys
use the existing SHA-256 disk filenames. No new cache, schema or dependency is introduced.
### Native font fallback (egui main experiment)

Eframe `system_fonts` enumerates installed fonts on a background thread and uses
read-only memory-mapped OS font files for missing glyphs, including native color
emoji. No font download or font-file copy is added. Upstream fallback can wait
for enumeration on its first missing glyph; its font/cache memory is framework
overhead, separate from Serein message/image budgets. OS font availability and
emoji coverage vary by platform. Bundled text faces and Twemoji remain in use.


### Inline MP3/WAV preview (September 11, 2026)

A deliberate Play action starts one lazy output-only worker. One replaceable request
retains bounded validated attachment URL metadata; no account credential is sent.
The credential-free downloader refuses redirects and content encoding and requires
the declared length, capped at 20 MiB with a 60-second deadline. Audio stays in RAM:
at most 64 MiB of decoded f32 samples and ten minutes, mono/stereo at 8?96 kHz,
plus bounded decoder/transport buffers. PCM vector reallocation may temporarily
retain old and new allocations (up to roughly 128 MiB combined), separately from
the encoded buffer, decoder, audio device and process overhead. MP3 ID3 tags are skipped without decoding;
WAV metadata is removed before demuxing. No media files or playback preferences
are persisted. Playback stops when its card leaves view, the attachment changes,
the conversation changes, the window is minimized/occluded, or the session ends.
An atomic generation gate mutes obsolete output; the single worker releases its
stream/buffers on cancellation. Pausing retains the current bounded decoded clip.

## Screen sharing

Screen/window labels, selected source identifiers, settings, raw pixels and encoded video exist only in session memory. They are not written to SQLite, diagnostics, previews or video files. Sources and video queues use the limits in [screen-sharing compatibility](discord-compatibility.md#outgoing-screen-sharing--september-11-2026). Stream credentials and DAVE identities are ephemeral and redacted; the signing key is shared with the active voice call and zeroized when its final owner drops. Native OS/driver capture surfaces are distinct from application-owned frame buffers. Synthetic PR screenshots are development evidence, excluded from runtime assets.


### Own game activity (September 11, 2026)

Sharing is off by default. The application-wide `game_activity` SQLite singleton stores
one constrained boolean; disabling deletes the override. The independent additive table
is created even for existing schema-10/12 databases, requires no message migration, and survives
account logout like appearance. A failed load stays off; failed writes are visible in settings.
Preview controls never load or save this preference.

While enabled in an authenticated connection, Serein owns one standard Discord IPC endpoint
and at most eight connected game workers. Windows uses a current-user-only pipe DACL and
rejects remote connections; Unix uses a 0600 socket and checks peer UID. Occupied paths are
never replaced or unlinked. Unix removes only its own device/inode on teardown; Windows
explicitly disconnects clients, including blocked writers. No process enumeration remains.
The handshake returns only the current user ID/name and empty legacy avatar/discriminator
fields; no token, chat, account-read, authentication, call or microphone API is exposed.

Each client frame is capped at 16 KiB before allocation. Replies assemble one temporary
buffer capped at 16 KiB plus the eight-byte header per writing client, released after the
write completes or is cancelled. The handshake timeout is 10 seconds,
partial-frame and write deadlines are five seconds. Idle clients do not poll. At most one
new client is admitted per five seconds; each client processes at most ten frames per second.
A 16-item update queue carries activities bounded to 1,152 string bytes plus fixed fields;
eight latest per-client activities and the Gateway current/last values have the same bound.
The latest updated connected game wins; clearing/disconnecting it restores another active game.

The UI report retains that game's name/details/state (at most 384 UTF-8 bytes) and one
fixed-size registered artwork/application reference. The state owner keeps one validated
local display activity capped at 4 KiB of retained heap. Profile cards and member rows
borrow it for the current account, without copying it into remote presence caches.
It is not persisted and is cleared on sharing/game/session teardown. Equal reports do
not invalidate the timeline; changed details/artwork repaint even when the name is unchanged.

Each connection lazily requests public application metadata and registered assets once, using
credential-free HTTPS with redirects/proxies disabled and ten-second request deadlines.
A shared lookup mutex serializes requests and preserves Retry-After cooldowns across clients.
Responses are capped at 256 KiB, asset lists at 1,024 entries with 256-byte names. These are
session RAM only (up to eight lists), not disk caches. Missing artwork is omitted; name lookup
failure clears that connection and reports an error. Game-supplied URLs, secrets, buttons and
join/party actions are never forwarded. There is no activity history or telemetry.

Disabling sharing cancels the listener, clients and metadata work and queues an empty Gateway
activity; publication including clears remains subject to the existing five-second interval.
Connection teardown cancels IPC with the authenticated session. Demo mode never binds IPC or
looks up metadata. The saved boolean and database schema are unchanged.


### Tray and account activity privacy (September 11, 2026)

The tray icon is off by default. One strict integer in the existing independent
`minimize_to_tray` singleton table survives restart/logout; disabling deletes its
row. Schema 12 receives the additive table without migrating messages. Demo toggles
are memory-only. Failed loads stay off and failed saves remain visible.

The Windows adapter owns one icon/menu and a window procedure hook on the existing
UI thread. Three event bits coalesce Show/Quit/failure; there is no worker, polling
timer, autostart or new dependency. Minimizing stays in the taskbar; enabling or disabling
the tray icon does not hide or restore the window. Shell failure restores the window.
Closing still follows existing application exit gates.

While local game sharing is enabled, one cancellable account-settings operation reads
Discord's actual sharing preference. A one-slot request channel permits an explicit
refresh or enable action; a fixed-size watch result carries completion. Only the
explicit Enable on Discord action can write the account preference. The existing
1 MiB response limit and 4,096-field protobuf parser apply; the retained status
subtree is capped at 16 KiB and discarded after each request. No raw settings are
logged or saved. Server activity diagnostics borrow at most 64 KiB / 16 sessions /
16 activities per list and retain only a fixed enum, never session identities or
raw presence payloads. Connection teardown clears these reports and workers.

### Account menu presence (September 12, 2026)

Presence and custom status are session-only. The editor retains one draft capped at
128 Unicode characters (512 UTF-8 bytes), and the host publishes one replaceable
watch value. The gateway keeps the desired and last-attempted bounded values;
there is no status history, disk write, or additional queue. A new login resets
the choice and account generation changes clear the editor draft. Demo changes
never publish, persist or initialize account transports.

### Inline attachment video

Video data is memory-only. One lazy worker handles the latest requested attachment,
with a replaceable pending request and cancellation fencing. The network source retains
up to eight 256 KiB ranges (2 MiB, including an in-flight range); responses are checked for exact range/total/body lengths and reject
redirects and content encoding. Encoded attachment size is capped at 100 MiB.
The application queues at most two 1080p RGBA frames, one replaceable display frame,
one UI texture, one second of stereo float PCM (at most 768,000 bytes), and one decoded
audio packet plus at most two seconds of timestamp-gap silence. Each native decoded
sample is rejected above 16 MiB before copying. OS decoder/GPU allocations are additional
and are released with the player. No file cache or media URL/byte diagnostics are written.


### Friends overview

The friends page reuses the bounded relationship store (4,000 friends / 2 MiB)
and the existing presence cache (256 records / 512 KiB), now admitting known
unblocked friends as well as DM recipients. Startup presence admission keeps its
256-user bound. Presence outside retained/received data stays unavailable.
The UI retains only a 128-character search and Online/All selection, renders visible
64-point rows, and reuses the avatar cache and existing user actions. No new storage,
network endpoint or friend-management writes are added.

### Opt-in Windows startup

General settings can register this executable for the current Windows user's sign-in,
off by default. The sole source of truth is the `Serein` REG_SZ value under
`HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`: a quoted absolute
executable path with `--autostart` and optionally `--start-minimized`, at most 260
UTF-16 code units plus its terminator. It contains no account data or credentials.
Turning startup off removes only this value. It survives logout; moving the portable
executable requires enabling startup again from its new location. Windows Startup Apps
can independently block the entry; Serein does not override that OS decision.
One off-thread operation and one fixed-size completion may exist at a time. Failed
writes restore the last known setting and display an error. Demo mode does not read
or write the startup entry. Automatic launches use the existing saved-login behavior;
ordinary manual launches remain visible even when Start Serein minimized is selected.

### Account-type badges (schema 14)

Cached messages retain one checked `account_kind` integer: 0 (ordinary/unknown), 1 (bot),
2 (explicit application-generated author). The existing independent `webhook` marker
continues to control webhook profile behavior. Migration adds the column transactionally;
legacy messages default to 0 until service history refreshes, without guessing from names.
Unknown stored values are rejected. No new table, cache, queue, payload collection or
network request is introduced; account, item, byte and page limits remain unchanged.
Older schema-13 clients cannot reopen a schema-14 cache. User metadata serialized in
bounded existing caches defaults missing kinds to ordinary/unknown.

Message delete protector is opt-in and session-only. Retained deleted payloads share
the existing 500-row / 4 MiB timeline limit and resident-history budget. They are
excluded from normal message iteration, service actions and disk cache writes.
Disabling the plugin, logout, lost channel access or eviction releases the payloads.
The usual disk-cache deletion still runs when a deletion arrives.


### Server role editor

One in-memory role catalog is retained for the open guild: at most 512 roles,
512 KiB including allocated string/vector capacity, and 256 bounded feature names.
There is one UI draft and one server-administration request at a time; role-member
search reuses the existing bounded member page. Unknown role permission bits are
retained and edits carry a changed-bit mask. Closing settings/logout releases the
editor state; no new database table or persistent role cache is added.

Role icon preparation reuses the off-thread image worker, bounded source input and
PNG data URI limits. Its preview is at most 128 by 128 pixels; the completion is
scoped to account generation, guild, role and picker request. The retained core
request drops the image payload. CDN role icons share the existing bounded image
cache and are confined to a validated numeric role ID and icon hash.

### Server invite settings

One active guild invite snapshot uses the existing in-memory administration lane:
at most 1,000 invites and 1 MiB including allocated strings/vector capacity, with
at most 256 feature names and 100 assigned role IDs per invite. REST bodies are
limited to 2 MiB (64 KiB for a revoke response). Larger responses fail visibly;
they are not silently truncated. Codes and inviter metadata are released when
settings close, access is lost or the account resets. No invite database, logs,
background collection or automatic write retry is added. Create Invite retains
the existing bounded, session-only invite dialog behavior.


Server integration settings retain one guild's on-demand metadata in session RAM:
at most 50 integrations and 1,000 webhooks, further bounded by 1 MiB combined.
The HTTP decoder caps each list response at 2 MiB and write responses at 64 KiB
(4 KiB for empty delete responses). Webhook execution tokens and URLs have no
model fields and are skipped during decoding. The editor retains one bounded
80-character/320-byte webhook name draft; no integration data or draft is written
to SQLite. Closing settings, changing guild/session, and permission revocation
release the applicable metadata. Existing bounded avatar caches remain shared.

### Messaging permissions

The account preference snapshot stays in session RAM, with at most 4,096 IDs in
each of two guild lists (64 KiB combined allocated ID storage). A single pending
update uses the existing command queue; bulk changes carry at most 4,096 IDs
(32 KiB). HTTP responses are capped at 1 MiB and preserved protobuf subtrees at
128 KiB each. Disconnect, resync and account reset invalidate the snapshot and
pending request. No preferences or raw settings payloads are saved to SQLite or logs.

### Server audit log

One active guild audit view retains at most 500 entries, 1,000 referenced users
and 2 MiB of metadata in session RAM. Each on-demand response has at most 50
entries and 100 users within 1 MiB decoded metadata; HTTP input is capped at
2 MiB. Changes are bounded display values with explicit absent/null states,
not retained arbitrary JSON. Individual values, nesting and per-entry change
counts are limited before admission. Filters replace the current history;
closing settings, switching guild/session and permission loss release it.
No audit log, filter or expansion state is saved to SQLite or diagnostic logs.
The shared bounded avatar cache is reused.

### Application updates

The existing device-wide `app_preferences` JSON stores `auto_update` (default false)
and `update_nightly` (default true). Missing fields in older settings use those
defaults; the existing 16 KiB row bound still applies. These preferences survive
account logout. Update checks wait for preferences to load; demo actions remain
in memory and never open update transports or create installation files.

The updater uses a separate credential-free HTTPS client for the Serein GitHub
release repository. It keeps one worker/result slot, at most 2 MiB of release
metadata, a 64 KiB checksum list and one streamed archive capped at 512 MiB.
Progress is coalesced into one atomic byte counter; release bodies, URLs and
package contents never enter application diagnostics or account caches.
The ZIP central directory is checked before allocation (4 MiB / 8,192 entries);
ZIP64 packages are rejected. Extracted data is capped at 1 GiB. Paths, duplicate
names, symlinks and special files are validated before writing to private staging
beside the installation. Staging records the app/helper owner and is reused or
cleaned before another download; backups from interrupted replacements are kept
for recovery and block another installation instead of being deleted.

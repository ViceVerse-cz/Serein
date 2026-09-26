# Discord compatibility — checked 2026-09-10

## Message pictures and gifv - September 23, 2026

A single chat image fits inside 550 by 350 points, the 11 to 7 mosaic cap used by the official client.
The image keeps its aspect ratio. Opening it draws the file at its own pixel size when that fits the window.
Scroll zoom enlarges it. A larger file fits the window first.

A still message picture requests an unofficial media-proxy WebP with `quality=lossless`, sized to the texture.
An animated GIF or WebP requests a sized proxy GIF first.
`quality=lossless` and `animated=true` are unofficial.
A rejected request stays a placeholder.
Tenor and Klipy previews stay on their original URL.

A gifv embed with an allowed MP4, MOV, or M4V URL fetches that file and plays the frames through the platform video decoder.
A WebM URL stays on the poster or GIF path, because this decoder does not read WebM.
If the clip does not decode to at least two frames, the embed falls back to its GIF or poster.
Live acceptance of the proxy query is unverified.

## Slash commands - September 22, 2026

Typing `/` opens a native, searchable command picker with built-in/application filters,
keyboard selection and a bounded argument form. Local commands are `/gif`, `/sticker`,
`/me`, `/msg`, `/shrug`, `/spoiler`, `/tableflip` and `/unflip`; they reuse existing
message, DM and media flows. Scheduled sends (`/schedule`) are not implemented.
Select with Up/Down and Tab/Enter, or click a row. Selection fills the composer;
the next explicit send executes a built-in. App command arguments appear as compact
labeled fields inside the composer, with contextual help above it. Use the composer
Send button or the configured Send Message shortcut (Enter by default) in a text
argument to submit; Tab moves between fields. Click
the command name to return to the picker. `/msg @user [text]` opens an existing DM (or a new friend DM) and
stages text for review without replacing an occupied destination draft. `/gif [query]`
and `/sticker [query]` open the existing picker. Failed app submissions expose the
error and retain fields behind Edit again; retries require another explicit submission.

Application discovery uses the unofficial normal-account
`GET /guilds/{guild}/application-command-index` route for guild conversations and
`GET /channels/{channel}/application-command-index` for one-to-one bot/app DMs.
The [maintained client implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/abc.py#L2403-L2457)
establishes these routes and their rate-limit sensitivity. The active conversation's
catalog loads on demand and is reused until navigation, invalidation or explicit refresh;
typing filters locally. Account-installed commands from `/users/@me/application-command-index`,
ordinary/group-DM app discovery, user/message context commands and Activities are excluded.

Slash submissions use type `2` on the existing unofficial `POST /interactions` route,
including the active Gateway session, nonce, received command ID/version and validated
typed arguments. Root commands, subcommands and one subcommand-group level support strings,
integers, numbers, booleans, static choices and user/channel/role/mentionable IDs.
Entity pickers use already loaded account data; they do not fetch a complete directory.
Required fields, declared limits, choices, command context and channel access are checked
before sending. Serein requires Send Messages (Send Messages in Threads for threads) for
all slash commands, plus Use Application Commands for guild application commands. The picker and
submission path also check default member permission bits and received application/command
overrides for the current user, roles and channel. User overrides precede role overrides;
role allows win among matching roles, and threads inherit the parent channel's rules.
Owners/administrators bypass command restrictions. Apps with no available commands are
hidden from the rail, and read-only channels disable the composer and all slash commands.
Permission changes that invalidate the catalog cause it to reload; changes made solely to
server command overrides may require the picker's Refresh action. These index fields are
[unofficial account metadata](https://github.com/dolfies/discord.py-self/blob/master/discord/types/command.py#L141-L171).
Discord remains authoritative, and bot-internal access checks cannot be predicted locally.
Age-restricted commands are omitted until account/channel eligibility is supported.
The inner command guild ID is
included only for guild-scoped definitions, independently of the invocation channel's guild.
See the [official command schema](https://docs.discord.com/developers/interactions/application-commands)
and [maintained submission implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/commands.py#L931-L973).

Autocomplete requests and attachment arguments are not implemented. An autocomplete-enabled
scalar accepts a manually entered value; required attachment arguments block submission with
an explicit explanation, while optional attachments can remain omitted. Commands run only
after an explicit submit and are never automatically replayed. Existing Gateway success,
failure, modal and private-reply handling is reused; HTTP acceptance is not application
completion. Offline fixtures and loopback HTTP checks cover the local contract; live
normal-account discovery, execution and application responses remain unverified.

## Stickers - September 20, 2026

The composer media picker has a Stickers tab with local name/tag/source search,
collapsible server and standard-pack groups, a section rail, session-only recent
choices, and a hovered preview. Clicking a sticker sends it as a separate message
and preserves the typed draft. Reply references, pending/failed delivery and the
existing nonce reconciliation are reused; uncertain writes are never retried.
Channel send and external-sticker permissions gate selection. Server stickers
outside their source server (including DMs) also require a session-confirmed
Nitro or Nitro Basic entitlement (`premium_type` 2 or 3), populated by READY and
updated by the current user's USER_UPDATE. Missing, unknown, Classic, or expired
entitlements do not unlock external stickers. Local selection and send checks
share this gate; Discord remains authoritative. There is no purchase flow. See
[Discord subscription benefits](https://support.discord.com/hc/en-us/articles/115000435108-What-are-Nitro-Nitro-Basic).

Received `sticker_items` and legacy `stickers` render transparent artwork in chat.
Clicking opens details and related previews; View More Stickers opens the source
in the picker. Standard packs and missing metadata load only on demand. Guild
catalogs hydrate from READY/GUILD_CREATE and GUILD_STICKERS_UPDATE. The official
[sticker resource](https://docs.discord.com/developers/resources/sticker) and
[Create Message](https://docs.discord.com/developers/resources/message#create-message)
document the object/REST shapes; normal-account interoperability is unofficial
and live-unverified.

PNG, APNG and GIF use the existing bounded image worker and animation preference.
GIF uses the media host specified by Discord's
[CDN reference](https://github.com/discord/discord-api-docs/blob/main/developers/reference.mdx).
Lottie JSON is fetched from Discord's documented CDN endpoint, capped at 512 KiB,
and rasterized off the UI thread to one 160px static PNG preview. The rendered PNG
shares the account-isolated image disk cache, so reopening a received sticker or
picker entry does not parse or render it again. Unsupported Lottie features retain
the sticker name and an unavailable placeholder. Long APNG/GIF animations share the
existing frame/pixel limits and may remain static. Sticker upload/edit/delete administration
and synchronized cross-device favorites are not included.

`--features demo -- --demo --demo-stickers` seeds an original offline catalog,
received message and picker. Tests use only synthetic data/local HTTP. Native
computer-use capture is blocked in this environment by a missing native pipe;
framebuffer previews do not verify OS input, accessibility or live compatibility.


## Server-wide member lookup (September 17, 2026)

`@name` autocomplete requests matching members beyond the first 100 subscribed
sidebar positions.
Queries use Gateway opcode 8 and nonce-matched `GUILD_MEMBERS_CHUNK` replies;
no administrator permission or bot-only REST member search is used. The
[documented Gateway request](https://docs.discord.com/developers/events/gateway-events#request-guild-members)
specifies the wire shape; normal-account interoperability remains unofficial and
live-unverified. Name matching is controlled by the service; numeric queries request
an exact user ID. Results are capped at 100 matches. The unfiltered live sidebar
still retains its first 100 positions and has no search field. Lookups are on
demand, debounced by 350 ms, spaced by at least one second, and time out after
15 seconds. No complete directory is downloaded. Offline debug validation covers
remote nickname mention insertion, stale replies and response limits.

## Chat links — September 16, 2026

Clicked message and embed links matching HTTPS `/channels/{guild|@me}/{channel}`
with an optional message ID navigate inside Serein. Exact discord.com and legacy
discordapp.com hosts, including www, ptb and canary, are recognized. Known channels
can be in another joined server or an existing DM/group DM. Message links reuse
the bounded history window and target highlight; loaded messages scroll locally.
Guild/channel identity and current view/history permissions are checked first.
Unknown channels (including unloaded archived threads), unsupported channel kinds,
and unavailable messages retain explicit unavailable/error feedback. Links do not
join servers, open unknown DMs or join calls. Explicit Open in Discord controls
still open the browser; unrelated links keep their existing confirmation behavior.
Parsing is click-triggered and capped at 2,048 bytes, with no new cache or transport.
Verification uses synthetic data only; live Discord interoperability is unverified.

## Forum post context menu — September 15, 2026

Forum/media post cards and sidebar entries support right-click and Shift+F10.
The menu reuses read acknowledgements and device-local favorites, and adds
follow/unfollow, close/reopen, lock/unlock, title editing, pin/unpin and confirmed
deletion through Discord's channel and thread-member routes. Owner/Manage Threads
permissions gate edits; locking, pinning and deletion require Manage Threads.
Thread mute durations and notification levels use the unofficial
`PATCH /channels/{id}/thread-members/@me/settings` route for followed posts.
One bounded, session-only details snapshot is loaded on demand. Archived cards
reuse the existing single transient channel admission without selecting the post.
The fast-pass debug check is synthetic; live interoperability remains unverified.

## Group DM calling — September 15, 2026

Group conversations now share one-to-one call discovery, ringing, Answer/Decline/Join,
media controls, camera, screen sharing and viewing. The existing unofficial private-call
Gateway and REST paths are reused with `guild_id: null`; multi-party encryption uses the
bounded authenticated voice roster already used by server calls. Participant metadata is
restricted to the account and current recipients. See [group workflow and limits](voice.md#group-dm-calls).
The offline group-call debug check passed for this `!fast` pass; live Discord interoperability,
physical devices and production readiness remain unverified.

## Gateway login payload bound — September 14, 2026

The [Gateway](https://docs.discord.com/developers/events/gateway) documents zlib-stream
transport compression but no READY size; a normal account's READY (unofficial, inlines every
server's channels, roles, emojis and user settings) is routinely larger than 4 MiB decompressed.
The Gateway transport, outer packet, READY and READY_SUPPLEMENTAL decoders accept up to
64 MiB. Ordinary dispatch decoders and REST bodies retain their existing limits. Account
navigation and permission projections support 131,072 entries, subject to a combined
128 MiB estimated allocation budget and a 64 MiB permission sub-budget. These are finite
client safety ceilings, not Discord quotas or a whole-process memory guarantee.

READY read state, notification settings, session/friend presence and individual guild emoji
catalogs decode independently. Rejected optional sections remain unavailable and produce a
bounded feature warning; they do not abort otherwise valid login. Unknown read state is not
treated as read. Unknown guild notification settings or DND suppress desktop alerts; DMs use
default delivery when global DM settings are absent unless a known mute, block, spam/request,
or DND state suppresses them. Identity,
session/resume address, relationships, navigation, permissions and voice data remain strict.
Supplemental optional metadata follows the same policy. No raw payload or parser error is logged.

The Gateway prepares the permission mirror before transferring one atomic startup event to
the UI. Its single reserved slot permits up to 128 MiB while ordinary events retain their
4 MiB limit and separate 32 MiB queue budget. Subsequent channel/thread updates use the same
account capacity as login. Verified with synthetic offline data only; issue #154's affected
build and exact live payload remain unknown. See docs/authentication.md.

## Received forwards — September 13, 2026

Received forward references display their single attached
[message snapshot](https://docs.discord.com/developers/resources/message#message-snapshot-object)
with a Forwarded label, inset rail, and the existing text, embed, image, video and
audio renderers. The outer message retains its sender, identity and notification
metadata; no original-channel lookup is required. Snapshot content is immutable
and cached with its forwarded marker. Missing or unsupported snapshots retain an
unavailable fallback. This adds received-forward display, not a forwarding action.
Synthetic checks cover decoding, cache persistence and rendering; live acceptance
has not been tested by the agent.

## Channel context menu — September 12, 2026

Guild channel rows support right-click and Shift+F10 actions. Mark As Read uses
the existing acknowledgement adapter without changing the selected conversation;
Invite to Channel opens the existing invite dialog for that channel. Favorites
and pinned DMs are account-isolated local shortcuts, not Discord-synchronized
favorites. Pinned DMs sit above the home list. Guild channels use the existing
Favorites shelf. Neither uses an unverified settings-proto field.

The empty channel-list area also has a server-scoped right-click menu. Hide Muted
Channels is Discord's per-server `hide_muted_channels` flag on user guild settings,
the same account record as channel mute. READY and User Guild Settings Update
restore it, and the toggle sends `PATCH /users/@me/guilds/{guild}/settings`.
The selected channel stays visible. A preloaded user-settings proto update does
not clear these mutes. Create Channel, Create Category and Invite to Server reuse the same
permission checks, confirmation UI and request lanes as the existing channel and
server menus.

Text/announcement editing (name, topic, slowmode and age restriction), channel
duplication, text/voice/announcement/forum channel and category creation, and confirmed deletion use the documented
[channel routes](https://docs.discord.com/developers/resources/channel#modify-channel)
and [guild channel creation route](https://docs.discord.com/developers/resources/guild#create-guild-channel).
Duplication reads current settings and permission overwrites first; creating under
a category copies that category's overwrites. Admin actions require known View
Channel and Manage Channels permissions and surface server rejection.
The Create Channel dialog groups Text, Voice, Announcement, and Forum (posts) choices
with a name field and category permission inheritance context. Announcement sends type 5;
the dialog explains that it requires a Community server. Guild feature eligibility remains
service-authoritative, with rejection shown in the existing channel action notice. The selected type
is checked against the creation response before admitting the new channel. Voice
settings and forum layout use service defaults; private access remains available
through the existing channel Permissions editor. Coverage is synthetic; live
normal-account creation remains unverified.

Channel hierarchy reordering (September 16): accounts with known View Channel and
Manage Channels permissions can drag guild channels and categories. Dropping on a
channel changes its position within that parent; dropping on a category moves the
channel into it and asks Discord to synchronize the category permission overwrites.
The write uses the documented
[Modify Guild Channel Positions](https://docs.discord.com/developers/resources/guild#modify-guild-channel-positions)
route, then reconciles the final hierarchy from Channel Update gateway events.
Coverage is synthetic; live normal-account reordering remains unverified.

Category/channel permissions (September 14): Edit Category and Edit Channel expose
Overview and Permissions for text, announcement, voice, stage, category, forum and
media channels. Category/nontext overview edits rename only. Permission saves use
the documented bulk channel PATCH and additionally require Manage Roles
(Manage Permissions in the UI). Grantable bits are checked before dispatch; the
service remains authoritative. Private controls edit the @everyone View Channel
overwrite, and advanced controls edit individual role/member allow/deny bits.
Unknown bits and untouched overwrites are retained. A fresh GET rejects conflicting
overwrite snapshots before saving. Discord's documented
[category syncing](https://docs.discord.com/developers/topics/permissions#permission-syncing)
applies to already-synced children; this client does not rewrite unsynced children.
Member suggestions use the loaded member window, with explicit member ID entry
for other members. Live permission changes and category propagation are unverified.

Mute durations and notification overrides use the existing unofficial
`PATCH /users/@me/guilds/{guild}/settings` route. Channel mute expiry is decoded from
service settings; newer permanent mutes replace earlier timers. Edits send only
fields changed in the form and reject conflicting changes found before saving.
Read state and invites retain their previously documented limitations.
No live account/channel mutations were performed; normal-account compatibility
and native screenshots remain unverified.

Webhook author profiles (September 12, 2026): the documented message
[`webhook_id`](https://docs.discord.com/developers/resources/message#message-object)
marks the author as a webhook. Its popout shows the message's name/avatar and a
Webhook label, with no user-profile request or retry. Ordinary bot/user profiles
retain their existing behavior. Synthetic parser, cache, and UI checks cover this
path; live webhook interoperability and native visual verification are unverified.

## Server settings — September 23, 2026

Server Profile and Engagement are visible only when current guild permissions
establish Manage Server (including the owner/administrator cases). The editor
uses Discord's documented [guild settings route](https://docs.discord.com/developers/resources/guild#modify-guild)
and the separately observed [guild profile route](https://docs.discord.food/resources/discovery#modify-guild-profile)
for name/icon, description, profile color and traits. Activity Feed changes the
mutually exclusive features described by the [unofficial guild reference](https://docs.discord.food/resources/guild#mutable-guild-features).
These routes are wired for normal-account use but live interoperability remains
unverified. The offline server-settings preview changes synthetic RAM only.
Selected icons are prepared off the render thread; only Save uploads them.

The permission-gated Stickers page loads the guild sticker catalog and supports
creating, editing and deleting stickers through Discord's documented
[guild sticker routes](https://docs.discord.com/developers/resources/sticker#guild-sticker-resource).
Static PNG, JPEG and WebP source artwork is decoded off the render thread, center-cropped
and resized to a 320 × 320 PNG, then rejected if the prepared file exceeds 512 KiB.
The picker reads at most 8 MiB and does not retain the selected path. Create and Manage
Expressions permissions gate writes, including creator-only edits where applicable.
Offline decoder, reducer and UI checks do not establish live normal-account interoperability.

## Group conversation actions — September 11, 2026

Group DM rows and the conversation header expose Edit Group, Mute/Unmute
Conversation, and Leave Group. Edit changes the name and optionally the group
icon; selecting an image prepares a local preview, and only Save sends it.
Leave requires confirmation and is blocked by pending messages or an active
call. Failed writes keep the editor draft; confirmed leave preserves text drafts.

Group edit uses documented `PATCH /channels/{id}` group-DM name/icon fields;
leave uses `DELETE /channels/{id}`. See Discord's [channel resource](https://docs.discord.com/developers/resources/channel#modify-channel)
and [group chat help](https://support.discord.com/hc/en-us/articles/223657667-Group-Chat-and-Calls).
Mute reuses the existing unofficial account notification-settings route and
lasts until explicitly unmuted. Icon hashes hydrate from channel snapshots and
absent/null/value channel updates; static artwork uses the existing bounded CDN
worker with `channel-icons/{channel}/{hash}.png`.

Offline HTTP, reducer, image-boundary and egui interaction tests cover these
paths. The demo includes a synthetic group. No live group was edited, left, or
muted, and normal-account service interoperability remains unverified.

## Embed image galleries — September 11, 2026

Consecutive same-URL image embeds now share one native card: two columns, with a
tall left image and stacked right images for three items. Image-only continuations
and repeated card metadata are grouped; distinct metadata, missing/different URLs,
videos and thumbnail-only entries remain separate. All retained images use the
existing bounded, credential-free preview path and explicit external-open confirmation.

The single-image wire fields are documented in the
[message resource](https://github.com/discord/discord-api-docs/blob/main/developers/resources/message.mdx).
Same-URL gallery behavior is implementation evidence from a
[Discord API repository discussion](https://github.com/discord/discord-api-docs/discussions/3253),
not a documented normal-user compatibility guarantee. Offline egui tests cover
grouping, suppression, tile geometry and individual image actions. Live Discord
interoperability and native screenshot inspection remain unverified.

## Existing DM calls — September 11, 2026

Viewing a supported one-to-one or group DM requests its call state using the existing unofficial
Gateway opcode 13. CALL_CREATE/UPDATE keep an ongoing-call banner independently of ringing
or local media; CALL_DELETE/unavailability removes it. Join never rings an already known
call. [Primary implementation evidence and owner-controlled live checks](voice.md) distinguish
the passing local WebSocket/reducer/UI tests from still-unverified Discord discovery and audio.

## Partial user profiles — September 22, 2026

The extended-profile decoder accepts explicit null summary lists as unavailable,
retains the returned user identity, and keeps the existing byte/item limits and
malformed-data rejection. Empty mutual-server lists alone do not indicate failure.
Missing profile metadata already marks the result as limited, consistent with the
[public profile implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/profile.py).
Limited results and failed requests show “Unable to load parts of profile” above
the identity; failed requests retain Retry with the fixed error category on hover.
This does not infer whether someone blocked the account or bypass service access
restrictions. Synthetic parser/UI checks are not live normal-account verification.

## Own profile editing — September 11, 2026

The native editor updates global display name, bio, pronouns and accent color through
the unofficial normal-user `PATCH /users/@me` route, then verifies a fresh global
profile read. Only deliberate changed fields are sent; failures remain visible and
uncertain saves require reload. Mock HTTP and reducer/UI tests pass; no live profile
was read or changed. Avatar/banner uploads, username, guild profiles and security
settings remain outside this editor. Sources, limits and reproduction are in
in-app profile settings.

## Outgoing camera capture — September 12, 2026

The existing camera sender now accepts native capture on Windows (Media Foundation and DirectShow)
and Linux (V4L2), alongside macOS (AVFoundation). Both new adapters feed the existing
H264 negotiation, opcode 12 video announcement, DAVE encryption and bounded RTP
sender. These normal-user video extensions remain unofficial and live-unverified.
The button requires a connected call, channel video permission and negotiated H264;
capture requires an explicit click. No camera opens in the synthetic demo.
September 13: Windows settings and call controls now enumerate/select cameras,
including DirectShow-only virtual sources. A read-only native enumeration test
found three registered virtual cameras on the Windows test machine; it did not
activate any source. Actual capture and Discord delivery remain unverified.
Native limits, platform requirements and the owner-operated validation gate are in
[Camera in calls](voice.md#camera-in-calls-macos-windows-and-linux). Receiving video
and recording remain unsupported. This section supersedes older camera-exclusion
statements in the historical voice/screen-sharing notes below.

## Outgoing screen sharing — September 11, 2026

September 21 interoperability correction: camera and screen-share H.264 SPS metadata
is normalized before DAVE encryption to specify no frame reordering and bounded
decoder buffering. The [WebRTC receiver rewrites unsuitable SPS metadata](https://github.com/webrtc-mirror/webrtc/blob/main/modules/rtp_rtcp/source/video_rtp_depacketizer_h264.cc)
before frame decryption; that changes DAVE-authenticated bytes, while Serein's
receiver preserves them. The offline debug command
`cargo run --locked -p discord-voice --example video_interop` exercises this
authentication failure and normalized encryption/decryption/decoding with synthetic
video. Official Discord Android/desktop playback and resolution of issue #345
remain subject to an owner-operated live retest.

September 22 streaming follow-up: outgoing H.264 packets are paced in two-millisecond
batches within a byte budget, allowing audio, feedback and DAVE signaling between batches.
Only one packetized access unit is pending (the existing 2,048-fragment / 1,200-byte packet
limits); encryption transitions discard its remainder. Linux hardware
capture now retains encoded reference pictures while the transport queue is full and wakes
when capacity returns. Failed application-audio monitors are retired for rediscovery;
retiring a finished audio worker no longer stops video. Viewer stream snapshots retire old
SSRC/RTX mappings without resetting unchanged pictures, and failed video authentication
requests a fresh keyframe. No encryption fallback or additional codec is introduced.

Authenticated RTCP receiver/sender reports and matching REMB estimates now adjust the
encoder target once per second, between 250 kbps and the selected preset. At least 5%
reported loss reduces the target by 20%; fresh low-loss feedback permits gradual 5%
recovery after two seconds without congestion. Missing feedback does not increase the
rate. REMB reserves wire overhead and expires after five seconds. Native encoder rates
update in place where supported; software encoders and older Linux plugins restart with
an IDR. Linux restarts retain the approved portal session and application audio.

Matching authenticated NACKs use RFC 4588 RTX (PT 102) on the server-assigned RTX SSRC,
retaining the original DAVE ciphertext with a fresh transport nonce. History is bounded
to 2,048 entries / 2 MiB of accounted packet storage / one second, with 128 pending
sequence numbers, two retries per packet and a 50 ms retry cooldown. Repair traffic is
capped at 20% of the encoder target within the combined 125% wire budget, with a small
bounded token burst. Missing or exhausted history requests an IDR instead. A 500 ms lack
of sending progress discards the pending frame and requests recovery; slowly draining
large keyframes are not repeatedly truncated. Rekeys clear history and pending repairs.

The offline `video_interop` example exercises authenticated feedback, rate changes,
loss/RTX recovery, paced packetization, transport/DAVE round trips, source replacement
and the existing playback debug checks. Native Linux/Windows rate changes, real network
behavior and official-client error 2012 still require an owner-operated retest. This is
a bounded loss/REMB controller, not WebRTC's full congestion-control algorithm;
RTCP sender-clock synchronization remains absent. The
[Discord-RE sender](https://github.com/Discord-RE/Discord-video-stream/blob/master/src/client/voice/WebRtcWrapper.ts)
provides the comparison for pacing, feedback and retransmission behavior.

September 15 transport follow-up: screen audio keeps stereo 48 kHz Opus, 20 ms
frames, the stream connection's audio SSRC, and opcode 5 Soundshare. It now also
marks each audio packet with the native speaking RTP extension (ID 9, value 4).
The RFC 8285 preamble is authenticated and extension elements are transport-encrypted
before the existing DAVE payload. This matches the public
[voice_media implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/voice_media.py)
and [native sender](https://github.com/dolfies/discord-native-voice/blob/master/discord/ext/native_voice/client.py),
not a documented Discord guarantee. Viewers announce a receive-only video state and
request media with opcode 15 (`any: 100`), following that native receiver implementation.
The missing packet marker and receive request are interoperability differences; their
role in the owner's silent browser-audio report is not yet live-confirmed.

The sender now receives authenticated RTCP picture-loss indications for its own video
SSRC, coalescing requests to at most twice per second. Windows/macOS can freshly encode
one retained current snapshot on an idle keyframe request, without waiting for screen
movement. The additional snapshot is bounded to 33,177,600 bytes, normally reduced to
the chosen output resolution. Ordinary idle time does not encode extra frames. Linux
already requests periodic PipeWire keepalive frames. Sender RTX repair, receiver NACK
requests and adaptive bitrate remain unsupported. The viewer accepts announced RTX
packets and bounded packet reordering within the current picture; native/live video
quality is not established by these changes.

Watched streams have a session-only 0–200% audio volume and mute control, independent
of participant voice levels. Audio playout retains at most two queued 20 ms frames
and clears delayed mixer output after a stall. Decoder input is bounded to 16 pictures
and 16 MiB, with a 150 ms age limit and keyframe recovery. These bounds reduce backlog;
they do not implement sender-clock audio/video synchronization or establish live sync.

A device-free localhost test runs the actual sender and viewer through an MLS exchange,
stereo Opus and H.264, encrypted UDP forwarding, decoded audio and video. It verifies the
local media handoff, not PulseAudio/WASAPI capture, speaker output, or Discord forwarding.
Discord's [August 2025 patch notes](https://discord.com/blog/discord-patch-notes-august-4-2025)
confirm Linux application-audio sharing; older articles saying Linux audio is unavailable
are outdated. Unlike Discord's selected-application mode, Serein currently captures other
eligible applications too, even when sharing one window, excluding its own playback.

September 12 local-preview update: DM and guild call stages display the owner's
camera and a bounded screen-share preview while waiting alone. Local capture is
independent of DAVE readiness; outgoing media still waits for encryption readiness.
Synthetic tests cover these paths, not native capture or live Discord acceptance.

The standard build adds macOS 14+ ScreenCaptureKit and Windows Graphics Capture senders for an existing connected DM/server voice call. Share opens a native egui source/settings dialog first. It exposes 720p/1080p and 15/30/60 fps to all accounts, plus cursor visibility; only the explicit Share screen action creates a stream. No subscription fields are changed. This paragraph describes the original video sender; subsequent audio and receive extensions supersede its original limits.

Linux extension (September 15, 2026): the existing H.264/DAVE sender now accepts
portal-approved PipeWire screen/window capture. It uses the system picker, an ephemeral
portal session and VA-API/NVENC with OpenH264 fallback. The pipeline has one source
queue (at most 7680×4320 / 132,710,400 bytes per buffer; PipeWire negotiates 2–4 buffers), one raw frame per encode/preview
branch (at most 33,177,600 bytes each), one appsink frame per branch (encoded ≤2 MiB;
software raw ≤33,177,600 bytes; preview ≤925,696 bytes including padding), and the existing
three-frame / 6-MiB encoded transport queue. Each processing stage can additionally hold
its current buffer. Driver, compositor and codec surface pools are separate native
allocations. Raw queues discard old frames before encoding; encoded pressure blocks
upstream and a lost reference frame requests a new IDR. Bus events are reduced to one
failure flag. Portal responses are admitted at ≤64 KiB; signal queues hold one message
and the connection queue holds two, with zbus's separate 128-MiB wire-message ceiling.
Cancellation is checked during portal waits every 50 ms and media waits within 100 ms;
native driver startup/shutdown can still block, retaining the existing retirement barrier.
Each of at most four encoder attempts gets a fresh PipeWire remote under the same
approved session. No source, restore token, pixel buffer or stream is persisted.
Native X11 now offers an explicitly selected whole-desktop source through GStreamer
`ximagesrc`, reusing the bounded encoder/preview pipeline without a portal. It requires
GStreamer Good and never activates after portal cancellation or failure. Individual
X11 window selection and native/live validation remain outstanding. AV1/H.265 sending
remains unsupported.
The offline debug example does not establish native Linux capture, hardware acceleration,
measured performance, packaging or live Discord interoperability.

System audio extension (September 15, 2026): Linux PulseAudio/PipeWire application monitors
and Windows process loopback feed the existing stereo Opus/DAVE stream audio sender.
Both are opt-in and exclude Serein's playback, including received call and stream audio.
Other applications are included even for a single-window share. macOS ScreenCaptureKit
continues excluding the current process. Windows requires build 20348+ and uses native
process-tree exclusion; unsupported systems fail visibly without whole-output fallback.
Linux uses `pa_stream_set_monitor_stream` before connecting each recording stream,
rejects Serein/unknown application identities and disables recording-stream movement.
Audio access is separate from portal-approved video, using the existing PulseAudio socket.
See [audio behavior and owner test steps](voice.md#screen-sharing).

Audio chunks are at most 9,600 floats / 38,400 bytes (100 ms). The capture channel holds
four chunks / 153,600 bytes. Linux admits at most 32 identified application streams,
each with at most 100 ms / 38,400 bytes of pending PCM (1,228,800 bytes total), and
limits each discovery response to 256 entries. Excess streams fail visibly.
The transport reserves 9,600 floats / 38,400 bytes, trims before appending, drains a
bounded queue snapshot and sends at most one 20 ms frame per tick. Current processing
buffers and native audio-server/driver storage are additional. Rekeys and stalls clear
the transport's queued/pending audio; generation tags reject in-flight old buffers.
Linux audio runs on a separate bounded worker; the portal closes before waiting
for audio retirement if a native server/driver call stalls. Synthetic checks cover gates, bounds, sanitization,
pacing and teardown; native output selection, sound quality, echo and live interoperability
remain unverified. Linux adds the native `libpulse-sys 1.23.0` bindings; no new Flatpak
permission, virtual device, output rerouting or recording file is added.

Gateway opcodes 18/19 and STREAM_CREATE/STREAM_SERVER_UPDATE/STREAM_DELETE are unofficial normal-user behavior, checked against [discord.py-self](https://github.com/dolfies/discord.py-self/blob/master/discord/gateway.py). A separate RTC connection uses the stream RTC server/channel IDs and the parent call session, sharing one ephemeral DAVE signing identity. The `rtc_server_id - 1` MLS group mapping comes from [discord-native-voice](https://github.com/dolfies/discord-native-voice/blob/master/discord/ext/native_voice/stream_client.py); it is not an official protocol guarantee. H264 negotiation and UDP transport are required; mismatches fail visibly. Video is DAVE-encrypted before RFC 6184 packetization and per-packet transport AEAD. There is no plaintext fallback. [DAVE protocol](https://github.com/discord/dave-protocol/blob/main/protocol.md) supplies the encryption requirement.

Screen source discovery and capture occur off the UI/audio threads. Application-owned source lists are capped at 64 labels of 256 bytes. Raw BGRA frames are capped at 3840×2160/33,177,600 bytes; macOS requests the selected output dimensions. Windows prechecks source size and stops on oversized callback frames, but its upstream driver adapter can resize its native GPU pool before that callback. One raw frame and three encoded frames can be queued; H264 frames are capped at 2 MiB, encrypted packetization at 2,048 fragments of at most 1,200 transport bytes. Quality presets target 4–16 Mbps, with frame dropping under load. They are limits/targets, not measured delivery guarantees.

Call and stream transports send an eight-byte native UDP ping after discovery and every
five seconds, including receive-only streams and muted calls. The packet uses the
`0x1337CAFE` prefix and a little-endian sequence, following the unofficial
[native transport implementation](https://github.com/dolfies/discord-native-voice/blob/master/src/transport/udp.rs).
Signaling heartbeats are separate; neither audio capture nor DAVE readiness gates these
media-free pings. Pong packets cannot enter the authenticated media decoder. A localhost
test covers repeated idle-viewer pings followed by encrypted audio/video delivery.
Whether this resolves the owner's approximately 16-second receive stall remains unverified live.

Video reassembly now propagates packet gaps, unfinished pictures and malformed frames
to the existing keyframe recovery path. Dependent pictures are gated until an intact
keyframe arrives; the existing rate-limited PLI asks the sender for that refresh.
Previously these losses were discarded silently and recovery relied on a later decoder
error or periodic keyframe. A regression reproduces the missing request before the fix
and verifies recovery afterward. The owner's later Linux-to-macOS video-only lag is
not yet attributed to this defect by matching logs.

Stop, call teardown, changed generation, lost video permission or stream-server replacement stop capture and transport. New sharing waits for native worker retirement and matching STREAM_DELETE. No automatic retry or recording is performed. Rekeying pauses capture readiness, drains queued encoded frames, and requires an IDR before resuming transmission. The encoder requests periodic IDRs every two seconds of encoded frames. The current sender handles RTCP PLI and optional system audio, but has no NACK/RTX repair or congestion adaptation; lossy-network quality and service acceptance of high-quality presets remain unverified.

Owner-operated validation is still required: allow screen-recording permission, join a private call, choose a window, verify viewing in the official client, then test Stop, source closure, permission loss, viewer changes/rekey and both quality presets. No live account/capture was used for agent tests. Native Windows execution and live Discord video interoperability are not established by macOS builds or synthetic tests.


User context actions (checked September 11, 2026): Close DM uses the documented
[Delete/Close Channel route](https://docs.discord.com/developers/resources/channel#deleteclose-channel)
only for a known one-to-one DM. It closes navigation after successful HTTP completion;
messages and drafts are not deleted. Pending messages and an active call prevent closing.
Block/unblock use `PUT` (type 2)/`DELETE /users/@me/relationships/{user}`; mute/unmute use
`PATCH /users/@me/guilds/@me/settings` with a single channel override. These account routes are
unofficial and unstable, supported by the public [HTTP implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py)
and [channel settings implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/settings.py).
Mute means DM notifications until explicitly unmuted, not voice audio. The response must confirm
the requested channel and mute value. Block state hydrates from READY relationships and follows
relationship add/update/remove events, based on the public [gateway implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py).
Unknown relationship/settings state is distinguished from false. One write is pending at a time;
rejections, disconnects and ambiguous outcomes remain visible without automatic retries.
Gateway updates take precedence over late HTTP confirmations. Notification suppression reuses
the existing account settings and rejects alerts from known blocked authors. Existing timeline
content is retained; this does not implement Discord's collapsed blocked-message presentation.
Offline reducer, decoder and local HTTP tests cover these actions; normal-user acceptance,
cross-device behavior and native interaction remain live-unverified. No accounts were accessed.

Inline spoilers (September 10): Discord's [Spoiler Tags help](https://support.discord.com/hc/en-us/articles/360022320632-Spoiler-Tags)
documents paired pipe delimiters and exempts code blocks. The native timeline recognizes
bounded paired literal delimiters outside code, with up to 32 independently revealed text
regions. Hidden spans create a keyboard-focusable reveal control rather than hidden text,
links, references or emoji widgets. Spoiler attachments/cards keep a separate media reveal;
reply previews remain conservatively concealed. Parser limits use conservative concealment
instead of exposing uncertain source. This is a local Markdown subset, not full Discord parsing
parity; native accessibility and live rendering equivalence remain unverified.

Incoming typing (September 10): Discord's [Typing Start event](https://docs.discord.com/developers/events/gateway-events#typing-start)
documents channel/user IDs and a Unix-seconds timestamp. The decoder accepts only these fields
within 16 KiB and discards optional guild/member metadata; invalid signals are ignored. The
desktop retains at most eight current-conversation users for at most ten seconds, rejects old
timestamps and more than five seconds of future skew, and clears state on navigation, messages
from that author, disconnect and access/session changes. Names come only from existing DM,
People or timeline state; missing identities use a generic label. Signals never cause profile
requests, outgoing typing, subscriptions, storage writes or timeline re-layout. A separate
eight-slot lossy inbox preserves reliable-event capacity; duplicate/burst wakeups are bounded
to eight per two seconds. The UI schedules only the next visible expiry, with no dot animation.
Existing People typing subscriptions are unchanged; guild delivery may depend on that pane's
subscription. Service availability and normal-account acceptance remain live-unverified.

Message components (September 19, issue #313): native action rows, buttons, string/user/
role/mentionable/channel selects, Components V2 sections, containers, text, thumbnails,
media galleries, files and separators retain bounded typed service data. Modal controls
include labels, text inputs, selects, radio groups, checkbox groups, checkboxes and
explicit native file selection. User/mentionable selectors reuse on-demand member
search; role/channel selectors use the loaded account catalog.
Buttons and form submits use the unofficial normal-account `POST /interactions` path,
with the active Gateway session, a unique nonce, and no automatic write replay. Gateway
success/failure/modal events are correlated; an HTTP acceptance alone is not completion.
Private replies are session-only and never saved with channel history. Premium purchase
buttons are omitted; unknown component types retain a visible marker. Media links use
the existing safe preview and destination-confirmation policy.

Select menus show selected labels, option descriptions and emoji. Single selections
submit directly; optional selections can be cleared. Form inputs have larger padding
and multiline fields have room for longer answers. Link buttons keep their text on one
line and display an external-link icon. Rendered components do not add an Open in
Discord button. Channel category labels also stay on one line with truncation.

The [official component reference](https://docs.discord.com/developers/components/reference)
describes schemas; normal-account submission and modal Gateway events are unofficial
([first-hand interaction reference](https://docs.discord.food/interactions/receiving-and-responding)).
This is not full Discord component parity: premium purchases are unavailable, and
synthetic checks do not verify live application responses, modal uploads or purchases.
Normal-account interoperability and Windows/Linux visual equivalence remain unverified.
Polls and stickers still retain presence markers and an Open in Discord fallback.
Old cached component markers acquire controls only after normal history refresh.

Run the offline native component preview with:

```bash
cargo run --locked -p serein --features demo -- --demo --demo-components
```

Choose a dropdown option or click **Open sample form** to inspect the synthetic controls.
The separate `--demo --demo-check-components` mode checks the synthetic interaction flow.

External fallback (September 10): unsupported channel rows and message placeholders offer
Open in Discord through an explicit browser confirmation. URLs use the fixed Discord HTTPS
origin and typed guild/channel/message IDs; DMs use @me with the conversation ID. No content,
names, credentials or signed media URLs enter the route. Zero IDs, missing DM/guild identity
and unavailable view permission disable the action. Discord's [message-link help](https://support.discord.com/hc/en-us/articles/206346498-Where-can-I-find-my-User-Server-Message-ID)
documents linking to an accessible message/conversation; its [Trust and Safety example](https://support.discord.com/hc/en-us/community/posts/1500000159602-How-to-Properly-Report-People-On-Discord)
shows the guild and @me path structure on the older discordapp.com domain. The current
discord.com route is an integration assumption, not a new API guarantee. Headless tests cover
construction and explicit confirmation; native launch, browser account selection and destination
resolution remain owner-unverified. The browser uses its own session and Discord authorization.

Loaded threads (September 10): READY guild thread arrays follow the original [discord.py-self guild parser](https://github.com/dolfies/discord.py-self/blob/master/discord/guild.py); active create/update/delete, scoped sync, archive eviction and owner-removal handling are informed by its [dispatch implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py). Discord's [Gateway thread events](https://docs.discord.com/developers/events/gateway-events#thread-list-sync) document the guild/parent scope and membership fields. These primary sources establish wire evidence, not normal-account acceptance; no implementation blocks were copied. Active discovery is limited to service-supplied snapshots/events; explicit archive reads are described below, with existing subscriptions unchanged. Clicking an unresolved channel mention performs one bounded documented channel read and admits only a same-guild thread whose loaded parent remains viewable. Unknown updates do not otherwise hydrate a missing thread. See native navigation scope.

Serein is unofficial and not endorsed by Discord. No normal-user live session has been tested. Technical compatibility does not imply approval. Discord forbids normal-account automation outside its OAuth2/bot API and warns of account termination ([policy](https://support.discord.com/hc/en-us/articles/115002192352-Automated-User-Accounts-Self-Bots)); its [terms](https://discord.com/terms) also apply.

Server identity tags (September 19): Discord's documented [User object](https://docs.discord.com/developers/resources/user#user-object) may include `primary_guild` with an enabled identity, guild ID, tag and badge hash. Serein retains a valid identity from ordinary user payloads for profile cards, server-channel and thread member rows, and one-to-one DM rows. Profile cards can therefore show it before the unofficial extended-profile request completes or when that request fails; a completed extended profile remains authoritative, including removal of a stale tag. The older duplicated `clan` shape is accepted only as a bounded fallback. Normal-account delivery remains live-unverified.

| Capability | Credential / evidence | Classification | Serein status / fallback |
|---|---|---|---|
| Supported sign-in | OAuth2 access token; [scopes](https://docs.discord.com/developers/topics/oauth2) | Documented for limited scopes; RPC/other scopes restricted | No full replacement-client grant established. No invented OAuth login |
| Token sign-in | Normal-user session credential intentionally entered by its owner; [Abaddon source](https://github.com/uowuo/abaddon/tree/master/src/discord) reviewed today | Unofficial, unstable, account risk | Available on the main sign-in screen in every build, saved like a normal login, no extraction from other software; actual service validation pending |
| Official login webview | The owner completes Discord’s hosted login; [Wry](https://docs.rs/wry/0.57.0/wry/) supplies an ephemeral platform webview | Official hosted UI, **unofficial/unstable handoff** | Implemented bounded same-origin Authorization-header observation in the temporary webview; synthetic bridge tests only; email, QR, MFA, CAPTCHA and passkeys are individually live-unverified |
| Saved login | Normal session credential in [OS keyring](https://docs.rs/keyring/4.2.0/keyring/v1/) | Native credential-store integration | Save after verified native readiness, restore at launch, delete on logout; real credential-store round trip not exercised |
| Account and guild summaries | Normal session `/users/@me`, `/users/@me/guilds`; [user API](https://docs.discord.com/developers/resources/user) and Abaddon | Public resource shapes; normal-session behavior unofficial | Experimental adapter, not live verified |
| Channels / existing DMs | Normal session; READY navigation and guild channel resource; Abaddon | Unofficial session snapshot | Fail visibly on oversized/incompatible metadata; no member scraping |
| Server categories | [Channel resource](https://docs.discord.com/developers/resources/channel), existing session snapshot and channel events | Documented metadata; normal-user delivery unofficial | Ordered collapsible headings, orphan fallback, partial create/update/delete; offline and keyboard tests; details |
| Server icons | [Image reference](https://docs.discord.com/developers/reference#image-formatting), guild metadata and GUILD_UPDATE | Documented CDN path; nested READY properties unofficial | Cached static icons and hash updates, initials fallback, offline tests; details |
| Message embeds | [Message resource](https://docs.discord.com/developers/resources/message#embed-object) | Documented attributes; normal-user delivery/proxy conversion unverified | Native cards, partial embed-only updates, suppression/spoilers, cached static service-proxy previews; video opens externally, limits |
| People / member pane | Normal session; [discord.py-self Gateway](https://github.com/dolfies/discord.py-self/blob/master/discord/gateway.py), [member-list identity](https://github.com/dolfies/discord.py-self/blob/master/discord/abc.py), [wire types](https://github.com/dolfies/discord.py-self/blob/master/discord/types/gateway.py) | Unofficial and unstable | On-demand opcode 37 with the required guild typing subscription, first 100 list positions, typed incremental operations, identity/request filtering and 15-second timeout; list identity resolves from current bounded permission metadata rather than the initial channel snapshot; DM recipients from READY. Missing metadata or unsupported replies show unavailable; live-unverified |
| Profile pictures | [Discord image formatting](https://docs.discord.com/developers/reference#image-formatting), [User resource](https://docs.discord.com/developers/resources/user#user-object) | Documented CDN paths and user metadata; normal-session acquisition unofficial | Static PNGs, credential-free requests, account-isolated disk cache, bounded decode/textures, fallback initials; offline transport/cache tests only |
| User mentions | [Message formatting](https://docs.discord.com/developers/reference#message-formatting) | Documented syntax; normal-user notification behavior unverified | Local @ autocomplete, clickable names/profile cards, exact user allowlists, bounded SQLite metadata; tests and limits |
| User profile cards | Normal session `/users/{id}/profile`; documented [User object](https://docs.discord.com/developers/resources/user#user-object); [public implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py) | Base server identity shape documented; extended route and normal-session delivery unofficial/live-unverified | Anchored popout with banner/avatar/bio/pronouns/badge artwork/server tag/theme colors/connections/mutual servers and retained presence/custom status, cancellable requests and visible failures. Bounded base-user identity supplies compact badge/tag chips in profile, member and direct-message rows while the extended profile is unavailable; badge CDN paths are observed, not documented; evidence |
| History, send, edit, delete, replies | Normal session; [message resource](https://docs.discord.com/developers/resources/message) and Abaddon | Documented bot-facing resource; user compatibility unofficial | Experimental text adapter; owner permissions remain server-authoritative |
| Realtime, resume | Normal session; [Gateway](https://docs.discord.com/developers/events/gateway), Abaddon Identify/READY | Documented lifecycle; normal-user Identify and dispatch differences unofficial | Bounded JSON transport; no compression requested; incompatible snapshots fail |
| Rate limits | Credential-specific response headers/body; [rate limits](https://docs.discord.com/developers/topics/rate-limits) | Documented; do not assume bot quotas | Conservative shared cooldown, no blind write retry |
| Nonce correlation | Message nonce; message resource | Documented finite deduplication; user applicability unverified | Correlate confirmations only; no automatic ambiguous resend or indefinite idempotency claim |
| Native message formatting | Existing message content; [pulldown-cmark source](https://github.com/pulldown-cmark/pulldown-cmark) and [egui LayoutJob](https://docs.rs/egui/0.36.2/egui/text/struct.LayoutJob.html) | Local rendering; no additional service capability | Bounded emphasis/code/quotes/lists/strike, inert HTML, explicit HTTP(S) link confirmation, up to 32 inline text spoilers with separate media reveal; CommonMark differs from Discord Markdown; no automatic previews |
| Read markers | Normal session; [discord.py-self HTTP](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py) and [dispatch implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py) | Unofficial and unstable | Explicit per-message acknowledgement, optional manual mark-unread (`ack_message(manual=True)` plus the previous snowflake), guild `POST /guilds/{id}/ack`, bounded READY read state, MESSAGE_ACK and PASSIVE_UPDATE_V2; synthetic checks only, actual cross-device behavior unverified |
| Relationships | Normal session or restricted Social SDK scopes; OAuth scope table and Abaddon | Unofficial / restricted | Unsupported |
| Reactions | Normal session; [message reaction resource](https://docs.discord.com/developers/resources/message#reaction-object) and [Gateway reaction events](https://docs.discord.com/developers/events/gateway-events#message-reaction-add), rechecked September 10 | Documented routes/shapes; normal-user acceptance live-unverified | Native counts, eight-emoji picker, existing Unicode/custom emoji toggles, normal own-reaction PUT/DELETE and bounded message readback. Synthetic HTTP/Gateway/keyboard tests pass; no live validation |
| Upload / preview / save | Normal session + separate credential-free media transfer; [attachment reference](https://docs.discord.com/developers/resources/message#attachment-object) and [normal-user staged upload](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/abc.py#L1551) | Documented attachment metadata; staged upload unofficial; service delivery/conversion live-unverified | Up to ten explicitly selected files totaling 500,000,000 bytes, streamed upload/cancel and message confirmation; Discord enforces the account's lower entitlement limit. Static previews, native viewer, image/video attachment context menus with native clipboard copy, and explicit image/file Save As (1 byte through 100 MiB). Upload limits below; preview/save limits |
| Conversation search | Normal session; [discord.py-self search flow](https://github.com/dolfies/discord.py-self/blob/master/discord/abc.py), [HTTP routes](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py), [response types](https://github.com/dolfies/discord.py-self/blob/master/discord/types/message.py) checked September 10 | Unofficial and unstable | Search current conversation, newest/older result pages, snapshot snippets, history revalidation on Open message; indexing/permission failures are visible. Live-unverified |
| Pinned messages | Normal session; [Get Channel Pins](https://docs.discord.com/developers/resources/message#get-channel-pins) and [normal-user implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py) | Documented route; normal-user acceptance live-unverified | Manual 25-pin pages, Older/Retry, Reload newest and history-validated Open message; pin/unpin mutations unsupported |
| Threads / forums | Normal session; navigation and archive scope, archive sources below | Wire evidence checked September 10; normal-user behavior live-unverified | READY/thread-event reconciliation, parent/post hierarchy and manual public/private/joined-private archive pages implemented; complete active directory and create/join controls remain incomplete |
| One-to-one DM voice | Normal session + per-call credentials; [Discord voice](https://docs.discord.com/developers/topics/voice-connections), [DAVE](https://daveprotocol.com/), [discord.py-self signaling](https://github.com/dolfies/discord.py-self/blob/master/discord/gateway.py) | Voice transport/DAVE documented; normal-user DM entry/ringing unofficial; implementation live-unverified | Native Opus/CPAL/DAVE v1 path, call controls and bounded resume implemented; synthetic tests only. Physical audio and official-client two-way call gate blocked; voice included in every build |
| Logout | Local credential/task disposal; OAuth revocation applies only to OAuth tokens | Local behavior implemented; remote session revocation unverified | Local logout only, does not delete sent messages or claim server token revocation |

`messages.read` refers to local RPC; it is not a general REST chat grant. `rpc` needs approved-partner access and depends on the official application. Social SDK relationship access does not establish general desktop-client chat access. No client secret, token broker, fingerprint imitation, native password endpoint, CAPTCHA/MFA workaround, or harvested QR flow is implemented. The owner explicitly requested the official login page in an authentication-only webview and secure saved login; OAuth is not the product login.

Discord states DAVE is required for the listed call types from March 1, 2026. libdave supplies encryption components, not capture, playback, codecs, jitter handling, or a complete call engine. No legacy unencrypted fallback is acceptable.

All implemented network features remain **experimental and live-unverified** until the manual gate in authentication.md passes. Public bot documentation is protocol evidence, not proof of normal-user acceptance. Abaddon was inspected for wire evidence only; no GPL source is copied into this dual-licensed implementation.

September 10 continuation: Gateway documentation was rechecked for heartbeat, Resume and invalid-session handling. Offline coverage now includes a real loopback WebSocket lifecycle, including non-resumable invalidation and credential-expiry close; no Discord connection was made. The localhost dial override is compiled only into unit tests and never changes production/resume origin validation. Native formatting, font glyph coverage and persisted appearance have offline tests; these do not expand the live-verified capability set.

## Authentication evidence and changed requirements

The owner superseded the initial storage/login constraints during implementation: official-login webview, saved OS credential-store token, and subsequently bounded local SQLite caches/drafts are now required/allowed. The documentation records the final policy.

[Discord Userdoccers’ original protocol research](https://docs.discord.food/authentication), checked 2026-09-09, describes normal-user password login, MFA tickets, and authentication tokens. It is unofficial evidence, not Discord approval. The final implementation delegates that login to Discord’s hosted UI instead of implementing these endpoints itself. Email/phone login, QR, TOTP, CAPTCHA, passkeys and device verification are offered only as the official page permits in the platform engine; none was tested with a real account here.

The handoff is original code in `crates/platform/src/login-handoff.js` and the Linux WebKit resource observer. It observes an Authorization header only for this temporary webview’s own `https://discord.com/api/v*/` requests, after user-controlled authentication. It reads neither localStorage nor other applications, profiles or tabs, and does not inspect passwords or QR secrets. A random per-webview capability scopes the IPC handoff; Rust validates that capability, the IPC origin, and token length, then verifies the account via `/users/@me` and rejects bot accounts. This handoff is a technical hypothesis with offline tests; Discord’s current page may use an unsupported transport, reject the embedded engine, or change its behavior. Failure must not be called successful login.

Navigation is limited to Discord’s HTTPS origin; new windows and downloads are blocked. Third-party challenge subframes are left to the platform engine; popup-dependent methods may fail. No spoofed official client user agent/properties are supplied. Resume URLs are restricted to recognized Discord gateway hosts.

September 10 People/avatar continuation: the member subscription uses the opcode 14 shape found in the current original discord.py-self implementation. [Original lazy-guild research](https://arandomnewaccount.gitlab.io/discord-unofficial-docs/lazy_guilds.html) describes list positions including groups and ambiguous empty SYNC responses; it is unofficial evidence, not a service guarantee. A newer opcode 37 has also been [reported by Userdoccers](https://github.com/discord-userdoccers/discord-userdoccers/issues/191); Serein does not claim opcode 14 works for every account. Channel-specific list identities require the available everyone-role permissions and channel overwrites. The small noncryptographic Murmur3 identity calculation is implemented locally and checked against known vectors; no third-party client code blocks were copied. This identity selects a list; it grants no permissions. No complete member directory is fetched or persisted. Profile cards expose only available name, ID and avatar, not invented bios, roles or relationships. Server-specific custom avatars and full profile endpoints remain unsupported.

## DM voice evidence — September 10

Normal-user DM entry uses guild_id:null with main Gateway opcodes 13/4, based on the original [discord.py-self Gateway implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/gateway.py). CALL_CREATE/UPDATE/DELETE and voice state/server shapes follow its [dispatch source](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py) and [voice types](https://github.com/dolfies/discord.py-self/blob/master/discord/types/voice.py). Ring/stop-ringing use the [HTTP implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py); its [DM connect flow](https://github.com/dolfies/discord.py-self/blob/master/discord/channel.py) establishes voice before ringing. These are unofficial interoperability evidence, not approved normal-account APIs or a bot-token workaround. No source-code blocks were copied.

Discord's documented voice transport and [DAVE protocol](https://daveprotocol.com/) supply encryption/protocol requirements. Serein uses Davey 0.1.4/OpenMLS, an unofficial implementation, rather than claiming to ship Discord's libdave or an independently audited engine. Real MLS, DAVE, RTP, Opus and loopback WebSocket/UDP tests exercise the adapter with synthetic participants. They do not verify current Discord acceptance, microphone permission, device quality or a remote official client. Existing group DMs, Stage channels and camera video remain unsupported; outgoing screen sharing is covered by the September 11 addendum. Guild voice is implemented separately below, with its live gate still unverified. See [voice scope and owner-operated gate](voice.md) and [adapter details](../crates/discord-voice/README.md).

Image attachments: documented wire metadata and spoiler bit 3 are implemented; additional sensitive flags and proxy PNG conversion rely on unofficial implementation evidence. Native viewing and bounded cache/patch behavior have offline coverage; actual account image delivery remains unverified. Scope and sources.

Read-state continuation (September 10): channel read cursors and latest-message IDs drive boolean unread badges; absent service read state remains unknown. The guild rail tick ignores muted guilds, muted categories, muted channels, and voice channels. Voice chat stays behind Show chat, so leftover text-in-voice cursors do not light the server icon. Mentions on those channels still add the red count. A live-edge page whose rows sit at or behind the cursor is not unread. The message menu sends one acknowledgement only after an explicit action on a loaded message in a fresh authenticated view. Incoming manual mark-unread updates are honored. A later Gateway update wins over an in-flight HTTP completion. Snapshot/update lists are capped at 4000 entries, retained cursors are limited to known navigation channels, and only one write is pending.
Outgoing mark-unread and guild acknowledgement (September 16): Mark Unread ACKs the previous snowflake with `manual: true`, matching unofficial [discord.py-self `ack_message`](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py). Server Mark As Read uses `POST /guilds/{guild_id}/ack` from the same source. A manual unread in a DM or group sends `mention_count` for the loaded messages now past the cursor, because Discord does not calculate that badge. Mark Unread in a guild channel sends `mention_count` for loaded pings in that range. Server Mark As Read still omits it. Live Discord acceptance is unverified. A legacy acknowledgement token, if supplied, is capped at 2048 bytes in zeroized session memory; the response body is capped at 4096 bytes. Neither cursors nor acknowledgement tokens are written to SQLite. The linked primary client implementation supplies unofficial wire evidence; local HTTP/WebSocket fixtures do not establish Discord acceptance.

Search continuation (September 10): guild conversations use the guild search route with an exact channel filter; DMs use the channel route. Search content is percent-encoded, with timestamp-descending order and explicit max_id pagination. One replaceable task uses existing REST permits, deadlines and cooldowns. Indexing responses require another deliberate Search action after the service delay; no automatic polling, broad account search, advanced filters, NSFW override or search-result persistence is implemented. Service totals and partial-index status are displayed as supplied, not asserted complete. Opening a result fetches up to 50 history messages ending at that ID and positions the timeline there; unavailable results are reported. Existing reload returns to latest history. Search snapshots are cleared on relevant edits/deletes, navigation, disconnect, permission invalidation and logout. Original-client sources supply wire evidence only; no source-code blocks were copied and no authenticated service request was used as validation.

Login compatibility correction (September 10): READY read_state accepts both the legacy array and the versioned entries/version/partial object, under the same 4000-entry bound. Serein's Identify does not request the versioned_read_states capability; rejecting the legacy shape previously rejected the entire login payload. The capability's effect is described in the original [discord.py-self capability definitions](https://github.com/dolfies/discord.py-self/blob/master/discord/flags.py), rechecked September 10. Partial snapshots leave omitted channels unknown. Identify capabilities remain unchanged. Static error labels distinguish account verification, Gateway discovery, READY decoding and connection setup without exposing payloads, credentials or remote error text. Synthetic regression and loopback evidence do not establish actual account login success.

## Reaction refresh and pinned messages - September 10

Incoming reaction updates (September 14): validated Gateway add/remove events now
update the affected Unicode/custom emoji count immediately, including burst counts.
Remove-emoji and remove-all events update the same visible list. Own normal/burst
membership flags prevent optimistic toggles from double-counting their Gateway echoes.
The existing bounded, coalesced message readback remains in the background to reconcile
HTTP/Gateway ordering; known counts stay visible while it runs. Unknown snapshots are
not treated as zero, and unsupported event details retain the conservative readback
path. Repeated/stale sequenced reaction dispatches are ignored. Late history responses
preserve newer reaction values without discarding newer message content. These are
synthetic parser/Gateway/reducer/egui checks, not live normal-account validation.
Wire types follow the official [Gateway reaction events](https://docs.discord.com/developers/events/gateway-events#message-reaction-add).

Reaction readback now uses a history request with limit=1 and around=the exact message ID, matching the current normal-user [discord.py-self get_message implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py). The prior single-message GET can reject normal-user sessions; its Forbidden result caused the existing channel-invalidation policy to clear the conversation. Exactly one matching channel/message record is accepted. Missing targets, neighboring messages and malformed/oversized-count replies leave reaction state unavailable without substituting content. Genuine Forbidden history responses still revoke the channel and its cached history. No write is automatically retried.

Pinned-message browsing uses GET /channels/{channel}/messages/pins?limit=25, following [Discord's Get Channel Pins reference](https://docs.discord.com/developers/resources/message#get-channel-pins) and the primary normal-user implementation's [pins_from request](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py) and [pin iterator](https://github.com/dolfies/discord.py-self/blob/master/discord/abc.py), rechecked September 10. Older pages send the last pin's timestamp as the ISO8601 before cursor, independently of message IDs. Nanosecond precision is retained; each page must advance in pin order. Pages replace the previous 25-item snapshot, with explicit Older pins, Retry older pins after failure, and Reload to newest. Returned has_more controls continuation; no complete/stable listing is promised while remote pins change. Summaries remain text-only with concealed spoilers. No pin/unpin mutation, background polling or automatic acknowledgement is implemented. Open message revalidates normal history. Search, pins and archives share one cancellable task/result slot; none of these snapshots is persisted. No owner-controlled live service test has validated these changes.

## Single-file uploads - September 10

The selected file is uploaded only after Send. Serein requests a staging target with authenticated `POST /channels/{channel}/attachments`, using `files:[{id:"0",filename,file_size}]`; streams a credential-free PUT to its `upload_url`; then creates a message with `attachments:[{id:"0",filename,uploaded_filename}]`. Existing content, reply, explicit mention allowlists and nonce correlation are preserved, including attachment-only messages. This sequence follows the primary [HTTP implementation](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/http.py#L1073), [route](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/http.py#L1527) and [file serialization](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/file.py#L197), inspected at commit `2ba64a9a997e151a9c259984e0a179b1fdf4aff4`. No implementation source was copied. Public bot multipart examples do not verify this normal-user path.

Only `https://discord-attachments-uploads-prd.storage.googleapis.com` on effective port 443 is accepted. This exact origin was independently observed in the public `Content-Security-Policy` returned by unauthenticated `curl.exe --silent --head https://discord.com/app` on September 10. Userinfo, fragments and redirects are rejected; signed path/query values remain opaque, bounded and unlogged. A separate HTTP client sends neither Discord authorization nor cookies to storage. Test-only loopback origins are absent from shipped builds.

The local limit is **up to ten nonempty regular files totaling at most 500,000,000 bytes**, Discord's published Nitro maximum. Discord's [File Attachments FAQ](https://support.discord.com/hc/en-us/articles/25444343291031-File-Attachments-FAQ), updated August 13, 2026, states a 20 MB free limit, 50 MB Nitro Basic limit and 500 MB Nitro limit; server permissions, experiments and rejections remain authoritative. Serein does not infer the account tier or compress/rewrite files, so Discord may reject a selection below the local ceiling.

Application chunks are at most 64 KiB; negotiation and storage responses at most 64 KiB, signed URLs 4096 bytes, server upload names 1024 bytes, local paths 4096 encoded bytes and filenames 256 UTF-8 bytes. One upload job uses latest-value progress, not an expanding event queue. The PUT has a 300-second overall deadline, 10-second connection timeout and 30-second read timeout. Filesystem work stays outside rendering.

Path and open-file size/modification metadata are checked before transfer and again before message creation. Missing or observably changed files fail explicitly. These checks are not an immutable snapshot or a defense against a writer restoring identical metadata; no hidden recovery copy is created. Cancellation before message creation prevents the message POST, but staged bytes already sent may remain remotely; Serein does not claim remote deletion or a retention deadline. Cancellation after message POST begins reports an unknown outcome. No transfer or message write is automatically retried. Signed upload targets and local source paths are session-only.

Synthetic tests cover actual loopback HTTP, credential isolation, redirects, file bounds/changes, cancellation during PUT and cancellation during message creation. They do not establish normal-user interoperability: an owner-controlled developer-session test with an official-client recipient remains required. Multiple attachments, tier-dependent larger files and remote staging cleanup remain unimplemented.

File selection also accepts one native file dropped into the active conversation window. The pinned egui 0.36.2 DroppedFileHandle exposes a path; Serein moves the event handles, accepts only one absolute path within the existing path limit, and never invokes their whole-file bytes API. Drops reuse the picker validation and cancellation slot. They cannot replace an existing selection or active operation and never start an upload themselves. Unsupported/multiple drops and unavailable conversation states report an error. A composer hover hint explains the limit and explicit Send behavior. Native OS drag/drop delivery remains unverified; synthetic handle admission and late-result isolation are tested.

Archived-thread browsing (September 10): [Discord public/private/joined-private archive endpoints](https://docs.discord.com/developers/resources/channel#list-public-archived-threads) describe public and private pages ordered by archive timestamp with an ISO8601 before cursor; joined-private pages use descending thread IDs and a snowflake cursor. The primary normal-user implementation exposes the [same three HTTP routes](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py) and [channel archive selection](https://github.com/dolfies/discord.py-self/blob/master/discord/channel.py). These sources supply wire evidence, not live account acceptance. Serein makes only explicit GET requests, limits each page to 25 entries, checks archived metadata, guild/parent/type, duplicates and cursor progress, and ignores member summaries. Public/private timestamps preserve nanoseconds. Private archive enumeration requires the service permissions described by Discord, including MANAGE_THREADS; joined-private is a separate choice. Open loads history without a join/reopen mutation. No live account was used to validate these routes.

### Unicode emoji artwork (September 10, 2026)

Formatted messages (including existing formatted embed/search surfaces), Unicode reaction
counts and the reaction menu use bundled [Twemoji 17.0.3](https://github.com/jdecked/twemoji/releases/tag/v17.0.3)
artwork from Twitter and contributors, under CC BY 4.0. This is a local rendering feature,
not a new protocol endpoint or proof of matching Discord's current artwork revision.
Grapheme matching supports flags, modifiers, keycaps and ZWJ sequences, including optional
emoji presentation selectors. Code, explicit text-presentation and unknown sequences remain
literal. Original message/reaction strings and outgoing requests are unchanged. Whole-message
Copy preserves the original text; drag-selection of rendered text excludes inline image widgets.
The editable composer continues to use native font text. Custom server emoji and animation
are not supplied by Twemoji and retain their existing text fallback. Validation is synthetic;
no live Discord session was used.

### Custom server emoji, chat picker and copying (September 10, 2026)

Joined servers' catalogs are received from READY and known-guild GUILD_CREATE, updated
by GUILD_EMOJIS_UPDATE, and cleared on GUILD_DELETE. The documented emoji fields and update
shape are supported by [Emoji Resource](https://docs.discord.com/developers/resources/emoji)
and [Gateway Events](https://docs.discord.com/developers/events/gateway-events#guild-emojis-update).
Normal-user READY remains unofficial/unstable; this change was tested with synthetic events
and local sockets, not a live account. Joining new guilds' full navigation remains pre-existing
unsupported behavior. Missing catalogs are displayed as unavailable rather than empty.

Formatted message/profile/embed text renders `<:name:id>` and `<a:name:id>` as static CDN
images, using the documented [custom emoji CDN endpoint](https://docs.discord.com/developers/reference#image-formatting-cdn-endpoints).
Custom reactions use the same bounded media worker. Deleted/failing previews have a fixed
placeholder and retain copyable original markup. Standard Unicode and custom image widgets
participate in text selection: copying a selection retains Unicode sequences/custom markup,
and right-click Copy emoji copies the entire token. Selection endpoints treat each image as
one item, avoiding broken ZWJ sequences or partial custom markup. Whole-message Copy is unchanged.

The chat Emoji button opens a searchable Unicode/name palette and a joined-server rail, also
available in DMs. Search matches custom emoji names and source server names across loaded
catalogs; `:name` autocomplete includes usable customs with their source server. Choosing
inserts at the saved text cursor or replaces its selection, preserves Unicode presentation
selectors, and records the draft without sending. Escape/close restores keyboard focus.
Catalog entries must be explicitly available and unmanaged, with known role restrictions
matched against the account's known source-server roles. A destination guild must allow
USE_EXTERNAL_EMOJIS for another server's emoji; DMs have no guild permission gate. New custom
reactions use the same eligibility rules, while existing reaction/removal semantics remain.
Unknown eligibility remains disabled. Account/Nitro entitlement inference is not implemented;
Discord remains authoritative for actual sends and reactions, including entitlement rejection.
Animated emoji are inserted with their original animated markup and shown as still previews.
Clicking a rendered message, embed or profile emoji opens a native information card. Standard
emoji show their shortcode and default-emoji explanation; customs use their catalog name and
source server when the ID is known, otherwise explicitly report unknown provenance. Code and
concealed spoilers stay inert, links keep their link action, and selection/copy retains the
original Unicode or custom markup. These paths are verified offline, not on a live account.
The composer now displays known user mentions as `@name`, Unicode as bundled Twemoji, and custom emoji as static server artwork, while retaining original wire text for editing/copy/send. Unresolved user IDs remain literal; unavailable server artwork shows its name.


## Server voice — September 10, 2026

Guild voice entry uses the documented [Gateway voice state update and voice allocation flow](https://docs.discord.com/developers/topics/voice-connections): guild-scoped join/mute/leave, matching owner session plus guild server update, and guild ID as voice Identify/Resume server ID. The DAVE group remains identified by channel ID. This protocol documentation is not approval or live evidence for normal-user accounts. Server channel rosters use READY, GUILD_CREATE, VOICE_STATE_UPDATE and unofficial READY_SUPPLEMENTAL/PASSIVE_UPDATE_V2 shapes from the original [discord.py-self dispatcher](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py).

The media engine extends the existing DAVE/Opus path to bounded group membership and simultaneous remote audio mixing. Server mute/deafen gates local media. Empty rooms wait without opening audio devices; microphone capture requires an established encrypted group. Stage channels and group DMs remain visibly unsupported. Main Gateway disconnect, channel/session moves, permission invalidation or voice server migration stop the current media session and require deliberate rejoin. No automatic call or DM ringing is triggered by viewing a roster or joining a guild channel.

The implementation is tested with synthetic protocol/crypto/UI data. The owner-controlled official-client two-way audio, multi-party join/leave, permission, device and network tests in [voice.md](voice.md) remain required before claiming working live interoperability.

### Server people subscription repair (September 10, 2026)

The member pane previously sent deprecated opcode 14 with `typing:false`. Current
[Gateway implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/gateway.py)
and [channel subscription prerequisites](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py)
show opcode 37 and a guild typing subscription before requesting channel member ranges.
Serein now enables that subscription only for the active member pane and clears it with
channel ranges when the pane closes or navigation changes. This receives typing events;
it does not send typing notifications. Incoming typing for the selected conversation is
handled as described below; other conversations' typing is discarded.
The request still covers only positions 0–99, with the existing 128-KiB retained member
budget, request/list identity filtering and timeout. No full-directory fetch was added.

A read-only observation of the owner's already-open app confirmed an unavailable pane
with a nonzero server total. That is reproduction evidence, not successful validation of
this repaired build. Tests use a synthetic local WebSocket; normal-user acceptance of the
new request remains unverified. Role display is **not implemented**: only role permissions
for list identity are read; member role IDs, group headings and role colors are discarded.
Role administration is outside the product scope.


## Channel visibility and history revocation - September 10, 2026

The documented [CHANNEL_OBFUSCATED flag](https://docs.discord.com/developers/resources/channel#obfuscated-channels)
(bit 17 / 131072), described in the [obfuscation change log](https://docs.discord.com/developers/change-log#channel-obfuscation-for-users-and-bots),
is authoritative for this path. Placeholder names are not permission evidence. READY omits flagged
channels and threads under flagged parents; an independently visible child of an obfuscated category
remains visible. CHANNEL_CREATE/UPDATE and guild channel snapshots revoke flagged IDs through the
existing removal path. Thread create/update/archive admission follows the same flag boundary.
Raw navigation item and duplicate-ID limits apply before filtering. Thread sync carries explicit
hidden-ID removals in the same bounded event, including a currently browsed archived thread;
ordinary absence from an active-thread snapshot still preserves that archived view.

An unflagged guild CHANNEL_UPDATE with a valid ID, type and name can restore missing navigation
only for an already loaded guild. Optional flags, position and parent fields need not be present.
Existing channels still receive absent/null-aware patches; restoration does not replace their
omitted fields, rejoin voice or accept an old history result. Incomplete updates wait for a full
update or READY. No guild identity is guessed from an obfuscated payload.

Accepted READY replaces the readable navigation set and cancels prior history requests. Reload
requires a currently loaded text channel, as do HTTP results and disk-cache hydration. Revocation
clears the active view and ends affected voice state; restoration requires deliberate selection or
join. Voice allowances and guild roster snapshots exclude obfuscated channels.
This closes explicit visibility and stale-response paths, not the full role/overwrite permission
mirror. Service permission failures remain authoritative; normal-user live behavior is unverified.

## Permission-aware actions (September 10, 2026)

The session now mirrors the current account's guild owner, roles, self-member role IDs and
timeout, and channel role/self overwrites. Unknown metadata is distinct from a known zero-bit
result and cannot enable guild actions. Owner/administrator bypass, everyone then combined
role then self overwrites, and timeout restrictions follow the documented
[permission calculation](https://docs.discord.com/developers/topics/permissions).
Threads use their loaded text/announcement/forum/media parent's overwrites and
SEND_MESSAGES_IN_THREADS; category overwrites are not recursively applied to children.
Existing DM/group-DM text access remains service-authoritative without guild metadata.

READY and known-guild GUILD_CREATE install bounded snapshots. Role create/update/delete,
self GUILD_MEMBER_UPDATE, owner and channel overwrite updates replace the relevant metadata;
READY_SUPPLEMENTAL and PASSIVE_UPDATE_V2 can update already supplied self-member data.
No full member request or new subscription is added. Normal-user evidence is pinned to
[READY merged members](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/state.py#L1731),
[guild owner properties](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/guild.py#L691),
and [member updates](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/member.py#L375),
plus the [documented member event](https://docs.discord.com/developers/events/gateway-events#guild-member-update).
These are wire references, not live account acceptance or a guarantee of event delivery.
Absent incremental fields preserve known state; explicit null makes roles/overwrites/owner
unknown or clears a timeout. A full self-member snapshot without a timeout means no timeout.

Shared command/UI checks cover sending, attachment selection/drop/upload, own-message editing
and deletion, reactions, history/search/pins/archive reads, and voice admission/capture.
VIEW without READ_MESSAGE_HISTORY permits fresh live messages and separately authorized sends,
but cannot fetch, seed or persist history. Permission loss clears the affected loaded view,
pending reads, result pages and reply target; late content cannot restore it. Recovery drafts
remain. Read restoration requires deliberate reload/reselection. New reactions require
ADD_REACTIONS; an existing emoji can be added without that bit, with history and timeout checks.
Removing one's existing reaction remains separate. Private archive enumeration additionally
requires MANAGE_THREADS. Voice CONNECT permits listen-only joining; SPEAK gates capture;
without USE_VAD, focused push-to-talk must be enabled and held. Restoration never rejoins.

Discord remains authoritative for action rejection, private-thread membership, account emoji
entitlements and delivery of permission updates. Role names/colors, role administration,
moderating other users' messages, new guild joining, and full member-directory synchronization
are outside this slice. Synthetic parser, localhost transport and reducer/UI guard tests do
not establish live compatibility. Native automation remains paused after owner Escape stops;
no live account action, microphone capture or new native screenshot was performed.

## Composer IME shortcuts

Empty IME preedit/commit updates while idle do not block Enter-to-send or the
composer's ArrowUp edit shortcut. The pinned egui-winit integration documents
repeated empty preedits on Linux/Wayland with Fcitx. Active composition, text
commits and composition dismissal still guard shortcuts for that frame;
Shift+Enter continues to insert a newline. The shared composer applies this to
new messages and inline edits. Synthetic event tests cover these transitions;
native Ubuntu input-method interoperability remains unverified.

## Composer and notifications — September 10, 2026

See notifications for service badge/read-state reconciliation, focused
view ACKs, session-only native opt-in and fail-closed mute/DND handling. READY read-state
counts, settings and session presence use isolated unofficial normal-user wire shapes.
Synthetic protocol/UI tests are not live Discord validation. Role/everyone mention events and silent flags are handled through bounded supplied metadata
and current self roles/settings; see notifications.md for conservative classification and
live limits. Blocked relationships and complete protobuf notification preferences remain
unsupported.


### Loaded People presence (updated September 20, 2026)

The open guild People pane consumes standalone PRESENCE_UPDATE for users in its current
100-row subscription mirror. [Discord's presence event](https://docs.discord.com/developers/events/gateway-events#presence-update)
documents partial user objects and online/idle/dnd/offline status. The pinned normal-user
[dispatcher](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/state.py#L2040)
allows an optional guild scope. Serein ignores guildless updates and does not request a
friends/global directory or additional subscription flags. These are primary wire references,
not proof that a normal-user account receives these updates through this client's subscription.

Absent status preserves the known value; explicit null or unknown strings clear it to
Presence unavailable. Online, Away, Do not disturb and Offline are service-reported labels;
Offline does not distinguish an invisible user. Custom activity type 4 is normalized using the
same parser as member snapshots: at most 128 characters / 512 UTF-8 bytes, with optional Unicode
emoji and no controls. Omitted activities preserve known custom text; null, empty or no custom
activity clears it. These absent/null choices are defensive client policy, not a documented
normal-user delivery guarantee. The documented `client_status` desktop, mobile, web and VR
keys are retained as four fixed booleans; absent updates preserve them and offline/invalid
statuses clear them. Mobile presence replaces the avatar dot with a green phone glyph matching
Discord's member UI; desktop, web and VR sessions keep the ordinary presence dot and are not
listed separately. Unknown keys and values are discarded.
Other activities and partial profiles remain discarded; the guild subscription sends
activities=true to receive member presence and rich text. Bursts coalesce within a fixed 100-ms window; stale request/session/access
updates cannot modify the pane. Self-session DND notification suppression keeps its separate
existing path. Synthetic localhost Gateway, reducer and headless UI tests supply local evidence;
normal-account delivery and native screenshots remain owner-controlled validation gates.


### Keyboard conversation navigation (September 10, 2026)

Find conversation / Ctrl+K (Command+K) searches loaded navigation and retained friends by display
name, nickname and username, respecting current VIEW decisions for channels. Friends with an open
one-to-one DM appear once; group membership does not hide a friend. Direct and friend results use
their retained Discord avatar through the existing bounded avatar worker, with initials only while
the image is loading or unavailable. Selecting a friend without
an open DM reuses the bounded [Create DM](https://docs.discord.com/developers/resources/user#create-dm)
request already used by server invites, then opens its confirmed channel through ordinary history
loading. It sends no message and adds no subscription or relationship lookup. Requests are never
automatically retried; stale sessions, mismatched recipients and later navigation cannot redirect
the user. Other selections reuse the sidebar history/roster path, including cancellation and
service-authoritative permission failures. A voice result only opens the roster and does not join.
Normal-user Create DM interoperability remains unofficial and live-unverified. The query is session
UI state and never enters message content, REST requests or SQLite. Headless keyboard/IME checks
are synthetic; real platform input, screen readers and live navigation remain unverified.

## September 10: native system-message descriptions

Discord's [documented message type IDs](https://docs.discord.com/developers/resources/message#message-types) were checked on 2026-09-10. The decoder now retains the type through REST/Gateway messages and the local history cache. The timeline describes joins/welcomes, recipient changes, calls, channel name/icon changes, pins, boosts/tiers, channel follows, discovery notices, threads, invite reminders, AutoMod, subscriptions/offers, Stage events, incident alerts, purchases and poll results. Original content/embeds/attachments still render separately. Copy and loaded reply previews include the description. Unknown types keep an explicit placeholder; legacy cached unsupported rows remain unknown until history revalidation.

These are native textual descriptions, not full interactive cards or proof of normal-user protocol compatibility. Call outcome/duration, subscription details, missing thread content and poll votes are not inferred. Search/pins snapshot excerpts remain their existing content-only previews; opening a hit loads the described timeline message. No live account, call or microphone validation was performed. Open PR #27 adds the external fallback and #28 adds unsupported payload markers; neither implemented these descriptions. Integration must retain their controls/markers without restoring a generic system placeholder for recognized types.


### Reply targets (2026-09-10)

[Discord's message reference documentation](https://docs.discord.com/developers/resources/message#message-reference-structure)
distinguishes an absent referenced_message (unknown) from explicit null (deleted), and supplies
channel IDs on received references. This client accepts navigation for type 19 replies and type
23 context-menu message references only, with default reference type 0, the same channel and a
positive earlier message ID. Crossposts, forwards, thread-parent references and unknown reference
shapes stay non-navigable, retaining their system description or unsupported fallback. Nested original bodies are discarded with a
bounded object visitor rather than recursively hydrated. One existing history page before
target+1 retrieves an unloaded original; there is no bulk search or background traversal.
This is public protocol evidence, not proof of normal-user endpoint acceptance. Live validation
is unperformed; offline regressions cover reference shape, missing/null data and deletion races.


### Server member identity after hydration (September 10, 2026)

A guild refresh triggered by subscribing can recreate channel navigation objects without
the READY-only member-list ID. Member requests now compute that ID from the existing
bounded role/overwrite mirror, so GUILD_CREATE and newly delivered/restored channels
subscribe with current metadata. A change in list identity retires the active request
and lets the visible pane request again; unchanged metadata preserves pending replies.
Missing metadata still means unavailable for ordinary guild channels. Thread participants now
use the separate Gateway snapshot described below.
The shared hash accepts the same u128 permission values as the permission parser.

The original [subscription lifecycle](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/state.py)
and [list identity algorithm](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/abc.py)
were rechecked. This changes local identity selection, not the opcode, requested ranges,
permissions or account access. Synthetic regression tests establish the hydration bug
and its repair, not acceptance by Discord; owner-operated live verification remains unrun.


### Member list row headers and refresh bursts (September 10, 2026)

The original [Gateway wire types](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/types/gateway.py)
distinguish top-level group summaries (ID plus count) from group items inside SYNC ranges
(ID only). Serein incorrectly required a count in every group item, rejecting the complete
member update. It now reads only the group ID needed to recognize an index placeholder;
counts are not fabricated. An exact synthetic ID-only-header regression loads the following
member at the correct position. The owner-run redacted trace confirms that member replies
were rejected by the decoder; the owner subsequently confirmed the repaired list loads.

GUILD_CREATE can emit a synchronous channel-navigation burst. The previous eight-item
reliable queue could terminate the session before the UI drained it. Admission now shares
the same 32 MiB estimated byte budget across up to 4,008 events, preserving FIFO order,
per-item limits and failure on real exhaustion. Full-burst/byte-exhaustion/cleanup tests
are synthetic; this does not promise every large account fits existing account budgets.

The owner confirmed the affected server member list loads after restarting the repaired
diagnostic build on September 10, 2026. This does not establish general live compatibility
or long-running capacity behavior.

### Member role display

The active server member pane follows the gateway member-list index. It starts with at most
100 positions and requests the next chunk only at the bottom, extending the scrollbar by up to
100 positions once that chunk starts arriving, up to the reported total (capped at 250,000).
Reopening the pane or changing conversation/session resets the prefix and scroll position.
Only visible rows are rendered. The live subscription is the visible chunk, or two
chunks when the pane spans a boundary or requests its next page. It does not clear
the guild channel subscription. The chunk loaded when the channel opened stays in a 1 MiB cache
and paints immediately on the way back. A loading snapshot does not erase it. A fresh snapshot
for a range the user already left is still stored there. Group headers and person rows come from
those slots.
The pane does not regroup loaded members or show a partial-list hint. Highest nonzero role color
sets online names independently of the hoisted role used for grouping on the gateway. Offline
names remain muted. Unknown role group ids fall back to the label Role. Role changes/removals
reuse the live permission mirror, and member list SYNC/UPDATE supplies role membership. No
directory fetch was added.

Role name, position, hoist and primary color are documented fields in
[Discord's role object](https://docs.discord.com/developers/topics/permissions#role-object).
Modern `colors.primary_color` takes precedence over legacy `color`; role gradients are not
rendered. Equal positions favor the lower role ID, consistent with
[discord.py role comparison](https://github.com/Rapptz/discord.py/blob/master/discord/role.py).
Names retain hue when readable; the theme adjusts insufficient contrast, including hover.
Member list subscriptions and lazy-range focus remain unofficial. Synthetic role evidence does
not establish live role behavior for every account.


### Authorized message deletion - September 10, 2026

A loaded guild message can be deleted after explicit confirmation when its author is the current
user or the current effective channel permissions include MANAGE_MESSAGES. Others' DM/group-DM
messages are excluded; edit remains author-only. Permission overwrites and thread-parent
inheritance use the existing permission resolver. The confirmation rechecks access; the server
remains authoritative and HTTP rejection/uncertainty does not synthesize successful deletion.

The [official message resource](https://docs.discord.com/developers/resources/message#delete-message)
documents deleting others' guild messages with MANAGE_MESSAGES and lists deletable message types.
Known non-deletable and unknown types are denied locally; automoderation notices require
MANAGE_MESSAGES even when their supplied author matches the user. This is dated protocol evidence,
not proof of normal-user service acceptance. Offline permission/menu/local HTTP tests are synthetic;
no message was deleted on Discord, and native confirmation/screen-reader evidence remains unverified.


## Received rich presence (September 10, 2026)

Activity types and text fields follow the [documented Gateway activity object](https://docs.discord.com/developers/events/gateway-events#activity-object).
The existing member-list snapshot and PRESENCE_UPDATE path now retains bounded rich text alongside
custom status. DM presence is admitted only for already-known accessible DM recipients; initial
friend presence from READY/READY_SUPPLEMENTAL and guild-less updates use unofficial normal-user
shapes observed in [discord.py-self's state implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py).
These sources were checked September 10; developer documentation is not proof of normal-user
support. The initial text implementation added no subscription or endpoint; artwork lookup is
described below. No agent-operated account action or live session validation was performed.
The guild subscription sets `activities: true` so the Gateway delivers member game activity
and rich text alongside presence. Only received activity metadata is displayed; missing events remain
unavailable. Offline status clears activity. Disconnect hides cached DM presence; a successful resume restores it and applies replayed changes. Fresh READY, resync and session reset discard the cache.
Join/spectate actions and progress timers remain unsupported.

Startup correction: Identify does not enable `DEDUPE_USER_OBJECTS`, so initial friend presence
arrives in `READY.presences` with `user.id`. The bootstrap now accepts that format as well as
`merged_presences.friends` with `user_id`; both use the same recipient filter and byte/item limits.
The [unofficial READY capability description](https://docs.discord.food/gateway/gateway-events#ready)
documents this format distinction. A synthetic already-running Genshin Impact activity exercises
the missing startup path. This does not verify the owner's reported live payload, change activity
subscriptions, or promote guild-scoped presence into globally authoritative DM presence.

Activity artwork: profile cards show a static image to the left of the text. The decoder prefers
`assets.large_image`, then `small_image` when large is absent, then the application icon. Numeric
assets and `mp:` references follow the [documented activity asset formats](https://docs.discord.com/developers/events/gateway-events#activity-object-activity-asset-image)
and [CDN paths](https://docs.discord.com/developers/reference#image-formatting). Media-proxy
references must also pass the existing safe image-loader path restrictions; direct external
URLs and unsupported asset schemes are never fetched. A missing/failed image keeps the text.
Application-only activities use the unauthenticated
[`GET /applications/{id}/rpc` endpoint](https://docs.discord.food/resources/application#get-rpc-application-unauthenticated)
to obtain an icon hash, then the fixed Discord CDN app-icons path. This metadata route is
unofficial; a public sample application returned HTTP 200 without credentials on September 10.
Synthetic decoder, local HTTP, cache and egui tests cover the implementation, not normal-user
artwork interoperability. The owner confirmed presence text after launching the prior build;
new artwork has not been tested against the owner's live session.

Profile card presentation (September 16, 2026): member profiles and the bottom-left account
preview share a compact card with an activity-type header, app name, single-line details/state,
large artwork and an overlapping small image. When large artwork has no small image, the existing
application-icon lookup supplies the badge. Both references use the existing bounded image cache
and path validation. Received Gateway and local IPC start timestamps are retained as Unix
milliseconds; visible cards repaint once per second to show elapsed time. Missing timestamps
omit the timer; future starts show no negative time. The overflow menu copies the activity text
locally. No join action, new transport or persistence is added. Synthetic tests cover metadata,
layout and account-preview updates/clearing; native screenshots and live compatibility remain
unverified.


### Inline audio attachments (September 11, 2026)

MP3 and PCM WAV attachments have local Play/Pause, seek, time and volume controls.
Preview eligibility accepts a supported filename extension or MIME type, even when
the other metadata disagrees. Audio hints take precedence over image grouping;
the decoder still validates the bytes before playback.
The existing restricted Discord CDN attachment URL validator is reused; signed URLs
are neither logged nor sent with account authorization. Download/Open original remain
available for unsupported formats and clips exceeding preview limits. There is no
new Discord endpoint or voice protocol. The synthetic demo generates a WAV tone even
for its MP3-labeled card; a separate original MP3 fixture checks real MP3 decoding.
Live CDN/account interoperability and native output on other OSes remain unverified.

### Unknown Gateway variants (September 10, 2026)

Unknown dispatches remain ignored without granting capabilities; unsupported opcodes retain the
existing protocol-error behavior. Opt-in `SEREIN_GATEWAY_DIAGNOSTICS=1` now records bounded fixed
categories for these cases and missing dispatch names. Received names and payloads never enter
diagnostics. This changes observability, not the supported service contract. Offline local-socket
checks cover continued message delivery and heartbeat cursor advancement; normal-user service
behavior remains unverified. See storage-policy.md for exact per-run limits and stderr handling.

### Outgoing game IPC Rich Presence (September 11, 2026)

The saved, off-by-default Game Activity setting now hosts a local activity-only IPC endpoint
instead of polling an executable allowlist. It tries `discord-ipc-0` through `discord-ipc-9`
without replacing an occupied endpoint. Windows named pipes and Unix runtime/temp sockets
follow [Discord's RPC transport](https://docs.discord.com/developers/topics/rpc).
Games must connect to Serein; IPC is point-to-point, not an eavesdropping/subscription feed
from an already-running Discord instance. Enable sharing before launching the game; another
Discord client may win the game's connection. Games without IPC integration remain unsupported.

Supported: v1 handshake/READY, SET_ACTIVITY, null or omitted clear, PING/PONG, disconnect cleanup,
name/application ID, activity type, details/state, timestamps and registered large/small artwork.
The newest active game's update wins, with fallback to another connected game when it clears.
Other RPC commands return correlated errors without disconnecting games that subscribe to join
events. Join/spectate actions, secrets, buttons, party actions and game-provided URLs are omitted.
Legacy omitted clears and callback subscriptions are verified against Discord's original
[SDK serializer](https://github.com/discord/discord-rpc/blob/master/src/serialization.cpp) and
[SDK runtime](https://github.com/discord/discord-rpc/blob/master/src/discord_rpc.cpp).
Timestamps accept legacy seconds and modern milliseconds (as emitted by
[discordjs/RPC](https://github.com/discordjs/RPC/blob/master/src/client.js)); values below 10^10
are interpreted as seconds. This threshold is our contemporary-date heuristic, not a documented
universal conversion rule.

Public `/applications/{id}/rpc` resolves the application name; the unofficial
`/oauth2/applications/{id}/assets` route resolves registered asset keys to IDs, following the
[public HTTP implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py). Both are
credential-free and bounded, with no redirects or automatic retries; Retry-After cooldowns
are shared across the session's metadata lookups. Unresolved artwork is
omitted and application-name lookup failures are visible. These routes and normal-user Gateway
publication remain unofficial/unstable. The existing [Update Presence](https://docs.discord.com/developers/events/gateway-events#update-presence)
path waits for READY/RESUMED, coalesces updates and limits attempts (including clears) to one
per five seconds. The Gateway payload retains the existing online/afk/since behavior.

Offline tests cover native Windows IPC, malformed/oversized frames, handshake/update/clear,
unsupported subscriptions, shutdown, rich Gateway fields, rate spacing and reconnect.
Native screenshots use only `--demo`; they demonstrate settings, not IPC or Discord publication.
No live account/game compatibility was tested. Linux/macOS native execution remains unverified
on this Windows host; builds/tests or fixtures do not establish normal-user service compatibility.

Reply-framing correction (September 11, 2026): assemble the header and payload before writing.
The [C# SDK used by osu!](https://github.com/Lachee/discord-rpc-csharp/blob/master/DiscordRPC/IO/ManagedNamedPipeClient.cs)
parses each completed pipe read as a frame. Against the installed osu! SDK 1.5.0.51,
isolated synthetic Windows pipes decoded 10/10 combined replies and 0/10 split replies;
split replies disconnected even without an artificial delay. A native Rust regression
received only four bytes of a 240-byte READY before the fix and the complete reply after it.
This verifies local framing, not live game-to-Gateway publication. Byte streams do not
guarantee whole-frame delivery universally; this SDK also has a 16,384-byte read buffer
including the header. Normal READY/activity acknowledgements fit comfortably within it;
a maximum-sized 16 KiB PONG payload remains outside that SDK's single-read capacity.

Own activity display (September 11, 2026): the accepted local game report now carries
name, details, state and a registered artwork reference into one bounded session value.
The existing profile/member selectors use it for the current account without depending
on a Gateway self-presence echo. This is local presentation, not confirmation of remote
publication. Stopping sharing, clearing/disconnecting the game or ending the session
removes the local value; existing remote presence remains available as a fallback.
Other users and remote presence caches are unchanged. Synthetic rendered tests cover
both surfaces, same-game detail changes, clearing, dark/light and narrow/wide layouts.

### Server folders (September 11, 2026)

Server ordering, grouping, folder names and RGB colors use the normal-user
`GET/PATCH /users/@me/settings-proto/1` endpoint. This is unofficial and live-unverified.
The bounded adapter patches only the guild-folder subtree, retains unknown fields
and guild positions, checks the freshly read data version, and requires a confirming
response before changing the displayed layout. Conflicts and uncertain saves expose
a refresh/retry action. Other-client changes require the rail context menu's explicit
refresh; Gateway settings updates are not consumed in this slice.

Primary implementation evidence checked: [settings schema](https://github.com/discord-userdoccers/discord-protos)
and [discord.py-self HTTP adapter](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py).
Limits: 200 servers, 200 folder entries, 100 characters/400 bytes per name, 16 KiB
retained layout, and a 6 MiB response cap for Discord's 5 MiB encoded settings value
plus its JSON envelope. Oversized settings disable organization
without hiding normal server navigation. Demo edits stay in memory; live edits persist
through Discord. No live account actions were performed in fast local validation.

### Invite acceptance (September 11, 2026)

Native invite cards offer a deliberate server join after a valid, bounded preview. The
normal-user `POST /invites/{code}` with an empty JSON body is unofficial, based on
[discord.py-self's accept_invite implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py).
It is single-attempt and uses the existing rate-limit and session-challenge handling.
A successful response confirms invite acceptance only; gateway guild/channel/permission
updates supply actual access. New gateway guilds enter the server rail. Expired or rejected
invites and uncertain writes show errors. Challenges, membership screening, and application
requirements remain unsupported in the native join flow. No challenge bypass or automatic retry.
Live acceptance and restricted-server flows remain unverified; offline demo cannot join.

Joined-server invite cards (September 16): a fresh invite preview for a known joined guild
offers **Go To Server**. It uses the same navigation as the server rail, restoring an accessible
last-viewed channel or choosing the first accessible channel. It never sends another join request.
Accepted invites without gateway-confirmed guild membership keep the pending-access behavior.
Synthetic click tests cover navigation, preserved drafts and the absence of a second join request;
native visuals and live account interoperability remain unverified.

Standalone joining (September 12): the server-rail **+** opens **Join a Server**,
including when no conversation is selected. Paste a bare code or a supported Discord
invite URL, choose **Check Invite**, review the server, then choose **Join Server**.
The explicit lookup reuses the 32-entry/4-KiB-per-preview/five-minute cache and
single in-flight lookup guard; automatic message cards still require readable history.
Input is limited to 512 characters/2048 bytes, with invite codes limited to 100 ASCII
characters. Dialog state clears across account generations. Discovery opens Discord's
public server directory in the browser; native Discovery is not implemented.


### Account activity sharing and server observations (September 11, 2026)

The local Share game activity switch does not itself enable Discord's account-wide
`show_current_game` preference. Serein now reads that preference after local opt-in
and offers Enable on Discord only when it is disabled. This explicit action uses
normal-user `GET/PATCH /users/@me/settings-proto/1`, preserving other status/custom
status bytes and guarding the fresh data version. Unconfirmed writes are not retried
automatically. Turning local sharing off clears this client's game; it does not change
other clients' account preference. External preference changes are rechecked on a
new local opt-in cycle or an explicit retry after a hidden/error response.

Post-send `SESSIONS_REPLACE` observations distinguish an aggregate public game,
hidden game, current-session receipt, aggregate omission, and no confirmation.
They match application ID/type, not every beatmap detail. Local profile/member
previews remain local; neither a successful socket write nor a local preview proves
peer visibility. Per-server, per-game and friend privacy may still limit visibility.
No speculative activity fields or client identity changes were introduced.

Sources checked: [Discord's Activity Sharing FAQ](https://support.discord.com/hc/en-us/articles/7931156448919-Activity-Sharing-on-Discord-FAQ),
[settings protobuf schema](https://github.com/discord-userdoccers/discord-protos/blob/master/discord_protos/discord_users/v1/PreloadedUserSettings.proto),
and [discord.py-self session handling](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py).
The normal-user protobuf/session behavior remains unofficial and live-unverified.
Local mock HTTP/WebSocket tests verify bounds, preservation, write confirmation,
listed/hidden/missing observations and reconnect resets. No live account was used.


### Server dropdown actions (September 11, 2026)

Create Invite uses documented [Create Channel Invite](https://docs.discord.com/developers/resources/channel#create-channel-invite)
(`POST /channels/{channel.id}/invites`) with editable expiry, use limits and temporary
membership. The default is 30 days, unlimited uses, non-temporary membership and a
new invite. The 30-day option follows the normal-user reference UI and exceeds the
public developer documentation's 7-day maximum; acceptance remains unverified.
The response must confirm the selected settings; rejection does not silently change them.
It requires known effective
Create Instant Invite and View Channel permissions for the selected guild channel.
The bounded response must confirm guild/channel IDs and a safe code before the UI
constructs a `https://discord.gg/` link. It never posts or shares the link automatically.
Selecting Create invite generates the link once. Each friend's Invite button separately
opens/reuses that friend's DM and sends the link through the existing message transport.
Only current friend relationships appear; each send needs a confirmed message response
before showing Sent. Ambiguous sends remain uncertain and cannot be retried for that link.
The friend list comes from unofficial READY/relationship events; individual sends use the
documented Create DM and Create Message routes. No live friend invites have been tested.
Leave Server uses documented [Leave Guild](https://docs.discord.com/developers/resources/user#leave-guild)
(`DELETE /users/@me/guilds/{guild.id}`). Known owners cannot leave here; pending
messages or an active guild call prevent the request. Confirmed success removes
navigation and access through existing channel cleanup while preserving drafts.
One pending action and one bounded result are retained only in the session. Requests
never automatically retry; ambiguous results remain visibly uncertain, and stale
responses cannot change a new session or a subsequently rejoined guild. Malformed
successful invite responses are treated as uncertain writes.
These routes are documented developer API protocol evidence, not approval or proof
of normal-user compatibility. Synthetic state/HTTP checks cover this adapter; live
normal-user creation/leaving, service challenges and restricted guilds remain unverified.

### Account menu and session presence (September 12, 2026)

The account card now opens a compact status submenu with Online, Idle, Do Not Disturb
and Invisible, including the latter two explanations. Custom text opens its own bounded
editor with Apply/Clear. This reuses session presence publication; no automatic expiry,
emoji picker, account switching or public-profile editing is claimed.

The popout itself was restyled to match Discord's account panel: a 64-point banner with
a ring-punched 72-point avatar, a sunken card holding display name, handle, pronouns and
any custom status, then icon rows for the presence submenu (leading status glyph,
trailing chevron) and the custom-status editor (smiley, or pencil once set). Only
layout changed; presence publication, profile loading and the bounded editor are the
same. `--demo --demo-account` (or `--demo-account=status`) opens the popout with the
synthetic profile at startup for screenshots; dark and light rendering were captured
natively on macOS.

### Account-type badges (September 12, 2026)

Chat author headers and member-list names share a compact theme-aware badge, reserving
space when a nickname is long. BOT is driven by the user object's explicit `bot` flag;
WEBHOOK requires message `webhook_id`; APP takes precedence for bot/webhook messages
with an explicit `application_id`. Ordinary users and rich-presence embeds are not
relabeled from their names, profile errors or generic `application` objects.
Member payloads normally expose only `bot`, so these display BOT, not a guessed APP
classification. Webhooks are message authors, not invented guild members.

Sources checked September 12:
[Discord User Resource](https://docs.discord.com/developers/resources/user),
[Discord Message Resource](https://docs.discord.com/developers/resources/message).
These are documented metadata fields, not a live normal-session verification.
One fixed-size model enum survives cache schema 14; legacy messages remain unclassified
until refreshed. Tests cover metadata precedence, migration/roundtrip, grouping changes,
both named UI surfaces and narrow/light/dark badge layout. Native visual evidence and
live-account validation are not available in this agent environment.

### Private notes and friend nicknames (September 12, 2026)

Shared user menus expose Add Note and Add/Edit Friend Nickname; the latter requires a
confirmed friend and never changes a server nickname. Notes load before editing,
failed writes retain the draft, and clearing is explicit. Private names appear in
Friends/search, DM navigation/header/composer, message author labels and profile cards.
Public names, usernames and IDs remain unchanged.

These normal-user routes are **unofficial and live-unverified**:
GET/PUT `/users/@me/notes/{id}` and PATCH `/users/@me/relationships/{id}`.
Note 404 means no saved note; removal sends an empty note or a null nickname.
Sources checked September 12:
[HTTP implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py),
[nickname edits](https://github.com/dolfies/discord.py-self/blob/master/discord/relationship.py),
[note Gateway updates](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py).
READY/relationship patches restore private names and USER_NOTE_UPDATE refreshes the
single active note. Newer service events beat outstanding local acknowledgements.

Client limits: 256 Unicode characters/1024 UTF-8 bytes per note, 32/128 per nickname;
one retained note and at most 4000 tightly allocated nickname strings plus bounded
map overhead. Reads are capped at 2 KiB. State resets at account boundaries; no private
notes or nicknames are added to local storage/logging. Offline fixtures and local HTTP
tests are not proof of live persistence or synchronization. Native screenshots remain
unavailable in the current agent environment.

### Session presence publication

Click the footer avatar or account name to preview the global profile, select Online,
Idle, Do Not Disturb or Invisible, and apply/clear a custom status. Serein reads
the account's Discord status from unofficial `settings-proto/1` before identify
and writes that field when the owner changes it. A local `account_presence` row
is only the fallback when that read fails. Profile loading reuses the existing
bounded global-profile adapter.

The existing Gateway opcode 3 publisher coalesces both game and custom activity into
one update, with at least five seconds between attempts and publication after
READY/RESUMED. Invisible omits activities from outbound updates while retaining the
local choices. Reidentification preserves the selected status. Do Not Disturb also
suppresses this client's native notification alerts.

[Discord's Update Presence and Activity Object documentation](https://docs.discord.com/developers/events/gateway-events#update-presence)
documents the status values and type-4 custom activity shape. The 128-character /
512-byte text ceiling is this client's limit. Normal-user custom-status acceptance,
cross-client interactions and public visibility remain unverified. The menu says
public visibility is unconfirmed; an outbound socket write is not confirmation.
Synthetic tests exercise UI actions, bounds, coalescing, clearing and reconnect.

### Inline attachment video (September 12, 2026)

MOV/MP4 attachments, plus WebM/Matroska where the platform provides codecs, play
inside the message with Discord-style overlay controls: a
centered play button on the picture, and a translucent bar over its lower edge with seek,
elapsed/total time, volume and fullscreen that hides while playing until the pointer or keyboard
focus returns. Fullscreen reuses the active player and texture; Escape or its exit button restores
the previous window mode. File actions (save, copy and open original) use the video's right-click
menu. The seek range remains the full duration while a requested position buffers.
Every platform decodes through the same credential-free, validated Discord CDN
range reader; no attachment is opened as an OS URL and no webview is involved.
The reader retains up to eight 256 KiB ranges to avoid refetching data when the decoder
switches between audio and video. Linux polls both bounded output queues without waiting
on one track while the other needs draining. Clock-only UI updates run at 10 Hz; decoded
frames and playback-state changes request immediate repaint.

* Windows: Media Foundation. MPEG-4/MOV and EBML-based WebM/Matroska containers are admitted;
  installed Windows codec availability still controls their video/audio coverage (including HEVC).
  GPU frames use their actual row layout, and track rotation is applied once; an unavailable
  native rotation control falls back to the software reader.
* macOS: a bounded Rust MPEG-4 demuxer feeds VideoToolbox (H.264 and HEVC, including
  B-frame reordering) and Symphonia's pure-Rust AAC-LC decoder. HE-AAC, MP3-in-MP4,
  fragmented files, external data references and encrypted tracks are rejected.
* Linux: GStreamer `decodebin` pulls bounded byte ranges from that reader through
  `appsrc`; the installed plugins (base, good and libav are recommended by the package)
  decide which codecs play. Output is 48 kHz stereo PCM and RGBA pictures with
  orientation tags applied.

Unsupported containers/codecs show an error inside the player with the existing
download/open actions in the context menu. Limits are 100 MiB encoded, two hours, 1920 pixels per side and
1920x1080 total pixels, and mono/stereo audio up to 96 kHz. Rotated portrait video uses
the same pixel budget.
Only an explicit attachment Play starts decoding; leaving its visible message (unless
fullscreen) or channel, hiding the app, logout, replacement or cancellation stops that player. Embeds with web
video pages continue using their external link action. No live Discord media was tested.

The native MPEG-4 source does not support external tracks; it receives an unnamed byte
stream without a base URL and Media Foundation starts with socket support disabled.
See [Microsoft MPEG-4 source documentation](https://learn.microsoft.com/en-us/windows/win32/medfound/mpeg-4-file-source).

### Home rail request badge (September 15, 2026)

The Direct Messages tile on the server rail shows a bottom-right count. The count is
incoming friend requests that are not `is_spam_request`, plus pending non-spam
message-request DMs from people who are not friends and not blocked.
Official Message Requests are DMs from people who are not friends
([Discord Support](https://support.discord.com/hc/en-us/articles/7924992471191-Message-Requests)).
A loaded request channel still counts when that person also has a pending friend
request. A message-request id with no loaded channel or with no other recipient
is not counted. Outgoing friend requests do not increment the friend addend.
Spam-flagged incoming friend requests stay on the Pending list and do not
increment the badge. Spam-folder DMs do not increment it and do not appear on
the unread rail or the Direct Messages sidebar. The unofficial relationship field
`is_spam_request` comes from the
[docs.discord.food relationship object](https://docs.discord.food/resources/relationships#relationship-object).
The unofficial channel fields `is_message_request` and `is_spam`, plus channel
flag `IS_SPAM` (`1 << 5`), come from the
[docs.discord.food channel object](https://docs.discord.food/resources/channel#channel-object).
READY, CHANNEL_CREATE, and CHANNEL_UPDATE fill the channel inbox sets.
RELATIONSHIP_ADD, RELATIONSHIP_UPDATE, and RELATIONSHIP_REMOVE update the
incoming friend-request spam set. This change does not add a Message Requests
or Spam inbox.

Unread private conversations on the rail sort by latest known activity, newest first.
An active DM call stays visible and pinned at the top. The rail keeps at most 15 rows
after that sort.

### Friend requests (September 12, 2026)

Friends now includes Add Friend and Pending, with searchable incoming/outgoing lists,
accept/decline/cancel controls, and explicit empty, disconnected and failure states.
Sending accepts modern unique usernames; personalized greeting notes are not supported.
The relationship adapter uses unofficial normal-user routes: POST
`/users/@me/relationships` with username and null discriminator, PUT
`/users/@me/relationships/{id}` with an empty object to accept, and DELETE to decline
or cancel. These shapes and incoming/outgoing types 3/4 are corroborated by the
[discord.py-self HTTP implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py)
and [relationship enum](https://github.com/dolfies/discord.py-self/blob/master/discord/enums.py),
not official support or proof of live interoperability.

READY and relationship dispatches retain at most 4,000 pending profiles / 2 MiB,
separate from confirmed friends. Missing profile metadata remains explicitly unknown.
Writes reuse the existing single pending user-action slot and session generation;
newer Gateway state wins over late HTTP results. No automatic write retries occur.
Username-based sends wait for Gateway identity data rather than inventing an outgoing row.
Synthetic reducer, protocol and local HTTP tests cover this path; live requests,
service challenges and recipient privacy restrictions remain unverified.

Profile cards also expose confirmed friendship, incoming requests and outgoing requests.
Adding from a card uses its user ID with PUT `/users/@me/relationships/{id}` and an empty object;
removing a confirmed friend uses DELETE only after a named confirmation dialog.
Acknowledged ID requests retain a bounded outgoing row with unknown profile metadata until
Gateway data arrives. Failed writes preserve friendship; newer Gateway updates win over
late acknowledgements. Cancellation and account changes discard pending UI confirmations.
These paths have offline UI/reducer/HTTP coverage, not live-account verification.

### Blocked and ignored users (September 21, 2026)

Friends includes a searchable Blocked & Ignored tab populated from READY relationship
profiles and subsequent relationship add/update/remove events. Relationship type 2 is
shown as Blocked; the separate unofficial `user_ignored` relationship flag is shown as
Ignored, following the public
[relationship object documentation](https://docs.discord.food/resources/relationships#relationship-object).
Profiles are session-only and bounded separately from friends. Existing block/unblock
actions remain available; changing Discord's distinct ignored state is not included.
Synthetic protocol, reducer and UI tests cover loading and updates. Normal-user payload
availability, cross-device changes and service behavior remain live-unverified.

### Additional system message display (September 12, 2026)

The timeline now describes friend-request acceptance (67), Stage speaking requests (30),
HD-stream upgrades (55), reported-message deletion (58), timeout/kick/ban results
(59–61), resolved reports (62), and voice hangouts (65). These use the existing compact
system rows: small event-specific icons, muted sentences, strong clickable names and
inline timestamps. Copy/reply summaries share the same model descriptions. Unknown
types and unsupported rich content retain their explicit external-open fallback.

Type names and field meanings were checked against the
[Discord Userdoccers protocol research](https://docs.discord.food/resources/message#message-types).
The newer normal-user types are unofficial, not official compatibility guarantees.
Only existing author/mention/content fields are used; missing moderation targets say
"a member". No duration, moderator reason, subscription entitlement or call outcome
is invented. This adds display support, not moderation, stream upgrades or automatic
call joining. Synthetic model/parser/UI tests cover the descriptions and fallback;
live rendering and native visual comparison remain unverified.


### Server role administration (September 12, 2026)

Server Settings now includes Roles for accounts with Manage Roles. The role list,
@everyone permissions, create/edit/delete, hierarchy reordering, display options,
and member assignment use the existing single server-administration request lane.
Managed roles and roles at or above the actor's highest role remain read-only;
owners retain their hierarchy exemption. Unknown permission bits are preserved,
and only changed permission bits are merged into a fresh role before a write.

The role and image shapes follow Discord's [role documentation](https://docs.discord.com/developers/topics/permissions#role-object)
and [image formatting reference](https://docs.discord.com/developers/reference#image-formatting).
Role icons require ROLE_ICONS; enhanced colors require ENHANCED_ROLE_COLORS.
There is no boost purchase flow or invented role-link support. Static icon uploads
reuse the bounded native image worker. Member counts remain unknown when the count
route is unavailable. Member search retains the existing unofficial normal-user
members-search path and additionally requires Manage Server. No directory scraping
fallback is used. These normal-account transport paths remain live-unverified.

Writes reconcile from a fresh catalog. An ambiguous response requires an explicit
reload before another write; creates and deletes are never automatically retried.
Synthetic fixtures and tests do not demonstrate live Discord interoperability.

### Server invite administration (September 12, 2026)

The Invites settings page requires Manage Server and loads the guild invite list
on demand. Uses, limits, inviter, timestamps, channel and assigned role IDs come
from returned metadata; missing values stay unknown. Creating an invite reuses the
existing channel invite flow. Revocation requires a code in the loaded guild list
and current Manage Server access. Pause/resume changes only INVITES_DISABLED in a
freshly fetched guild feature list, preserving unknown features. Both writes verify
their scoped response and reload; uncertain outcomes require explicit reload and
are never automatically retried.

The routes and permission requirements follow the official
[guild invite listing](https://docs.discord.com/developers/resources/guild#get-guild-invites),
[mutable guild features](https://docs.discord.com/developers/resources/guild#mutable-guild-features),
and [invite deletion/object documentation](https://docs.discord.com/developers/resources/invite#delete-invite).
The list route also accepts View Audit Log, but full invite metadata requires
Manage Server, which is the gate used here. There is no invite-event subscription,
scraping, invented pagination or automatic background refresh. Synthetic protocol,
reducer and local HTTP tests cover bounds, scope, permissions and write reconciliation;
normal-account compatibility and service-side concurrent edits remain live-unverified.


### Channel integrations (September 22, 2026)

Channel Settings > Integrations reuses the native webhook and followed-channel
pages with a channel-scoped snapshot. Manage Webhooks plus View Channel on that
channel permits entry without Manage Channels or server-wide Manage Webhooks.
The [documented channel webhook endpoint](https://docs.discord.com/developers/resources/webhook#get-channel-webhooks)
loads only that channel's metadata; create defaults to that channel, and rename,
move to another permitted channel, and confirmed delete reuse the existing worker.
Moving a webhook refreshes the original channel and removes it from that list.
Guild-wide app integrations remain under Server Settings. Scope and permissions
are rechecked for responses and writes; existing item/byte limits still apply.
The shared editor supports explicit Copy Webhook URL for incoming webhooks; avatar uploads remain unsupported.
Synthetic checks do not establish live normal-account interoperability.

### Server integrations (September 12, 2026)

Server Settings > Integrations loads the guild integration list on demand with
Manage Server, and independently loads webhooks/followed channels with Manage
Webhooks. Cards open native integration details and webhook management. Incoming
webhooks can be created or renamed/moved to an accessible text, announcement,
forum or media channel; known webhooks and followed-channel subscriptions can be
deleted after confirmation. Removing an integration also removes its associated
webhooks and bot membership according to the service contract; the UI states that
consequence before confirmation. Current guild/channel permissions and request
and session generations are checked again before accepting results.

The adapter uses the documented [guild integration list and delete routes](https://docs.discord.com/developers/resources/guild#get-guild-integrations)
and [authenticated webhook management routes](https://docs.discord.com/developers/resources/webhook).
These developer API contracts are protocol evidence, not approval or proof of
normal-user session interoperability. Normal-user live validation has not been
performed. Forbidden, unsupported, oversized and uncertain-write responses are
reported without automatic mutation retries. An uncertain outcome requires an
explicit reload before another write.

Integration reads retain at most 50 integrations and 1,000 webhooks within a
combined 1 MiB metadata budget; HTTP responses are capped at 2 MiB. The service's
50-integration endpoint limit is not presented as a complete count for larger
guilds. Missing metadata stays absent; last synchronization is not represented as
an installation date. List decoding discards webhook execution tokens and URLs.
Copy Webhook URL performs an authenticated read, capped at 64 KiB, validates the
webhook identity and current channel permission, and hands a zeroizing URL to the
clipboard once. Tokens are bounded to 256 URL-safe bytes; neither tokens nor URLs
are logged or persisted. OAuth command
permission editing, avatar uploads, and creator
subscription settings are not part of this slice.

### Server audit log (September 12, 2026)

Server Settings > Audit Log is available with View Audit Log permission; Manage
Server alone does not grant access. The read-only view fetches the documented
[guild audit log endpoint](https://docs.discord.com/developers/resources/audit-log#get-guild-audit-log)
on demand, with user/action filters and explicit backward pagination. Entry cards
show the available actor, action, timestamp, reason and before/after changes.
Unknown action codes and missing actors remain visible without invented metadata.
Change values preserve absent versus null; no audit mutation or background polling
is added. Permissions, guild/session scope and request sequence are rechecked
before accepting a response. Closing settings or losing access releases history.

Requests use 50-entry pages; retained history stops at 500 entries or 2 MiB and
offers reload/filtering instead of unbounded collection. The user selector lists
actors in loaded results, not a separately scraped member directory. The service
documents a 45-day retention window; this client does not archive audit history.
Developer documentation describes the protocol, not approval or evidence of
normal-user session interoperability. Parser, reducer and local HTTP tests are
synthetic; live normal-account behavior remains unverified.


## Messaging permissions

Messaging Permissions exposes spam filtering, all-server/per-server DM and message
request preferences, friend request sources and personalized messages, and connected
game messaging preferences. Content Filters is not included. Reads and version-guarded
writes use the unofficial `/users/@me/settings-proto/1` route and preserve untouched
fields in the changed subtree. The UI updates after the service confirms the values;
these are account preferences, not a new local spam classifier or game integration.
Field mappings follow the community-maintained
[PreloadedUserSettings schema](https://github.com/discord-userdoccers/discord-protos/blob/master/discord_protos/discord_users/v1/PreloadedUserSettings.proto).
Live normal-user interoperability remains unverified. Offline demo changes stay in RAM.

## User notification preferences

Notification settings are device-local only: desktop alerts, message and ring
sounds, and the Windows unread badge. Per-channel and per-server mute preferences
still come from the account through the existing Gateway user settings events.
The user guild settings decoder accepts string and integer `guild_id` values,
including in legacy and versioned READY snapshots. Missing, null, and zero IDs
identify private-channel settings; [legacy zero IDs have been observed in READY](https://github.com/bwmarrin/discordgo/issues/624#issuecomment-456934135).
These entries must not discard the account's saved mutes. Numeric IDs stay exact
64-bit integers; invalid IDs remain rejected. This compatibility case is covered by the offline
`cargo run --locked -p discord-protocol --example notification_settings` check;
live normal-account restart behavior remains unverified.
The client no longer reads or writes Discord's account notification overview
(`/users/@me/settings-proto/1` notification subtree) or email categories
(`/users/@me/email-settings`), and it ignores `NOTIFICATION_CENTER_ITEM_CREATE`;
stream, scheduled-event, reaction, friend-online and profile-update alerts are not
raised. Only received messages and incoming calls produce notifications.

## Thread participants — September 15, 2026

Opening a thread's People pane subscribes to that thread through the existing
unofficial user Gateway subscription (`thread_member_lists`) and receives
`THREAD_MEMBER_LIST_UPDATE`. The previous REST List Thread Members route is
[restricted to bot accounts](https://docs.discord.food/resources/channel#list-thread-members),
which caused normal-account participant requests to fail. The user-client packet
and event shapes follow the upstream
[subscription implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/gateway.py)
and [thread snapshot handler](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py).

The pane retains the first 100 participants and their supplied presence within
128 KiB; payloads are capped at 512 KiB. It uses one existing member subscription,
clears it on pane close or navigation, and ignores other guild/thread snapshots.
Reload resubscribes; a missing response uses the existing 15-second timeout.
No bulk guild subscription, background pagination or thread join is performed.
The offline debug check exercises the packet, decoder and state admission.
Live acceptance remains unofficial and unverified.

## Rich Presence transports, artwork and game detection — September 16, 2026

Game Activity now covers what an out-of-client Rich Presence bridge is expected to do,
following the feature set of [arRPC](https://github.com/OpenAsar/arrpc) and
[rsRPC](https://github.com/SpikeHD/rsRPC) without reusing their code.

**WebSocket transport.** Alongside the existing IPC endpoint, sharing binds the first free
port in Discord's RPC range 6463–6472 on loopback only. The upgrade request must carry
`v=1` and a numeric `client_id`, and either no `Origin` or one of Discord's own web origins;
anything else is refused without a reported error, because browsers and port scanners reach
these ports routinely. Accepted sockets receive the same READY dispatch and speak the same
JSON commands as IPC, without opcode framing. Messages and frames are capped at 16 KiB.

**Artwork.** `large_image`/`small_image` now accept three forms: a registered asset key or ID,
an already-proxied `mp:` key passed through after the presence path checks, and an absolute
HTTPS URL. A URL is never fetched or forwarded raw; it is exchanged for a media-proxy path
through `/applications/{id}/external-assets`, the same unofficial route Discord's client uses,
and cached per connection so repeated identical updates cost nothing. Registered assets are
refetched at most once a minute when a key misses, so artwork uploaded seconds after a game
connects still resolves. This addresses [#237](https://github.com/ViceVerse-cz/Serein/issues/237),
where a plugin published its cover art as a URL.

A small image is a corner badge. Both the outgoing path and received presences now fall back
to the application icon when no large image resolves, instead of promoting the badge into the
artwork — the second symptom reported in that issue.

**Detection.** Games that never speak RPC are matched by executable name against Discord's
public `/applications/detectable` list (about 24,000 applications, 10,451 of them detectable,
12.7 MB). The list is reduced on download to id, name and non-launcher executable names
(1.1 MB), cached for a day under the local data directory, and re-checked against the same
bounds when read back. Matching compares path suffixes longest-first, plus `.app` bundle
components for macOS entries; an executable-name index keeps a full sweep at roughly 6 ms for
900 processes instead of ~920 ms for a linear scan. Processes are enumerated read-only through
`/proc` on Linux, `ps` on macOS and `tasklist` on Windows: executable paths only, never
arguments, environment, memory or another user's processes. Detection is polled every ten
seconds while sharing is on, keeps a running match stable rather than flapping between two
matches, and always loses to a connected client, which knows more than a process name.

**Invites.** `INVITE_BROWSER` prefills the existing Join a Server dialog and is acknowledged.
It never joins: the user still confirms the lookup and the join. `GUILD_TEMPLATE_BROWSER`,
`DEEP_LINK`, `AUTHORIZE` and join/spectate remain unsupported and answer with correlated errors.

Offline tests cover the IPC and WebSocket transports end to end, origin and handshake refusal,
URL proxying, late artwork uploads, proxy-path traversal attempts, badge fallback, detection
precedence and cache round-tripping. The detectable list, its endpoint and the external-assets
route were checked against live responses; game-to-Gateway publication remains unverified.

### Visible chat author roles

Guild history already carries `member.roles` on each message when Discord sends
it. Those IDs are the first-paint color source and are stored with the account
history window. A separate bounded Gateway opcode 8 `user_ids` lookup, sharing
the mention search transport and its one-second send interval, refreshes
membership when history omits it or a live row changes. Up to 100 visible
authors are requested at once. Empty live rows do not wipe a message snapshot.
Failed lookups keep the snapshot or the normal fallback color. Profile cards
already request guild membership through the profile endpoint. Normal-account
Gateway behavior remains unofficial and live compatibility is unverified by the
synthetic check.

Forum cards now load a bounded recent-message page for visible active posts through
[Get Channel Messages](https://discord.com/developers/docs/resources/message#get-channel-messages).
Up to four visible cards load concurrently under the existing REST permit bound.
They show the latest plain preview with the author's known guild role color and count IDs newer than the service read cursor;
`50+ New` indicates that the cursor precedes the retained 50-message window. Failed
or unavailable summaries remain explicitly unavailable until refresh or new activity;
successful summaries are reused when returning to a forum during the same session.
Startup preserves cursors for threads loaded after READY. These changes have synthetic
offline coverage; normal-account behavior remains unofficial and live-unverified.

### Spotify profile activity — September 22, 2026

Spotify cards use supplied album artwork, song title and artist, with a large Spotify
logo when artwork is absent. Valid start/end timestamps drive a clamped, read-only
elapsed/total progress bar. This does not control playback or implement Listen Along.
The unofficial `spotify:` image mapping follows
[discord.py](https://github.com/Rapptz/discord.py/blob/master/discord/activity.py):
only a 40-character hexadecimal image ID can resolve to `https://i.scdn.co/image/{id}`.
Artwork uses the existing credential-free, redirect-free bounded image worker and
account-isolated image cache, with the existing 1024px decode limit for media previews.
Normal-account behavior and live artwork delivery remain unverified.

Serein also polls the linked Spotify account at most every 15 seconds while visible (sooner
at track end, with a one-second minimum interval), independently
of local game detection. It reads the connection's `show_activity` preference, obtains a
session-only bearer through Discord's unofficial connection access-token endpoint, and reads
Spotify's [`GET /v1/me/player`](https://developer.spotify.com/documentation/web-api/reference/get-information-about-the-users-current-playback).
The token route and outgoing activity fields follow
[discord.py-self](https://github.com/dolfies/discord.py-self/blob/master/discord/connections.py).
The linked token must grant `user-read-playback-state`; that scope and end-to-end publication
remain live-unverified. No local Spotify IPC, playback control or extra Spotify login is used.
Paused, private, local-file, ad, episode, unavailable or failed playback clears the activity;
Invisible stops polling and clears publication. Spotify shares the existing rate-limited Gateway
sender alongside games/custom status. The local profile previews Spotify when no game is active.
Unlinking or disabling Spotify activity is detected on the next poll; service cooldowns apply.
Track-end polls retain the previous activity while awaiting the next response, with a
10–30 second request timeout instead of cancelling immediately at the old track deadline.
The offline debug command is `cargo run --locked -p serein --features demo -- --demo --demo-check-spotify`.

### Outgoing message forwarding

The message toolbar and context menu open a native searchable destination picker for up to
five already-loaded, writable conversations. Forward creation uses the documented
[`message_reference` type 1](https://github.com/discord/discord-api-docs/blob/main/developers/resources/message.mdx),
including the source channel/message and guild when known. The service captures the snapshot;
Serein does not download and re-upload source attachments. Basic supported message types are
eligible; ephemeral, poll and unsupported payloads are excluded. Forwarded context-menu command
snapshots (type 23) use the existing bounded renderer too.

An optional note is an independent ordinary message queued after each forward, so either write
can fail independently. The picker reports per-destination sending, sent, failed and ambiguous
outcomes. Writes are never automatically retried; existing composer drafts are preserved.
Destinations are limited to loaded text conversations, not a server-wide discovery search.
The local synthetic debug check does not establish live normal-account compatibility.

### Member list recovery — September 22, 2026

The bounded Gateway mirror accepts omitted member counts/group summaries on incremental
updates and presence both inside and beside a member, matching the shapes used by
[SakuraCord's member decoder](https://github.com/SakuraCordApp/SakuraCord/blob/f9953d6086d4090dd1d63c0ecef2b0107ba199ad/Packages/DiscordProtocol/Sources/DiscordProtocol/GuildMemberListUpdateDTO.swift).
Member-only UPDATE preserves omitted presence for the same user; explicit null/offline
still clears it. List length includes supplied group headers and visible group counts.
Up to 512 role groups plus online/offline fit the existing role-catalog limit.

Malformed operations leave the last valid snapshot intact. Invalidation keeps cached rows
while awaiting a SYNC; other incremental updates cannot cancel that recovery. A stalled
subscription resets its channel ranges at the existing 15-second interval before requesting
them again. An empty reply only completes loading when the service explicitly reports zero
members. Guild presence remains visible during range loading and transient reconnects;
unavailable/access-revoked state still hides it. All caches remain session/request scoped.

Offline examples `discord-gateway --example member_lists` and `ui --example member_flicker`
exercise decoding, partial presence, failed-update rollback, recovery state and retained
activity rendering. No live server verification or performance measurement was performed
in this fast pass; external client source is wire evidence, not proof of compatibility.

### Member list resilience — September 23, 2026

A stuck "Loading people…" pane traced to all-or-nothing handling: one member row, presence or
activity this client could not represent (for example a non-integer activity timestamp) failed
the whole `GUILD_MEMBER_LIST_UPDATE`. Every 15-second resubscribe then received and rejected the
same SYNC, so the list for that server stayed empty until the member changed or the app restarted.

Rows now fail individually. An unrepresentable activity is dropped while status stays; an
unreadable presence is treated as omitted; an unreadable row becomes a placeholder at its index
so later INSERT/DELETE/UPDATE positions stay aligned. An unreadable UPDATE keeps the previous
row, and unknown operation names are skipped. Only structurally impossible packets (inverted
ranges, missing indices) are still rejected, with the last valid snapshot retained. Up to 1,024
operations per packet are accepted, and a heavy page sheds activity details instead of failing
the 256 KiB budget. Thread participants use the same presence handling.

The member-list ID is computed from cached permissions using the unofficial algorithm
documented by discord.py-self (`_is_everyone_member_list` and `member_list_id`). If the service
replies for the open channel's viewport with a different ID before the first SYNC, and the ID
is not a list this connection recently left, the mirror follows the service's ID for that
subscription. Stalled-subscription resets back off from 15 to 30, 60, then 120 seconds.

SakuraCord's member decoder uses the same wire shapes (optional counts and groups, presence
beside or inside a member). Offline unit tests cover each case. This was not verified against
a live server; the original failing payload was not captured.

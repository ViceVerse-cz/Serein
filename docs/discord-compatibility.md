# Discord compatibility — checked 2026-09-10

## Explicit Force Sync (October 30, 2026)

**Force Sync This Server** refreshes the selected server. **Settings → Storage & Sync →
Sync Entire Account** covers joined servers and existing DMs/group DMs in the current
authenticated navigation snapshot. Both fetch a fixed 21-day history interval and preserve
the selected conversation, drafts, reading position and unread/mention acknowledgements.
Closing progress leaves the job active; Pause, Resume and Cancel are explicit controls.
Restarted jobs are offered paused rather than automatically starting heavy network work.

Read adapters reuse the ordinary member-readable guild, channel, role, expression,
soundboard, active-thread, archive and message routes. Results pass through the existing
typed decoders and reconciliation rather than privileged editor authorization. Complete
catalogs may remove absent entries; partial fields preserve absent/null/value semantics.
Active threads use the unofficial normal-account [per-channel thread-search route](https://docs.discord.food/resources/channel#search-threads),
with bounded, creation-ordered pagination. The [guild-wide active-thread route](https://docs.discord.food/resources/channel#list-guild-active-threads) is bot-only.
Search indexing delays are retried within the job's limits; a still-unavailable page is
reported as partial. Archive requests are limited to eligible parent types and current
history permissions. Successful channel refreshes are not labelled unavailable solely
because one thread or archive request fails. The regular forum browser also respects
indexing cooldowns and offers retry instead of falling back to the bot-only route.
Only successful, correlated history pages establish coverage. An old thread creation ID
does not exclude recent messages, and joined-private archive pagination is not stopped
using a creation timestamp. Signed media links remain refreshable metadata; the job does
not download every media file.

Full roster requests use nonce-matched Gateway member chunks only when current permission
state allows `MANAGE_ROLES`, `KICK_MEMBERS`, `BAN_MEMBERS` or administrator/owner access.
The [normal-account request restrictions](https://docs.discord.food/gateway/gateway-events#request-guild-members)
are unofficial protocol evidence, not an interoperability guarantee. Without full roster
access, synchronization refreshes available/known members and recent participants, and
reports partial coverage. Exact-ID member reads are bounded to 100 requested identities;
the HTTP fallback permits at most two concurrent reads. Bot-only directory endpoints and
username-prefix enumeration are not used.

The job uses spare HTTP capacity and the existing REST/Gateway cooldowns, keeps only one
active job, and bounds its navigation/participant targets and checkpoints. Correlation,
resource-specific observation revisions, storage epochs and current permission checks
reject obsolete results. Recoverable failures stay with their stage and do not revoke
account authentication. Progress distinguishes updated, partial, unavailable and failed
sections; storage pressure cannot certify complete history.

Archived history carries its approved server/parent cleanup scope even when the thread
is absent from sidebar navigation. Thread removal and scoped permission changes fence
pending reads, while parent cleanup removes retained descendants without restoring them
from a late response. This cleanup metadata never grants access.

An incomplete own-member refresh does not combine new role permission bits with older
assignments. Transient failures preserve the existing validated live permission mirror
and report partial role/member coverage; explicit denial revokes that scope. History
targets that lose access are counted as unavailable rather than silently skipped.

Fresh member names, nicknames, roles and server-avatar context feed the shared display
state used by the conversation, member list, typing indicator and mention picker. Avatar
patches preserve missing versus explicit removal and use the existing restricted server
avatar CDN path. Persisted display hints grant no access or expression eligibility. Guild
banner hashes have a separate session-only, access-checked presentation record; Serein
does not introduce a new server-banner layout as part of this refresh.

Partial member-list snapshots retain the distinction between omitted nicknames or roles
and explicit removals, including across channel subscriptions. Ordinary-member sync
also refreshes known session identities, not only authors found in recent history. An
exact-ID response with no nickname field can refresh its role assignments without clearing
a known nickname; the job reports incomplete nickname coverage instead of replacing the
stored display hint with an unverified removal. Complete roster chunks carry the same
nickname-presence information while retaining their independently verified member IDs.

Offline adapters and fixtures exercise these paths. Cross-client synchronization, large
live rosters and account-wide acceptance remain owner-controlled checks; successful
synthetic tests do not establish live Discord interoperability.

## Connection recovery and delivery pressure

Gateway operation cooldowns now belong to the account connection rather than a socket;
RESUME and fresh synchronization preserve unexpired penalties. Sequenced dispatches advance
the resume cursor only after complete admission, and older/duplicate dispatches are skipped.
Heartbeat send receipts are retained in a bounded FIFO so an older ACK cannot retire a newer
probe. Existing heartbeat deadlines and the documented connection-wide sending limit remain.

REST global penalties are separate from route/bucket penalties, including major resource
parameters. Cooling requests wait outside concurrency permits. The retained table is bounded
to 256 entries / 64 KiB; active penalties are never evicted to make room. Safe startup reads
may retry two transient failures within 30 seconds; authentication rejection stays terminal.
Busy queues and oversized optional responses fail their operation. Accepted or uncertain
writes are never automatically replayed.

If account-event delivery loses an update, the account remains authenticated while the
Gateway owner retires and waits for queue capacity. An independent, generation-stamped
pressure notification prevents old queued updates from restoring access. A fresh snapshot
must follow an explicit synchronization barrier before normal delivery and queued writes
resume. Media is retired when permissions become uncertain; this exceptional recovery may
interrupt an active call/share. Ordinary socket RESUME keeps its existing voice behavior.
Malformed optional catalogs are invalidated; uncertain live permission metadata, including
self-member permissions in a passive update, requires authoritative synchronization.
Identity, framing and unsafe session-wide bounds remain strict.

Copy Issue Diagnostics retains fixed failure-stage labels and queue occupancy in the existing
32-entry / 8-KiB session ring, without payloads, credentials or account identifiers.
Local synthetic WebSocket, HTTP, delivery-queue and reducer regressions exercise these paths.
Normal-account member subscriptions/rate-limit details remain **unofficial**; owner-controlled
Discord, sleep/wake and network-interruption acceptance remains **unverified**.

## Channel preference synchronization and Browse Channels — October 26, 2026

Browse Channels is a main-content page with the server sidebar and Back/Forward navigation.
Its checkboxes edit saved membership; opening a channel is a separate navigation action.
All / In My Channel List / Not In My Channel List and Muted filters distinguish membership,
muting and temporary reveals. Category selection is inherited by its children and their threads;
removing one inherited child materializes the siblings before removing the category selection.
Show All keeps the custom list, and an explicit hide returns to that list. Unknown preferences
show a loading/error state and disable the affected controls; accessible sidebar rows remain
available with an unsynced hint. Browsing does not acknowledge the selected conversation.

Personal channel and server notification changes apply immediately, debounce for 250 ms and
save through one serialized lane. Confirmed settings remain separate from pending choices.
Gateway updates and REST confirmations share normalized field presence, full/sparse collection
handling and server versions. Request/observation ordering fences legacy unversioned responses,
including refreshes. Sparse writes use the latest confirmed flags and preserve unknown bits.
A conflicting authoritative choice is shown with Apply Again. Rejected writes restore confirmed
values and offer Retry; uncertain writes stay visibly unconfirmed and are never replayed
without a new explicit choice. Unsent work is revalidated after waiting behind another mutation
and again at worker entry. Only a proven-unsent stale flag payload may be rebased automatically.

Refresh requests the current representation using an empty PATCH to the existing
[unofficial account guild-settings route](https://docs.discord.food/resources/user-settings#modify-user-guild-settings).
Closing the page preserves authorized saves; account/guild/access changes retire stale work.
Local synthetic decoding, HTTP, reducer and UI tests cover these paths. Synchronization in both
directions with an owner-controlled Discord session remains **unverified**.

## Server administration and personal channel controls — October 23, 2026

This milestone connects the existing native administration pages with personal channel
selection, member review, protected actions and community access controls. Normal-account
settings, member search, bulk completion, application review, MFA and command-permission
transport use **unofficial** interfaces described by [Userdoccers](https://docs.discord.food/).
Documented resource schemas do not establish normal-account authorization or service acceptance.
Discord remains authoritative for
permissions, eligibility and completion. Earlier dated sections below retain their original scope.

Personal channel selection, account favorites and category collapse share the existing
`/users/@me/guilds/{guild}/settings` transport and Gateway settings reconciliation.
Decoded raw flags retain unknown bits; edits change only their known masks and omit unrelated
preferences. Unknown or null preference rows disable writes instead of supplying guessed values.
Responses must confirm the requested flags. The first hide operation initializes the other
accessible channels so enabling a customized list cannot accidentally hide everything.
Show All Channels preserves those saved selections. Hiding changes sidebar presentation only:
it does not mute, acknowledge messages, change thread membership or grant channel access.
Selected conversations, active voice and unsuppressed mentions can temporarily reveal a hidden
channel without changing its saved choice. Browse Channels remains available to restore it.
Device-local collections stay separate; importing their favorites into account preferences is explicit.

Member search supports names/IDs, role, join/account-age, join-method and supplied safety filters.
Mod View uses a separate scoped snapshot, not the visible timeline, history or read state.
It retains at most 256 KiB of metadata, 25 text excerpts and 20 member-related audit entries
from the latest 50 server audit events. The loaded audit time window is shown; this is not a
complete disciplinary record. Missing safety data remains unknown, and media is not loaded
for these excerpts. Current access, blocked-user and spoiler handling still apply.
Moderation rechecks hierarchy and permissions, offers explicit history-deletion choices,
and never automatically resends an uncertain write. Server deafen uses the existing member
voice PATCH alongside mute, move and disconnect.

Service AutoMod and incident-message rows offer explicit member-review links. The reader
chooses a recent-join or supplied-safety filter, or opens Mod View for a received mention.
These links do not classify message text, identify a raid's participants, or treat a report
as proof of misconduct. Ordinary messages cannot acquire the links by copying alert text.

Bulk banning accepts at most 100 explicitly selected targets. The normal-account
`POST /guilds/{guild}/bulk-ban/v2` HTTP acknowledgement means **accepted**, not completed.
Only a valid account/guild-scoped `GUILD_BULK_BAN_UPDATE` matching the active target set
confirms individual successes or failures. Unconfirmed targets become unknown after the
bounded wait; an explicit refresh checks current bans without submitting another ban.
Completion, readback or dismissal retires the operation's later events. The service supplies
no operation nonce, so identical target sets cannot provide stronger correlation.

A valid administrative MFA challenge suspends the exact rejected HTTP operation while the
rest of the session remains usable. Only advertised authenticator, backup-code, SMS and
password methods are offered; SMS transmission requires an explicit click. Account/request
scope and current permissions are checked before one continuation. Tickets, inputs and
proofs stay bounded, redacted and zeroizing, expire within five minutes and are not persisted
or exposed to extensions. Passkey-only challenges require another advertised method or the
official client; Serein does not emulate Discord's WebAuthn origin.

Safety Setup uses service-provided incident state and expiry for temporary invitation/non-friend
DM restrictions of at most 24 hours, with explicit early-resume actions and authoritative readback.
The separate invitation pause in Invites remains independent. Missing incident metadata stays
unknown and disables these writes; it does not mean restrictions were removed. There is no local
raid detection. Owners can change the server's moderator MFA requirement; this does not enroll
any member in two-factor authentication.

Member applications use the **unofficial** [join-request interfaces](https://docs.discord.food/resources/guild#guild-join-request-object),
with separate pending/approved/rejected views, bounded answers, explicit decisions and interview
creation. Requests use their canonical request IDs. Opening an interview sends no message.
Application review is distinct from rules acceptance. Queues load on demand; Applications
shows the loaded pending count. Members and Applications show a neutral update marker when
Gateway changes make that count unknown. They do not scan unopened servers or infer counts.
Gateway changes fence late reads so a withdrawn application cannot be restored by an older response.
Onboarding, screening and Guide previews display the unsaved draft without submitting answers.

Application/command permission edits
preserve untouched and unknown overrides, detect conflicting edits and confirm readback before
refreshing command suggestions. Role preview evaluates selected roles in a separate read-only
presentation context available to owners/administrators; it never replaces the account's
permissions or dispatches simulated actions.
Voice suppression, private-thread membership and member-specific restrictions are not simulated.
Community configuration preserves unknown server features and untouched channel fields. Ownership
transfer verifies a current human member and the current owner, requires the server name to be
confirmed, and succeeds only after an authoritative owner readback. If accepted delivery cannot
be confirmed, the outcome stays unknown and is not automatically repeated. A confirmed transfer
may legitimately remove the former owner's administration access.

Core administration snapshots and retained actions share an 8 MiB session budget in
addition to existing page limits; application pages contain at most 100 entries and stay below
1 MiB. Typed contracts live in the model's `channel_visibility`, `moderation`, `server_access`
and `command_permissions` modules; client-core owns scope validation and memory accounting,
transports perform bounded reads/writes, and UI panels consume those snapshots. At most two
replaceable administrative reads and one mutation run concurrently; closed views retire their
reads rather than polling. Live normal-account acceptance and native interaction checks require
separate owner-controlled evidence; synthetic coverage does not establish them.

## Own server profiles and account privacy settings — October 17, 2026

Own server-profile editing uses the **unofficial** normal-account forms of
`PATCH /guilds/{id}/members/@me` (nickname and avatar) and
`PATCH /guilds/{id}/profile/@me` (bio, pronouns, banner and theme colors), followed
by a full `GET /users/{own_id}/profile?guild_id={id}` read with mutual lists disabled.
The [Userdoccers guild resource](https://docs.discord.food/resources/guild#modify-current-guild-member)
and [profile metadata](https://docs.discord.food/resources/user#profile-metadata-object)
describe these fields; they do not establish normal-account acceptance. A nickname
edit requires the loaded Change Nickname permission. Setting rich overrides requires
full Nitro reported by the session; removing an old override does not create an
entitlement. Discord remains authoritative and can refuse any edit. Untouched fields
are omitted; null restores inheritance, while an explicit empty bio/pronoun remains
distinct. Member and profile writes are not transactional. Each changed route is
attempted once, failed drafts are retained, and any unconfirmed outcome requires an
explicit reload before another save. Replies from an old account, guild or request
are retired. Server profile snapshots are separate from the global profile editor.

Profile Privacy and Content Filters use the same version-guarded
`/users/@me/settings-proto/1` path as Messaging Permissions. The community-maintained
[PreloadedUserSettings schema](https://github.com/discord-userdoccers/discord-protos/blob/master/discord_protos/discord_users/v1/PreloadedUserSettings.proto)
defines profile visibility (privacy field 32), sensitive-media/gore/self-harm
preferences for servers, friend DMs and non-friend DMs (text fields 29, 32 and 35),
and keyword filters (text field 30). Unknown enums stay unknown. Writes preserve
unmodified nested fields, carry the required data version and require the response
to confirm the requested values. Unconfirmed changes are retained for review after
an explicit reload; reads never repeat a write. These configure service preferences;
Serein does not infer sensitive content or run a local text/media classifier. An
acknowledged preference alone does not prove service-side classification or rendering
enforcement for already-loaded messages. Age, rollout and account restrictions remain
service-controlled.

Private account overview reads `GET /users/@me`. Native connection management uses
`GET /users/@me/connections` and the unofficial normal-account PATCH/DELETE connection
routes described by [Userdoccers](https://docs.discord.food/resources/connected-accounts).
Only visibility, metadata visibility, activity sharing and friend sync are mutable;
unsupported or absent field values remain unavailable. Each connection mutation
rechecks the current list and verifies a fresh full list afterward. Disconnect requires
an explicit confirmation that linked integration membership may be lost. External
account IDs are encoded as one path segment. Third-party tokens and integrations
are discarded, and token-bearing response buffers are zeroized. Email and phone are
private own-account information and stay concealed until explicitly revealed.
Purchases, subscriptions, external OAuth linking, password/email changes, MFA setup
and locked-account recovery remain service-managed official-client operations.

Owned cosmetic equip uses bounded `GET /users/@me/collectibles-purchases?variants_return_style=1`
(the individual-products form), then freshly reads the scoped own profile before one explicit
PATCH. The [Userdoccers collectibles resource](https://docs.discord.food/resources/collectibles)
documents the inventory and expiry/subscription purchase types. Global and server avatar
decorations and nameplates use the own-account/member routes. Profile effects and frames
use the current [public client's profile update builder](https://github.com/Wumpus-Central/discord-mobile-datamining/blob/main/discord_app/modules/user_profile/ProfileUpdateRequestUtils.tsx)
`collectibles_sku_ids` replacement field; all bounded unknown equipped SKU entries are preserved.
Only affirmatively returned, unexpired owned items can be set. Subscription items and server
sets also require current full Nitro; removals remain available. Unsupported product kinds
are left untouched. Serein makes no purchase, gift, claim, entitlement or subscription write.
Every equip requires readback confirmation and failures preserve an explicit draft behind a
reload barrier. Current service acceptance is not established by these synthetic checks.

Global banner/theme and global/server display-name style editors use documented nullable
own-account/member/profile fields. The native style picker offers known nondeprecated fonts,
effects and one or two colors; unfamiliar current styles are preserved until explicitly changed.
Nitro settings are gated again at dispatch. Server theme viewing uses the server's own theme,
while the server editor retains the global fallback separately. Server accent color remains a
wire field but is not exposed as a cosmetic control because the documented client does not
respect it on guild profiles. Settings Escape handling now respects IME composition and nested
popups; new controls remain reachable in the existing narrow-window settings picker.
Native avatar/banner uploads still normalize the chosen image to a bounded PNG;
animated GIF/WebP uploads and the official client's image-crop workflow are not implemented.
The existing native name-style renderer draws static effects rather than Discord's animations.

All tests for these workflows are synthetic/offline. No owner account, external
connection, profile, credential or microphone has been used for validation. Live
normal-account interoperability and comparison with the supplied official desktop
build remain separate owner-controlled evidence gates.

## Native moderation and AutoMod — October 17, 2026

The desktop comparison reference supplied by the owner is Discord Stable 641004
(`c9ac31f`), host 1.0.9262 x64 (`94186`). No live official-client comparison was run here.
This feature branch extends fork `f9e39f7`; it does not alter parent #567/#620 or merge
other parity work. Existing roles, channel/category organization, permission synchronization,
invites and server lifecycle implementations remain in place.

Server settings now expose bans/unbans, timeouts/removal, kicks, optional audit reasons
and native AutoMod rules. Moderators can look up a specific member or choose one already
loaded in the conversation. Ban pages retain at most 100 entries/256 KiB. Self, owner,
equal/higher role, unknown hierarchy and administrator timeout cases fail closed.
Current permissions are rechecked before dispatch, and permission changes retire results.
An uncertain mutation requires a refresh and a deliberate decision; no destructive write
is automatically repeated. The default ban action deletes no message history.

The documented [moderation and AutoMod resource](https://docs.discord.com/developers/resources/auto-moderation)
routes use bounded reads and `X-Audit-Log-Reason`. AutoMod supports keyword, spam,
keyword preset, mention spam and member-profile triggers, with compatible block, alert,
timeout and member-interaction actions. A catalog retains at most 32 rules/512 KiB.
The editor keeps rejected drafts and offers explicit reapplication after refreshing.
Sparse edits preserve unrelated settings, unknown trigger/action metadata and concurrent
exemption additions. Rule/role/channel IDs are scoped to the current server. User accounts
do not reliably receive bot-only rule lifecycle dispatches; refresh is explicit.

These are native implementations with synthetic parser/reducer/HTTP/UI checks. Service
acceptance, entitlement differences and owner-controlled moderation interoperability
remain unverified. Scheduled events, Stage participation and administrative onboarding
are separate unfinished roadmap items; these settings do not enable Stage microphone use.

## Scheduled events and Stage channels — October 17, 2026

Native server events now provide a bounded list and detail view, canonical event links,
create/edit/delete, start/end/cancel and explicit Interested/Not interested actions. The
editor accepts RFC3339 times with an explicit UTC offset and previews the local time;
recurrence uses the UTC weekday and clock time, so local daylight saving changes do
not silently shift the recurrence rule. Daily, weekday, weekly, fortnightly, monthly
nth-weekday and yearly rules use the current documented writable subset. Existing
service-only end/count/year-day rules survive unrelated sparse edits, with occurrence
controls unavailable for rules the client cannot expand safely. The current series
exposes up to eight upcoming occurrences plus its retained exceptions: edit, cancel,
restore the series schedule, RSVP/skip and reset the individual RSVP to the series.
Unknown subscription state is displayed as unavailable; confirming one RSVP never
turns the other unknown responses into “not interested.”

Schemas, permission bits and event/Stage resource routes were checked against the
[official scheduled-event resource](https://github.com/discord/discord-api-docs/blob/f7416a2cf165455f72a31f5ea4e671a4f047f2f7/developers/resources/guild-scheduled-event.mdx),
[Stage resource](https://github.com/discord/discord-api-docs/blob/f7416a2cf165455f72a31f5ea4e671a4f047f2f7/developers/resources/stage-instance.mdx)
and [voice connection documentation](https://github.com/discord/discord-api-docs/blob/f7416a2cf165455f72a31f5ea4e671a4f047f2f7/developers/topics/voice-connections.mdx).
Normal-account subscription reads/writes and recurring exception routes are **unofficial**,
checked against the maintained [Userdoccers event reference at `01624d9`](https://github.com/discord-userdoccers/discord-userdoccers/blob/01624d9b7a75cd8b1e838e009b434d108d9fcdff/pages/resources/guild-scheduled-event.mdx):
`GET /users/@me/scheduled-events?guild_ids=...`, event `users/@me`, exception
`POST/PATCH/DELETE /guilds/{guild}/scheduled-events/{event}/exceptions[/{occurrence}]`
and `PUT/DELETE /guilds/{guild}/scheduled-events/{event}/{occurrence}/users/@me`.
Occurrences are identified by the original UTC start snowflake, bounded to the loaded
series. Restoring an exception deliberately removes that occurrence override; writes
are never repeated automatically after an uncertain outcome.

Stage channels have native audience/speaker state, request/cancel request, invitation
accept/decline, moderator invitations/demotion, topic editing and confirmed Stage end.
Audience joins disable microphone device opening and transmission. A service speaker
state or invitation cannot grant local microphone consent. Only an explicit
Accept invitation/Become speaker action grants consent; permission loss, demotion,
Stage end, a failed acceptance or a fresh media rejoin revokes it. Authorization
revisions retire queued PCM even when permission loss and restoration happen between
UI polls. Moderation and replies are scoped to the current account, guild, channel,
call request and current permissions.

The immutable media mode comes from the authenticated current channel type. The
[official DAVE scope](https://github.com/discord/dave-protocol/blob/1b2b706ad4a562ede5c2ef901f9d77f6dbe79446/protocol.md#introduction)
excludes Stage channels. Stage therefore requires negotiated
`aead_xchacha20_poly1305_rtpsize` transport encryption and DAVE version 0; ordinary
voice, DMs and separate Go Live transports retain mandatory DAVE version 1. There is
no service-selected downgrade between these modes. Stage audio can be processed by
Discord and has no end-to-end privacy code. A Stage signaling outage uses the existing
bounded fresh-media rejoin path, returning to audience consent. A Stage roster can
contain 4,096 entries / 1 MiB, while at most 63 remote authenticated audio sources
receive decoder/jitter working sets; audience announcements do not allocate an audio
resource per audience member. Member controls page 64 rows at a time.

One community worker and one result lane are retained. Event lists are capped at
100 events / 768 KiB with a 1 MiB response, 64 exceptions per series and 400 subscription
records. Access/session changes cancel outstanding work, late responses cannot reopen
closed views or restore revoked access, and uncertain writes require an explicit
appropriate refresh before another write. Drafts stay in session RAM through failures.
No automatic RSVP, request to speak, notification, Stage creation or destructive retry
runs during browsing.

Evidence is synthetic protocol/model/reducer/native-egui tests and loopback HTTP/voice
fixtures. These fixtures do **not** establish normal-account route acceptance, real
Stage server negotiation, physical microphone/speaker behavior or agreement with the
owner-supplied Stable 641004 (`c9ac31f`), host 1.0.9262 x64 (`94186`) client. No live
Discord/account/microphone traffic was used. These current capabilities supersede the
historical “Stage unsupported” statements below; historical measurements remain historical.



## Desktop parity inventory — October 17, 2026

Scope: the full desktop roadmap, including server administration. The implementation
baseline is fork `f9e39f7`; personal-controls work is separately reviewed in
[MCShotty/Serein#8](https://github.com/MCShotty/Serein/pull/8), and the FFmpeg media
follow-up is [#7](https://github.com/MCShotty/Serein/pull/7). Neither open PR is assumed
merged. This inventory supersedes older scope statements below without rewriting
historical evidence. **Native implementation and live Discord acceptance are separate.**
The named sibling branches are separate implementation slices, not changes already merged
into the fork. Each receives its own checks and PR; #567 and #620 remain untouched.

The owner supplied the official-client reference: **Stable 641004 (`c9ac31f`), host
1.0.9262 x64 (`94186`)**, on October 17. The public Stable metadata/download routes
returned HTTP 403 here; no installed official desktop client or owner-controlled
session is available in this workspace. The owner-supplied build is recorded, but
entitlement-specific UI, side-by-side native behavior and Windows/macOS performance
remain **not verified here**. Public Discord resource/help references establish
workflow/schema evidence, not a completed client comparison.

| Roadmap workflow | Current implementation / remaining gap | Source or next verification |
| --- | --- | --- |
| Unified Inbox | Native unread, recent-mention and bookmark tabs in this branch; bounded filters, explicit dismissal and jumps | `client-core/inbox.rs`, `ui/inbox.rs`; normal-account acceptance unverified |
| Bookmarks/reminders | Explicit list/save/remove and reminder deadlines; service determines account availability | [Saved-message account routes](https://docs.discord.food/resources/user#list-saved-messages); restricted/unofficial |
| Mutes, notification inheritance | Channel/server/DM support; timed choices and category inheritance extended in #8 | `client-core/notifications.rs`; cross-client expiry remains owner-gated |
| Read actions, folders, navigation | Existing per-channel/server/all read actions and folders; category/folder scope and unread shortcuts in #8 | `read_state.rs`, `ui/guild_folders.rs`, `ui/keybinds.rs` |
| Blocked/ignored users | Native relationship actions and collapsed timeline, reply, search/pin, Inbox and forward previews; explicit per-message reveal | `user_actions.rs`, `ui/restricted.rs`; current-client comparison remains owner-gated |
| Message requests | Native accept/decline/spam request flows | `user_actions.rs`; owner-controlled cross-client reconciliation pending |
| Search | Native filters, pagination and permission-checked message navigation | `search.rs`, `ui/search_navigation_tests.rs`; indexing/permission races require owner check |
| Reply/edit/send/uploads | Existing bounded drafts, nonce correlation and explicit uncertain-write handling | `message_actions.rs`, `replies.rs`, `uploads.rs`; interaction audit remains partial |
| Reactions, polls, pins, forwarding | Native workflows already exist | `reactions.rs`, `polls.rs`, `forwarding.rs`, `message_actions.rs`; official-client comparison pending |
| Slash command basics | Native schema-driven commands and modal lifecycle | `application_commands.rs`, `interactions.rs` |
| Command autocomplete/files/context/account install | Native autocomplete, attachment arguments, user/message context commands and eligible account discovery in [fork #10](https://github.com/MCShotty/Serein/pull/10) | Independent branch; signed file staging and explicit failure recovery covered offline; live acceptance unverified |
| Threads/forums | Native public/private creation, archives, join/leave, invitations, owner/moderator removals, post tags and management | `channel_actions.rs`, `ui/thread_members.rs`, `post_menu.rs`; service membership and private archive behavior remain live-unverified |
| Announcement following/publishing | Native confirmation for publishing and explicit destination selection for following; existing integration tools manage/unfollow feeds | `channel_actions.rs`, `ui/announcements.rs`; normal-user acceptance unverified |
| Group-DM membership | Native group creation, friend invitations, owner transfer, confirmed removals, leave and group editing; explicit refresh after uncertain membership writes | `group_actions.rs`, `ui/group_members.rs`; owner-controlled reconciliation pending |
| Server settings and lifecycle | Native settings and confirmed deletion; permissions and draft checks exist | `server_settings.rs`, `server_actions.rs`; owner/admin/member matrix needs expansion |
| Roles and hierarchy | Native create/edit/delete, ordering, assignment and hierarchy gates | `server_roles.rs`; no replacement implementation needed |
| Channel/category organization | Native creation, movement, reordering and permission synchronization | `channel_actions.rs`, `ui/channel_menu.rs` |
| Member moderation | Ban/unban/timeout/reason tools in [fork #11](https://github.com/MCShotty/Serein/pull/11), extending existing moderation; bans require a current member lookup | Owner/admin/moderator/member and stale-permission regressions; independent branch under validation |
| Invites/integrations/audit log | Native permission-scoped settings with bounded reads and explicit writes | `server_invites.rs`, `server_integrations.rs`, `server_audit_log.rs` |
| AutoMod | Native bounded rule editor in [fork #11](https://github.com/MCShotty/Serein/pull/11), with sparse edits and explicit conflict recovery | [Official resource](https://docs.discord.com/developers/resources/auto-moderation); owner-controlled service validation remains |
| Scheduled events | Native browse/create/edit/RSVP/cancel and recurring occurrence controls in [fork #15](https://github.com/MCShotty/Serein/pull/15) | `community.rs`; synthetic parser/reducer/HTTP/UI checks; owner service checks pending |
| Stage participation/moderation | Native audience/speaker, request/invite, topic and moderation controls in [fork #15](https://github.com/MCShotty/Serein/pull/15); microphone sending requires explicit local consent | `client-core/voice.rs`, `community.rs`; ordinary voice retains mandatory DAVE; native/service checks pending |
| Onboarding, Channels & Roles | Native participant answers plus administrative default-channel and question editing in [fork #14](https://github.com/MCShotty/Serein/pull/14) | `community_admin.rs`; hierarchy, stale responses and deleted-role/channel regressions |
| Rules screening / Server Guide | Native screening/Guide administration and member Guide tasks, resources and progress in [fork #14](https://github.com/MCShotty/Serein/pull/14) | `community_admin.rs`, `member_guide.rs`; normal-account routes unofficial and owner-unverified |
| Encoding/capture/HDR | Existing video work retained; #567 scope and path limit unchanged | `docs/voice.md`, video evidence; physical display/official-client gates remain |
| Soundboard | Playback, management and trimmed uploads integrated in [fork #12](https://github.com/MCShotty/Serein/pull/12) from upstream #579, with bounded cancellation and call-scoped responses | Depends on #7; independent branch under validation; live acceptance unverified |
| Voice messages | Opt-in native Record/Stop/Listen/Send and pitch-preserving speed/volume controls in [fork #12](https://github.com/MCShotty/Serein/pull/12) | Adapted from #566/#662; bounded capture lease, privacy generations and cancellation; physical audio unverified |
| Video/audio playback | Existing native playback; FFmpeg preference is separate #7 | `apps/desktop/src/audio.rs`, `video.rs`; media performance unmeasured on Windows/macOS |
| Embedded video | Native Discord-proxied video playback in #12; browser-only provider players use confirmed external links | Adapted from #439; anonymous bounded reader, explicit Play and existing media privacy gates |
| Device routing / screen audio | Independent media output in #12 alongside selected voice input/output; platform screen-audio limits remain | Adapted from #533; Linux/Windows mixes eligible apps, macOS application-filtered; physical routing pending |
| Call layout / adaptation | Scrollable focused call strip, existing bitrate adaptation and bounded receive-clock scheduling in #12 | `ui/voice.rs`, `stream_sync.rs`; automatic resolution/FPS and simulcast selection remain absent |
| Profiles / server profiles | Native global/server profile, theme and name-style editing in [fork #13](https://github.com/MCShotty/Serein/pull/13) | Animated uploads, crop workflow and animated name effects remain gaps; live entitlement checks pending |
| Cosmetics / connections | Owned cosmetic equip and connection management in #13; purchases/linking remain official handoffs | Only fresh owned entitlements authorize equip; handoffs do not count as native commerce |
| Presence / account switching | Native presence and saved-account controls | `presence.rs`, desktop credentials worker; failure/reconnect tests exist |
| Privacy / notification overview | Native profile visibility, content-filter preferences and private account overview in #13; notifications extended in #8 | Service classification remains authoritative; preference acknowledgement does not prove enforcement |
| Accessibility / desktop layout | Native focus, scaling, reduced motion and IME-safe settings; added accessibility controls in #13 | Full assistive-technology, RTL and physical narrow-window/platform comparison remains unverified |
| Activities | Games and Activities excluded by the owner | No embedded-app webview or game integration added |
| Overlay / discovery / commerce | Capability assessment required; external handoffs do not count as native parity | No purchase, subscription or overlay support is implied |

Integration candidates were checked against their remote heads on October 17: recent
mentions #599 (`d253542`), soundboard #579 (`aa9c1dc`), playback controls #662
(`d2d256c`), recording #566, media routing #533 and embedded video #439. Open upstream
work remains unverified until integrated and tested with this fork's media/privacy paths.
The Inbox reuses the original recent-mentions implementation from #599 (`11b5144`),
with current lifecycle handling retained. Imported code keeps its original attribution.
A later read-only refresh found parent `main` at `a5e935d` (folder-read actions #672
and call mention/unread rendering #681). Those updates are recorded as upstream work;
this feature branch still targets the unchanged fork baseline.

Acceptance gates apply to every remaining implementation: bounded ownership and wire
input; stale-response/session rejection; current permissions before writes and display;
no automatic repeat of uncertain writes; preserved drafts; cancellation/reconnect and
resource release tests. Each feature PR must carry focused/workspace checks, synthetic
native evidence where available, current-nightly and fork release comparisons, native
package CI and explicit owner-controlled interoperability limitations. This table is an
inventory, **not a claim that the remaining gaps are completed**.

## Unified Inbox lifecycle

Inbox Gateway mutation notices mark the retained snapshot stale instead of retiring its
REST correlation. Reads cancel on navigation; an accepted Save, Unsave or Dismiss stays
bounded to one write until its confirmation settles. Closing the view retires its cached
bodies, so a late confirmation reports its outcome without restoring closed content.

## Desktop messaging follow-up — October 17, 2026

Known blocked and ignored authors now have collapsed bodies in the timeline (including
thread starters), search and pins, Inbox mentions/bookmarks, and forward previews.
Loaded replies use a restricted-message label in place of the author's preview. Show/Hide
is explicit for each body; showing one does not reveal another preview. Hidden rows do
not call author artwork, markdown, component, embed, attachment or media renderers.
Changing restrictions, account, source body or view revokes the bounded session-only
reveal. Lightboxes/fullscreen, active playback and reaction overlays close when their
source becomes hidden. Forwarding requires revealing the selected source first.
Known restricted authors cannot enqueue alerts or deliver an already queued preview;
their new locally observed mentions do not add to the mention badge. Other members of
a group DM can still notify when a recipient is restricted. Service read counts remain
authoritative. Existing cached bodies are retained under the existing history limits.

Private-thread creation uses the documented channel thread route, type 12 and explicit
`invitable`; public/private creation requires View Channel and the corresponding
Create Threads permission. The creation result records its exact requested/returned ID,
and a Gateway-first creation cannot replace a newer title with the REST snapshot.
The native member manager uses documented thread member add/remove routes. Invitations
accept a friend or an explicitly entered server-member ID; private invitations require
current membership with `invitable`, or Manage Threads. Removal requires Manage Threads
or ownership of a private thread, and a current loaded member. Archived threads must be
reopened explicitly. The existing
public/private/joined-private archive permission, paging and transient admission paths
are retained; no archive read joins or reopens a thread.

Existing forum posts now have a native tag editor. At most five known tags may be
selected; a required tag set cannot be empty, and adding **or removing** a moderated tag
requires Manage Threads. A changed service tag selection requires explicit reload before
saving. The editor preserves its draft after a rejected or uncertain write.

Publishing uses documented `POST /channels/{channel}/messages/{message}/crosspost` and
checks Send Messages (plus Manage Messages for another author's message). Confirmation
requires the exact channel/message and the published flag; it never replaces newer text.
Follow selects one known text/announcement destination with current Manage Webhooks,
then uses documented `POST /channels/{source}/followers` with `webhook_channel_id`.
Existing destination-server integration tools list and remove the resulting feed.

Group member management reads one bounded owner/recipient snapshot before mutation.
New Group DM in Friends selects 2–9 known friends and creates one group with
`POST /users/@me/channels` and a `recipients` array. The reducer validates the exact
recipient set and owner, admits the returned ID, and leaves selection to the still-current
dialog. Make Owner confirms the loss of owner controls before sending the observed
`PATCH /channels/{channel}` field `owner`, then requires the matching owner snapshot.
Friend-only invitations use `PUT /channels/{channel}/recipients/{user}`; owner-only
confirmed removals use DELETE on that route. These normal-user writes are **unofficial**:
Discord's public group-recipient route describes an OAuth flow instead. Wire evidence
was inspected in the pinned [discord.py-self HTTP adapter](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/http.py)
(`start_group`, `add_group_recipient`, `remove_group_recipient`, `publish_message`,
`follow_webhook`) and [GroupChannel.edit](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/channel.py);
no implementation source was copied. Service recipient/channel events invalidate cached
ownership and reject stale reads. An uncertain membership write requires an explicit
refresh before another change. No mutation is repeated automatically, and normal composer
drafts remain intact. Poll creation now waits for the connected session, like poll votes.

The documented thread member routes, private-creator removal exception, announcement
follow result's source `channel_id`, and crosspost permissions were checked against
[Discord's channel resource](https://github.com/discord/discord-api-docs/blob/f7416a2cf165455f72a31f5ea4e671a4f047f2f7/developers/resources/channel.mdx),
[message resource](https://github.com/discord/discord-api-docs/blob/f7416a2cf165455f72a31f5ea4e671a4f047f2f7/developers/resources/message.mdx),
and [thread permissions](https://github.com/discord/discord-api-docs/blob/f7416a2cf165455f72a31f5ea4e671a4f047f2f7/developers/topics/threads.mdx).
Focused reducer, headless egui and local HTTP checks are synthetic/offline evidence.
They do not verify normal-account Discord acceptance, private-thread visibility across
accounts, server ownership transfer, account entitlements, or parity with the owner's
Stable 641004 (`c9ac31f`), host 1.0.9262 x64 (`94186`) client. The broader reply/edit/upload,
reaction, poll, request, search and navigation audit remains partial; this follow-up does
not mark all desktop messaging parity complete.

## Game mentions (October 14, 2026)

The native composer offers `@game`, a searchable game catalog and application references
using `<@$APPLICATION_ID>`. This syntax is **unofficial**; service acceptance and rendering
by another Discord account remain **unverified**. Serein treats these as game references,
never user or role IDs, and does not add them to the user/role notification allowlists.
Names and icons come from the existing credential-free `/applications/detectable` route,
including entries without detectable executables. The request runs lazily on first game
search or a loaded timeline reference, independently of local activity sharing. Catalog
failures require an explicit retry and respect the request's rate-limit cooldown. The
loader scans already-loaded message text in memory to decide when to request the catalog;
no message text is sent by that lookup. It does not access game processes, account settings,
credentials or the microphone. Synthetic tests verify insertion, parsing and layout
semantics only.

## Profile badges, badge directory and badge settings — October 12, 2026

Newer badges include Account Age, Streaming, Game Time and Game Variety. They are tiered
badges with rarity and progress. The badge directory and badge hiding/reordering come from
a newer normal-client badge system. All of it is **unofficial** and absent from Discord's
public documentation and from Userdoccers as of this date. The routes, fields and rules
below were taken from the decompiled
[official mobile client](https://github.com/Wumpus-Central/discord-mobile-datamining)
(`modules/badges`, `BadgeId`, `LegacyBadgeIdMap`, `PendingBadgeSettings`). Live delivery
to normal accounts is unverified. The client lists the four tiered badges as beta
(`BETA_BADGE_IDS`). Badge management and directory updates sit behind the
`2026-08-badge-management` and `2026-10-badge-directory-updates` experiments. On
October 13, the owner's account showed no tiered badges in the official client either,
so they appear to be rolled out to some accounts only.

- **Catalog.** `GET /users/{id}/badges` returns `badges[]` with numeric `badge_id`s and
  these fields:
  - `owned`, `hidden` and `is_earnable`
  - `name`, `description` and `info_label`
  - `simple_icon_url`, `simple_icon_raster_url` and `complex_icon_static_url`
  - `rarity` (1 common, 2 rare, 3 epic, 5 mythic)
  - `tiers[]` with keys, names, art, `milestone_text` and `requirements[].threshold`
  - `current_tier` and `next_tier`
  - `progress[]` with current, threshold and helper text
  - `obtained_at` and `tier_obtained_at`

  Serein reads it beside every opened profile, as the official client does for badge
  art. The account's own catalog uses `/users/@me/badges`, as the clients do.
  On October 12, a live owner test returned `404 Not Found` (`code` 0) for both
  routes while Serein identified as web build 328,246. Unauthenticated requests to the
  same routes return 401, so the routes exist. After the shared client fingerprint was
  updated to the current web build, 634,304 (from the public `discord.com/app`
  `GLOBAL_ENV`), the catalog loaded with 22 entries. This suggests a minimum-build gate;
  Discord does not document one. Catalog PNG art can be large and is decoded within a
  4 MiB / 4096 px budget, then scaled to 256 px. The read is optional, bounded to 256 KiB and shares the profile's two-second
  enrichment deadline. Failures leave the profile's own badge list. The retained catalog
  is capped at 40 badges and 48 KiB, and at 12 tiers per badge, keeping a window around
  the current and next tiers. Over budget, Serein drops locked badges' other tiers, then
  tier art, then owned badges' other tiers, then locked badges. Owned badges go last.
  SVG icons are skipped in favor of raster art. Oversized optional catalog responses leave
  the loaded profile available; authentication failures still end the session. Limited
  catalogs remain read-only so a truncated owned list cannot replace account settings.
- **Profile row.** Profile badges keep their string ids, mapped to catalog numbers with the
  client's table (for example `premium_tenure_12_month_v2` → 1, `account_age` → 18).
  Owned, visible catalog badges missing from the profile response are added with catalog
  art. Badges the catalog marks hidden are removed. The row follows the catalog order and
  shows at most six badges, like the official client's `MAX_DISPLAYED_PROFILE_BADGES`.
  This also covers profile responses that omit the new badges, as observed for the
  owner's own profile.
  Non-hash `/badge-icons/` names (slugs up to 64 characters) are now accepted.
  Hovering a badge shows its art, name, tier and `info_label`.
- **Directory.** It shows owned badges, the account's earnable badges, art, date unlocked,
  rarity, progress toward the next tier and every tier. Other users' directories show only
  their owned badges.
- **Settings.** `PATCH /users/@me/badges/settings` takes `{"display_order": [ids],
  "hidden_badges": [ids]}`. Staff (22) is pinned first and is never sent; Nitro tenure
  (1) can be reordered but not hidden. Clicking one's own badges opens the customize menu.
  There, clicking a badge hides or shows it and dragging reorders it. Nothing is written
  until Save, a single request without retries. On success the cached catalog and badge
  row update in place. Demo sessions confirm locally. Viewing another user's progress
  (`with_progress`) and per-badge summary reads are not used.

## Profile cosmetics — October 11, 2026

Serein shows equipped avatar decorations, nameplates, profile effects and profile frames.
All four are unofficial normal-client behavior: none appears in Discord's public bot
documentation. Shapes come from the Userdoccers [user](https://docs.discord.food/resources/user#avatar-decoration-data-object),
[collectibles](https://docs.discord.food/resources/collectibles) and
[CDN](https://docs.discord.food/reference#cdn-endpoints) references. Layout,
palette and timing rules were checked against the decompiled
[official mobile client](https://github.com/Wumpus-Central/discord-mobile-datamining)
(`modules/collectibles`) and a public [catalog snapshot](https://github.com/aamiaa/discord-api-diff)
on October 11. Live delivery to normal accounts is unverified; offline fixtures do not
establish it.

- **Avatar decorations:** `avatar_decoration_data` on user objects, with guild member
  values overriding it in that server (member rows, message members and server profiles).
  Artwork is `/avatar-decoration-presets/{asset}.png` with `passthrough=true` for `a_`
  presets. It is drawn at 1.2 times avatars of 30 points or larger, and animates while
  hovered or on an open profile, like animated avatars.
- **Nameplates:** `collectibles.nameplate`, also overridable per member. The still is
  `/assets/collectibles/{asset}static.png`. The animated image comes from the
  mobile-client route `/media/v1/collectibles-shop/{sku}/animated`, which is fetched only
  while hovered. The official WebM is not decoded. Rows draw the right-aligned 16:3
  artwork over a left-to-right gradient of the palette color from 0% to 30% opacity.
  Dark and light colors are those observed in the client. Offline member rows and muted
  DMs show only a 10% image until hovered. Nameplates appear in server member lists and
  the one-to-one DM list.
- **Name styles (Nitro display-name styles):** `display_name_styles` on user and guild
  member objects (`font_id`, `effect_id`, `colors`), with a member's value overriding the
  user's in that server. The shape and the font and effect ids come from the official web
  client's public JavaScript (October 15, 2026); this is unofficial and live delivery is
  unverified. One to four 0xRRGGBB colours are accepted; anything else drops the style.
  Gradient effects spread every colour evenly across the name.
  Effects 1–8 (solid, gradient, neon, toon, pop, glow, prism, gummy) are drawn as still
  approximations: gradients across the name, and halo, outline or shadow layers behind it.
  Discord's animations are not reproduced. Fonts 1–16 except 7 use bundled OFL faces with
  the same family names (see `assets/README.md`); Néo-Castel (7), Hexagon (17), Default (11)
  and unknown ids keep the interface font. The bundled files may differ from the builds
  Discord serves. Chat, member and search rows show the colours and effect while the row is
  hovered; profile cards and the account panel always show them.
- **Profile effects and frames:** profile responses list them in `user_profile` and
  `guild_member_profile` `collectibles` (type 1 = effect, 3 = frame). The older
  `profile_effect.id` is accepted as the effect SKU, and a server profile's own values
  take precedence. Details come from `GET /collectibles-products/{sku}`. This is the
  same optional read the clients make. It is bounded to 256 KiB, has a two-second
  deadline shared with board enrichment, and is cached for at most 64 SKUs per session.
  Only an expired or challenged session fails the profile. Timeouts, oversized or refused
  products and other errors leave it without that cosmetic. Oversized and refused SKUs are
  remembered for the session so they are not read again. Effects and frames hidden in
  Settings skip this read.
- **Effects** play layers from `/assets/content/{hash}` or
  `/media/v1/collectibles-shop/{hash}`. They use the official 450 x 880 canvas scaled to
  the card, z-order, `start`, `duration`, `loop` and `loopDelay`, and a 500 ms intro
  delay. The timeline starts once every layer has loaded or failed, so a short opening
  layer is not skipped while it downloads. It restarts when the card reopens, and one
  shared random index picks among `randomizedSources`. With animation disabled or the
  window unfocused, the card shows `staticFrameSrc`, falling back to the first frame of `reducedMotionSrc`.
- **Frames** fetch static layers from `/media/v1/collectibles-shop/{sku}/{layer}/static`
  and scale them from `inner_width` (default 1200). Staples pin to the top or bottom and
  overflow by `overflow_top`/`overflow_bottom`/`overflow_horizontal`. Rails align inside
  the card height and are hidden when responsive and the card is at least 0.625 as wide
  as it is tall. Borders tile down the card. Back layers sit behind the card and front
  layers over it. The popout keeps the overflow on screen.

Unknown palettes, layer kinds, anchors, malformed values and expired items are dropped
without rejecting the user, member row or profile. Owned cosmetic equip is documented in the October 17 section above. All
five are opt-in: they stay hidden, unrequested and without catalog reads until enabled in
Settings › Appearance › Profile cosmetics. This is a local display choice; nothing is sent
to Discord.

## Guild voice moderation — October 8, 2026

Server mute/unmute and voice movement/disconnection use the documented
[Modify Guild Member](https://docs.discord.com/developers/resources/guild#modify-guild-member)
PATCH route with only `mute` or `channel_id` (null disconnects). A bounded 64 KiB
member response must identify the requested user; mute replies must echo the requested
value. Move completion is reflected by subsequent Gateway voice state, because the
member response does not identify a voice channel. Source roster membership and
loaded source/destination permissions are rechecked before dispatch. Existing REST
deadlines, write ambiguity handling and the single administration request apply.
Normal-account acceptance and live moderation have not been verified.

## Gateway recovery after failed resumes — October 8, 2026

An established connection first attempts to resume using the service-supplied
resume address. After three consecutive failed resume attempts, it discards the
old session, sequence and address, then identifies through the original validated
gateway URL. A resumable Invalid Session received before RESUMED counts as a
failed attempt even though it uses its own retry delay. Successful READY/RESUMED
resets the counter; explicit recovery cancellation does not consume it. Existing
connection deadlines, capped backoff and the six-attempt initial-login limit remain
unchanged.

Close codes received before HELLO follow the same policy as established sockets:
expired credentials stop, invalid or expired sessions require a fresh Identify,
and resumable closes retain their session within the failure budget. This follows
Discord's documented [disconnect fallback](https://docs.discord.com/developers/events/gateway#handling-a-disconnect)
and [Gateway close codes](https://docs.discord.com/developers/topics/opcodes-and-status-codes#gateway-gateway-close-event-codes).
Normal-account use remains unofficial. Local WebSocket regressions exercise the
recovery policy; they do not establish the cause of issue #577's macOS Wi-Fi
failure or prove live Discord recovery.
## Mentions inbox — October 8, 2026

Inbox reads one on-demand page from the unofficial
[List Recent Mentions](https://docs.discord.food/resources/user#list-recent-mentions)
normal-account route. Requests explicitly include role and everyone/here mentions,
use a 25-message limit and paginate with `before`. The service documents a seven-day
window; Serein displays only returned messages in known accessible conversations and
does not claim an exhaustive historical archive. One page is retained in session memory
(25 messages, at most 1 MiB; wire replies capped at 2 MiB). Older pages replace the
current page. Refresh/retry are explicit; there is no background polling or persistence.
Opening a result uses the existing permission-checked message-history navigation.
Closing, account changes, disconnects and relevant content/permission invalidation clear
the snapshot. Source research and local HTTP/UI fixtures do not prove live acceptance.

## Profile boards — October 7, 2026

The full profile reads game widgets from the existing on-demand profile response,
then optionally resolves their names and artwork with at most two bounded
`GET /games` requests using repeated `game_ids` query parameters. Current `media.cover`
and `media.icon` hash/Discord-proxied URL assets are supported alongside legacy
image hashes. Arbitrary external artwork URLs are not fetched. Favorite Game, Games in Rotation, Games I've Played and
Want to Play display service-supplied entries, comments and tags. Missing or
restricted boards remain unavailable; failed metadata lookups retain the game ID
without substituting another game's title or artwork. Application-provided stats
widgets and board editing are not supported. The Activity tab uses current
presence, including while profile metadata is loading; an empty presence snapshot shows “No activity shared.” Selected-user historical activity is not yet loaded. Optional board and collectible enrichment run together, each with a 600 ms deadline, so a slow read cannot hold the card; without it the card still opens complete, just without those names or artwork.

The primary reverse-engineering references are the Userdoccers
[profile response](https://docs.discord.food/resources/user#get-user-profile),
[game widget schema](https://docs.discord.food/resources/widgets#game-widget-object),
[game metadata lookup](https://docs.discord.food/resources/game#list-games), and
[application cover/icon CDN paths](https://docs.discord.food/reference#cdn-endpoints).
The current request serialization and `media` shape were also checked against the
[public web client](https://discord.com/assets/web.90d3ab34abfe98da.js), build 630444,
on October 7; older community game documentation still describes the legacy fields.
These are unofficial normal-account interfaces, not Discord's public bot API.
Normal-account acceptance and cover availability remain unverified; synthetic
fixtures and offline checks are not service-compatibility evidence.

## Soundboard — October 6, 2026

Server voice channels can list, play and hear soundboard sounds. The catalog uses the
documented [`GET /soundboard-default-sounds` and `GET /guilds/{guild.id}/soundboard-sounds`](https://docs.discord.com/developers/resources/soundboard)
routes, and playing uses `POST /channels/{channel.id}/send-soundboard-sound` with `sound_id`
and, only for another server's sound, `source_guild_id`. Sounds from other participants arrive
as the documented [Voice Channel Effect Send](https://docs.discord.com/developers/events/gateway-events#voice-channel-effect-send)
Gateway event (`sound_id`, `sound_volume`); the audio itself is not relayed over voice, so each
client fetches `https://cdn.discordapp.com/soundboard-sounds/{sound_id}` without credentials
and plays it locally. Guild Soundboard Sound Create/Update/Delete and Sounds Update only mark
the cached server catalog stale. Request Soundboard Sounds (opcode 31) is not sent.

Sound identifiers are accepted as snowflake strings or integers, and a `guild_id` of `"0"` is
treated as a default sound; the integer and zero forms are tolerated defensively and are not
documented guarantees. The unofficial [user-account reference](https://docs.discord.food/resources/soundboard)
describes the same routes and adds that the sender must be connected and not server muted,
deafened or suppressed; [discord.py-self](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py)
has no soundboard implementation to compare against (checked October 6, 2026). Whether the
effect event is also delivered to the sender is not documented: Serein ignores effects from
its own user and plays its own sound when the request succeeds, so a sound should play once
either way. Normal-account acceptance, delivery of the effect event and audible playback are
**live-unverified**; no account, call or audio device was used. See
[workflow and limits](voice.md#soundboard).

Server Settings > Soundboard manages a server's sounds with the documented
`POST /guilds/{guild.id}/soundboard-sounds` (`name`, a base64 `sound` data URI, `volume`,
optional `emoji_name`), `PATCH` and `DELETE /guilds/{guild.id}/soundboard-sounds/{sound.id}`.
The list reads the same catalog route, whose `user` field is present only for members who
may manage expressions. Slot limits are not part of that response: they come from
`premium_tier` and the `MORE_SOUNDBOARD` feature of `GET /guilds/{guild.id}`, using Discord's
published boost perks (8/24/36/48, 96 with the feature), and are omitted when that read
fails. Trimmed uploads are sent as `audio/ogg` containing Opus; the documentation says
"MP3 or Ogg" without naming the Ogg codec, so acceptance of Opus is an assumption and
**live-unverified**. See [management workflow](voice.md#managing-server-sounds).

## GIF favorite synchronization — October 2, 2026

Opening the GIF picker lazily reads account favorites through the unofficial
`GET /users/@me/settings-proto/2` route. After a successful read, explicit star
changes save one URL using a fresh read and `required_data_version`. Failed,
conflicting or unconfirmed writes are never automatically retried; Refresh from
Discord restores synchronization. Existing local favorites remain a bounded
fallback and are never uploaded in bulk. Previously loaded remote favorites that
disappear on refresh are removed locally. The native list displays at most 100
entries; writes preserve unseen entries rather than replacing the list with that
projection.

The [primary FrecencyUserSettings schema](https://github.com/dolfies/discord-protos/blob/f5c06d6f5764a66302f7eede675fd703c31be08a/discord_protos/discord_users/v1/FrecencyUserSettings.proto)
establishes the URL-keyed favorite map, source, format, dimensions and order.
The [maintainer's endpoint reference](https://docs.discord.food/resources/user-settings-proto#modify-user-settings-proto)
specifies replacement of a supplied top-level subtree, so the complete raw
favorites subtree, tooltip setting and unknown entry fields survive each edit.
Returned saves must confirm the changed entry and unchanged other entries.
Service normalization may conservatively report an unconfirmed save; inspect via
Refresh before a deliberate new change.

Supported Tenor/KLIPY image sources reuse native previews. Video-only sources are
retained verbatim and show a placeholder, without converting or downloading an
invented image URL. Unsupported hosts/formats remain untouched on the server and
are omitted from the native projection. The native projection shows highest wire order first with URL tie-breaking;
a new entry receives the highest order plus one. Distinct local fallback entries
reserve space before remote entries when the combined list exceeds 100. A late
initial cache is skipped after an explicit star or removal-history overflow so it
cannot undo newer actions. Same-account reconnect clears interrupted requests and
requires a fresh read before another explicit server write. Cross-client ordering and normal-account service acceptance remain
unverified. No live account or service write was used for verification.

## Custom Rich Presence - September 28, 2026

The **Custom Rich Presence** catalog plugin adds an independent native implementation of
[Vencord CustomRPC](https://vencord.dev/plugins/CustomRPC). Enable it in Extensions, open its
editor, enter a nonzero Discord application ID and name, then Preview or Apply. Playing,
Streaming, Listening, Watching and Competing support details/state links, two artwork slots,
two link buttons, party size and elapsed/local-midnight/custom UTC timers. A draft preview
uses placeholder artwork; Apply resolves registered keys and public HTTPS images using the
existing Discord asset service. The editor reports sharing and resolution errors.

Saved settings resume on plugin activation. Stop preserves settings but disables resume;
disabling removes the contribution and extension data. Custom presence overrides detected
activity while sharing is enabled, and Stop restores detected activity. Invisible suppresses
publication. LocalDay refreshes once a minute and fails explicitly if native timezone lookup
is unavailable. The plugin requires an application ID even for text-only presence.

Both author configuration and resolved activity are bounded to 3 KiB of serialized JSON.
The aggregate Gateway packet stays within the documented 4-KiB sending limit; linked Spotify
is omitted when it cannot fit alongside the chosen activity and custom status. All limits are
byte limits, including JSON escaping. No plugin networking, credentials or reactive publishing
is exposed. The native card shows text, artwork and timers; the editor additionally previews
button labels and party size. Peer visibility and service acceptance of these unofficial
normal-account rich-presence fields remain unverified; synthetic tests do not establish them.


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

Sticker image fallback is built into the client. Stickers that pass this native
gate still send as stickers. Unavailable native selections enter the composer as named
artwork links. Explicit Send keeps Markdown links alongside text, or uses attachments
for artwork-only drafts when send/attachment permissions allow it. Animated APNG
attachments are transcoded to GIF to retain animation.

Received `sticker_items` and legacy `stickers` render transparent artwork in chat.
Clicking opens details (artwork, name, source) and related previews; View More Stickers
opens the source in the picker. When the loaded sticker names a server this session does
not know, the card says it is from an invite-only or unavailable server; failed loads
offer an icon retry. Standard packs and missing metadata load only on demand. Guild
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

## Chat links — October 8, 2026

Clicked message and embed links matching HTTPS `/channels/{guild|@me}/{channel}`
with an optional message ID navigate inside Serein. Exact discord.com and legacy
discordapp.com hosts, including www, ptb and canary, are recognized, case-insensitively
and with the default HTTPS port. Valid query/fragment suffixes do not change the
target. Raw routing paths are validated before URL normalization; encoded paths,
dot segments, credentials, non-default ports, spoof hosts and malformed IDs are
not internal routes. Parsing is capped at 2,048 bytes.

Bare message URLs and angle autolinks render as destination pills: channel-kind icon and
name, then a message glyph or, for forum posts, the forum and post names. A link to
another joined server shows that server's icon and name. Channel and conversation names
come only from local metadata the session may view and read; rendering never fetches
metadata. Unknown or hidden destinations use a generic label. Masked Markdown links
retain their author-chosen text, code stays literal, and concealed spoilers do not expose
or activate a pill. Copying a selected pill copies its original URL. Adjacent copies of
the same URL stay separate pills, so a selection covering both copies both URLs. Pointer
and keyboard activation use the same internal route; explicit Open in Discord controls
still open the browser, and unrelated links retain existing confirmation behavior.

Known channels can be in another joined server, an existing DM/group DM, a thread
or a forum post (whose destination is the thread channel, not its parent).
Guild/channel identity and current view/history permissions are checked first.
An absent thread can be admitted only from the existing bounded archive page through
its normal permission and metadata-budget guards. Unloaded archives, unknown channels
and unsupported channel kinds remain unavailable; navigation does not discover
arbitrary channels, join servers, open unknown DMs, unarchive threads or join calls.

Fresh loaded messages, including an eligible dormant resident window that holds the
live target, scroll and highlight locally without recent-history or saved-cursor
revalidation; a restored window without it is not presented as loaded. Other message
targets reuse the bounded 50-message request for the window before the target. One request-scoped target
survives consumption of the UI scroll cue; completion verifies the exact message ID,
not just a neighboring result. Repeated pending clicks share the request. Superseded,
wrong-channel and old-session responses cannot settle the current target. Known
deletions report deletion; a successful page without the target reports that it was
not returned and may have been removed or become unavailable, not proof of deletion.
Offline misses leave the current conversation, draft and navigation unchanged. An
active window marked Stale by disconnection is not treated as Fresh; previously
Fresh dormant windows still use their permission/identity/mutation eligibility guards.
Ordinary reselection of an untouched dormant window that is under five minutes old and
ends at the channel's latest known message skips the recent-history request (see
`docs/storage-policy.md`). This is **unverified**: it assumes the gateway dispatched
every message, edit and reaction for that channel meanwhile, which may not hold for
very large servers; the age cap bounds the window. A direct conversation still sends
its existing call-state synchronization on open.

Hover/press preloading is **unofficial** behavior: it sends the same recent-history request
a click would send, earlier, for a conversation the pointer rests on for 120 ms or presses.
It is not a crawl: one request in flight, at most twelve per minute, no repeat within five minutes
for an unchanged conversation, no pagination. It uses only spare REST capacity and gives up
during the shared cooldown instead of waiting, so it never delays a click. Hovers that never become
clicks still cost requests, which the official client may not make; the owner can turn it off
in Settings › Chat › Channel list. Resting on a person for 300 ms requests the same
**unofficial** extended-profile route a card opens, with the same query and server context,
under its own one-in-flight and twelve-per-minute limits; that route is rate-limit sensitive,
so this is the preload most likely to meet a cooldown. Resting on the GIF button requests the
picker's existing home and favorites reads. Home hover retries are capped at twelve per minute;
fresh cache hits send no request and explicit picker retries keep their normal behavior.

The wire history route and permission requirements are documented in Discord's
[Message resource](https://docs.discord.com/developers/resources/message); forum/thread
relationships are documented in [Threads](https://docs.discord.com/developers/topics/threads).
Compact presentation and normal-account navigation are **unofficial compatibility**,
not a documented native UI contract. Source comparisons included Discohook's
[approximation renderer](https://github.com/discohook/discohook/blob/3e339e2f3fbe2c13570d25c1a5a52877c94f4fb2/packages/site/app/components/preview/Markdown.tsx#L432-L505),
Vencord's [exact returned-ID check](https://github.com/Vendicated/Vencord/blob/718c867256a9d181edc7a534afb296b9bb41ab58/src/plugins/messageLinkEmbeds/index.tsx#L133-L164),
Dissent's [loaded-row navigation](https://github.com/diamondburned/dissent/blob/6ff6182b1eac30d57e9c9c995942317eb42e3bf6/internal/messages/view.go#L398-L419)
and Abaddon's [selection-aware generic links](https://github.com/uowuo/abaddon/blob/7b3a4ff97ae6490a15adaa0a792ea9225c4c9e51/src/components/chatmessage.cpp#L881-L912).
These are pinned source inspections, not live client tests. Verification uses
synthetic data only; live Discord interoperability remains **unverified**.

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

Rename completion accepts an already observed matching name without restoring mutable
thread details from the REST response. A concurrent update may also change lock/archive
state, so the next menu opening reloads those details. The single retained owner ID still
requires current View Channel access before enabling edits. Synthetic regressions cover
rename echoes, a later conflicting name and revoked access; live behavior remains unverified.

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
or DND state suppresses them. The account's own identity, session ID and resume address remain
strict. Other entries degrade individually (September 28): an explicit `null` in a defaulted field
reads as its default; a malformed or conflicting guild, channel, thread, DM, DM recipient, contact,
voice state or other member row is dropped (the first of repeated IDs wins) and a startup warning
names the affected categories; more than 10,000 relationships are truncated with the same warning.
A malformed, truncated or conflicting role list, overwrite list, own member row or `merged_members`
alignment makes that permission metadata unknown, never granted; capacity ceilings still stop the
session. SakuraCord's lossy READY decoding was reviewed as prior art; no code was copied.
Supplemental optional metadata follows the same policy. No raw payload or parser error is logged;
when login still fails, the sign-in banner offers a copyable report with the schema path and
error kind of the first failure, with remote strings, numbers and ID-like map keys redacted.

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

Safety Setup (also Manage Server) edits the documented `verification_level`
(0–4) and `explicit_content_filter` (0–2) fields through the same guild route.
Servers with the `COMMUNITY` feature cannot choose verification None or partial
media scanning; the editor disables those choices and the reducer and encoder
reject such edits before any request. The banner color also accepts any
`#RRGGBB` value besides the presets. Live interoperability remains unverified.

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
The button requires a connected call, channel video permission and the negotiated
outgoing codec;
capture requires an explicit click. No camera opens in the synthetic demo.
September 13: Windows settings and call controls now enumerate/select cameras,
including DirectShow-only virtual sources. A read-only native enumeration test
found three registered virtual cameras on the Windows test machine; it did not
activate any source. Actual capture and Discord delivery remain unverified.
October 2: camera capture now selects formats near the existing 640×480,
15 fps stream, with a 1280×720 native input ceiling and the existing bounded
1920×1080 DirectShow fallback. Device-format/rate
selection does not change Discord signaling or establish live interoperability.
October 6: outgoing camera presets now extend through 7680×4320, with 15/30/60 fps
selection and a preserved 640×480/15 fps default. Higher resolution requires a
sufficient native capture mode; the selected frame rate requests the closest
supported native interval and caps encoding, without guaranteeing sustained fps.
Incoming playback now uses shared H.264/HEVC/AV1 decoding through 8K; the exact
6144×2560/60 cadence has an offline RTP/DAVE decoded-picture fixture. Physical 8K/60 fps capture
and official-client viewing remain unverified.
Native limits, platform requirements and the owner-operated validation gate are in
[Camera in calls](voice.md#camera-in-calls-macos-windows-and-linux). Camera recording remains unsupported. This section supersedes older camera-exclusion
statements in the historical voice/screen-sharing notes below.

## Outgoing screen sharing — September 11, 2026

October 3 reliability audit: UDP media sends drop on local socket pressure without
waiting inside the media loop. Established stream rekeys now retain a 30-second
response/execution deadline, including when WebSocket heartbeats continue.
The shared Opus reorder buffer preserves received successors after larger loss
bursts. Linux incoming native decoding enforces four queued access units and
8,650,752 payload bytes; GStreamer before 1.20 falls back to software decoding.
Synthetic pressure and encrypted local-socket tests verify those corrections.
They do not establish resolution of the reported intermittent Linux outgoing
lag or official-client error 2012. Capture/GPU/network/viewer conditions and
low-bitrate large-keyframe latency still need owner-controlled evidence; see
[the audit notes](voice.md#stream-lag-and-viewer-timeouts-october-3-2026).


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
a bounded loss/REMB controller, not WebRTC's full congestion-control algorithm.
The October 17 receive path additionally correlates authenticated sender reports
for bounded software presentation; physical timing remains unverified. The
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
already requests periodic PipeWire keepalive frames. These September 15 changes
predate the sender RTX repair and bitrate controller described above; receiver NACK
requests remain unsupported. The viewer accepts announced RTX
packets and bounded packet reordering within the current picture; native/live video
quality is not established by these changes.

Watched streams have a session-only 0–200% audio volume and mute control, independent
of participant voice levels. Audio playout retains at most two queued 20 ms frames
and clears delayed mixer output after a stall. Decoder input is bounded to 16 pictures
and 16 MiB, with a 150 ms age limit and keyframe recovery. These bounds reduce backlog;
those original bounds alone do not establish live sync. The October 17 receive
path adds bounded sender-report correlation at software playout, with native-device
timing and current service behavior still unverified.

September 27 preview update: hovering a currently streaming guild participant requests
Discord's still preview and shows it in a compact 16:9 popover with a Watch Stream action.
The response is accepted only when it names the requested stream under Discord's CDN/media
host and passes the existing bounded image decoder. One newer request cancels the previous
one; hidden previews show an unavailable state. Watching while idle first joins the voice
channel and then reuses the existing receive-only stream negotiation. The request route and
normal-account response remain unofficial and live-unverified.

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
portal-approved PipeWire screen/window capture. It uses the system picker and an ephemeral
portal session. Camera/screen encoding uses native FFmpeg with
NVENC/AMD AMF/Intel Quick Sync and H.264 OpenH264 fallback. The former
platform encoders are removed. FFmpeg excludes Media Foundation and VA-API
encoders. Quick Sync on Linux uses the iHD
VA driver interface for its device. These hardware paths and actual runtime
availability remain unverified locally. The pipeline has one source
queue (within the 7680-long-edge/4320-short-edge 8K envelope in either orientation, with a 265,420,800-byte raw ceiling including HDR;
PipeWire negotiates 2–4 buffers), one raw frame per encode/preview
branch, one raw appsink frame per branch
(preview ≤925,696 bytes including padding), and the existing
three-frame / 6-MiB encoded transport queue. Each processing stage can additionally hold
its current buffer. Driver, compositor and codec surface pools are separate native
allocations. Raw queues discard old frames before encoding; encoded pressure blocks
upstream and a lost reference frame requests a new IDR. Bus events are reduced to one
failure flag. Portal responses are admitted at ≤64 KiB; signal queues hold one message
and the connection queue holds two, with zbus's separate 128-MiB wire-message ceiling.
Cancellation is checked during portal waits every 50 ms and media waits within 100 ms;
native driver startup/shutdown can still block, retaining the existing retirement barrier.
One PipeWire remote remains under the approved session across FFmpeg encoder or
bitrate changes. Raw BGRA CPU conversion/cropping/scaling precedes encoding;
source buffers and each current conversion stage can additionally retain a
132,710,400-byte source picture. The worker retains one latest validated raw
picture for idle keyframe recovery; a security pause clears it. No source, restore
token, pixel buffer or stream is persisted.
Native X11 now offers an explicitly selected whole-desktop source through GStreamer
`ximagesrc`, reusing the bounded encoder/preview pipeline without a portal. It requires
GStreamer Good and never activates after portal cancellation or failure. Individual
X11 window selection and native/live validation remain outstanding. H.265/AV1
sending requires compatible hardware and explicit codec negotiation;
H.264 retains software fallback.
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

Gateway opcodes 18/19 and STREAM_CREATE/STREAM_SERVER_UPDATE/STREAM_DELETE are unofficial normal-user behavior, checked against [discord.py-self](https://github.com/dolfies/discord.py-self/blob/master/discord/gateway.py). A separate RTC connection uses the stream RTC server/channel IDs and the parent call session, sharing one ephemeral DAVE signing identity. The `rtc_server_id - 1` MLS group mapping comes from [discord-native-voice](https://github.com/dolfies/discord-native-voice/blob/master/discord/ext/native_voice/stream_client.py); it is not an official protocol guarantee. Explicit selected-codec negotiation and UDP transport are required; mismatches fail visibly. Video is DAVE-encrypted before matching H.264 RFC 6184, H.265 RFC 7798 or AV1 RTP packetization and per-packet transport AEAD. There is no plaintext fallback. [DAVE protocol](https://github.com/discord/dave-protocol/blob/main/protocol.md) supplies the encryption requirement.

Screen source discovery and capture occur off the UI/audio threads. Application-owned
source lists are capped at 64 labels of 256 bytes. Raw capture is capped at a
7680-pixel long edge, 4320-pixel short edge and 265,420,800 bytes, including HDR
half floats; BGRA uses at most 132,710,400 bytes. Source quality retains physical
pixels; presets derive landscape, portrait and custom aspect ratios from the approved
source and disable upscaling. Concrete even output dimensions are resolved before
stream announcements. macOS captures native pixels using the approved filter and
refreshes its geometry without reopening the picker. Windows prechecks source size and stops
on oversized callback frames, but its upstream driver adapter can resize its native GPU
pool before that callback. One raw frame and three encoded frames can be queued;
encoded frames are capped at 2 MiB, encrypted packetization at 2,048 fragments of at
most 1,200 transport bytes. Quality presets run through 1440p, 4K and 8K and target
2–50 Mbps, with frame dropping under load. They are limits/targets, not measured
delivery guarantees. Camera has separate saved resolution and 15/30/60 fps choices
through 8K, requiring a supported native camera mode; local previews remain bounded.
Incoming shared decoding admits validated pictures through 8K. High-resolution
delivery and native HDR interoperability still require owner-operated validation.

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

Stop, call teardown, changed generation, lost video permission or stream-server replacement stop capture and transport. New sharing waits for native worker retirement and matching STREAM_DELETE. No automatic retry or recording is performed. Rekeying pauses capture readiness, drains queued encoded frames, and requires an IDR before resuming transmission. The encoder requests periodic IDRs every two seconds of encoded frames. The current sender handles RTCP PLI, bounded authenticated NACK/RTX repair, optional system audio and loss/REMB bitrate adaptation as described above. Receiver NACK requests remain unsupported; lossy-network quality and service acceptance of high-quality presets remain unverified.

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
Stickers and unsupported future poll layouts retain presence markers and an Open in Discord fallback. Supported polls now have native cards, as described below.
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

Loaded threads (September 10): READY guild thread arrays follow the original [discord.py-self guild parser](https://github.com/dolfies/discord.py-self/blob/master/discord/guild.py); active create/update/delete, scoped sync, archive eviction and owner-removal handling are informed by its [dispatch implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py). Discord's [Gateway thread events](https://docs.discord.com/developers/events/gateway-events#thread-list-sync) document the guild/parent scope and membership fields. These primary sources establish wire evidence, not normal-account acceptance; no implementation blocks were copied. Active discovery is limited to service-supplied snapshots/events; explicit archive reads are described below, with existing subscriptions unchanged. A visible unresolved channel mention performs one bounded documented channel read and admits only a same-guild thread whose loaded parent remains viewable; resolved names are retained in a bounded session-only cache. Unknown updates do not otherwise hydrate a missing thread. See native navigation scope.

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
| Rate limits | Credential-specific response headers/body; [rate limits](https://docs.discord.com/developers/topics/rate-limits) | Documented; do not assume bot quotas | Conservative shared cooldown; a message send rejected with 429 (documented as not processed) stays pending and is resent with its nonce after the service's wait, at most 30 times, with the official client's "sending too quickly" notice. No other write is retried. Idempotent reads wait out up to two short JSON 429 responses. An HTML or plain-text 429 from `discord.com` (a Cloudflare block, unofficial) moves this process to `ptb.discord.com` and retries one idempotent read there; whether every Cloudflare ban is limited to one hostname is unverified |
| Nonce correlation | Message nonce; message resource | Documented finite deduplication; user applicability unverified | Correlate confirmations only; no automatic ambiguous resend or indefinite idempotency claim |
| Native message formatting | Existing message content; [pulldown-cmark source](https://github.com/pulldown-cmark/pulldown-cmark) and [egui LayoutJob](https://docs.rs/egui/0.36.2/egui/text/struct.LayoutJob.html) | Local rendering; no additional service capability | Bounded emphasis/code/quotes/lists/strike, inert HTML, explicit HTTP(S) link confirmation, up to 32 inline text spoilers with separate media reveal; CommonMark differs from Discord Markdown; no automatic previews |
| Read markers | Normal session; [discord.py-self HTTP](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py) and [dispatch implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py) | Unofficial and unstable | Explicit per-message acknowledgement, optional manual mark-unread (`ack_message(manual=True)` plus the previous snowflake), guild `POST /guilds/{id}/ack`, bounded READY read state, MESSAGE_ACK and PASSIVE_UPDATE_V2; synthetic checks only, actual cross-device behavior unverified |
| Relationships | Normal session or restricted Social SDK scopes; OAuth scope table and Abaddon | Unofficial / restricted | Unsupported |
| Reactions | Normal session; [message reaction resource](https://docs.discord.com/developers/resources/message#reaction-object) and [Gateway reaction events](https://docs.discord.com/developers/events/gateway-events#message-reaction-add), rechecked September 10 | Documented routes/shapes; normal-user acceptance live-unverified | Native counts, eight-emoji picker, existing Unicode/custom emoji toggles, normal own-reaction PUT/DELETE and bounded message readback. Synthetic HTTP/Gateway/keyboard tests pass; no live validation |
| Upload / preview / save | Normal session + separate credential-free media transfer; [attachment reference](https://docs.discord.com/developers/resources/message#attachment-object) and [normal-user staged upload](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/abc.py#L1551) | Documented attachment metadata; staged upload unofficial; service delivery/conversion live-unverified | Up to ten explicitly selected files totaling 500,000,000 bytes, streamed upload/cancel and message confirmation; Discord enforces the account's lower entitlement limit. Static previews, native viewer, image/video attachment context menus with native clipboard copy, and explicit image/file Save As (1 byte through 100 MiB). Upload limits below; preview/save limits |
| Conversation search | Normal session; [discord.py-self search flow](https://github.com/dolfies/discord.py-self/blob/master/discord/abc.py), [HTTP routes](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py), [response types](https://github.com/dolfies/discord.py-self/blob/master/discord/types/message.py) checked September 10 | Unofficial and unstable | Search current conversation, numbered result pages, snapshot snippets, history revalidation on Open message; indexing/permission failures are visible. Live-unverified |
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

Discord's documented voice transport and [DAVE protocol](https://daveprotocol.com/) supply encryption/protocol requirements. Serein uses Davey 0.1.4/OpenMLS, an unofficial implementation, rather than claiming to ship Discord's libdave or an independently audited engine. Real MLS, DAVE, RTP, Opus and loopback WebSocket/UDP tests exercise the adapter with synthetic participants. They do not verify current Discord acceptance, microphone permission, device quality or a remote official client. This September 10 snapshot excluded group DMs, Stage channels and camera video; the later group-DM/camera sections supersede those exclusions, with Stage tracked separately. Outgoing screen sharing is covered by the September 11 addendum. Guild voice is implemented separately below, with its live gate still unverified. See [voice scope and owner-operated gate](voice.md) and [adapter details](../crates/discord-voice/README.md).

Image attachments: documented wire metadata and spoiler bit 3 are implemented; additional sensitive flags and proxy PNG conversion rely on unofficial implementation evidence. Native viewing and bounded cache/patch behavior have offline coverage; actual account image delivery remains unverified. Scope and sources.

Read-state continuation (September 10): channel read cursors and latest-message IDs drive boolean unread badges; absent service read state remains unknown. The guild rail tick ignores muted guilds, muted categories, muted channels, and voice channels. Voice chat stays behind Show chat, so leftover text-in-voice cursors do not light the server icon. Mentions on those channels still add the red count. A live-edge page whose rows sit at or behind the cursor is not unread. The message menu sends one acknowledgement only after an explicit action on a loaded message in a fresh authenticated view. Incoming manual mark-unread updates are honored. A later Gateway update wins over an in-flight HTTP completion. Snapshot/update lists are capped at 4000 entries, retained cursors are limited to known navigation channels, and only one write is pending.
Outgoing mark-unread and guild acknowledgement (September 16): Mark Unread ACKs the previous snowflake with `manual: true`, matching unofficial [discord.py-self `ack_message`](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py). Server Mark As Read uses `POST /guilds/{guild_id}/ack` from the same source. A manual unread in a DM or group sends `mention_count` for the loaded messages now past the cursor, because Discord does not calculate that badge. Mark Unread in a guild channel sends `mention_count` for loaded pings in that range. Server Mark As Read still omits it. Live Discord acceptance is unverified. A legacy acknowledgement token, if supplied, is capped at 2048 bytes in zeroized session memory; the response body is capped at 4096 bytes. Neither cursors nor acknowledgement tokens are written to SQLite. The linked primary client implementation supplies unofficial wire evidence; local HTTP/WebSocket fixtures do not establish Discord acceptance.

Search continuation (September 10): guild conversations use the guild search route with an exact channel filter; DMs use the channel route. Search content is percent-encoded and sorted by timestamp, descending by default or ascending when Oldest is chosen (the service sorts the whole result set, so ascending page one starts at the earliest match), with bounded offset pagination. A message cursor continues with `max_id` when descending and `min_id` when ascending (unverified for ascending). `has:` accepts image, video, link, file, embed, sound, poll and sticker, plus forward sent as the service's unofficial `snapshot` value. Query date boundaries remain fixed across numbered pages. One replaceable task uses existing REST permits, deadlines and cooldowns. Indexing responses require another deliberate Search action after the service delay; no automatic polling, broad account search, advanced filters, NSFW override or search-result persistence is implemented. Service totals and partial-index status are displayed as supplied, not asserted complete. Opening a result fetches up to 50 history messages ending at that ID and positions the timeline there; unavailable results are reported. Existing reload returns to latest history. Search snapshots are cleared on relevant edits/deletes, navigation, disconnect, permission invalidation and logout. Original-client sources supply wire evidence only; no source-code blocks were copied and no authenticated service request was used as validation.

Flatpak locale containment (October 2, 2026): after GTK initialization, Linux login
and invite-verification webviews reject a non-UTF-8 effective encoding inside
Flatpak before constructing WebKit. The existing error surface provides runtime
language/restart guidance; sign-in and reconnect preserve the fixed local error
label instead of replacing it with a generic webview error. No environment/global locale, sandbox permission,
authentication handoff or storage setting is modified. This contains the known
C-locale subprocess-start failure; it does not repair missing runtime locales or
establish successful sign-in. The guard intentionally also blocks ASCII-only
Flatpak setups that might otherwise happen to work. Native non-Flatpak webview admission
is unchanged. See [Flatpak troubleshooting](../packaging/flatpak/README.md#loginverification-locale-troubleshooting).

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
preserve newer reaction values without discarding newer message content. Jumping to a
reply or another message target requeues a readback cancelled by the history request.
Both message action menus allow joining an existing reaction without Add Reactions.
Replying requires Read Message History in addition to the ordinary send permissions;
users without it can still send plain messages if allowed. These are synthetic checks,
not live normal-account validation.
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

Joined servers' catalogs are received from READY and GUILD_CREATE, updated
by GUILD_EMOJIS_UPDATE, and cleared on GUILD_DELETE. The documented emoji fields and update
shape are supported by [Emoji Resource](https://docs.discord.com/developers/resources/emoji)
and [Gateway Events](https://docs.discord.com/developers/events/gateway-events#guild-emojis-update).
Normal-user READY remains unofficial/unstable; this change was tested with synthetic events
and local sockets, not a live account. A GUILD_CREATE for a new or rejoined server restores
its bounded navigation and permission metadata. Missing catalogs are displayed as unavailable
rather than empty.

Formatted message/profile/embed text renders `<:name:id>` as a static CDN image, using the
documented [custom emoji CDN endpoint](https://docs.discord.com/developers/reference#image-formatting-cdn-endpoints).
With GIF animation enabled, `<a:name:id>` (and animated named artwork links) loads the same
endpoint's documented GIF format at 64px and plays through the existing bounded off-UI decoder
and animation pool, falling back to the still PNG when animation is off or the GIF fails.
Reactions do not carry an animated flag in Serein's model and stay static. Custom reactions
use the same bounded media worker. Deleted/failing previews have a fixed
placeholder and retain copyable original markup. Standard Unicode and custom image widgets
participate in text selection: copying a selection retains Unicode sequences/custom markup,
and right-click Copy emoji copies the entire token. Selection endpoints treat each image as
one item, avoiding broken ZWJ sequences or partial custom markup. Whole-message Copy is unchanged.

The chat Emoji button opens a searchable Unicode/name palette and a joined-server rail, also
available in DMs. It opens on the current server's loaded, nonempty catalog; the standard emoji category remains
available in the rail. Search matches custom emoji names and source server names across loaded
catalogs, prioritizing the current conversation's server before applying result limits. The same
priority applies to reaction and poll pickers, `:name` autocomplete and completed local shortcodes.
Autocomplete includes native and image-fallback customs with their source server. Choosing
inserts at the saved text cursor or replaces its selection, preserves Unicode presentation
selectors, and records the draft without sending. Escape/close restores keyboard focus.
Catalog entries must be explicitly available and unmanaged, with known role restrictions
matched against the account's known source-server roles. A destination guild must allow
USE_EXTERNAL_EMOJIS for another server's emoji; DMs have no guild permission gate. New custom
reactions and native form choices also require the account's animated/external emoji entitlement;
image-sharing fallback never grants reaction eligibility. Existing reactions remain joinable without
that entitlement, and removing one's own reaction is unchanged.
Unknown native eligibility remains disabled. The built-in picker and autocomplete use the current
session's confirmed Nitro entitlement for animated or external custom emoji, choosing
the native path when available and the image fallback otherwise. Static custom emoji
in their own server keep the native path without Nitro. Fallback selection requires
send permission and does not change reaction eligibility; artwork-only sends also
require attachment permission. Picking never sends fallback artwork immediately.
Discord remains authoritative for actual sends and reactions, including entitlement rejection.
Natively eligible animated emoji are inserted with their original animated markup
and shown as still previews.
Hovering a rendered emoji or received sticker shows its artwork, `:shortcode:` or catalog/sticker
name and a click hint; reaction summaries also name default emoji by shortcode.
Clicking a rendered message, embed or profile emoji opens a native information card. Standard
emoji show their shortcode and default-emoji explanation; customs use their catalog name and
source server when the ID is known, otherwise explicitly report unknown provenance. Code and
concealed spoilers stay inert, links keep their link action, and selection/copy retains the
original Unicode or custom markup. These paths are verified offline, not on a live account.
The composer now displays known user mentions as `@name`, Unicode as bundled Twemoji, and custom emoji as static server artwork, while retaining original wire text for editing/copy/send. Unresolved user IDs remain literal; unavailable server artwork shows its name.


## Server voice — September 10, 2026

Guild voice entry uses the documented [Gateway voice state update and voice allocation flow](https://docs.discord.com/developers/topics/voice-connections): guild-scoped join/mute/leave, matching owner session plus guild server update, and guild ID as voice Identify/Resume server ID. The DAVE group remains identified by channel ID. This protocol documentation is not approval or live evidence for normal-user accounts. Server channel rosters use READY, GUILD_CREATE, VOICE_STATE_UPDATE and unofficial READY_SUPPLEMENTAL/PASSIVE_UPDATE_V2 shapes from the original [discord.py-self dispatcher](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py).

The media engine extends the existing DAVE/Opus path to bounded group membership and simultaneous remote audio mixing. Server mute/deafen gates local media. Empty rooms wait without opening audio devices; microphone capture requires an established encrypted group. This September 10 milestone excluded Stage channels and group DMs. Existing group-DM calls are implemented by the later group workflow; Stage is tracked separately. Main Gateway disconnect, channel/session moves, permission invalidation or voice server migration stop the current media session and require deliberate rejoin. A dropped media session of a joined call (unacknowledged heartbeats, failed resume, negotiation timeout) is rejoined automatically up to three times by re-sending opcode 4 for the same channel; that Discord answers this with fresh voice credentials is unverified. Each scheduled rejoin first asks the main Gateway to confirm its socket with a heartbeat. A rejoin waits while the Gateway is disconnected, and one whose answer was lost to a Gateway outage asks again after reconnecting without spending an attempt. No automatic call or DM ringing is triggered by viewing a roster or joining a guild channel.

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

READY and GUILD_CREATE install bounded snapshots. Role create/update/delete,
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
moderating other users' messages and full member-directory synchronization
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
Modern `colors.primary_color` takes precedence over legacy `color`. Secondary and tertiary
colors survive READY, role updates and role catalog reconciliation. Chat authors, online
member names, profile names and role labels render static two-stop gradients or three-stop
holographic colors. Equal positions favor the lower role ID, consistent with
[discord.py role comparison](https://github.com/Rapptz/discord.py/blob/master/discord/role.py).
Names retain hue when readable; the theme adjusts insufficient contrast, including hover.
Profile name colors and role chips prefer current self/member assignments, including empty
assignments, and fall back to fetched profile roles when current membership is unavailable.
Gradients color shaped glyph vertices without splitting Unicode text or recoloring color
emoji. Inline mention pills and composer mention previews retain their primary color.
`--features demo -- --demo --demo-chat --demo-role-gradients` enables synthetic gradients;
add `--demo-light` for light mode. No live Discord account was used to verify this change.
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

The streaming player no longer rejects encoded attachments above 20 MiB. It reads
validated 16 KiB HTTP ranges into a one-second PCM queue instead of retaining the
file. The shared original-attachment validator still admits at most 100 MiB.
The separate 64 MiB cumulative decoded-audio and ten-minute limits remain;
servers without range support require external playback. The offline
`--demo --demo-check-audio` debug command checks public admission and decodes an entire
24 MiB WAV over synthetic local HTTP ranges, without retaining the decoded clip,
opening an audio device or contacting Discord.

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
Layouts left by other clients are normalized instead of rejected: a server listed
twice keeps its first placement, ID-less multi-server entries become standalone
servers, empty ID-less entries are dropped, zero or repeated folder IDs get a fresh
ID, invalid colors fall back to the default and control characters in names become
spaces. Stored servers that the account has left stay in the layout.

Primary implementation evidence checked: [settings schema](https://github.com/discord-userdoccers/discord-protos)
and [discord.py-self HTTP adapter](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py).
Limits: 1,000 stored servers, including left ones, 1,000 folder entries,
100 characters/400 bytes per name, 128 KiB retained layout, and a 6 MiB response cap for Discord's 5 MiB encoded settings value
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
Permanent GUILD_DELETE dispatches (no `unavailable: true`) use the same cleanup,
including departures made in another client. Temporary unavailable dispatches preserve
the joined-server identity while revoking access until GUILD_CREATE restores its snapshot.
Permanent departures also retire the Gateway's known-server entry, so a later create can
restore the server's navigation and permission metadata. Synthetic localhost/reducer tests
cover temporary outages, permanent departure and rejoin; service delivery remains unverified.
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

### Account-type badges (updated September 29, 2026)

Chat author headers and member-list names share a compact theme-aware badge, reserving
space when a nickname is long. BOT is driven by the user object's explicit `bot` flag;
WEBHOOK requires message `webhook_id`; APP takes precedence for bot/webhook messages
with an explicit `application_id`. Ordinary users and rich-presence embeds are not
relabeled from their names, profile errors or generic `application` objects.
Member payloads normally expose only `bot`, so these display BOT, not a guessed APP
classification. Webhooks are message authors, not invented guild members.

Verified bots use the documented `flags` or `public_flags` `VERIFIED_BOT` bit (`1 << 16`) only
when the same user object also explicitly sets `bot`. They display the compact APP
badge with a checkmark in chat and member lists; ordinary bots and application-generated
messages keep their existing unchecked badges. Missing flags remain unverified rather
than being inferred from popularity, names, profile data or application IDs.

Sources checked September 29:
[Discord User Resource](https://docs.discord.com/developers/resources/user),
[Discord Message Resource](https://docs.discord.com/developers/resources/message).
These are documented metadata fields, not a live normal-session verification.
One fixed-size model enum survives cache schema 25; legacy verified-bot state defaults
to false until refreshed. Tests cover metadata precedence, migration/roundtrip, grouping changes,
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

macOS retries native format/decode failures once through the optional installed FFmpeg
helper, including failures after opening and sources requiring downscaling to the
1080p preview bound. Playback resumes from its last displayed position. Native AAC
also accepts media timescales distinct from the PCM sample rate. This does not imply
support for every MOV/MP4 codec or live-service verification.

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
  decide which codecs play. Output is 48 kHz stereo PCM and NV12/I420 pictures (caps
  colorimetry) with orientation tags applied.

The native fallback decoders described above retain their platform formats and limits.
Shared FFmpeg decoding supports the larger SDR/HDR path described under
[video backend selection](#video-backend-and-codec-selection--october-5-2026).
VideoToolbox (attachments and native live decoding) and GStreamer hand over their native 4:2:0 planes;
the renderer converts them to RGB on the GPU with the tagged BT.601/BT.709/BT.2020/SMPTE 240M
matrix and range (BT.2020 gets its matrix only, no HDR tone mapping)
(untagged HD is treated as BT.709, SD as BT.601, matching VideoToolbox's own conversion).
Windows attachments still use the RGB32 reader; Windows live pictures stay NV12.

Unsupported containers/codecs show an error inside the player with the existing
download/open actions in the context menu. Native fallback limits are 100 MiB encoded, two hours, 1920 pixels per side and
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
game messaging preferences. The October 17 section adds Profile Privacy and Content Filters. Reads and version-guarded
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
The native taskbar/Dock badge excludes muted DMs and group DMs, guilds, categories,
and channels using the existing mute expiry rules. Muting preserves the in-app
unread counts; unmuting includes them again. The native total remains capped at
99+ and is separate from the in-app mention badges.
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
The displayed guild also fetches unopened forums serially through the existing
bounded active-post loader (up to 200 posts per forum). An open forum takes priority.
A guild revisit or changed parent last-post ID refreshes the sidebar; failed pages
do not retry every frame. Counts cover loaded active posts, not a complete directory.
Forum-level READY and acknowledgement cursors are preserved: only posts newer than
the parent cursor with unread starters contribute to `New`; replies remain independently unread.
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

Opcode 37 subscriptions explicitly disable `member_updates`, matching the complete bulk
subscription shape used by discord.py-self. This avoids relying on an omitted field when opening
large guilds that were not implicitly subscribed at login. Live large-guild acceptance remains
unverified.

SakuraCord's member decoder uses the same wire shapes (optional counts and groups, presence
beside or inside a member). Offline unit tests cover each case. This was not verified against
a live server; the original failing payload was not captured.

## Per-server notification preferences — September 29, 2026

The server dropdown and server-icon context menu expose Notification Settings to
ordinary members. All messages, Only mentions, Nothing, Use server default, mute,
and everyone/role suppression use the existing account-scoped
[`PATCH /users/@me/guilds/{guild_id}/settings`](https://docs.discord.food/resources/guild#modify-user-guild-settings)
route. Save sends only selected fields, requires confirmation of those fields,
and leaves channel overrides intact. A Gateway row matching the saved fields can
confirm a pending write; unrelated settings events cannot mask a successful REST
completion. Later Gateway events still update the displayed settings. Failed writes
are not automatically retried.

Guild defaults (documented [levels 0 and 1](https://docs.discord.com/developers/resources/guild#default-message-notification-level))
are retained from flat/nested READY metadata and guild updates. User level 3
inherits that default; category and channel overrides still take precedence.
A complete valid settings snapshot permits defaults for omitted guild entries,
matching the [primary unofficial client implementation](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/settings.py#L1986).
Missing or invalid snapshots, unknown DND, and explicit unknown values remain
conservative. Expired guild mute timers no longer suppress alerts indefinitely.
Personal-control extension (October 17): server, category, channel, DM and group-DM
menus offer timed and indefinite mutes. Timed writes retain an absolute deadline
and require the response to confirm it. Server-level drafts preserve existing
deadlines when only an unrelated option changes. The Notification Settings dialog
also offers explicit category/channel overrides using the existing account settings
route; this is personal configuration and does not require channel-management rights.
Category inheritance follows bounded, same-guild ancestry through threads. Expiry
refreshes local unread/sidebar caches without waiting for another Gateway event.

Category and folder Mark as Read use bounded batches of existing acknowledgement
requests, limited to their readable descendants or member servers. Partial results
retain the original request order, and newer read-state observations take precedence.
Unread navigation, category collapse and message-target notification activation
reuse permission-checked local navigation; selecting a conversation does not itself
acknowledge messages or join a voice channel. Timed DM/group settings use
`PATCH /users/@me/guilds/@me/settings`. These normal-account routes and payloads are
**unofficial**; synthetic HTTP/Gateway and native UI checks do not establish live
Discord synchronization or interoperability. Owner-controlled live checks remain pending.

The guild stream-preview part of issue #458 was already delivered in
[PR #457](https://github.com/ViceVerse-cz/Serein/pull/457): hover a LIVE participant
for a still preview and an explicit Watch Stream action. This change adds no
stream transport or automatic call joining. Notification writes and previews
remain unofficial normal-account behavior, verified with synthetic fixtures
only; neither is proof of live Discord interoperability.

## Search navigation — September 29, 2026

Ctrl+F (Command+F on macOS) opens and focuses search for the current conversation.
In a server channel, the shortcut prefills a readable `in:` filter for the current
channel and leaves the caret ready for search text. DMs retain their implicit
conversation scope. Repeated use preserves the query and current result page. The shortcut can be remapped
in Keybinds. It does not
interrupt settings, modal dialogs, or active IME composition.

Previous/next controls and a page-number field replace the older/newest-only pager.
Pages contain at most 25 hits; offsets stop at 9,975 (400 pages / 10,000 results),
following the [primary unofficial search reference](https://docs.discord.food/resources/message#search-guild-messages).
Larger searches need narrower filters. Service totals may change while browsing,
and partial indexing still reports missing results. Failed/indexing pages require
an explicit retry; there is no background pagination or automatic polling.

Jumping to a text-search result keeps the query, result page and scroll position
open while the existing bounded history request revalidates the target message.
Channel navigation, session/access invalidation and relevant message mutations
still discard stale results. Pins keep their separate timestamp pagination and
popout dismissal behavior. Verification is synthetic; normal-account offset
acceptance and live result ordering remain unverified.


## Own profile mutual sections - September 29, 2026

Own-profile popouts now request `with_mutual_guilds=false` and
`with_mutual_friends=false`, matching the existing profile editor and hidden self
mutual sections. The decoder accepts null unrequested mutual lists without marking
the profile incomplete. Requested mutual lists, missing profile metadata, unavailable
badges/connections, truncated lists and byte limits retain their existing warnings.
Malformed lists still fail decoding. This addresses a possible false warning;
the reported account response and live Discord behavior remain unverified.


## Attachment original URL admission - September 30, 2026

The shared video/audio/download URL validator now normalizes attachment links on
`media.discordapp.net` to `cdn.discordapp.com`, retaining the original path and
signed `ex`, `is`, and `hm` query, plus the observed storage selectors described
below. It still rejects transformed media queries,
foreign hosts, credentials, non-HTTPS URLs, fragments, mismatched attachment IDs,
encoded path separators and oversized metadata/files. Playback fetches the original
through the same bounded range reader; no decoder, cache or queue is changed.

The issue #485 error occurs before networking or native decoding when attachment
URL/metadata admission fails. A media-host URL was one reproducible rejected form;
the reporter's exact URL and native Linux/Windows playback remain unverified.

### Message-scoped attachment paths (October 2, 2026)

Audio, video and explicit downloads also accept
`/attachments/{channel_id}/{message_id}/{attachment_id}/{filename}`, alongside the
legacy path without a message ID. The additional form is described by the
[maintained unofficial CDN reference](https://github.com/discord-userdoccers/discord-userdoccers/blob/2e13ae4fb04253a9e9e8d345ec0be2e2950af2b3/pages/reference.mdx#L875).
Previously, it failed admission before audio decoding with “Audio attachment unavailable”.
Both source IDs must be valid nonzero Discord IDs; the attachment ID must still match
the received attachment. Source channel/message IDs can differ from a forwarded
message's outer identity. Signed paths and queries remain intact, and the existing
origin, metadata, file-size and transformation guards remain enforced. Synthetic
checks cover both forms and both hosts; the reported live message remains unverified.

### Storage backend selectors (October 7, 2026)

The shared original-attachment validator also retains `backend=b2` and
`backend=b3`, the storage selectors reported in issue #556. These are observed,
unofficial CDN behavior. Previously these URLs failed before networking, blocking
video/audio playback and explicit saves. Both admitted hosts and attachment path
forms keep the complete signed query; unknown/empty backend values, rendition
parameters, invalid origins/IDs and oversized metadata/files remain rejected.
Offline regression tests cover admission only. Actual decoding and the issue's
separate high-CPU playback report remain unverified and are not fixed here.

## Optional REST API proxy plugin (preview)

The API Proxy community plugin can select an HTTP/HTTPS CONNECT proxy
for Serein's Discord REST requests, including native account validation, Gateway
URL discovery and public application metadata. It is a connection-scoped plugin:
its narrow grant allows configuration before sign-in and exposes no account data,
Discord credentials, sockets or general network API to WebAssembly.

Manual URL mode preserves destination TLS validation and never retries directly if
the proxy fails. Automatic mode uses the process proxy environment and `NO_PROXY`;
it is not OS PAC discovery. Proxy credentials in URLs are rejected and must not be
stored in plugin data. Configuration changes replace the REST pool for subsequent
requests; a request already started finishes on its previous route.

The message Gateway WebSocket, voice signaling/media, calls, screen sharing,
CDN downloads/uploads, updater, extension catalog and login/verification webviews
remain on their existing routes. This REST-only plugin does not make Discord fully
reachable where those endpoints are also blocked. Owner-controlled live use and
cross-platform proxy interoperability remain unverified.

HTTP Basic proxy authentication uses the host-managed Proxy authentication section
in the plugin panel. Apply the manual URL first, then Save credentials. Only credentials
for the exact normalized proxy origin are attached to its HTTP/CONNECT requests.
Credentials are bounded, masked, excluded from Wasm/plugin storage and persisted only
in the OS credential store. Remove saved credentials explicitly to erase them; Direct
and plugin disable stop using them but retain the OS entry. Automatic mode still rejects
credential-bearing environment URLs. Live authentication remains unverified.

HTTP Basic credentials on an HTTP proxy cross an unencrypted proxy connection,
even when Discord destination TLS is preserved. The credential form displays
this warning for HTTP endpoints; HTTPS proxies encrypt the proxy connection.


## Native polls (September 30, 2026)

Poll messages now retain bounded questions, up to ten answers, optional emoji, expiry,
single/multiple choice and supplied results. Native cards use Serein surfaces and accent
colors with Discord-style answer rows, selection checkmarks, result fills and a vote footer.
The composer `+` menu's Create a poll option opens a creation dialog (300-character question, 55-character
answers, Unicode emoji and the usual 1h/4h/8h/1d/3d/1w durations). Readers can vote,
remove their vote, explicitly refresh results, and confirm ending their own poll.

Schemas and creation/end routes follow the [official poll reference](https://docs.discord.com/developers/resources/poll).
Normal-account voting uses the [unofficial first-hand message reference](https://docs.discord.food/resources/message#vote-on-poll):
PUT /channels/{channel}/polls/{message}/answers/@me with answer_ids; an empty array removes votes.
Writes are never automatically replayed. Voting reads back the exact message through history;
Gateway vote events update loaded results and invalidate inactive resident windows. Missing
results remain unknown or preserve matching earlier results; finalized tallies are authoritative.
While a poll is running, service counts may be approximate. Multiselect totals count selections.
Poll content is limited to 8 KiB retained and 128 KiB wire, and one action may be pending.

Typed poll data is currently session-only; the local history database keeps its existing presence
marker until ordinary history refresh. Forwarded/deleted/private-response polls are read-only.
Voter-name browsing and a custom-server-emoji creation picker are not included in this fast slice.
Extension snapshots retain their existing unsupported poll contract. No live Discord account was
used to verify normal-account interoperability. Use --demo --demo-polls for an offline preview
and --demo --demo-check-polls for the focused synthetic debug check.

Compact message display is a device-local layout: ordinary message rows show
the time, then a clickable author, then the formatted body in its own column,
without the avatar gutter or header row. Existing reply and system widgets remain.
Pending rows use the same time, author and body columns;
message selection, mentions, spoilers, media and delivery actions remain in
the existing native widgets. The saved compact choice also reduces divider and
reply/system spacing. This changes local presentation only.

## Message sending during Gateway recovery — October 2, 2026

A temporary Gateway outage no longer disables an explicit REST message send from
an authenticated session with last known send permission. Discord still authorizes
the request; an offline network, changed permissions or expired session can reject
it. The existing bounded write queue, service cooldown, single-attempt policy and
failed/ambiguous delivery states remain in use. Attachment uploads still require
the Gateway to be connected. Nothing automatically replays uncertain writes.

Refresh during reconnection, the Reconnect now button and an explicit message send
wake the Gateway's current retry without waiting for its capped backoff. Recovery
preserves the session/sequence for RESUME, does not reopen login or restart a REST
write, and cannot extend the bounded initial-login attempt limit. On an already
READY/RESUMED socket a recovery request sends one immediate opcode 1 heartbeat
instead; the socket is kept when it is acknowledged and replaced by a RESUME when
no acknowledgement arrives within five seconds, so a network change does not wait
for two missed timer heartbeats. That Discord acknowledges an extra heartbeat sent
outside the timer, and how quickly a dead socket is noticed live, are unverified.
Reconnect now is available in the account card on every platform. Refresh also
reloads readable message history through REST during an outage; repeated Gateway
retry notifications preserve that request. A successful RESUME still revalidates
the active history; a REST-only reload during an outage completes as stale rather
than leaving Refresh disabled in Loading.

The offline debug command is
`cargo run --locked -p serein --features demo -- --demo --demo-check-resume-send`.
It exercises synthetic composer/Refresh/Reconnect input, disconnected sending,
terminal auth and access gates, uncertain-write preservation and coalesced recovery
waits without opening a network connection or audio device. Workspace tests also
use a local synthetic WebSocket to check late recovery requests and preserved
RESUME session/cursor state. macOS sleep/wake and live Discord RESUME/delivery
remain unverified in this Linux fast pass.

## Quiet messages (`@silent`)

Start a new message with `@silent` followed by whitespace to request a quiet send.
The marker and exactly one separating whitespace character are removed; remaining
indentation, line breaks and trailing whitespace are preserved. The documented
[`SUPPRESS_NOTIFICATIONS` message flag](https://docs.discord.com/developers/resources/message#message-object-message-flags)
(4096) is set. Discord describes this as suppressing push/desktop notifications while
retaining notification badges. Existing mention selection and reply-mention preferences
still apply. Ordinary text, replies, attachment captions, sticker captions and forum
starter messages share this behavior. An attachment or sticker can use `@silent` alone;
a text-only empty message is rejected. Embedded occurrences, `@silently`, escaped/code
text and edits remain literal. The prefix is case-sensitive.

Pending/recovery text retains the typed prefix until service confirmation so a failed
send can be reviewed and deliberately retried with the same intent. Writes are never
automatically replayed. The flag is documented in Discord's developer API; normal-account
interoperability remains unofficial and live-unverified. Loopback tests inspect the
actual outgoing JSON and do not contact Discord.

Quiet-message admission is shared by the composer, forum editor, core state and
transport. A marker-only text draft is rejected before it or its reply is consumed;
attachment/sticker-only messages may carry the marker. New-message editors have an
eight-scalar marker allowance while the effective outgoing text retains its full
2,000-scalar limit. The allowance is separately bounded; edits treat the marker
literally. Existing per-draft byte ceilings and the session input budget remain
in force. A quiet forwarding note affects only that separately sent note.

## Public Catbox attachment links (October 2, 2026)

Each staged message-composer file has a **Host file…** action. Selecting it shows
per-file consent before an anonymous upload to Catbox; nothing happens automatically
when Discord rejects a file. The original bytes are streamed without compression.
Successful links remain in the dialog for **Copy link** or **Add to draft**, followed
by the user's ordinary Send action. Adding preserves existing text and observes the
composer's character, item and aggregate byte limits. A link can only be added to
its original conversation. Navigation cancels an active upload; a completed link
remains copyable until the dialog closes. Session/account reset clears the dialog.

The independent credential-free client uses fixed
`POST https://catbox.moe/user/api.php` with multipart `reqtype=fileupload` and
`fileToUpload`, following [Catbox's API documentation](https://catbox.moe/tools.php).
It disables redirects, retries, cookies and proxy discovery, caps each streamed
chunk at 64 KiB, and accepts at most 4 KiB of response containing only a plain HTTPS
`files.catbox.moe` file URL. No Discord token or channel/account metadata is sent.
Sources reuse the existing regular-file, size/modified-time and open-descriptor
checks, including revalidation before completing the multipart body and after the
response. This cannot provide an immutable snapshot against a concurrent writer
restoring identical metadata.

The service [advertises 200 MB uploads](https://catbox.moe/); the
[FAQ](https://catbox.moe/faq.php) rejects EXE/SCR/CPL/JAR and DOC-family extensions,
and GIFs above 20 MB. Its current anonymous retention is two years of inactivity,
not permanent account-associated storage. Files and their original embedded metadata
are public to anyone with the link. No host account, userhash, automatic fallback,
remote deletion or Litterbox integration is included. Cancellation, failed responses,
logout and deleting a Discord message cannot remove already received hosted bytes.
A successful upload removes only that staged file; errors retain its local selection.
UI consent and transport use the same UI-neutral admission rules. Public-upload
failures are typed outcomes, translated in English and Czech without displaying
raw service responses.

[SakuraCord's external-host flow](https://github.com/SakuraCordApp/SakuraCord/blob/main/App/Sources/SakuraCord/Services/ExternalAttachmentUploader.swift)
was inspected for behavior; this implementation is original and follows the service's
own documentation. This addresses the owner's external-host alternative to issue #128,
whose original compression request is not implemented. Synthetic localhost tests and
`--features demo -- --demo --demo-chat --demo-attachment=file --demo-external-upload`
verify local behavior without any real hosted upload or Discord session. Live service
acceptance, link embedding and other-platform native interaction remain unverified.

## Video backend and codec selection — October 5, 2026

Voice & Video defaults to Stable/H.264 using the original platform encoders.
Experimental uses FFmpeg and offers H.264, H.265 and AV1. H.264 has software fallback;
H.265/AV1 require compatible hardware (AV1 is unavailable on macOS with this recipe).
The outgoing codec is fixed for each call/share and must match the server's explicit
selection. H264/PT101/RTX102, H265/PT103/RTX104 and AV1/PT109/RTX110 follow the
public interoperability implementation; video signaling remains unofficial.
Camera mismatch disables video while audio continues; screen mismatch fails the share.
Receiving advertises H.264, HEVC and AV1 independently of the chosen outgoing codec.
Shared FFmpeg decoding accepts validated pictures through 8K. PQ/HLG metadata and
high-bit-depth planes survive decoding into native HDR presentation or automatic SDR
tone mapping. Bare codec negotiation does not verify receiver 10-bit compatibility;
outgoing HDR captures therefore use SDR conversion. DAVE codec selection and
bounded packetization have synthetic roundtrip fixtures, which do not prove live
Discord playback. Physical AMD/Intel/NVIDIA/Apple encoders and official-client
H.265/AV1/DAVE interoperability remain unverified; AV1 final-OBU size handling
especially requires a paired live check. See [voice settings](voice.md#video-backend-and-codec-settings).
## Voice Messages extension (preview, October 5, 2026)

The opt-in bundled Voice Messages plugin adds **Record a voice message** to the
composer `+` menu. Enable it and explicitly grant `voice_messages` (plus `storage`
for settings) in Extensions. The native dialog requires separate Record, Stop and
Send gestures. Importing, enabling, opening the plugin panel or saving settings
never opens the microphone or sends a message. Wasm receives configuration only,
without audio, device paths, microphone, filesystem or networking access.

The host records mono 48 kHz Ogg Opus in session memory, capped at 120 seconds and
8 MiB. The plugin selects a 5–120-second maximum, bars/line display and RNNoise
suppression. Native device selection, gain, mute/deafen, held push-to-mute,
push-to-talk and OS microphone permission gates apply. Recording is unavailable
while a call or local device test owns capture. Stop closes input before encoding
finalization. Cancel, navigation, permission/connection loss, disable, session reset,
logout and call commands discard the local clip and gate microphone callbacks;
asynchronous native retirement fences a replacement microphone owner. This records
an explicitly created message clip, never a call or another participant.

Sending requires ordinary send/attachment permission, fresh connected history and
[`SEND_VOICE_MESSAGES` (bit 46)](https://docs.discord.com/developers/topics/permissions).
One signed attachment upload produces `voice-message.ogg` with audio MIME,
`duration_secs`, at most 64 base64 amplitude bins and message flag
`IS_VOICE_MESSAGE` (8192). No caption or stickers are included; the existing text
draft remains for a separate message, while an explicit reply can accompany the clip.
The fields, one-audio-attachment restriction and Ogg Opus format follow
[Discord's voice-message documentation](https://docs.discord.com/developers/resources/message#voice-messages).
The normal-account staged upload/reply path was also inspected in the primary
[Vencord plugin](https://github.com/Vendicated/Vencord/blob/main/src/plugins/voiceMessages/index.tsx);
Serein's implementation is original.

No recording, local path, upload target or audio recovery file is written to disk.
Cancellation cannot erase bytes already uploaded or recall a message Discord has
accepted. Failed/uncertain sends require a new recording and are never replayed.
Returned message metadata uses the existing bounded timeline/history cache; ordinary
explicit attachment downloads remain a separate user action.

Device-free tests encode and decode synthetic Opus, check Ogg CRC/duration/bounds,
exercise recorder control/retirement, validate the real Wasm settings plugin, and
inspect local HTTP allocation/audio PUT/message POST including no-retry behavior.
`--features demo -- --demo --demo-chat --demo-recorder` enables a labeled synthetic
recorder preview; it uses generated amplitude/timer data and disables Send, without
opening any device or contacting Discord. A preview is not a recording test.
Normal-account sends, physical microphone/OS permissions and Windows/macOS native
interaction remain unofficial and live-unverified; no account or microphone was
used for this delivery.

## GIF favorites admission — October 6, 2026

The frecency decoder accepts the schema's absent optional version block (version
zero), default map values, and long URL metadata within the existing 4 KiB wire
entry budget. A long, undisplayable favorite no longer rejects the whole catalog;
it survives edits verbatim. Truncated, duplicate and over-budget catalogs still
fail closed, and writes still require version and read-back confirmation.
Unsupported-format and unconfirmed-save errors now retain their specific messages.
Issue #559 supplied no response payload, so its live failure remains unverified.

Message stars also cover uploaded GIF attachments, image galleries, video-only
GIF embeds and external GIFs with an admitted Discord media proxy. Shared HTTPS
links may come from any host; this does not authorize fetching arbitrary origins.
Previews still use the existing provider/Discord media admission and bounded
pipeline. The original share URL, including attachment signatures, is retained.
The offline debug check is `cargo run --locked -p serein --features demo --example gif_favorites`.

### Favorite GIF order (October 20, 2026)

Successful favorite synchronization follows the returned server order, including when the local
cache arrives later. A cached still preview can replace a synchronized video's preview metadata
without changing that favorite's position. Unsynchronized local-only favorites retain reserved
capacity within the existing 100-item limit and follow synchronized entries. Refreshes still remove
previously synchronized favorites deleted remotely; interrupted writes are not replayed. This is
covered by bounded offline fixtures, not a live account comparison.

## Private profiles - October 15, 2026

Profile Privacy ([help article](https://support.discord.com/hc/en-us/articles/38859942749463-Profile-Privacy-Setting-on-Discord))
lets an account hide its bio, custom status, pronouns, badges, connections, widgets and
wishlists from viewers outside its chosen audience. The profile response carries a
top-level boolean `private`, which the official web client stores with the profile and
reads as "private to this viewer"; this field is unofficial, taken from the public client
bundle, and not documented. When it is true, the card shows a "Private Profile" strip and
the official notice in place of the withheld details, and omits Member Since and the note,
matching the official client. Roles, mutuals and the message field stay. The service
withholds the hidden data itself; Serein never infers privacy from missing fields. No live
private-profile response has been used as validation.


## Application command parity — October 17, 2026

This branch extends fork `f9e39f7`. The owner-supplied desktop reference is Discord Stable
641004 (`c9ac31f`), host 1.0.9262 x64 (`94186`); no official client was run here.
Native user/message context commands appear in the Apps submenu of existing context menus.
They require a known target in the current accessible conversation, the current received
schema, current permissions and one explicit interaction. Discovery merges the bounded
conversation index with eligible account-installed commands from the unofficial
`GET /users/@me/application-command-index` route, using guild/bot-DM/private contexts
rather than treating every direct message as a bot conversation.

Autocomplete uses a separate replaceable worker and nonce/deadline correlation. A partial
form posts interaction type 4 with one focused string value, including numeric options.
HTTP acceptance leaves suggestions pending until the matching Gateway choices event.
Older, oversized, wrong-schema, disconnected and retired responses cannot replace the
current choices. Input changes are debounced; requests contain only the selected form's
bounded inputs. They never execute the command or block the ordinary interaction lifecycle.
There are at most 25 choices; failures expose an explicit retry. Account scopes and current
permissions retire suggestions. Command acceptance remains distinct from application response.

The [official command schema](https://docs.discord.com/developers/interactions/application-commands)
and [normal-user account interaction reference](https://docs.discord.food/interactions/application-commands)
provide protocol evidence. Account discovery and autocomplete dispatch are unofficial;
account entitlement and live service acceptance remain owner-controlled and unverified.
Attachment arguments use the native file picker, one file per option and at most ten
files / 500 MB per command. Typed attachment indexes refer to the same signed-upload
batch sent with the command. Suggestions never upload attachments. Storage receives
no Discord credentials, and the final interaction is submitted once.

Context-menu target evidence is scoped to the active conversation. A cached author
from another channel cannot make an unrelated Friends/notification contact eligible;
availability is checked again before dispatch.

The form stays until HTTP acceptance or a matching Gateway acknowledgement. Failed or
timed-out submissions retain arguments and file descriptors for a deliberate retry;
the next upload validates those descriptors again. A completed Gateway response wins
over a later ambiguous HTTP outcome. No application command is automatically retried.
Offline regressions cover signed staging, uncertain outcomes, stale acknowledgements,
file reindexing and keyboard selection; they do not establish live interoperability.

Mention suggestions and command user selectors prefer the guild nickname while inserting
the real user ID. Labels resolved outside the member sidebar remain available across
subsequent searches in the same accessible conversation, within the bounded lookup cache.
Changing account, channel or access retires those labels; display metadata never grants
permission to act on a member.

### Fork media parity integration — October 17, 2026

This branch depends explicitly on fork #7 (`f3abb03`) for FFmpeg-first video/audio playback.
It integrates reviewed upstream soundboard #579 and pitch-preserving playback controls
#662 while keeping that decoding preference and the fork's existing capture/privacy paths.
The owner comparison build is Discord Stable 641004 (`c9ac31f`), host 1.0.9262 x64 (`94186`).
Native/live comparison has not been performed here.

Soundboard sounds are local receive/playback only; the participant send action uses the
service route, never the microphone encoder. CDN reads are credential free, bounded to
1 MiB each and four active loads. Call stop cancels downloads/CPU decoders, retires waiting
plays and clears decoded PCM. Running sounds follow later listener mute/block/volume changes.
Catalog retries and play confirmations carry local request IDs; playback also checks the
call generation and a three-second deadline. Leaving, takeover and even a coalesced fast
reconnect cancel a pending send. Uncertain HTTP acceptance is reported without repeating
the sound. Deafening discards waiting plays, even if their downloads finish after undeafening.
Existing FFmpeg decode, HDR, GPU selection, codecs and source resolution remain unchanged.
Historical upstream screenshots/measurements are archive links and are not current-fork evidence.
The subsequent integration adds explicit voice-message capture/review/send, separate media
output, native Discord-proxied embedded video and receiver clock scheduling. Screen bitrate
adaptation already exists; resolution/FPS remain explicit selections. Provider player pages
retain confirmed external links; no new webview or network-enabled FFmpeg path is added.
Stage is tracked separately. See the October 17 audit in [voice.md](voice.md#stream-quality-and-synchronization-audit--october-17-2026).

New related-emoji values must match one complete entry from the existing bundled
Unicode palette. Unchanged service/custom values are preserved when editing other
fields; no new dependency or retained emoji cache is introduced.

### Native embedded videos — October 17, 2026

Reviewed native portions of upstream PR #439 add embed playback preferring validated HTTPS
`media.discordapp.net` and `images-ext-1/2.discordapp.net` external video proxies. When a
proxy is absent, Twitter MP4 paths, OGInstagram offloads and Instagram CDN MP4 files
are admitted after explicit Play, with at most three redirects between approved media origins.
Both media metadata fields are considered, including extensionless endpoints;
the decoder validates the actual container. Play is explicit. Unknown lengths require
a one-byte range probe with a nonzero total at most 100 MiB, or one complete response
of at most 256 KiB when a small server ignores Range. Unknown dimensions reserve a
normal 16:9 player instead of a 16 × 9 pixel control strip.
Anonymous bounded range reads feed the existing FFmpeg-first decoder and native
HDR/GPU presentation. Other redirects and all credentials are refused. Suppressed/spoiler
media retain their existing reveal gates; source changes revoke fullscreen playback.
YouTube watch, short, live and embed links resolve to one validated public video ID and
start time. Explicit Play opens its official privacy-enhanced web player in an isolated
native surface; it does not pass an iframe URL to FFmpeg. Other unsupported provider
HTML/JavaScript players remain confirmed external links. No general browser, provider
extraction service, arbitrary-origin file fetch or FFmpeg networking is introduced.
Native visual/decoder tests and owner-controlled playback acceptance
must be reported separately; historical upstream evidence is not current-fork evidence.

Ban browsing accepts either View Audit Log or Ban Members, matching the documented
resource permission. Unban/ban writes retain Ban Members, and other moderation writes
retain their distinct permissions and hierarchy checks.

### Native community setup and Server Guide (2026-10-17)

The community settings pages load and edit onboarding, default channels, join questions,
Channels & Roles questions (`in_onboarding: false`), rules screening and Server Guide.
They do not reuse or change the current member's existing participant onboarding answers.
Onboarding edits require `MANAGE_GUILD` and `MANAGE_ROLES`, including the normal owner and
administrator bypass. Screening and Guide administration require `MANAGE_GUILD`.
Role assignments additionally require an unmanaged role below the acting member's highest
role (owners bypass hierarchy). Deleted channels/roles and permission updates invalidate an
active administration snapshot and fence outstanding results.

Published schemas checked for this slice:

- **Documented schema:** [Discord API docs at f7416a2](https://github.com/discord/discord-api-docs/blob/f7416a2cf165455f72a31f5ea4e671a4f047f2f7/developers/resources/guild.mdx),
  `GET`/`PUT /guilds/{guild.id}/onboarding`, prompt types 0/1, default/advanced criteria and
  the separate write-only `emoji_id`, `emoji_name`, `emoji_animated` fields. New prompt and
  option IDs use unique decimal millisecond timestamps, matching the public client creator
  and serializer in [539916.js](https://github.com/Wumpus-Central/discrapper-canary/blob/1569032f33f7540de90b443ec443c5679df69120/chunks/539916.js)
  (SHA-256 `0d2b2d82816390da2b5e1be7377c46da51fcec2a504114eeffd7bb1189276ed8`).
  Existing IDs are retained; responses may assign IDs to new rows. The documented
  enablement criteria are seven counted channels, five allowing @everyone to view and send.
  Use with an existing normal account is **unofficial and live-unverified**.
- **Unofficial normal-account routes:** [discord-userdoccers at 01624d9](https://github.com/discord-userdoccers/discord-userdoccers/blob/01624d9b7a75cd8b1e838e009b434d108d9fcdff/pages/resources/guild.mdx),
  `GET`/`PATCH /guilds/{guild.id}/member-verification`, `GET`/`PUT
  /guilds/{guild.id}/new-member-welcome`, `PATCH /guilds/{guild.id}/new-member-actions/{channel.id}`,
  `PUT /guilds/{guild.id}/resource-channels/{channel.id}`, and guild `home_header` edits.
  Screening supports bounded terms, short text, paragraph and multiple-choice fields;
  non-terms fields require the server's manual-approval feature. Disabling a gate carries the
  administrator's explicit approve/reject choice for pending applications.
- **Unofficial member Guide:** the same pinned public reference documents `GET
  /guilds/{guild.id}/new-member-actions` and `POST
  /guilds/{guild.id}/new-member-action/{channel.id}` with no request body. The native member
  Guide opens channel/resource links inside Serein. A VIEW step is queued after navigation;
  a CHAT step is queued only after an own message is confirmed by REST or Gateway. A fresh
  guide/progress read before completion verifies the task and its action type. Completion
  responses are guild/user-bound and must include the completed channel.
- **Observed public client serializer:** current public Canary 641446, datamining commit
  [1569032](https://github.com/Wumpus-Central/discrapper-canary/tree/1569032f33f7540de90b443ec443c5679df69120)
  (2026-10-17), [374084.js](https://github.com/Wumpus-Central/discrapper-canary/blob/1569032f33f7540de90b443ec443c5679df69120/chunks/374084.js)
  serializer `S`/export `Y4`, and its [PUT caller](https://github.com/Wumpus-Central/discrapper-canary/blob/1569032f33f7540de90b443ec443c5679df69120/chunks/199940.js).
  Both new-member action and resource writes use `title`, not the conflicting `name` in
  the userdoccers footnotes. Existing icon hashes are carried with collection edits, matching
  the serializer; actual image data uses the separate artwork routes. The same caller confirms
  PATCH action icons and PUT resource icons; [225142.js](https://github.com/Wumpus-Central/discrapper-canary/blob/1569032f33f7540de90b443ec443c5679df69120/chunks/225142.js)
  confirms member completion POST with no body. SHA-256, respectively:
  `df8d0fafcfd75068e228174fb3e45416571de4fd87400fee394187c60d795ef3`,
  `8d10606713be3009d98c1b05081017d077a5a4feabf40697d49c709dda101a3c`,
  `8f7db9c8fc569357887274e2bcffd5daefbc5a61be614c036ccef940fce8a0f7`.
  [539916.js](https://github.com/Wumpus-Central/discrapper-canary/blob/1569032f33f7540de90b443ec443c5679df69120/chunks/539916.js)
  generates prompt/option IDs from millisecond timestamps and includes them in onboarding
  writes (SHA-256 `0d2b2d82816390da2b5e1be7377c46da51fcec2a504114eeffd7bb1189276ed8`).
  Saved Guide artwork uses the [published CDN paths](https://github.com/discord-userdoccers/discord-userdoccers/blob/01624d9b7a75cd8b1e838e009b434d108d9fcdff/pages/reference.mdx)
  `home-headers`, `new-member-actions` and `resource-channels`, with validated IDs/hashes and
  the existing credential-free image worker.
  This is static unauthenticated reference inspection, with no code execution or owner data;
  it is newer Canary evidence, not a claim of having inspected the owner's Stable bundle.
- **Documented channel schema:** [Discord channel docs at f7416a2](https://github.com/discord/discord-api-docs/blob/f7416a2cf165455f72a31f5ea4e671a4f047f2f7/developers/resources/channel.mdx),
  voice/Stage bitrate, user limit, region override and video quality through channel PATCH;
  region choices come from `GET /voice/regions`. Stage bitrate is capped at 64000, ordinary
  voice user limits at 99, and Stage limits at 10000. Discord enforces Boost bitrate limits.
  Voice and Stage overviews also edit age restrictions and text-chat slowmode (0–21600
  seconds); topics stay limited to text/announcement/forum/media channels. The native editor
  admits the documented 4096-character forum/media guidelines.

Writes reread configuration and send only changed top-level fields. Replaced collections retain
fresh opaque row metadata by prompt/option/channel identity or original screening-field index;
reordered rules retain their automation metadata. A conflicting touched collection is rejected
before a write. Existing unknown question fields stay read-only. A rejected or uncertain draft
remains in RAM. Malformed successful writes, failed post-write reconciliation, permission races
and disconnects require a deliberate reload; no community write is automatically retried.

Admin and member Guide snapshots each cap retained data at 512 KiB. Onboarding permits at most
50 prompts, 50 options per prompt, 100 channel/role assignments per option and 500 default
channels; screening has at most 5 fields, 16 rules or 8 choices per field. Guide has a 7–300-character enabled welcome message and at most 5
steps, 7 resources and 10 authors. One admin request and one independent member request may be
pending; member completion intents are deduplicated and capped at five. Artwork uses one
cancellable native picker, at most 8 MiB source input, 4096×4096 decode limits, 64 MiB decoder
allocation, a maximum 512-pixel prepared edge and a 256 KiB PNG output. Guide icons and the
welcome sign have explicit image preview/save controls; the welcome sign respects the `BANNER`
feature. No paths, guide drafts or progress are written to disk.

The member Guide displays welcome authors from the active guild member window, already-loaded
administration members, the account, recent global profiles or friends. Guild nicknames are
scoped to the Guide's guild. Unknown authors show a neutral avatar and a labeled placeholder
with their ID on hover; opening the Guide adds no user/profile/member fetch. Known avatar
hashes use the existing image worker. Prepared welcome signs are admitted under its existing
bounded large-image budget.

This is offline implementation and fixture evidence. The owner reference remains Discord Stable
641004 (`c9ac31f`), Host 1.0.9262 x64 (`94186`). No owner session, live account, Discord mutation,
message or microphone was used. Current service acceptance, including unofficial Guide artwork
and screening writes, still requires the owner-controlled live developer-session gate. Native
screenshots, full checks and package/performance comparisons are coordinated by the parent parity
integration; synthetic fixtures are not evidence of live Discord behavior.

## Embedded playback recovery — October 21, 2026

Direct signed Discord video links use the FFmpeg-first player even when video metadata is
absent. A failed proxy may fall back once to the approved original. Anonymous requests send
a Serein User-Agent; HTML responses remain external-only. Independent compressed track
queues prevent video-first media from blocking the audio clock, with explicit byte/packet
limits and bounded startup/stall failures. YouTube uses its official IFrame API inside the
message card on Windows/macOS; Linux keeps an explicitly labeled separate window. Scrolling
pauses playback without losing position; enlargement and fullscreen are explicit. Search/pin
previews retain their own source/restriction checks. Offline tests and the reported public
clip's software decode do not establish Windows/macOS provider playback; native acceptance
is reported separately in the PR.

## Message menus and reporting — October 22, 2026

Quick reactions in the hover bar and context menu use the same account ranking,
including joined-server custom emoji. A bounded read of the unofficial
`GET /users/@me/settings-proto/2` imports reaction frecency (field 13, falling back
to legacy emoji frecency in field 6). Account settings updates trigger coalesced
refreshes; this is read-only synchronization, not an implementation of Discord's
undocumented score updates. Local session usage takes precedence, with existing
current-server priority, permission and Nitro restrictions. Unknown or unavailable
custom emoji are skipped; defaults fill only unused slots. The schema reference is
[`FrecencyUserSettings.proto` at `8fabce4`](https://github.com/discord-userdoccers/discord-protos/blob/8fabce4ed1e59b81ce27346f471174c7a1434b9e/discord_protos/discord_users/v1/FrecencyUserSettings.proto).

Author membership reads follow visible timeline rows instead of competing with a
newest-message batch. Bounded prior lookup results remain available while another
batch loads, and subsequent messages reuse those guild nicknames. Loaded member
metadata refreshes matching cached labels, including nickname removal, so moving
the sidebar away cannot restore an older name. It takes precedence over embedded labels.
Offline fixtures verify these paths; live account synchronization remains unverified.

Message and link context menus share ordered reaction, message, utility, destructive and
identifier groups. Link actions retain the exact clicked URL, including signed parameters.
Image cards and viewers use the shorter copy/save/media-link/message-ID menu. Message identity,
session generation and access are checked again before actions; toolbar overflow retains
Mark Read Through Here, View Reactions, Publish and extension actions.

Reminders reuse saved messages. Custom local times use the selected timezone, reject DST
gaps and require an explicit offset for folds. Apps reuses the received message-command index
and existing invocation checks. Interaction details show only received identifiers, user and
command information. Speak Message is local system speech; it excludes unrevealed spoilers,
resolves known mentions and stops after conversation/account changes or lost access.

Reporting follows the service-provided reason graph from the **unofficial**
`GET /reporting/menu/message` interface. Only an explicit final Submit posts to the fixed
`POST /reporting/message` endpoint; the menu cannot choose another submission URL. Forms are
session-only, expire after ten minutes and cap wire/retained data at 256/512 KiB, nodes at 256
and navigation at 64 steps. Checkbox, dropdown, plain-text and preview elements are supported;
unsupported validation/forms offer Open in Discord. An uncertain submission is never retried
automatically. Closing the flow cannot undo an already submitted report.

References: [Discord reporting guidance](https://discord.com/safety/360044103651-reporting-abusive-behavior-to-discord)
and [unofficial reports reference](https://docs.discord.food/topics/reports). Offline loopback,
decoder and state fixtures exercise confirmation, cancellation, stale responses and failures.
They submit no live reports and do not establish current service acceptance.

## Live member identity synchronization — October 28, 2026

The unofficial normal-account Gateway supplies partial member identities in
`GUILD_MEMBER_UPDATE`, `PASSIVE_UPDATE_V2` and `TYPING_START`. These now refresh
nicknames and assigned roles across the selected guild's loaded member rows,
mention results, messages and typing labels. Missing fields preserve known values;
explicit null nicknames and empty role lists clear them. Member removal retires
mention candidates. Permission authorization still uses the separate strict
permission snapshot, never this display cache.

Resolved mention results and visible authors receive sparse opcode 37 `members`
subscriptions, capped at 128 IDs, through the existing rate-limited writer.
They activate the guild's typing subscription even with the sidebar closed, as
required for member updates in large guilds. Range updates preserve those
subscriptions; guild changes retire them and RESUME restores them. Replayed
identities arriving before RESUMED remain available afterward. See the unofficial
[member update delivery rules](https://docs.discord.food/topics/gateway-events#guild-member-update).
No full directory is requested. Live identity observations are session-only,
capped at 128 entries / 128 KiB and charged to the resident-memory budget. Delayed
member queries cannot overwrite newer observations; account changes and fresh synchronization
retire the display cache.

The composer retains resolved identities while a new query is pending or cooling
down, includes visible-author results, and accepts spaces in nickname searches.
The typing indicator uses the supplied guild nickname even with the sidebar
closed. Synthetic WebSocket → decoder → reducer and UI fixtures cover these
paths; real account/server synchronization remains owner-controlled and unverified.

## Gateway recovery and shared application audio — October 26, 2026

Gateway recovery now retains bounded, fixed-label connection causes for the existing
Copy Issue Diagnostics action. The session-only report distinguishes service-requested
reconnects, close codes, heartbeat/probe expiry, socket failures and local admission limits.
It contains elapsed timings and counters, never credentials, payloads or account identifiers.
The report does not identify the cause of an owner-reported dropout until that event has
actually been observed.

Outgoing Gateway requests share bounded admission and a serialized writer, including the
[documented connection-wide rate limit](https://docs.discord.com/developers/events/gateway#rate-limiting).
Heartbeats and critical controls retain capacity while replaceable subscriptions, presence
and member queries coalesce. Per-operation RATE_LIMITED handling and normal-account member
query cooldowns follow [unofficial protocol documentation](https://docs.discord.food/gateway/using-gateway#rate-limiting);
these are not normal-account acceptance evidence. Server reconnects receive an immediate
RESUME attempt before repeated failures back off. Subscription retries retain their
increasing delay, and acknowledgement deadlines begin after actual transmission.
Buffered acknowledgements receive a bounded drain opportunity at expiry; continuous
dispatch traffic cannot extend it indefinitely. Recovery does not increase the heartbeat
timeout. TCP_NODELAY avoids delaying small heartbeat/control writes behind buffered data.

Shared audio follows the approved video source. Windows window capture uses WASAPI
include-process-tree capture for a validated window owner; monitor capture excludes
Serein from system audio. The existing API requires Windows build 20348 or newer
([Microsoft sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/)).
On macOS, ScreenCaptureKit's approved single-window filter includes that application's
audio, including its other windows ([Apple WWDC22 session 10155](https://developer.apple.com/videos/play/wwdc2022/10155/)).
This is application isolation, rather than individual browser-tab audio selection.

Linux preserves the portal's source type. Explicit monitor and X11-desktop sharing retain
the existing PulseAudio/PipeWire-Pulse mixer with Serein excluded. Window or unknown portal
sources continue video-only because the current portal contract does not identify the
window's owning application. Native audio initialization/isolation failure also continues
video-only with a visible explanation, without changing microphone processing or encoder
quality. Retired audio and queued samples remain fenced by the existing share/security
generation. Native cross-application isolation and live Gateway recovery remain subject
to owner-controlled acceptance.

## Bug-hunt coverage: November 16, 2026

These changes are relative to `76d038ad`. Complete member snapshots clear removed
nicknames and roles on cached messages and prevent delayed history from restoring old
identities. A frozen Force Sync request tolerates newer live traffic while preserving
overlapping edits and deletes. Gateway heartbeat tracking accepts the matching ACK that
arrives while its write is still completing; acknowledgement timeouts are unchanged.

Type 2 favorite-GIF settings updates invalidate the bounded picker projection while
preserving pending local writes. Channel suggestions filter inaccessible channels and
revalidate access when clicked. Search retains its selected channel within the 500-channel
option cap. Quick server mute requires an explicit Refresh when its settings baseline is
unknown.

Local camera preview skips video encoding. Linux platform dependencies explicitly enable
the existing X11 and Wayland window backends so isolated crate builds have the required
features. Extension catalog publication uses the publishing repository for commit-pinned
package and preview URLs. Package publication resolves that repository's configured Pages
root, including custom domains, while retaining an explicit URL override. Windows installer
guidance now names media-v5 archives and the bundled portable notification script.

Regression scenarios use synthetic fixtures and temporary local repositories. No live
Discord session, account mutation, message, call, microphone or native device acceptance was
performed for this coverage. The review also examined administration, extension runtime
and storage paths without demonstrating another fix there. Coverage is limited to the
examined paths and scenarios. AMF/Nix work tracked in PR #43 is separate.

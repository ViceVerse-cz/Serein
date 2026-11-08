# Catalog plugins

The published plugins, built against the one [SDK](../sdk) that
the [authoring guide](../../examples/extensions/README.md) documents. Any language
producing compatible WebAssembly can use this ABI; Rust authors can call
`serein_extension_sdk::export!(handler)`.

Build and package from `extensions/`, the workspace root (Python 3 is used only by the author):

```powershell
rustup target add wasm32-unknown-unknown
cargo build --locked --release --target wasm32-unknown-unknown
python pack.py plugins/message-delete-protector/manifest.json target/wasm32-unknown-unknown/release/message_delete_protector.wasm plugins/packages/message-delete-protector.serein-extension
```

A rebuilt module is not a release by itself: committed packages are the reviewed bytes
clients install. Replace one only with a reviewed rebuild and a bumped `manifest.version`.

Import the package in Settings > Extensions, review the capabilities, and enable it.
Message delete protector is an example activation plugin. Its `activation` action returns
`preserve_deleted_messages: true` after the user grants `deleted_messages`. The host
keeps already-loaded messages in bounded session memory and displays deleted text in red
by default. Hover and a local context menu can toggle that highlight or remove the
retained row. They never call Discord. The host never sends message bodies to the plugin,
saves deleted bodies to disk, restores messages deleted before loading, or gives deleted
messages live service actions.
Disabling, logout, permission revocation and timeline eviction release retained content.
Emoji and sticker image fallback is now built into the client; the former Emoji &
Sticker Images plugin is retired. Native eligibility uses the account's Nitro
entitlement. Unavailable native selections enter the composer as named artwork
links; explicit Send uses Markdown with text, or attachments for artwork alone.
Declarative themes are under `../themes/`.
The author packages compiled bytes; Serein never runs a repository's build scripts.

## Equicord-derived safe tools

These independent Rust adaptations use the same real Wasm sandbox and versioned SDK as the
original plugins; they do not evaluate Equicord JavaScript or add automatic send hooks. The 31
packages below map 61 bounded upstream workflow subsets. The authoritative inventory also maps 37
native subsets, for 98 explicit mappings across 373 audited directories; the remaining directories
are internal, already covered, blocked, unsuitable or still candidates, not compatibility claims.

| Package / upstream workflows | Grants | Behavior |
| --- | --- | --- |
| **Offline Composer Tools** — ClearURLs, Unindent, UnitConverter, MoreCommands, CharacterCounter | `composer` | Five invoked draft/count tools. URL cleanup uses a fixed offline tracker list; units and text transformations are deterministic; nothing is fetched or sent. |
| **Text Commands** — TextReplace, CustomCommands | `composer`, `storage` | Up to eight literal replacement rules and eight local message tags. Configuration stays bounded and corrupt storage is preserved. Composer edits require Apply. |
| **ViewRaw — safe message view** — ViewRaw | `selected_message` | Displays the selected raw text and public message IDs only; never exposes raw private host objects. |
| **External Search Tools** — Translate, ReplaceGoogleSearch | `selected_message`, `external_navigation`, `storage` | Configures a browser translation/search provider and target language; proposes a complete public HTTPS URL for Apply. No inline network translation, arbitrary request API, or silent navigation. |
| **Text Codecs** — BaseDecoder, WriteUpperCase | `composer`, `selected_message`, `clipboard_write` | Decodes standard Base64 spans in drafts or up to eight selected-message matches with strict UTF-8, and capitalizes sentence starts with upstream blocked-word behavior. Every replacement or combined copy requires Apply. |
| **Message Composition Tools** — SplitLargeMessages, BlockKeywords, MessageBurst | `composer`, `app_context`, `timeline`, `channel_details`, `message_details`, `message_manage`, `storage` | Reviews bounded draft segments, filters whole keyword lines, reviews burst dispersion, or merges a draft into the latest loaded ordinary own message with attachment opt-in. No automatic multi-send, timeline filtering or edits. |
| **Identity Tools** — CopyUserURLs, CopyUserMention | `selected_message`, `clipboard_write` | Copies a selected author's canonical profile URL or mention after Apply. Uses only the validated public author ID. |
| **Loaded Role Members** — ClickableRoles, InRole, BetterRoleContext, BetterRoleDot | `member_details`, `storage`, `navigation`, `clipboard_write` | Filters at most 20 loaded members by a loaded role and proposes a profile opening or canonical nonzero role-color copy after Apply. No guild-wide fetch, count, or guessed black. |
| **Status Presets** — StatusPresets, CustomStatusTimeouts | `account_control`, `storage` | Stores at most 32 named presence presets and proposes one status change after Apply. Expiry is bounded to one day; no background timers. |
| **Message History** — MessageLogger / MessageLoggerEnhanced | `message_events` | Reviewed consent enables bounded, persistent account-scoped history for subsequently loaded readable chats, attachment metadata/opt-ins, ghost alerts and bounded export. No deleted-before-load recovery. |
| **Message Reference Tools** — FindReply, GoogleThat, Husk | `selected_message`, `timeline`, `message_details`, `navigation`, `external_navigation`, `reactions_control` | Finds loaded reply context, proposes a reviewed Google search, or adds one Husk reaction after Apply. No history fetch or automatic writes. |
| **Draft Style Tools** — TalkInReverse, Signature, PolishWording | `composer`, `storage` | Reverses unprotected Unicode text, appends a bounded signature once, and applies deterministic prose cleanup after Apply. Never sends automatically. |
| **Command Palette** — CommandPalette | `app_context`, `channel_directory`, `guild_directory`, `read_state`, `navigation`, `storage` | Filters loaded channels and proposes fixed native navigation/search actions. Stores up to 16 pins; no arbitrary commands or global hotkey. |
| **Own Message Sed** — SedEnhanced | `app_context`, `timeline`, `message_details`, `message_manage` | Applies a literal first/global replacement only to a loaded own message after Apply. Empty results delete only when no attachments are confirmed. No arbitrary regex or other-author edits. |
| **Social Link Tools** — PendingFriendRequest, NormalizeMessageLinks, RepeatMessages | `relationships`, `relationship_control`, `selected_message`, `clipboard_write`, `message_send` | Cancels one loaded outgoing friend request, normalizes copied canary/ptb links, or proposes a repeat send/explicit reply with neutralized mentions and reply mention disabled after Apply. No attachments/stickers or automatic writes. |
| **Ingtoninator** — Ingtoninator | `composer` | Appends a case-preserving Ington suffix to one deterministic eligible draft word outside protected URL/code/token spans. Apply required. |
| **Voice Panel Tools** — VcPanelSettings | `audio_settings` | Proposes only changed output gain (0–200%) and input gain (0–100%) after Apply. Existing input gain above 100% is shown read-only and never silently lowered. |
| **Server Info Tools** — ServerInfo | `guild_directory`, `channel_directory`, `relationships`, `storage` | Shows bounded loaded guild/channel summaries and account-wide relationship counts. Does not fetch or invent ownership, membership, boost, vanity, image or joined-date metadata. |
| **Equicord Community Tools** — ExportMessages, HopOn, FastDeleteChannels, IgnoreCalls | `app_context`, `channel_control`, `clipboard_write`, `external_navigation`, `selected_message`, `storage`, `voice_connect` | Copies one selected message as bounded text, checks up to eight literal HopOn triggers, or proposes one permission-checked channel deletion or ignored-call decline after Apply. No background launch/delete/decline. |
| **Loaded Permissions Viewer** — PermissionsViewer | `channel_metadata`, `member_details` | Shows the current account's loaded channel decisions and one loaded member's role order. It does not derive another member's effective permissions or fetch masks/members/roles. |
| **Plus Reacts** — BetterPlusReacts | `composer`, `timeline`, `reactions_control` | Uses leading plus signs to select at most 50 loaded-message positions and proposes one add reaction after Apply. The draft and existing reactions are otherwise unchanged. |
| **Message Inspector Tools** — RichMagnetLinks, ShowRolesInChat | `selected_message`, `member_details`, `clipboard_write`, `storage` | Parses bounded magnet links for reviewed copying or inspects the selected author's highest loaded role with up to 64 exclusions. No inline chat renderer or media download. |
| **Voice Rejoin Tools** — VoiceRejoin | `app_events`, `voice_state`, `channel_directory`, `voice_connect`, `storage` | Remembers one valid active channel and proposes a manual muted/no-ring rejoin after Apply. No retry loop, timer, automatic join or microphone activation. |
| **Account & Navigation Tools** — ProfileSets, RecentDMSwitcher, RandomVoice | `app_events`, `app_context`, `account_profile`, `channel_directory`, `account_control`, `navigation`, `voice_connect`, `storage` | Stores up to 16 text-only profile presets and 20 DM IDs, or rotates deterministically through loaded voice/stage channels. Profile changes, navigation and joins are explicit Apply proposals; the rotation is not random. |
| **Message Navigation Tools** — JumpTo, LastActive | `app_context`, `timeline`, `channel_details`, `selected_message`, `navigation` | Proposes jumps to latest/oldest loaded messages or a selected/current author's loaded messages in a 50-row window. No history/search fetch or guild-wide scan. |
| **Music Link Tools** — SongLink.desktop | `selected_message`, `external_navigation` | Locally recognizes bounded Spotify, Apple Music, YouTube Music and TIDAL links and proposes a Songlink/provider HTTPS page after Apply. No resolver API, metadata card, provider-native URI or send. |
| **More Quick Reactions Tools** — MoreQuickReactions | `timeline`, `reactions_control`, `storage` | Configures 0–42 emoji and selects from 20 loaded messages for one Apply-reviewed add reaction. It is an explicit palette, not a hover-bar renderer or frecency picker. |
| **Linked Message Tools** — MessageLinkEmbeds | `selected_message`, `timeline`, `navigation` | Parses up to five exact Discord message links, previews only targets already in the loaded timeline, and proposes one jump after Apply. Never fetches linked content or media. |
| **Greet Sticker Tools** — GreetStickerPicker | `selected_message`, `message_content`, `message_send` | Revalidates and proposes re-sending one sticker already loaded on the selected message after Apply. No greet catalog, automatic send or welcome-panel injection. |
| **Better Search Tools** — BetterSearch | `navigation` | Builds one bounded query from validated extra token shapes and proposes native search after Apply. Discord support for those extra tokens is explicitly unverified. |
| **Voice & Chat Tools** — VoiceButtons, VoiceChannelLog, VoiceChatUtils | `voice_state`, `app_events`, `audio_settings`, `relationship_control`, `relationships`, `members`, `account_profile`, `composer`, `storage` | Proposes local participant audio/friend-DM actions, keeps a 256-entry current-call sequence log, or appends current-call mentions to the draft. No server moderation, cross-channel log, timestamp fabrication or automatic send. |

Original Vencord notices and GPL-3.0-or-later terms are retained in the sources and
`../licenses/Equicord-GPL-3.0.txt`. See each plugin README for exact behavior and source commit.
Every Equicord-derived package below is also built with
`--license ../licenses/Equicord-GPL-3.0.txt` (from the repository root), which embeds the full text
in an inert `serein.license` Wasm custom section. Package building never executes that section, and
`test_catalog.py` requires it for each port. Packaging is the same as above; substitute the
matching lowercase ID, for example:

```sh
python pack.py plugins/offline-composer-tools/manifest.json target/wasm32-unknown-unknown/release/offline_composer_tools.wasm plugins/packages/offline-composer-tools.serein-extension
python pack.py plugins/text-commands/manifest.json target/wasm32-unknown-unknown/release/text_commands.wasm plugins/packages/text-commands.serein-extension
python pack.py plugins/view-raw-message/manifest.json target/wasm32-unknown-unknown/release/view_raw_message.wasm plugins/packages/view-raw-message.serein-extension
python pack.py plugins/external-search-tools/manifest.json target/wasm32-unknown-unknown/release/external_search_tools.wasm plugins/packages/external-search-tools.serein-extension
python pack.py plugins/text-codecs/manifest.json target/wasm32-unknown-unknown/release/text_codecs.wasm plugins/packages/text-codecs.serein-extension
python pack.py plugins/message-composition-tools/manifest.json target/wasm32-unknown-unknown/release/message_composition_tools.wasm plugins/packages/message-composition-tools.serein-extension
python pack.py plugins/identity-tools/manifest.json target/wasm32-unknown-unknown/release/identity_tools.wasm plugins/packages/identity-tools.serein-extension
python pack.py plugins/loaded-role-members/manifest.json target/wasm32-unknown-unknown/release/loaded_role_members.wasm plugins/packages/loaded-role-members.serein-extension
python pack.py plugins/status-presets/manifest.json target/wasm32-unknown-unknown/release/status_presets.wasm plugins/packages/status-presets.serein-extension
python pack.py plugins/message-history/manifest.json target/wasm32-unknown-unknown/release/message_history.wasm plugins/packages/message-history.serein-extension
python pack.py plugins/message-reference-tools/manifest.json target/wasm32-unknown-unknown/release/message_reference_tools.wasm plugins/packages/message-reference-tools.serein-extension
python pack.py plugins/draft-style-tools/manifest.json target/wasm32-unknown-unknown/release/draft_style_tools.wasm plugins/packages/draft-style-tools.serein-extension
python pack.py plugins/command-palette/manifest.json target/wasm32-unknown-unknown/release/command_palette.wasm plugins/packages/command-palette.serein-extension
python pack.py plugins/ingtoninator/manifest.json target/wasm32-unknown-unknown/release/ingtoninator.wasm plugins/packages/ingtoninator.serein-extension
python pack.py plugins/own-message-sed/manifest.json target/wasm32-unknown-unknown/release/own_message_sed.wasm plugins/packages/own-message-sed.serein-extension
python pack.py plugins/social-link-tools/manifest.json target/wasm32-unknown-unknown/release/social_link_tools.wasm plugins/packages/social-link-tools.serein-extension
```

## ABI and limits

See [ABI version 1](../../examples/extensions/README.md#abi-version-1) in the SDK guide
for exports, input/output fields, panels and sandbox limits.

## Publishing

Add or update a package through a pull request; see the [extensions README](../README.md).
Maintainers must review the source and built artifact together before listing a version.
All versions and updates require explicit user consent.

## Custom Rich Presence (preview)

See [Custom RPC](custom-rpc/README.md) for setup and builds. Requires the preview
rich-presence SDK from Serein PR #465. Install through the shared catalog or import
the package locally. Older clients without this capability cannot install it.

## API Proxy (preview)

See [API Proxy](api-proxy/README.md) for the native REST-only proxy editor.
Requires the API proxy SDK capability; Gateway, CDN/media and calls stay direct.

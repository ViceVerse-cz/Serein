# Authentication and owner-controlled live validation

Large accounts: READY no longer shares the 4,000-entry account ceiling or the 4 MiB
ordinary-event queue limit. A bounded startup path retains all admitted navigation and
permissions, while failed optional read-state/settings/presence/emoji sections produce a
feature warning without aborting valid login. Account identity, relationships, session ID,
resume address, navigation and permissions remain required and validated. Safety-budget
failures still stop with a static explanation; neither an unlimited payload nor guaranteed
interoperability is promised. See [storage policy](storage-policy.md#large-account-startup-september-14-2026)
for component limits. The reported issue #154 build and live trigger are not yet verified.

Linux uses GTK4/WebKit6 with a fresh ephemeral NetworkSession and persistent credential
storage disabled. Normal TLS validation remains enabled. Scripts run at document start only
in the top Discord frame. Navigation permits HTTPS hcaptcha.com and its subdomains on
port 443 for embedded challenges, using the same origin validation as invite verification.
The main-document response and candidate origin checks still restrict login to
https://discord.com. Website-data access requests are allowed only for hcaptcha.com
or its subdomains embedded in discord.com, while the main page is still Discord.
The invite-verification view applies the same permission rule while its exact local
verification page (`https://discord.com/channels/@me`, never fetched) is open.
This uses the existing ephemeral sessions and does not enable persistent storage.
It removes a possible challenge-state blocker; live acceptance and the reported Linux
QR/CAPTCHA loop remain unverified. Popups, downloads, file choosers, other permission
requests, HTTP-auth, notifications and printing are denied; embedded challenge
availability remains unverified.

WebKit6 script-message callbacks lack trusted sender-frame metadata. The callback accepts
only a boolean wake signal. A protected main-frame closure retains one ASCII candidate of at
most 2113 bytes (65-byte capability plus 2048-byte token). A native main-frame query checks
origin and result bounds before creating a Rust string; Rust checks URI, capability, lifetime
and SessionSecret validation again. Linux WebKit also checks the temporary webview's bounded,
same-origin Discord API requests for an Authorization header, without logging its value.
Queries are at least 100 ms apart, with one cancellable evaluation and one secret slot. A child
frame can only request a query of the main frame. The Linux login and verification webviews
disable HTML media and Web Audio: a machine without a GStreamer audio sink otherwise crashes
the web process right after login, which surfaces as a distinct "stopped unexpectedly" status.
The Linux login webview retains a WebKit/Safari identity. Earlier claims that an
engine/user-agent mismatch necessarily causes challenge rejection or restrictions
were unverified.

**Transport identity (unofficial, live-unverified).** Native REST, uploads,
Gateway and voice WebSockets, and Discord CDN downloads use a shared desktop-style
profile from `client_core::fingerprint`: `Discord Client`, Electron 42.7.1 and
Chromium 148, with the Chromium 148 wreq/BoringSSL transport profile. The paired
engine/profile versions remain SakuraCord reference constants. Host
versions are platform-specific manifest observations (Linux 1.0.169, Windows
1.0.9268, macOS 0.0.422), and web build 670026 comes from the public `/app` bundle
on November 12. These do not establish a complete current Electron identity.
Super-properties retain some web fields.
REST also carries the Discord-issued installation ID as `X-Installation-ID`, and the Gateway identify
carries it as `installation_id`; with Discord's own cookies these keep one device across launches
(see [storage policy](storage-policy.md#discord-device-state-november-13-2026)). Captchas for
joins, DMs, friend requests and messages are offered in Serein with a warning that completing them
in the official client is safer: a new account reported a phone-verification request after one
was solved here (cause unverified).
See [compatibility notes](discord-compatibility.md#desktop-metadata-reference--november-12)
for the partial profile and omissions.

Temporary login and CAPTCHA views retain separate engine identities: Safari 26.4
on macOS/Linux WebKit and Chrome 149 on Windows WebView2. Native submission uses
the desktop profile. The interoperability and account-risk effects of this
difference are unknown. These profiles were not validated against current
Electron/browser captures and do not prove Discord accepts the client or that
enforcement stops.

Locale/timezone come from the OS with en-US/UTC fallbacks. Startup sends no
`GET /users/@me` or `GET /gateway`; accepted READY supplies the account identity.
Settings writers wait for that accepted identity, so saved-login account checks
complete before any presence, idle-timeout or activity-sharing settings write.
Hosts other than Discord (updater, extension downloads, external uploads, third-party
media) get no emulation; the updater and external uploads send a `Serein/x.y` user agent. Certificates are verified against the Mozilla
roots bundled with wreq, not the operating system trust store, so a TLS-inspecting proxy
whose root is installed only in the OS is rejected.

Close/drop invalidates pending results, clears the secret/scripts/handler, cancels evaluation,
stops loading, terminates the ephemeral web process and destroys the GTK window. GLib pumping
checks a 2-ms deadline between at most 16 callbacks; one native callback may exceed that time.
The login and verification views retain their GDK display while the window is live,
so flushing does not look up a display through a window already destroyed by a close event.
The nonblocking pump explicitly flushes GDK window requests. Teardown also flushes after
destroying the window, since successful handoff and cancellation stop the pump.
The earlier login-only fix had offline native Wayland validation: a stuck window after
a synthetic token handoff was reproduced and then confirmed to disappear with flushing.
Live Discord login remains unverified. The same flushing is applied to invite verification;
its reported X11 freeze and timeout still require testing on the affected Linux device.
These are implemented limits, not measured teardown/storage or live login compatibility.

Serein uses Discord’s official login page in a temporary platform webview, not OAuth. The credential handoff is unofficial and live-unverified; see the compatibility matrix. Complete authentication yourself, in the application. Never send passwords, tokens, MFA codes, QR screenshots, or private message contents to the coding agent, issues, logs, or CI.

1. Build `cargo run --locked` on a supported platform. Use a private conversation controlled by the account owner. The owner enables the private-test acknowledgment and presses **Sign in with Discord**.
2. Complete one of the real login methods available in Discord’s page. Do not bypass a challenge, and do not change the webview's engine identity or the identity described above to get past a rejection. Cancel if the page or handoff is unsupported. The webview expires after ten minutes and closes when it supplies a candidate token.
3. Native REST verifies `/users/@me`, rejects a bot account, retrieves the gateway location, and waits for normal-user READY. A socket opening is not authentication success. The token is saved in the OS credential store only after readiness; if a keyring exists but saving fails, the UI reports session-only login. On a desktop without any OS credential store (Linux with no Secret Service provider such as GNOME Keyring, KWallet or KeePassXC), the login is saved to a bounded, private, unencrypted file instead and the post-login status says so; see [storage policy](storage-policy.md). Expiry cleanup does not report a removal failure for a store that does not exist. When no Secret Service answers, Serein D-Bus-activates `org.kde.kwalletd6`/`kwalletd5` (KWallet is often not activatable as `org.freedesktop.secrets` outside Plasma) and retries the store at most every 15 seconds rather than for the rest of the run; KWallet's Secret Service integration must be enabled (unverified on a live KDE desktop). `AUTH_SESSION_CHANGE` only updates the auth-session hash and does not end the session; revocation is HTTP 401 or Gateway close 4004.
4. Choose the existing private channel/DM. Load one 50-message page. Deliberately compose and send one short test message. Verify it appears in an official Discord client. Reply from that official client and verify the reply appears natively through Gateway. Do not count an offline fixture, matching text, or a bot reply as success.
5. In the same conversation, verify an edit, deletion, HTTP/Gateway confirmation ordering, permission rejection if available, disconnect/resume and non-resumable reload. Keep traffic small; run stress tests only against synthetic transports.
6. Quit after local saves finish. Relaunch and verify credential-store restoration and saved draft recovery without opening the webview. Inspect recovered drafts before sending: an interrupted send can have succeeded remotely.
7. Log out and verify saved-login deletion and local account cache/draft cleanup. The UI must show credential-store or SQLite deletion failures. Local logout does not claim remote session revocation.

Record only date, OS/build, methods tested, pass/fail and redacted failure category in the task PR description. Never record credentials, account/channel IDs, signed URLs, message contents or QR data. **No real owner-controlled session was supplied or exercised during implementation; the live milestone remains blocked.**

Every build's sign-in screen also offers "Sign in with a token": the owner pastes a session token they already hold, for example from another signed-in Serein install. It skips Discord's hosted login page entirely, still requires the owner-authorization checkbox, still validates through `SessionSecret::from_owner_input` (which drops surrounding whitespace and one pair of surrounding double quotes, as browser storage displays the value), and is saved to the OS credential store the same way a normal login is. It never reads or extracts a credential from another application.

Saved-login startup reports credential lookup separately from Discord connection. A found credential advances the status immediately; absent/invalid/unavailable outcomes remain visible. The UI stops awaiting lookup after 10 seconds and permits manual hosted login. Manual login, preview, logout and timeout discard late lookup results. Native login credential IO uses one bounded worker (one active job and one queued job), with a five-second wait per operation. Expired queued work is skipped; a native operation already in progress may finish later. Plaintext IO and cleanup continue on the credential coordinator after that wait. Empty authority markers prevent late or older OS writes from reviving a deleted plaintext login or a logged-out session. Plaintext fallback applies only when no OS credential store exists; proxy credentials remain OS-store-only.

Gateway login keeps optional voice identity metadata within 4,096 entries / 1 MiB per cache.
Extra referenced users or merged members are omitted from that cache; eligible voice participants
retain their IDs and use the existing fallback when their name/avatar is unavailable. Previously,
crossing either cache budget could stop login before READY, even with no voice participants.
Offline WebSocket regressions cover 4,097 referenced users, UTF-8 names crossing the byte budget,
and 4,097 combined supplemental members. This reproduces one cause of issue #143; the affected
reporter's actual account payload has not been inspected or tested.

Large READY payloads (September 14): a normal account in many servers receives a READY whose
decompressed JSON exceeds the 4 MiB per-event bound because every joined server ships its
channels, roles, emojis and settings inline. The old 4 MiB Gateway limit reported
"Decompressed Gateway payload exceeds 4 MiB; connection stopped" and ended login. The WebSocket
frame, zlib-stream compressed/decompressed accumulation, outer Gateway packet, READY and
READY_SUPPLEMENTAL decoders now share a 64 MiB bound (`MAX_GATEWAY_WIRE`). Individual dispatch
events and REST responses keep their existing limits. Navigation and permission storage now
use the larger, separate account budgets above; voice rosters and presence caches remain bounded
at their existing sizes. An offline WebSocket regression logs in with a READY between
4 and 64 MiB. The reporter's actual payload size has not been inspected.

Hard Gateway frame/compressed/decompressed payload, navigation, actual voice roster, and desktop
synchronization queue limits remain enforced. Their capacity failures identify the limit using
fixed local text and still terminate the connection. An oversized frame stops immediately during
Hello or subsequent startup instead of being retried as a network failure. No payload contents,
account identifiers or credentials are added to the error. Credential saving still requires READY.

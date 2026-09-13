# Invite verification

An invite join can require a human-completed hCaptcha. Supported responses now keep
the existing account connected and open a native **Verification required** dialog.
The invite card offers **Verify** instead of a disabled Join button and a clipped
session error. The dialog shares the application's colors, typography, focus rules
and cancellation controls; the provider widget uses its light or dark theme.

Choose Verify, complete the provider's check, and Serein submits the resulting
passcode once for the same invite. A new challenge requires another explicit
verification. Cancellation, expiry, logout and replaced requests invalidate the old
response. Gateway membership still determines server access; a CAPTCHA passcode
alone never grants it. Unsupported invite CAPTCHA responses fail that join without
stopping the session. Authentication and account-level challenges retain the existing
stop behavior.

The sitekey is taken from the service response, never hardcoded. The challenge's
enterprise data, request token and session ID remain bound to that attempt. The
verification view has its own incognito context, an app-local reserved
origin and a random IPC capability. It receives no Discord account token and uses
no solver, browser-profile access, origin spoofing, fingerprint override or backend.
The widget's generated passcode is not saved to SQLite or diagnostics. The browser
engine and hCaptcha necessarily process the user's interaction; this is not a claim
that a third-party widget or OS leaves no traces.

One challenge lives for at most five minutes. Field limits are 128 bytes for the
sitekey, 4 KiB for enterprise data, 2 KiB for the request token, 512 bytes for the
session ID and 8 KiB for the solution. Challenge and solution Debug output is
redacted. The webview closes after its one-slot result is consumed or its scope is
cancelled. Discord can reject a passcode or this embedding environment; errors stay
visible, with no automatic challenge-solving or retry loop.

Implementation evidence checked September 13, 2026:

- [hCaptcha configuration](https://docs.hcaptcha.com/configuration): public sitekey,
  widget themes and completion/error/expiry callbacks.
- [hCaptcha native SDK](https://github.com/hCaptcha/react-native-hcaptcha):
  service-supplied enterprise verification data via `setData`.
- [discord.py-self HTTP implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py)
  and [error parsing](https://github.com/dolfies/discord.py-self/blob/master/discord/errors.py):
  challenge fields and `X-Captcha-Key`, `X-Captcha-Rqtoken`,
  `X-Captcha-Session-Id` headers. This is unofficial protocol evidence, not a
  documented Discord third-party client contract.

Windows and macOS embed verification; Linux opens a temporary GTK4/WebKit6 window. The owner reported a successful
manual live check on September 13, 2026; this is not an agent-observed interoperability
test. macOS/Linux live verification remains unverified. Linux uses an ephemeral network
session and a bounded main-frame query, with no cross-frame passcode IPC. Offline fixtures never
contact hCaptcha or join a server.

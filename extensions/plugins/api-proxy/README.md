# API Proxy (preview)

Configure an HTTP/HTTPS forward proxy for Discord REST API requests
through Serein's native extension panel. Gateway/WebSocket traffic, CDN/media and
voice calls remain direct. This is not a way to proxy calls.

1. Install the reviewed package and grant **API proxy** and **storage** capabilities.
2. Open **Configure API proxy**, including on the login screen before connecting.
3. Choose **Direct**, **Automatic**, or **URL**. Automatic uses environment proxy
   settings (no PAC). URL accepts a host and optional port, for example
   `http://127.0.0.1:8080`; credentials, extra paths, queries and fragments are rejected.
4. **Apply API proxy** validates, saves and submits the setting. Editing a draft or
   reopening the panel does not change the connection. Saved settings resume on
   activation. Invalid saved settings cause an activation error rather than a
   silent switch to Direct; explicitly disable and re-enable the plugin to remove
   invalid saved settings, then configure and Apply the proxy again.
   Disabling the plugin restores Direct mode.
5. For HTTP Basic proxy authentication, expand Serein's host-managed **Proxy authentication**
   section, enter a username and masked password, then **Save credentials**. Apply the URL
   first. Serein stores credentials only in the OS credential store and attaches them only
   to that exact proxy origin; the plugin never receives them. **Remove saved credentials**
   erases the OS entry. Direct mode and plugin disable stop using credentials but retain
   the OS entry. Automatic mode does not support credential-bearing environment URLs.
   A credential-store failure has no plaintext fallback.

This is a global connection setting, not an account preference. The proxy can see
connection destinations, and an HTTP forward proxy sees plaintext HTTP requests;
Serein's Discord API requests use HTTPS. The Wasm plugin performs no network or
filesystem operations and never receives account credentials. Dependency attribution
and licenses are preserved in [third-party notices](THIRD_PARTY_NOTICES.md).

Preview capability: requires the API Proxy host pull request; older clients reject
this package. Native/real-proxy interoperability remains a separate verification
from the synthetic checks.

Build from `extensions/`:

```sh
cargo test --locked -p api-proxy
cargo build --locked --release --target wasm32-unknown-unknown -p api-proxy
python pack.py plugins/api-proxy/manifest.json target/wasm32-unknown-unknown/release/api_proxy.wasm plugins/packages/api-proxy.serein-extension
```

# Getting Started with a Rust Client Using the Official API

## First, choose what product you're building

Discord's official API is not an API for recreating the full desktop client for a normal user
account in a third-party Rust application.

- **A bot or bot management tool:** use a bot identity created in the Discord Developer Portal and
  documented Bot API/Gateway flows.
- **An OAuth connection on a user's behalf:** use the official OAuth2 flow and only the scopes the
  user granted. OAuth2 does not provide access to every feature in Discord's first-party clients.
- **A “self-bot” using a normal user token:** do not do this. Sending your own account token as if
  it were a bot token, imitating a private client protocol, or extracting a token from another
  application is not a supported approach and puts the account at risk.

Accordingly, the example below is only a small **official Bot API** starter that reads the identity
of its own bot. It does not recreate a Serein feature using a normal user account.

## 1. Minimal project

Create a Rust project and add `tokio`, `reqwest` (with the `rustls` and `stream` features), `serde`,
and `serde_json`.
Do not put the token in source code, the application package, logs, or error messages. During local
development, it can be read from the `DISCORD_BOT_TOKEN` environment variable; do not embed a bot
secret in a distributed desktop binary.

This example makes a documented `/users/@me` read over HTTPS, rejects redirects, and limits the
JSON body to 64 KiB:

```rust
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::Deserialize;
use std::{io, time::Duration};

const MAX_BODY: usize = 64 * 1024;

#[derive(Deserialize)]
struct BotIdentity {
    id: String,
    username: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var("DISCORD_BOT_TOKEN")?;
    if token.is_empty() || token.len() > 4096 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid token length").into());
    }

    let mut authorization = HeaderValue::from_str(&format!("Bot {token}"))?;
    authorization.set_sensitive(true);

    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?;

    let mut response = client
        .get("https://discord.com/api/v10/users/@me")
        .header(AUTHORIZATION, authorization)
        .send()
        .await?
        .error_for_status()?;

    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "response too large").into());
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "response too large").into());
        }
        body.extend_from_slice(&chunk);
    }

    let bot: BotIdentity = serde_json::from_slice(&body)?;
    println!("Connected as {} ({})", bot.username, bot.id);
    Ok(())
}
```

The example only demonstrates connectivity and deserialization. In a real application, consider
managing the token with a wrapper such as `Zeroizing`, never writing sensitive values in errors, and
never distributing a bot secret in a desktop product. Follow `Retry-After` and Discord headers for
rate-limit responses; do not automatically retry all write requests. Always use the
[official API reference](https://discord.com/developers/docs/intro) for the current contract.

## 2. Split the project into layers

A suggested small starter layout:

```text
src/
  app/          UI and user flows
  model/        application domain types
  protocol/     serde DTOs and validation
  api/          REST client and typed operations
  gateway/      WebSocket session, if using a bot
  state/        event reduction and session generation
  storage/      local settings and user data
```

Use a bounded command queue instead of making network calls directly from the UI. API/Gateway
tasks produce typed events; the UI thread only reduces state and renders. Associate each response
with an account/channel/request generation so that a stale response cannot overwrite the new view
after navigation changes. Keep network, file, image decoding, and audio work away from render/audio
callbacks.

## 3. Sound REST defaults

1. Pin the API origin; do not accept complete URLs from external input.
2. Set HTTPS verification, reasonable connection/overall timeouts, and a redirect policy.
3. Bound concurrent requests, response bytes, JSON array lengths, and the number of stored objects.
4. Handle `429` and `Retry-After`/rate-limit headers; do not try to bypass limits.
5. Distinguish safe/idempotent reads from writes with ambiguous outcomes. Do not blindly retry a
   message or operation write just because the server response was lost.
6. Parse only the DTO fields you need; report malformed or unexpected responses as explicit errors.
   Do not produce empty defaults that look like success.
7. Authorization is determined by the server. Role/permission checks in the UI are not a security
   boundary.

## 4. If you need the Gateway

For a bot Gateway connection, follow Discord's current official documentation: Hello/heartbeat,
Identify, requested intents, Dispatch sequence numbers, `READY`, `INVALID_SESSION`, Resume, and
close codes. Do not request privileged intents you do not need. Bound reconnect delays; do not
start a new Identify loop after every connection error. If event queues fill up, stop the session
in a controlled way or resynchronize instead of silently losing messages.

Do not copy the private/unstable Gateway behavior Serein uses for normal user sessions into this
bot flow. The existence of code or passing synthetic fixtures does not show that Discord supports
normal-account clients.

## 5. Validate offline first

- Exercise the UI, reducer, and error flows with synthetic data.
- Use a loopback/mock server for REST; test timeouts, invalid JSON, oversized bodies, 401/403, 429,
  and redirects.
- Test the Gateway parser with synthetic frames; do not make connecting to a live account the
  default test.
- Bound local caches, queues, and payloads by both bytes and item count.
- If testing with a real account is necessary, do so only in an explicitly authorized developer
  session controlled by the application owner, using small and reversible steps. Never put a
  password, token, MFA code, or private message in source code, logs, or CI.

## What you can learn from Serein

Serein's **layer separation, bounded queues, typed DTO/event flow, and synthetic testing approach**
can be studied. Authentication with a normal account, private endpoints, and client identity
spoofing are not a general-purpose starter kit.

- [Code flow](how-serein-works.md)
- [Discord compatibility classifications](../discord-compatibility.md)
- [Architecture and capacity limits](../architecture.md)
- [Credential storage policy](../storage-policy.md)
- [Official Gateway documentation](https://discord.com/developers/docs/events/gateway)
- [Official OAuth2 documentation](https://discord.com/developers/docs/topics/oauth2)

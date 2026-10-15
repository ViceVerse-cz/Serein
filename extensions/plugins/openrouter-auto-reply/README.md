# OpenRouter Auto Reply (preview)

An optional Serein plugin for automatic replies in selected individual DMs or group
DMs. Requires a host with the preview `openrouter_auto_reply` capability. The
plugin enables Serein's native editor and service; Wasm receives no messages,
settings or API key and has no network access.

1. Import `../packages/openrouter-auto-reply.serein-extension` in Settings >
   Extensions and grant its auto-reply permission. After merge, the catalog
   workflow can list this reviewed package; it is not available in released
   clients yet.
2. Open **Configure OpenRouter auto replies**. Enter your own OpenRouter API key,
   a model ID from your OpenRouter account, and optional reply instructions.
3. Search the loaded private conversations and check the people or group DMs you
   want. Open a DM in Serein first if it is missing. Up to 32 conversations can be
   selected; server channels are excluded.
4. **Save and start auto replies** saves the key and settings in the OS credential
   store, scoped to this Discord account. Saving must succeed before requests run.
   Reopen this tool and use **Pause now** to stop, or disable the plugin.

Each request sends only the incoming text and reply instructions to
[OpenRouter's chat completions API](https://openrouter.ai/docs/api/api-reference/chat/create-a-chat-completion)
and its chosen model provider. No conversation history, attachments, author IDs,
channel IDs, other DMs or Discord token are sent. Provider processing and charges
depend on your chosen model/account. This is unofficial Discord automation;
offline checks do not establish live service compatibility or account acceptance.

Replies are prefixed `[Auto reply]` and mention pings are suppressed. Own messages,
bots, webhooks, system messages, edits, deletes, history loads and messages older
than the latest start are not triggers. A manual send cancels pending generation
in that conversation. New messages while busy or cooling down are skipped, with
no backlog. There is one request at a time, at least six seconds between requests,
one per conversation per minute, and at most 100 attempts per enabled plugin session.
The service runs only while Serein is open, connected and explicitly started.

Pause, disconnect, logout, account changes, plugin disable/error and access loss
discard pending generation. The service starts paused after restart or sign-in;
disconnect also requires starting again. Errors pause it without automatic retry. Pause cannot retract a request or reply
already handed to OpenRouter or Discord.
Generated text is limited to 2,000 characters/8,000 bytes including the label;
unfinished responses are discarded. Requests time out after 30 seconds, with a
64 KiB decoded response ceiling and 512 completion-token request limit.

The API key is masked, excluded from widget undo history and kept in a zeroizing
buffer with redacted diagnostics. **Forget saved key and settings** removes the
account's credential-store entry. Disabling the plugin stops work immediately but
retains that entry for reinstallation. Keys/settings are never in plugin storage,
SQLite, exported packages or logs. An unavailable credential store leaves the
service paused. Instruction text is limited to 1,024 UTF-8 bytes; the complete
credential record must fit 2,400 bytes (shorten instructions if saving fails).

Build from the repository root:

```powershell
cargo build --manifest-path extensions/Cargo.toml --locked --release --target wasm32-unknown-unknown -p openrouter-auto-reply
python extensions/pack.py extensions/plugins/openrouter-auto-reply/manifest.json extensions/target/wasm32-unknown-unknown/release/openrouter_auto_reply.wasm extensions/plugins/packages/openrouter-auto-reply.serein-extension
```

# Making a Serein extension

This is a complete offline example and a small Rust SDK. Copy a plugin directory and the
SDK into your own repository, edit its manifest and action, and publish the resulting
`.serein-extension` file. Any language producing compatible WebAssembly can use this ABI;
Rust authors can call `serein_extension_sdk::export!(handler)`. Keep the workspace
`Cargo.toml` and `Cargo.lock` alongside `sdk/` and the plugin directories; their
dependencies inherit from that workspace. Remove unused plugin members when copying.

Build and package from this directory (Python 3 is used only by the author):

```powershell
rustup target add wasm32-unknown-unknown
cargo build --locked --release --target wasm32-unknown-unknown
python pack.py message-delete-protector/manifest.json target/wasm32-unknown-unknown/release/message_delete_protector.wasm packages/message-delete-protector.serein-extension
python pack.py emoji-sticker-images/manifest.json target/wasm32-unknown-unknown/release/emoji_sticker_images.wasm packages/emoji-sticker-images.serein-extension
python pack.py message-counter/manifest.json target/wasm32-unknown-unknown/release/message_counter.wasm packages/message-counter.serein-extension
```

Import the package in Settings > Extensions, review the capabilities, and enable it.
Message delete protector is an example activation plugin. Its `activation` action returns
`preserve_deleted_messages: true` after the user grants `deleted_messages`. The host
keeps already-loaded messages in bounded session memory and displays deleted text in red
by default. Hover and a local context menu can toggle that highlight or remove the
retained row. They never call Discord. The host never sends message bodies to the plugin,
saves deleted bodies to disk, restores messages deleted before loading, or gives deleted
messages live service actions.
Disabling, logout, permission revocation and timeline eviction release retained content.
Emoji & Sticker Images requests `image_sharing` and returns `image_sharing: true`
from activation. While enabled, custom emoji and sticker selections stage image attachments. Wasm receives no conversation text or image bytes and cannot fetch
or send anything. Selecting artwork authorizes an immediate image send after validation; text drafts
remain intact. Disable/logout revoke the option.
Message Counter is a reactive example: it requests `message_events` and `storage`,
counts delivered create/update/delete events without storing message text or IDs,
and shows its counters only when the user opens its panel. Reset clears the counts.
Ocean, Midnight, Rose, Forest and Latte are declarative themes under `extensions/`.
The author packages compiled bytes; Serein never runs a repository's build scripts.

## Test and develop locally

Handlers remain ordinary `fn(Invocation) -> Output` functions. `Invocation`, `Output`
and `Element` implement `Clone`, `Debug`, `PartialEq`, `Eq`, `Serialize` and `Deserialize`
so tests can construct, inspect and compare actual values. For example:

```rust
use serein_extension_sdk::{dispatch, Invocation, Output, serde_json};

fn handle(input: Invocation) -> Output {
    Output { replacement: input.composer.map(|text| text.to_uppercase()), ..Default::default() }
}

#[test]
fn uppercase_draft() {
    let bytes = dispatch(br#"{"action":"upper","composer":"hello"}"#, handle).unwrap();
    let output: Output = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(output.replacement.as_deref(), Some("HELLO"));
}
```

`dispatch` uses the same decoding and encoding as `export!`, without raw pointers.
It reports `InputTooLarge`, `InvalidInput`, `OutputTooLarge` or `InvalidOutput`.
Serialization stops at `MAX_IO_BYTES` (256 KiB), including escaped JSON bytes,
instead of allocating the entire oversized response first. The handler's own data
allocations still consume sandbox memory. Host checks for capability grants, action
surfaces, panel structure, theme values and execution fuel remain separate.

Use `input.value("name")` for a borrowed string and
`input.parse_value::<bool>("enabled")` / `input.parse_value::<i32>("size")` for
checkboxes and sliders. Parsing returns `Ok(None)` for an absent input and `Err`
for a malformed one; range checks remain the handler's responsibility.
`input.storage_json::<YourSettings>()` similarly distinguishes missing storage from
invalid JSON. `output.set_storage_json(&settings)` encodes into the existing opaque
storage field and preserves its previous value on failure. The final response must
still fit the I/O limit after escaping that stored JSON. These helpers do not grant
storage access or change its format; existing raw string storage remains supported.

From the repository root:

```powershell
cargo test --manifest-path examples/extensions/Cargo.toml --workspace --locked
cargo clippy --manifest-path examples/extensions/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo build --manifest-path examples/extensions/Cargo.toml --workspace --locked --release --target wasm32-unknown-unknown
cargo run --locked --release -p extensions --example sdk_check -- examples/extensions/target/wasm32-unknown-unknown/release
```

The last command tests the original packaged examples and rebuilt modules through
the real offline host sandbox, including validation, and prints module/package sizes
and invocation timings. It also exercises Message Counter's event, panel and reset
actions, including malformed stored data. The dedicated SDK CI job runs these checks.
Generate local API documentation with
`cargo doc --manifest-path examples/extensions/Cargo.toml --locked -p serein-extension-sdk --no-deps`.

## Reactive message plugins

Declare `message_events` in `capabilities` and one action with
`"surface": "message_event"`. The host invokes that action automatically after
the user grants access and enables the plugin. There can be at most one such
action per plugin. For example, the [Message Counter manifest](message-counter/manifest.json)
contains:

```json
"capabilities": ["message_events", "storage"],
"actions": [
  {"id": "message-event", "label": "Count message events", "surface": "message_event"},
  {"id": "show", "label": "Show message counts", "surface": "panel"},
  {"id": "reset", "label": "Reset message counts", "surface": "panel"}
]
```

Use `fn handle(input: EventInvocation) -> Output` with `export!(handle)`.
`EventInvocation` wraps the existing `Invocation`: read action and storage through
`input.invocation`, and the optional typed event through `input.message_event`.
Call `dispatch_typed(json, handle)` for offline handler tests. See the complete
[Message Counter handler and lifecycle test](message-counter/src/lib.rs).
It stores three saturating `u64` counters, leaves malformed storage untouched,
and offers an explicit reset in its ordinary panel. It does not store event content.

The JSON remains flat; a create event looks like this:

```json
{
  "action": "message-event",
  "message_event": {
    "kind": "create",
    "channel_id": "100",
    "message_id": "200",
    "author_id": "300",
    "content": "Synthetic message"
  }
}
```

`kind` is `create`, `update` or `delete` (`MessageEventKind` in Rust). Channel,
message and author IDs are decimal strings containing nonzero `u64` values.
Create includes `author_id` and `content`. Updates may omit either field;
absent content means no text patch, while `""` is an empty text patch.
Delete contains only kind, channel ID and message ID, never deleted text.
Content is at most 16 KiB of UTF-8; larger events are dropped, not truncated.
Attachments, embeds, raw Gateway JSON, credentials and native objects are not exposed.

Delivery covers only accepted live timeline events in the active, accessible
conversation. History loads, search results, cached messages, ephemeral messages
and other conversations do not generate events. Delivery is best effort, not
exactly once: do not use counters as an audit log or assume a complete history.
Updates/deletes require a loaded message. Duplicate creates, including sends
already reconciled from a send result, may be skipped.
The host queues at most 32 pending invocations totaling 64 KiB and starts at most
10 event invocations per second. Overload drops events. Conversation/account
changes, permission loss and disabling a plugin discard pending work and stale
results. Each call still uses the ordinary Wasm fuel, memory and I/O limits.

Event input has no selected-message context, composer text or panel values.
Events may return `storage` or `appearance` only with those separate grants.
They cannot send messages, propose composer replacements, open panels in the
background or enable activation-only features. Return an empty `panel`; use a
separate user-invoked `panel` action to display results.

This capability requires a supporting host. An older host rejects a manifest
containing `message_events` or `message_event`; API version 1 does not imply
support for every capability. There is no runtime capability-probe API.

## ABI version 1

The SDK preserves public fields, `fn(Invocation) -> Output`, `export!(handler)` and
the version 1 buffer ABI. Existing SDK `Invocation` and `Output` struct literal
shapes are unchanged; reactive handlers opt into the separate `EventInvocation` type.
Existing plugins need no source or manifest changes and
rebuilding is optional. False activation flags are now omitted from JSON, keeping
their default behavior while avoiding unknown-field failures on hosts that predate
an unused capability. Enabling a capability still requires a supporting host.

Export a 32-bit linear `memory`, `serein_alloc(i32 length) -> i32 pointer`, and
`serein_invoke(i32 pointer, i32 length) -> i64 output`. The host writes UTF-8 JSON into the
allocation and invokes the action. Pack the response pointer in the high 32 bits and its
byte length in the low 32 bits of the returned i64. A fresh instance is used each time;
memory and leaked ABI buffers are destroyed afterward. Do not import WASI or any functions.

Input fields are `action`, optional `selected_message`, optional `composer`, optional
`storage`, and `values` (input IDs mapped to strings; checkbox values are `true`/`false`).
Reactive actions additionally receive `message_event` as described above; ordinary
invocations omit it. Only the invoked action's context is included, after capability consent.
Output fields are optional `replacement`, optional `storage`, optional `appearance`, `panel` (array), and
`preserve_deleted_messages` and `image_sharing` (booleans, default false).
Only activation with `image_sharing` capability may enable image attachment mode. Only an `activation` action
with the `deleted_messages` capability may request preservation. There is at most
one activation action per plugin, invoked by the worker on enable/account load.
Activation itself does not require deleted-message access: each returned effect
requires its own capability. With `appearance`, return a [theme object](../../docs/theme-api.md)
to customize app colors and native controls. With `storage`, activation receives
the previously saved value so appearance settings can be restored.
This added capability requires a host version that supports it.
Storage is one opaque UTF-8 value, replacing the previous value when present.

Panel elements use the `type` tag: `text` (`text`), `row` (`children`), `button` (`id`, `label`),
`text_input` (`id`, `label`, `value`), `checkbox` (`id`, `label`, `checked`),
`heading` (`text`), `separator`, `select` (`id`, `label`, `options`, `value`), and
`slider` (`id`, `label`, `min`, `max`, `value`). Select options are 1-32 unique strings,
each at most 128 UTF-8 bytes, and the selected value must match one. Sliders use
32-bit integers with `min < max` and an in-range value. Select values and slider
numbers return as strings in `values`. Headings/labels are bounded to 128 bytes.
 A button's ID
must name a manifest action with `surface: "panel"`. Panel actions receive current input
values and granted storage; they do not receive a previous message or draft context.
IDs must be lowercase ASCII letters, digits or hyphens, start with a letter/digit, and be
at most 64 bytes; reserved Windows device names are rejected.

Limits: 4 MiB Wasm, 16 MiB JSON package, 16 MiB linear memory, 5 million execution fuel,
128 calls, 256 KiB interpreter stack, 256 KiB serialized input/output, 64 panel elements,
8 row nesting levels, 4 KiB text/input values, and 16 manifest actions. Storage has a 1 MiB
disk ceiling; because it travels in the invocation, it must also fit the 256 KiB I/O budget
alongside other fields. Exceeding any limit is an error, never silent truncation.
Wasmi's strict compilation limits also apply. Plugin panics and exhausted fuel produce a
visible error. Disabled plugins lose their package and stored data; re-enable starts fresh.

For catalog inclusion, submit a manifest, source commit (40/64 hex), immutable HTTPS release
URL, exact `download_bytes`, and SHA-256. Maintainers must review the source and built
artifact together before listing that version. These examples are source templates, not
an automatic trust designation. All versions and updates require explicit user consent.

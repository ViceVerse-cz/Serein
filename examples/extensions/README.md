# Build your first Serein plugin

> **Preview SDK — PR #411, not yet released.** The branch adds approved reply,
> sticker, forward, channel, server, role, moderation and host-mediated media
> actions. Install a matching host build before using those grants or variants.

A plugin is a function: Serein passes it JSON, it returns JSON, and the host renders
native controls or presents an action for the user to apply. Each call gets a fresh
Wasm instance. Save persistent choices through `storage`, not global variables.

The workflow is **declare permissions → read input → return output**.
Reading a snapshot does not fetch data. Editing an input object does not change the
app; return the appropriate output or host action instead.

| I want to… | Read this |
| --- | --- |
| Understand what plugins can do | [SDK overview](../../docs/extension-sdk-overview.md) |
| Build and run a first plugin | Follow this page |
| Diagnose an import, handler or Apply failure | [Troubleshooting](../../docs/extension-sdk-troubleshooting.md) |
| Understand handler input and reactive events | [Inputs and events](../../docs/extension-sdk-reference.md#invocation-and-events) |
| Read channels, messages, members, voice or settings | [App data fields](../../docs/extension-sdk-reference.md#app-data) |
| Navigate, change settings or control the current call | [Outputs and host actions](../../docs/extension-sdk-actions.md#outputs-and-host-actions) |
| Build a form and save its values | [Panels and storage](../../docs/extension-sdk-actions.md#panels-and-storage) |
| Choose permissions | [Capability reference](../../docs/extensions.md#capability-reference) |
| Change colors or native control sizes | [Theme fields](../../docs/theme-api.md) |

## Before you start

You need a Serein source checkout, Rust installed through `rustup`, and Python 3
available as `python`. Run commands in that checkout so Rust uses its pinned
`rust-toolchain.toml`. Building the native demo also needs the platform build
prerequisites in the [repository README](../../README.md). No Discord account or
credentials are needed for this tutorial.

You will create **Hello Context**, a small panel that displays the selected
synthetic channel. The steps are: copy the manifest, copy the handler, build a
Wasm module, package it, then import and enable it in the offline demo. A manifest
declares permissions and entry points; it does not contain the plugin's code.

Check your tools from the repository root:

```powershell
rustup show active-toolchain
python --version
```

The first command should show the checkout's pinned toolchain, and the second
Python 3. The first Rust command can install a missing pinned toolchain.

## Start with a working example

Use a checkout of the source revision shown in the wiki banner. Preview capabilities
may not be available in a released build.

| Example | What it demonstrates |
| --- | --- |
| [App Toolbox](app-toolbox/src/lib.rs) | App snapshots and all supported host actions |
| [Guild Inspector](guild-inspector/src/lib.rs) | Loaded channel/thread permissions, member nicknames, roles and server profiles |
| [Conversation Inspector](conversation-inspector/src/lib.rs) | Rich summaries, forum flags, typing/pins and host discovery |
| [Message Counter](message-counter/src/lib.rs) | Reactive events, saved counters and a reset button |
| [Message Delete Protector](message-delete-protector/src/lib.rs) | Opt-in activation enabling host-managed message retention |
| [Emoji & Sticker Images](emoji-sticker-images/src/lib.rs) | Activation enabling image attachment mode |

For this tutorial, use `app-toolbox/` in a development copy. Keep its `Cargo.toml`,
and replace `manifest.json` and `src/lib.rs` with the examples below. The Cargo
package stays named `app-toolbox`; the manifest gives the installed plugin its identity.

For a separate repository, also copy `sdk/`, `pack.py`, and this directory's
workspace `Cargo.toml` and `Cargo.lock`. Keep the relative directory layout and
remove unused plugin members. Dependencies inherit from that workspace.
After changing workspace members or dependencies, run `cargo check --workspace`
once in the copied workspace to update its lockfile. Review and commit that
`Cargo.lock`, then use `--locked` for reproducible builds.

## Configure the manifest

Step 1: replace `examples/extensions/app-toolbox/manifest.json` with this complete
manifest. It defines one tool that displays the current channel:

```json
{
  "api_version": 1,
  "id": "hello-context",
  "name": "Hello Context",
  "version": "1.0.0",
  "author": "Your name",
  "license": "MIT",
  "source": "https://example.org/hello-context",
  "kind": "plugin",
  "capabilities": ["app_context"],
  "actions": [
    {"id": "show", "label": "Show current channel", "surface": "panel"}
  ]
}
```

Replace the example author and source URL before publishing.

| Field | Type | What to put here / how it is used |
| --- | --- | --- |
| `api_version` | integer | `1`, the contract version. It does not imply every capability exists on an older host. |
| `id` | string | Stable, unique plugin identifier used for installation, grants and storage. Keep it across updates. |
| `name` | string | Plugin name displayed to users. |
| `version` | string | Release label such as `1.0.0`; semantic versioning is not enforced. |
| `author` | string | Creator attribution. |
| `license` | string | License label; include the actual license in your source too. |
| `source` | string | Public HTTPS source link, at most 2,048 UTF-8 bytes, without embedded credentials. It is metadata, not code to execute. |
| `kind` | string | `plugin` for Wasm; declarative themes use `theme`. |
| `capabilities` | string array | Only the permissions needed. Each requires consent; names must be known and unique. At most 64 declarations, with 51 supported today; see the [capability reference](../../docs/extensions.md#capability-reference) for their scopes. |
| `actions` | object array | Entry points invoked by users or the host. Plugins need 1–16 actions with unique IDs. |

Plugins that use `data_queries` or `action_feedback` also declare `app_events` and
one `app_event` action. Decode that handler with `ExtendedAppInvocation`; older
`AppInvocation` handlers remain source-compatible. A tracked action result confirms
native admission after Apply, not eventual service completion.

`name`, `version`, `author` and `license` must be nonempty, at most 128 UTF-8 bytes,
without control characters. IDs use lowercase ASCII letters, digits and hyphens,
start with a letter or digit, and are at most 64 bytes. Windows device names such
as `con` and `nul` are reserved. Themes declare no actions or capabilities.

### Action fields

| Field | Type | How it is used |
| --- | --- | --- |
| `id` | string | Arrives as `input.action` (or `input.invocation.action` with a wrapper). Panel buttons invoke this same ID. |
| `label` | string | Display name; nonempty, at most 128 UTF-8 bytes, without control characters. |
| `surface` | string | When the action runs and what context it receives; choose below. |

| Surface | When it runs | Required capability / input |
| --- | --- | --- |
| `panel` | User opens a tool or clicks a panel button | Form values on button clicks; other data requires its own grant. |
| `message` | User chooses a message action | `selected_message`; receives the chosen message's text. |
| `composer` | User chooses a draft action | `composer`; receives the draft and can propose replacement text. |
| `activation` | Enable or account load | Each returned feature needs its own grant; used to restore appearance or enable activation features. |
| `message_event` | Accepted live message change | `message_events`; receives one typed event. |
| `app_event` | Supported app change | `app_events`; receives the reason and separately granted snapshots. |

At most one action of **each** automatic surface is allowed: `activation`,
`message_event` and `app_event`. A capability alone does not register a handler;
declare its action too.

### Generate and check a typed manifest

The SDK also exports `Manifest`, `Action`, `Surface`, `ExtensionKind` and
`Capability` for authoring tools. This complete native Rust program generates
the same manifest as above:

```rust
use serein_extension_sdk::{
    Action, Capability, ExtensionKind, Manifest, Surface, serde_json,
};

fn main() -> Result<(), serde_json::Error> {
    let manifest = Manifest {
        api_version: 1,
        id: "hello-context".into(),
        name: "Hello Context".into(),
        version: "1.0.0".into(),
        author: "Your name".into(),
        license: "MIT".into(),
        source: "https://example.org/hello-context".into(),
        kind: ExtensionKind::Plugin,
        capabilities: vec![Capability::AppContext],
        actions: vec![Action {
            id: "show".into(),
            label: "Show current channel".into(),
            surface: Surface::Panel,
        }],
    };
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}
```

These types serialize and deserialize the host's manifest shape. Unknown fields
and enum names are rejected; omitted `capabilities` and `actions` decode as empty
vectors. Typed construction or successful JSON decoding does **not** validate
IDs, limits, duplicate declarations or capability/surface combinations.

From the repository root, check a standalone manifest with the host's authoritative
`Manifest::validate` rules:

```powershell
cargo run --locked -p extensions --example manifest_check -- examples/extensions/app-toolbox/manifest.json
```

Replace the path to check your own file. The checker accepts a manifest JSON
document of at most 16 KiB, not a packaged extension. It does not run Wasm,
install a plugin or grant permissions. Passing checks only the manifest: package,
Wasm imports, output validation, fuel and live compatibility remain separate.
Import validates the manifest again using that installed host's supported rules;
older hosts can reject capabilities accepted by a newer checker.

## Write the handler

Step 2: replace `examples/extensions/app-toolbox/src/lib.rs` with this handler.
The `show` action matches the manifest above:

```rust
use serein_extension_sdk::{AppInvocation, AppOutput, Element, Output};

fn handle(input: AppInvocation) -> AppOutput {
    if input.invocation.action != "show" {
        return AppOutput::default();
    }
    let channel = input.app.as_ref()
        .and_then(|app| app.context.as_ref())
        .and_then(|context| context.channel.as_ref());
    let text = match channel {
        Some(channel) => format!("You are in {} (ID {}).", channel.name, channel.id),
        None => "No accessible channel is selected.".into(),
    };
    AppOutput {
        output: Output { panel: vec![Element::Text { text }], ..Default::default() },
        ..Default::default()
    }
}
serein_extension_sdk::export!(handle);
```

- `input.invocation.action` identifies the manifest action.
- `input.app` holds granted, available data. Check each optional layer; a missing
  selected channel is a normal state.
- `output.panel` asks the host to render text immediately, without an Apply click.
- `export!` generates Wasm exports. Your handler remains directly testable Rust.

## Build and package

Step 3: open a terminal in `examples/extensions/` and run:

```powershell
rustup target add wasm32-unknown-unknown
cargo build --locked --release --target wasm32-unknown-unknown -p app-toolbox
python pack.py app-toolbox/manifest.json target/wasm32-unknown-unknown/release/app_toolbox.wasm packages/hello-context.serein-extension
```

The build creates `target/wasm32-unknown-unknown/release/app_toolbox.wasm`.
`pack.py` prints the package path, byte count and SHA-256, and creates
`packages/hello-context.serein-extension`. It combines compiled Wasm and the
manifest into one JSON package. Python is an authoring tool, not an end-user
dependency. The package filename may differ from the manifest ID.

If you set `CARGO_TARGET_DIR`, use that build directory in the Wasm path passed
to `pack.py`; Cargo will not necessarily write into this example's `target/`.

Step 4: return to the repository root and start the offline app:

```powershell
cargo run --locked -p serein -- --demo
```

Step 5: in **Settings > Extensions**, import
`examples/extensions/packages/hello-context.serein-extension`, review the
`app_context` grant and enable it. On the **Hello Context** card, choose **Open tool**, then
**Show current channel**. It displays
the selected synthetic channel, or the unavailable-context message. Import alone
does not grant permissions or execute the plugin.

Success means the native panel shows `You are in ... (ID ...).` or
`No accessible channel is selected.` Neither result sends a message. If the tool
is missing or reports an error, use the [troubleshooting checklist](../../docs/extension-sdk-troubleshooting.md).

After editing the handler, rebuild and repackage, then import the new package and
review its grants again. Replacing Rust source alone does not update an installed
Wasm module.

For an unchanged example, use its own manifest and matching compiled filename:
`app_toolbox.wasm`, `guild_inspector.wasm`, `conversation_inspector.wasm`, `message_counter.wasm`, `message_delete_protector.wasm`, or
`emoji_sticker_images.wasm`.

## Test and develop locally

Step 6: append this offline test to the tutorial handler:

```rust
#[test]
fn missing_channel_is_handled() {
    use serein_extension_sdk::{dispatch_typed, serde_json, AppOutput, Element};
    let bytes = dispatch_typed(br#"{"action":"show"}"#, handle).unwrap();
    let output: AppOutput = serde_json::from_slice(&bytes).unwrap();
    assert!(matches!(&output.output.panel[0], Element::Text { text }
        if text == "No accessible channel is selected."));
}
```

Run from the repository root:

```powershell
cargo test --manifest-path examples/extensions/Cargo.toml --workspace --locked
cargo clippy --manifest-path examples/extensions/Cargo.toml --workspace --all-targets --locked -- -D warnings
```

Expect the `missing_channel_is_handled` test to pass. A passing native test proves
this handler can decode the synthetic input and return the expected text; it does
not prove Wasm imports, fuel limits or native UI behavior.

`dispatch` handles `Invocation`; `dispatch_typed` supports wrappers. Both exercise
JSON decoding/encoding and the 256 KiB I/O bound without raw pointers. Errors are
`InputTooLarge`, `InvalidInput`, `OutputTooLarge` and `InvalidOutput`. Host checks
for permissions, panels, imports and fuel remain separate.

For the **unchanged repository examples**, also run:

```powershell
cargo build --manifest-path examples/extensions/Cargo.toml --workspace --locked --release --target wasm32-unknown-unknown
cargo run --locked --release -p extensions --example sdk_check -- examples/extensions/target/wasm32-unknown-unknown/release
```

`sdk_check` runs committed packages and rebuilt modules through the offline host
sandbox. It expects the original example behavior, so it is not a test runner for
the modified Hello Context plugin. It reports sizes/timings and validates proposals
without touching an account, clipboard or call. SDK CI runs these checks.

The separate compatibility check uses an immutable compiled App Toolbox package
from source commit `3d94c76228d9f2918fa8d22e78f40233ae4f90dc`, rather than rebuilding
it against today's SDK:

```powershell
cargo run --locked --release -p extensions --example legacy_sdk_check
```

It exercises old foreground snapshots and five old data-event variants in the
real offline Wasm sandbox, and rejects newer event kinds against the original
manifest before execution. See [fixture provenance](../../crates/extensions/tests/fixtures/sdk-legacy/README.md).
This check does not test install/enable/disable/reload or account lifecycle.

Generate Rust API docs with:

```powershell
cargo doc --manifest-path examples/extensions/Cargo.toml --locked -p serein-extension-sdk --no-deps
```

## Choose your next step

- Add a native form and a Save button with [Panels and storage](../../docs/extension-sdk-actions.md#panels-and-storage).
- Read another granted group with [App data fields](../../docs/extension-sdk-reference.md#app-data).
- Propose navigation or a settings change with [Outputs and host actions](../../docs/extension-sdk-actions.md#outputs-and-host-actions).
- Choose a smaller starting example with the [SDK overview](../../docs/extension-sdk-overview.md).

Keep the first plugin working before adding more permissions. The sections below
explain existing examples and the ABI; they are references, not extra tutorial steps.

## Try the conversation actions example

[Conversation Actions](app-actions/src/lib.rs) provides a compact form for sending,
editing/deleting, reactions, pins, read markers and thread creation. It requests
separate write grants and produces one proposal per click. Inspect the destination
and text in the native confirmation before choosing Apply. Importing or opening
the form never sends a message.

From `examples/extensions`, build and package it:

```powershell
cargo test --locked -p app-actions
cargo build --locked --release --target wasm32-unknown-unknown -p app-actions
python pack.py app-actions/manifest.json target/wasm32-unknown-unknown/release/app_actions.wasm packages/app-actions.serein-extension
```

Use synthetic `--demo` data to check rendering and proposal validation. A demo
build does not establish service compatibility and does not authorize live
Discord messages or calls. The host's sandbox check exercises all ten form
operations offline through real Wasm.

## Reactive message plugins

Start with [Message Counter's manifest](message-counter/manifest.json) and
[handler](message-counter/src/lib.rs), then read the
[message event fields](../../docs/extension-sdk-reference.md#message-event-fields).
Declare `message_events` and a `message_event` action, match the kind, and handle
partial updates. Saving state requires a separate `storage` grant.

Delivery covers the active accessible conversation and is best effort, without
history replay. Events cannot open background panels; expose a separate `panel`
action to display saved results.

## App snapshots and host actions

The [app data reference](../../docs/extension-sdk-reference.md#app-data) explains
every snapshot field, including unavailable and partial data.
[Outputs and actions](../../docs/extension-sdk-actions.md#outputs-and-host-actions)
explains proposals and their grants. [App Toolbox](app-toolbox/src/lib.rs)
demonstrates loaded account profiles, joined servers, selected-channel details,
message metadata, relationships, scrolling preferences and device-local notification
settings, and the original 12 host-effect types.
[Conversation Actions](app-actions/src/lib.rs) demonstrates explicit message,
reaction, pin, read-state and thread proposals using `AppAction`. Its separate
write grants never authorize background actions; each proposal needs Apply.
Preview reply, loaded-sticker, forward, channel, group/DM, server, role,
moderation and host-mediated media operations
are listed with complete JSON fields in the
[action reference](../../docs/extension-sdk-actions.md#app-actions).
App Toolbox's passive observer requests `data_events` along
with `app_events` and the relevant read grants; it stores no event counts or
conversation data. Detailed events are coalesced invalidation hints, not a full
change log. See [event grants and reasons](../../docs/extension-sdk-reference.md#appeventkind-why-an-app-observer-ran).

[Guild Inspector](guild-inspector/src/lib.rs) is a smaller, separate example for
`channel_metadata` and `member_details`. It displays unknown topic/settings and
permission values explicitly, caps its display to five loaded member rows and
eight role labels, and fetches nothing. Its observer adds `app_events` and
`data_events` and remains passive. Build/package it with the same commands using
`-p guild-inspector`, `guild_inspector.wasm`, and its own manifest.

[Conversation Inspector](conversation-inspector/src/lib.rs) demonstrates
`message_content`, `forum_data`, `conversation_activity` and public `host`
discovery, including unknown pins and unsupported polls. It uses reusable offline
fixtures and stays passive for events. Build it with `-p conversation-inspector`
and package `conversation_inspector.wasm` with its own manifest.

For failures, the host reports [sanitized execution categories](../../docs/extensions.md#safe-execution-diagnostics)
with practical next checks. Native dispatch tests do not substitute for real Wasm
fuel/memory validation; private input/output is never echoed in these errors.

Inputs are read-only copies. Returning `effects` proposes a change needing
**Apply**. Storage and appearance have different timing; see the output reference
and [Panels and storage](../../docs/extension-sdk-actions.md#panels-and-storage).

## Activation examples

[Message delete protector](message-delete-protector/src/lib.rs) is an opt-in
activation plugin whose current handler returns `Output::default()`. The host
interprets successful activation with the `deleted_messages` grant as consent to keep
loaded deleted messages in bounded session memory while enabled. The handler does
not need to return `preserve_deleted_messages`; that is a compatibility field. The host highlights
retained text and offers local controls without calling Discord. Deleted bodies
are never supplied to this plugin, written to disk, or recovered from before they
were loaded. Disable, logout, permission revocation and eviction release them.

Emoji & Sticker Images requests `image_sharing` and returns `image_sharing: true`
from activation. Selecting artwork authorizes an immediate image send after
validation, preserving text drafts. Wasm receives no image bytes and cannot fetch
or send anything. Disable/logout revoke the option. Only an activation action
with the grant may enable this mode.

There is at most one activation action per plugin, run on enable/account load.
Activation itself does not require deleted-message access. Granted `appearance`
can return a [theme object](../../docs/theme-api.md); granted `storage` can restore
saved choices. Storage is one opaque UTF-8 value, replaced when returned.
Ocean, Midnight, Rose, Forest and Latte are declarative themes under `extensions/`.
Authors package compiled bytes; Serein never runs their build scripts.

## ABI version 1

Existing `Invocation`, `Output`, `dispatch` and `export!` APIs and struct literal
shapes remain supported. Opt into events with `EventInvocation`, or app data and
actions with `AppInvocation` / `AppOutput`. Existing plugins need no rebuild.

Older hosts reject unsupported capabilities/surfaces. `api_version: 1` is not a
capability probe. Current hosts inject a public support catalog available as
`AppInvocation.host`; missing means an older host. Its string-based
`supports("message_content")` and `supports_event("typing")` helpers report host
support, never grants. Unsupported required manifest capabilities still prevent
installation before your handler runs. See [HostInfo](../../docs/extension-sdk-reference.md#hostinfo-discover-supported-names).

Rust authors use `export!`. Other languages must export:

| Export | Contract |
| --- | --- |
| `memory` | 32-bit linear Wasm memory. No WASI or function imports. |
| `serein_alloc(i32 length) -> i32 pointer` | Allocate room for the host's UTF-8 JSON input. |
| `serein_invoke(i32 pointer, i32 length) -> i64 output` | Return UTF-8 JSON: output pointer in the high 32 bits, byte length in the low 32 bits. |

A fresh instance is destroyed after each call, including its ABI buffers.
Rust wrappers flatten into top-level JSON: there are no `invocation` or `output`
keys. Discord IDs are decimal strings, not JSON numbers. See the
[input](../../docs/extension-sdk-reference.md#invocation-and-events) and
[output](../../docs/extension-sdk-actions.md#outputs-and-host-actions) field references
and [sandbox limits](../../docs/extensions.md#resource-and-privacy-limits).
Fitting a byte limit does not guarantee a handler fits the execution-fuel budget.

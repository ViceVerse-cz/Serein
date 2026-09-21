# Making a Serein extension

This is a complete offline example and a small Rust SDK. Copy a plugin directory and the
SDK into your own repository, edit its manifest and action, and publish the resulting
`.serein-extension` file. Any language producing compatible WebAssembly can use this ABI;
Rust authors can call `serein_extension_sdk::export!(handler)`.

Build and package from this directory (Python 3 is used only by the author):

```powershell
rustup target add wasm32-unknown-unknown
cargo build --locked --release --target wasm32-unknown-unknown
python pack.py message-delete-protector/manifest.json target/wasm32-unknown-unknown/release/message_delete_protector.wasm packages/message-delete-protector.serein-extension
```

Import the package in Settings > Extensions, review the capabilities, and enable it.
Build `rgb-cycle` the same way; its packaged output goes to
`packages/rgb-cycle.serein-extension`. It has a `tick` action with the
`appearance` capability, which the host re-invokes on its own schedule (see
"Tick" below) to sweep enabled theme tokens through the color wheel, and a
`settings` panel action (`storage` capability) letting you check or uncheck
every individual token - surfaces, text, accent, status colors, mentions,
and the background gradient - from Settings > Extensions > RGB Cycle >
Open tool. Choices persist across restarts via plugin storage.
`positive`/`warning`/`danger` default off, since a rotating hue can make a
destructive action briefly read as safe; every other token defaults on.

Message delete protector is the sole example message-context plugin. Its `activation` action returns
`preserve_deleted_messages: true` after the user grants `deleted_messages`. The host
keeps already-loaded messages in bounded session memory and displays deleted text in red
by default. Hover and a local context menu can toggle that highlight or remove the
retained row. They never call Discord. The host never sends message bodies to the plugin,
saves deleted bodies to disk, restores messages deleted before loading, or gives deleted
messages live service actions.
Disabling, logout, permission revocation and timeline eviction release retained content.
Ocean, Midnight, Rose, Forest and Latte are declarative themes under `extensions/`.
The author packages compiled bytes; Serein never runs a repository's build scripts.

## ABI version 1

Export a 32-bit linear `memory`, `serein_alloc(i32 length) -> i32 pointer`, and
`serein_invoke(i32 pointer, i32 length) -> i64 output`. The host writes UTF-8 JSON into the
allocation and invokes the action. Pack the response pointer in the high 32 bits and its
byte length in the low 32 bits of the returned i64. A fresh instance is used each time;
memory and leaked ABI buffers are destroyed afterward. Do not import WASI or any functions.

Input fields are `action`, optional `selected_message`, optional `composer`, optional
`storage`, and `values` (input IDs mapped to strings; checkbox values are `true`/`false`).
Only the explicitly selected action's context is included and only after capability consent.
Output fields are optional `replacement`, optional `storage`, optional `appearance`, `panel` (array), and
`preserve_deleted_messages` (boolean, defaults false). Only an `activation` action
with the `deleted_messages` capability may request preservation. There is at most
one activation action per plugin, invoked by the worker on enable/account load.
Activation itself does not require deleted-message access: each returned effect
requires its own capability. With `appearance`, return a [theme object](../../docs/theme-api.md)
to customize app colors and native controls. With `storage`, activation receives
the previously saved value so appearance settings can be restored.
This added capability requires a host version that supports it.
Storage is one opaque UTF-8 value, replacing the previous value when present.

### Tick

A plugin may declare at most one action with `"surface": "tick"`, and it
requires the `appearance` capability (enforced at manifest validation, not
just at runtime). Unlike every other surface, the host invokes a `tick`
action itself, repeatedly, for as long as the plugin stays enabled and the
app is in the foreground - the user never clicks anything to trigger it.
Each call still runs in its own fresh, fuel-bounded Wasm instance, exactly
like every other invocation; nothing is retained between calls, no WASI or
host imports are added, and execution never happens inside the UI's render
or audio callback. `extensions::TICK_MIN_INTERVAL_MS` (~250ms) is a floor,
not a target: the host also never lets a second `tick` invocation for a
plugin queue up before the first one resolves, so a slow invocation still
can't flood the shared, single-worker extension queue that every other
action -- Import, Refresh, a plugin's own settings panel -- goes through
too. The invocation carries
`tick_ms`, milliseconds elapsed since the plugin was enabled this session,
and the plugin must derive its output solely from that value - there is no
selected message, composer, or stored state on a tick call, and any output
field other than `appearance` (and, implicitly, an empty `panel`) is
rejected the same as it would be from any other capability mismatch. This
is how `rgb-cycle` animates a theme: it returns a new `appearance` overlay
each call, and the host swaps straight to it (there's no cross-fade), so
pick a rotation slow enough relative to the tick interval that each step
reads as gradual motion rather than a visible jump.

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

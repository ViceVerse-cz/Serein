# Extension SDK troubleshooting

Start with the [SDK overview](extension-sdk-overview.md) or the
[first-plugin tutorial](../examples/extensions/README.md). Use synthetic inputs
and the offline `--demo` app when reproducing a problem. Record the source/build
revision, manifest action and sanitized error category; a token, private message
or complete account snapshot is not needed.

## Import and enable

| Symptom | Likely cause | What to check |
| --- | --- | --- |
| The package fails to import | Invalid manifest/package, unsupported capability or API version, or invalid Wasm | Run the manifest checker below. Check that you packaged the intended manifest and matching Wasm, rather than importing a raw `.wasm` file. |
| A capability works in source checks but the installed app rejects it | The app predates that capability | Compare the host build with the documentation's source revision. `api_version: 1` does not mean all capabilities exist. An unknown required capability is rejected before the handler can inspect discovery. |
| Imported tool does not run | Import does not grant permissions or enable the plugin | Review capabilities and enable the package in Settings > Extensions. Check the card's error status. |
| The app still uses old handler behavior | Installed package contains earlier compiled bytes | Rebuild, repackage and import that package. Editing Rust source or the manifest on disk does not replace installed bytes. |

From the repository root, validate the manifest with the current host rules:

```powershell
cargo run --locked -p extensions --example manifest_check -- examples/extensions/app-toolbox/manifest.json
```

Replace the path with your manifest. This checks a standalone JSON manifest of
at most 16 KiB, not the Wasm module or a `.serein-extension` package. See
[manifest fields](../examples/extensions/README.md#configure-the-manifest) and
[host discovery](extension-sdk-reference.md#hostinfo-discover-supported-names).

## Missing data or controls

| Symptom | Likely cause | What to check |
| --- | --- | --- |
| `input.app` or a group is absent | Missing grant, unsupported host, wrong action surface or unavailable context | Declare only the needed capability, obtain consent, and check every `Option`. App snapshots accompany foreground/app-event actions, not activation/message-event calls. Read that group's availability rules. |
| A group is present but empty, unknown or truncated | The host has only partial loaded state | Respect optional fields and `truncated`. Snapshots do not trigger fetches; an empty or unavailable value is not proof the service has no data. |
| Returned panel button is rejected | Its ID has no manifest action with `surface: "panel"` | Match the button ID and action ID exactly. Every control ID must also be unique within the returned panel. |
| Editing a checkbox has no effect | A form edit changes only panel values | Return a button, handle its action, read `input.invocation.values`, and return storage or a host proposal as appropriate. |
| An event handler never runs | A capability is declared without its automatic action, or a detailed reason lacks its grants | Declare one `message_event` or `app_event` action as appropriate. Detailed app reasons also need `data_events` and their data grants. Event delivery is best effort, not a replay log. |

See [App data](extension-sdk-reference.md#app-data),
[event grants](extension-sdk-reference.md#appeventkind-why-an-app-observer-ran),
[panel IDs](extension-sdk-actions.md#ids-and-panel-limits) and
[form values](extension-sdk-actions.md#how-a-button-receives-form-values).

To isolate panel rendering, return this complete minimal output first:

```json
{"panel":[{"type":"text","text":"Handler ran."}]}
```

There is no top-level `output` wrapper in the JSON. `AppOutput` and `Output`
flatten their fields onto the same object. Add controls one at a time after this
works; there are at most 64 elements, including nested row children.

## Proposals and Apply

| Symptom | Likely cause | What to check |
| --- | --- | --- |
| Handler returned an effect but the app did not change | Effects are proposals | Review the result and click Apply. Closing it makes no change. A plugin cannot approve its own proposal. |
| Apply says the account or conversation changed | The result belongs to an earlier context | Run the tool again in the intended context. The host also rechecks permissions, enabled status and the identity of a controlled voice call. |
| Effect is rejected with `Capability` | Missing grant or disallowed action surface | Foreground `message`, `composer` and `panel` actions may propose one effect. Activation, message-event and app-event handlers cannot return effects. |
| A settings patch is rejected | Empty/all-null patch, unknown field or invalid range | Supply at least one valid value. Validation rejects the whole patch; omitted/null fields preserve Apply-time preferences. |

A complete reading-settings proposal looks like this, with the `local_settings`
grant and a foreground action:

```json
{"effects":[{"type":"set_local_settings","settings":{"zoom_percent":110}}]}
```

Use `type`, not `kind`, for effect tags. Return at most one effect, at most 8 KiB
serialized, and do not combine it with a composer replacement. See
[host action rules](extension-sdk-actions.md#rules-for-every-host-action) and
[reading settings](extension-sdk-actions.md#change-local-reading-settings).

## Saved values disappear

Each call starts a fresh Wasm instance: global variables are not persistent.
Request `storage`, read the optional saved string and return a new string from a
normal action to replace it. Omitting storage preserves the previous value;
`""` replaces it with an empty string. If you encode JSON inside storage, that
empty string is not valid JSON.

Returning storage from activation is ignored in this build. Disabling a plugin
or logging out clears that account's extension data; do not use those actions as
a persistence test. An ordinary restart is different: enabled plugin storage
belongs to the same account and plugin ID. Check that your save action actually
returned storage and succeeded before restarting. Storage has a 1-MiB disk limit
but must also fit inside the 256-KiB serialized invocation/output budgets.
See [storage semantics](extension-sdk-actions.md#storage-is-one-value-not-a-filesystem).

## Execution errors

The names below are host error categories. The native UI shows fixed explanatory
messages, not private input/output bytes or Wasm panic text. A category identifies
a useful check; it is not a full crash trace.

| Category | Next check |
| --- | --- |
| `Invalid`, `Version`, `Limit` | Check schema, API version and package/manifest bounds before execution. |
| `Module`, `Execution` | Build `wasm32-unknown-unknown`, use `export!`, and check the required exports. No WASI or other imports are accepted. |
| `Input`, `InputLimit` | Check action/input shape; reduce requested data, form values or saved storage. Public host discovery also counts toward the 256-KiB input limit. |
| `Handler` | The ABI returned zero output bytes. Check SDK input decoding and output serialization. Native `dispatch_typed` reports its own decoding/size errors directly. |
| `Output`, `OutputLimit` | Check output JSON, enum tags, control IDs and ranges, ABI buffer, panel count and serialized size. Valid JSON alone is not valid host output. |
| `Capability` | Check declarations, consent and action surface. Repeating execution does not grant permissions. |
| `Fuel` | Reduce loops, JSON work and requested data. Parsing and execution share 5,000,000 fuel; byte-size compliance does not ensure enough fuel. |
| `Memory` | Reduce allocations and table size. Linear memory is capped at 16 MiB; this category can also mean engine allocation failure. |
| `Stack` | Reduce recursion and stack allocations. |
| `Trap` | Check panics, invalid memory access and arithmetic traps with a synthetic native test. The host does not expose panic text. |

Invalid output or an exhausted budget can disable the failing plugin. Fix the
cause before enabling it again. See [sandbox limits](extensions.md#resource-and-privacy-limits)
and [ABI exports](../examples/extensions/README.md#abi-version-1).

## Reproduce offline

From the repository root, run SDK/example native tests:

```powershell
cargo test --manifest-path examples/extensions/Cargo.toml --workspace --locked
```

Use a small `dispatch_typed` test with a synthetic action/input to check your
handler's decoding and result. The [tutorial test](../examples/extensions/README.md#test-and-develop-locally)
shows this without raw pointers. It does not exercise Wasm fuel or host grants.

For unchanged repository examples, build their Wasm and run the host sandbox check:

```powershell
cargo build --manifest-path examples/extensions/Cargo.toml --workspace --locked --release --target wasm32-unknown-unknown
cargo run --locked --release -p extensions --example sdk_check -- examples/extensions/target/wasm32-unknown-unknown/release
```

If `CARGO_TARGET_DIR` is set, pass its actual `wasm32-unknown-unknown/release`
directory instead. `sdk_check` expects the repository examples' original behavior;
it is not a generic runner for a modified Hello Context plugin. For your plugin,
follow the [build/import steps](../examples/extensions/README.md#build-and-package)
and keep `--demo` when launching the app. Native tests, offline Wasm checks and
demo behavior are separate evidence; none proves live Discord compatibility.

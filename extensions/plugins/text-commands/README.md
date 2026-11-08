# Text Commands

A bounded, local-only Serein port of safe workflows from Equicord's **TextReplace** and
**CustomCommands** plugins. Upstream source is pinned to Equicord/Equicord commit
[`200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e):
`src/plugins/textReplace/index.tsx` and `src/plugins/customCommands/index.ts`.
The upstream Vencord GPL notice is retained in `src/lib.rs`; this port is licensed
`GPL-3.0-or-later`.

## Replacement rules

Open the native panel and enter a literal, case-sensitive **Find** value, its replacement,
and an optional **Only if includes** condition. Find is the rule's identity: saving the
same Find replaces that rule in place, while a new Find appends a rule. Delete uses the
same exact Find value and leaves storage alone when there is no match. Replacement text
may be empty. Literal `\n` in replacement text becomes a newline.

Rules run once, in saved order. A condition is checked against the result of preceding
rules, matching can make text available to later rules, and there is no recursive pass.
Literal matching deliberately avoids upstream TextReplace's regex engine, incoming-message
rewrites, channel scopes, import/export, and automatic send hooks.

## Tags

A tag has a lowercase ASCII name and a message template. Names are 1–32 bytes, start with
a letter, and then contain only letters, digits, or hyphens. Saving an existing name
replaces its template in place; deleting an absent name is a no-op. Invoke a tag in the
composer as:

```text
/name
/name key:value
/name first:some text second:more text
```

Argument values are trimmed and can contain spaces. With multiple values, a space followed
by another not-yet-supplied placeholder's exact `key:` starts the next value; all other
text remains in the current value. Placeholders use `{{arg}}` or `{{arg = default}}`.
Names may contain spaces when the supplied argument uses the same exact name, for example
`{{first name}}` with `/name first name:Ada`. Names and defaults are trimmed, duplicate
placeholder definitions use the first definition, and a supplied empty value is different
from a missing value. Literal `\n` in the completed template becomes a newline. Expansion
is single-pass, not recursive.

Unknown or malformed command names and text not beginning with `/` receive no proposal.
Unknown arguments, malformed arguments, an empty expansion, or missing required arguments
leave the draft untouched and explain the problem in a native result panel.

## Limits and safety

- At most 8 rules and 8 tags. Find, replacement, and condition are each at most 4,096
  UTF-8 bytes; a nonblank tag template is at most 4,096 UTF-8 bytes.
- The complete stored JSON is at most 64 KiB. Unknown fields, duplicate identities,
  semantic validation failures, oversized data, and malformed JSON are reported with a
  fixed message. Mutations are blocked and original storage is never returned or reset.
- Draft input and transformed output are limited to 8,000 UTF-8 bytes. Every rule and tag
  pass is iterative and size-checked rather than recursively expanding text. Storage
  responses are also checked against the SDK's complete serialized-output limit.
- At most 64 distinct tag arguments are read from one template. Missing arguments never
  fall through to a send.
- Composer changes are only `replacement` proposals requiring the host's **Apply** action.
  Nothing is sent automatically. The host's normal draft character and account limits
  still apply.
- Only `composer` and `storage` capabilities are requested. There is no generic command
  execution, remote slash-command registration, bot message, network access, raw Discord
  service API, or credential access.

## Storage shape

```json
{
  "rules": [
    {"find": "brb", "replace": "be right back", "only_if_includes": ""}
  ],
  "tags": [
    {"name": "greet", "message": "Hello {{name = friend}}!"}
  ]
}
```

Unit tests in `src/lib.rs` cover sequential rules, escaped newlines, corrupt-storage
preservation, defaults, missing arguments, Unicode values, and unknown commands. Native
handler tests are not Wasm sandbox or live-service compatibility evidence.

## Packaging

Package from `extensions/` with the license text embedded as an inert Wasm custom section:

```sh
python pack.py plugins/text-commands/manifest.json target/wasm32-unknown-unknown/release/text_commands.wasm plugins/packages/text-commands.serein-extension --license licenses/Equicord-GPL-3.0.txt
```

The host ignores custom sections, so this does not change the executable ABI.

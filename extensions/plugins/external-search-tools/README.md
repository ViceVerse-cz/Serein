# External Search Tools

A safe Serein port of Equicord/Vencord **Translate** and **ReplaceGoogleSearch**
workflows. It reads the selected message text and proposes an external public
browser page for translation or search. Serein validates and displays that URL,
and only opens it after the user confirms **Apply**.

Unlike the upstream plugins, this port performs no inline translation, no API
request, no context-menu navigation and no message replacement.

## Upstream source

Adapted from the Equicord upstream at commit
[`200dba124066146a8f043fde7fc8b05e3dc5e68e`](https://github.com/Equicord/Equicord/tree/200dba124066146a8f043fde7fc8b05e3dc5e68e),
especially:

- `src/plugins/translate/index.tsx`
- `src/plugins/translate/utils.ts`
- `src/plugins/translate/languages.ts`
- `src/plugins/replaceGoogleSearch/index.tsx`
- `src/plugins/replaceGoogleSearch/README.md`

Upstream is GPL-3.0-or-later. The upstream copyright header is preserved in
`src/lib.rs`.

## Actions

| Action | Surface | Behavior |
| --- | --- | --- |
| `translate` | message | Opens Google Translate or DeepL with the selected message text and configured target language. |
| `search` | message | Opens the configured search engine with the selected message text as the query. |
| `open` | panel | Shows the current provider, target language and search engine. Does not write storage. |
| `save` | panel | Validates the selected allowlist values and saves them. |

The selected text is trimmed, empty text is rejected, and both translation and
search input are capped at 1024 UTF-8 bytes on a character boundary. Both
translation and search URLs are also kept within 1024 bytes by reducing the
input on a character boundary before rebuilding the URL. Only one `HostEffect::AppAction` with
`AppAction::OpenExternalLink` is emitted for a successful translate or search.
Panel actions emit no effect. There is no replacement text, no message send, and
no use of `selected_message_ref`.

## Allowlists

- Translation providers: Google Translate (default), DeepL.
- Target languages: `en` (default), `cs`, `de`, `es`, `fr`, `it`, `ja`, `pl`,
  `pt`, `ru`, `tr`, `uk`, `zh`.
- Search engines: Google, DuckDuckGo (default), Brave, Bing, Wikipedia.

Google translation uses `sl=auto` and the selected `tl` code. DeepL uses its
public `#auto/{language}/{text}` browser fragment. Search queries are built with
the `url` crate. All destinations are fixed HTTPS hosts; custom provider URLs,
custom search endpoints and regex configuration are not supported.

## Storage and privacy boundaries

- Storage contains only the three allowlisted preferences. It is bounded and
  never placed in a URL.
- Malformed storage, unknown fields, or out-of-allowlist values are treated as
  corrupt. On `open`, `translate` and `search`, defaults are used for that
  response and the existing stored value is left untouched. A successful
  explicit `save` is the only way to replace it.
- The plugin performs no networking, filesystem access, credential access, or
  API-key handling. It never receives a DeepL or Google API key.
- The selected message text is the only message content read. The plugin does
  not read channel IDs, authors, attachments or other host data.
- The URL is proposed as a reviewed effect. The native host validates it,
  displays it, and requires explicit **Apply** before opening the browser. These
  are syntax/HTTPS/grant checks, not DNS or public-IP filtering: the OS browser
  owns the connection and the user's native external-link confirmation setting
  still applies.

## Differences from inline Equicord behavior

- Equicord Translate calls Google/DeepL/Kagi APIs and renders translated text
  in place. This port opens a public browser page and does not translate inline.
- Equicord Translate can replace displayed message content and uses API keys for
  DeepL/Kagi. This port does neither and has no key handling.
- Equicord ReplaceGoogleSearch opens a new browser tab directly from a context
  menu and supports custom engine URLs. This port emits a host-reviewed effect,
  requires Apply confirmation, and supports only the fixed allowlist above.
- This port has no auto-translation, caching, message patching, context-menu
  patch or chat-bar button.

## Implemented host API (Preview)

This package uses the implemented author-safe external-navigation contract:

- manifest capability `external_navigation` (`Capability::ExternalNavigation`);
- `AppAction::OpenExternalLink { url: String }`;
- `HostEffect::AppAction { action }`;
- `MessageActionInvocation` wrapping `AppInvocation` wrapping `Invocation`.

The plugin does not depend on `selected_message_ref`.

## Integration

The package is a member of the `extensions` workspace. It declares the usual
workspace dependencies (`serein-extension-sdk`, `url`, `serde`, `serde_json`)
and exposes `handle(MessageActionInvocation) -> AppOutput` through
`serein_extension_sdk::export!`. Installing it requires a matching Preview host
with the Preview `external_navigation` API from PR #937; older hosts reject the manifest.
This SDK change is not yet released. All
validation in this repository is synthetic/offline and does not prove third-party
site behavior.

## License

GPL-3.0-or-later. See the repository license and third-party notices.

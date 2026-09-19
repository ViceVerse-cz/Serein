# Community extensions

Serein extensions are local, opt-in tools for the native client. The Extensions
and Themes pages in Settings contain packages, links to their source, their
requested capabilities and their review status. A plugin cannot call Discord,
send a message, read credentials, open files or make network requests.

## Install and remove

Open Extensions to browse bundled starters offline; Refresh explicitly checks the
remote free catalog. Bundled packages use the same capability review before enabling.
Enable downloads the selected, hash-pinned package after its capabilities have
been accepted. Updates are manual and require renewed capability consent.
Import selects a local `.serein-extension` JSON package. An import is unreviewed;
importing alone does not grant it capabilities or execute it.

Disable stops accepting results immediately, then removes Serein's downloaded
package, temporary files and extension data. A failure to remove files is shown
and cleanup is retried on the next load. Re-enabling requires downloading or
importing the package again and starts with fresh extension settings. Serein
never deletes the creator's Git repository, the user's imported original, or an
exported theme/image source.

Plugin grants and data belong to the signed-in account. Logout invalidates
plugin results, drains bounded in-flight work and clears that account's extension data. Theme selection is a device
preference. There are no background catalog refreshes or automatic updates.

## Creator workflow

1. Keep source and license in a public Git repository. Use the standalone Rust
   example under `examples/extensions/message-delete-protector` and its small SDK.
2. Build a Wasm module implementing the version 1 ABI documented by the starter.
   No native binary, installer, Git hook or build script runs on an end user's
   computer. Other languages can implement the same Wasm buffer/JSON contract.
3. Package the manifest and Wasm bytes (or declarative theme) as a single JSON
   file. Test through Import with an offline `--demo` build first.
4. Publish the package as a versioned release artifact. Submit a pull request
   changing `extensions/catalog.json`, with the source commit, build procedure,
   license, artifact URL, byte size and SHA-256 digest.
5. Maintainers review each listed version, its capabilities and the source to
   artifact relationship. A catalog checksum identifies reviewed bytes; it is
   not a signature or a guarantee that code is harmless. Updates need review too.

The in-app catalog reads the default branch. A new catalog entry is not publicly
available through that endpoint until its pull request is merged. Empty catalogs
are valid; imports allow development before a release is listed.

## Shop previews

Catalog entries may include a short `description` (at most 256 characters and
1,024 UTF-8 bytes, without control characters) and a `preview` object:

```json
"preview": {
  "url": "https://example.org/releases/v1/preview.png",
  "sha256": "<64 hexadecimal SHA-256 digits>",
  "download_bytes": 12345
}
```

Use an original or licensed PNG/JPEG screenshot showing the theme or plugin in
use. Pin its URL to an immutable release or source commit, then record the exact
file size and digest. Prefer a 16:9 image; the shop preserves its aspect ratio.
Bundled themes show palette thumbnails using their actual colors. Clicking a bundled
or installed theme thumbnail previews its full appearance in the normal app, with
Back to themes and Customize actions. Bundled themes can be customized without installing
them first; saving creates an editable local copy and preserves the original package.
Editor-created themes may embed a separate local card cover; it replaces the palette
illustration and is center-cropped to 16:9. It is not a catalog preview URL and does
not change the conversation background.
The protector shows a deleted-row illustration. Other entries without an image
remain valid and show a built-in illustration.
Previews describe the listed version, including when an installed version has
an update available; they are creator-provided, not proof of compatibility.

Only visible shop cards request previews. Images use the credential-free,
public-IP-pinned HTTPS downloader with a five-second deadline and must match their
own hash and byte count. Preview loading does not block shop actions; an action
cancels preview-only work before starting.
Decoding runs on the existing cancellable worker: at most 256 KiB compressed,
4,096 pixels per edge, 4,194,304 source pixels, and 32 MiB decoder allocation
budget. Only a static image is decoded; thumbnails shrink to at most 640 x 360.
Missing, invalid or unavailable images fall back without blocking installation.
The UI retains at most eight thumbnails (at most 7,372,800 RGBA bytes), clears
changed-image metadata and releases them with its extension runtime state.
There is no preview disk cache, telemetry, new dependency, or plugin permission.

## Host contract

The `extensions` crate defines the versioned manifest, capability, action,
invocation, result and theme types. These serialized types are the compatibility
boundary; internal `client-core` structures and egui objects are not an SDK.
Unknown API versions and invalid packages are rejected before installation.

Actions are invoked by a message context-menu item, composer tool or panel
button. A plugin may also declare one `activation` action that runs in the worker
on enable/account load. With explicit `deleted_messages` consent, its boolean
`preserve_deleted_messages` output enables session-only retention of loaded deleted
messages. Deleted text is red by default. Hover and a local context menu can toggle
that highlight or remove the retained row. They never call Discord. Live service
actions stay unavailable. Default deletion, tombstone reconciliation and disk-cache
removal remain unchanged. Disabling clears
retained deleted content across active and dormant windows. Logout, lost channel
access and ordinary timeline eviction also clear it. The same 500-row / 4 MiB
per-window budget includes both live and retained deleted payloads. Input is restricted to the granted context and bounded form values.
Results can propose a composer replacement or return native headings, text, rows,
separators, buttons, text inputs, checkboxes, dropdowns and integer sliders. Standalone
panel actions are available from the composer Tools menu as well as the shop. Composer proposals require Apply, retain
the ordinary Send action and are discarded when their originating context is
stale. Account/session changes invalidate outstanding results.

Themes override named colors and native typography, spacing, padding and corner
radii, plus an optional embedded PNG/JPEG background with per-mode opacity and fit.
Settings > Themes > Create theme provides a native editor and portable export.
See the complete [theme API](theme-api.md) for fields, bounds and inheritance.

Plugins can request the `appearance` capability to return an `appearance` object
using that same theme schema. This works from activation or a user-invoked action,
so creators can build native appearance settings panels with dropdowns and sliders.
An action replaces that plugin's previous appearance object; omit it to leave the
current appearance unchanged, or return `{}` to remove its overrides. A plugin can
use granted `storage` to save choices and read them during activation on the next
account load. Activation does not receive conversation text. The host overlays
plugin appearances on the selected theme in ascending plugin-ID order; the last
explicit value wins. The user's own accent color setting still takes precedence.
Disabling a plugin removes its overrides; Ctrl+Shift+F12 resets community themes
and appearance plugins.

This supports app-wide palette changes and shared native control styling, not
arbitrary code injection into egui, replacement of the app layout, custom fonts,
network access, or automatic Discord actions. Fixed-size custom-painted components
keep their existing geometry. Missing fields inherit the underlying theme.

## Resource and privacy limits

Wasm executes on an on-demand background worker with fuel, stack and memory
limits and no WASI or host imports. Each invocation gets a new runtime; it is
dropped when the call completes. Plugin execution never happens in a render or
audio callback. The application bounds package sizes, installed plugin count,
invocation input/output, panel complexity, queues and plugin storage.

Disabled plugins have no retained instance, worker, package or plugin data once
cleanup succeeds. Shared host code and bounded catalog metadata still cost some
application space. Freed allocations may remain in the process allocator; an
unchanged RSS reading alone does not mean an instance remains active.

Plugin storage is ordinary local data, not encrypted by Serein. Do not use it
for credentials. No extension diagnostics or private invocation data are
uploaded. Sandboxing and review reduce exposure but cannot prove absence of
bugs in the runtime or host; keep Serein updated.

The complete ABI and numeric limits are documented in
[`examples/extensions/README.md`](../examples/extensions/README.md). The demo uses
a separate bounded temporary `serein-extension-demo` profile; it can import local
fixtures and browse the embedded starter catalog/previews but cannot download a
catalog, preview or package. `Ctrl+Shift+F12` resets a
community theme if its colors make controls difficult to read.

# Equicord interaction tools

Native Serein behavior ported from selected Equicord workflows. This directory
is a wave record, not a packaged extension: there is no Wasm, manifest or
catalog entry.

Upstream reference: `Equicord/Equicord@200dba124066146a8f043de7fc8b05e3dc5e68e`.

## What changed

- **Always expand role lists** — Profile presentation now has a device-local
  switch that shows every already-loaded role chip instead of collapsing after
  two rows.
- **GIF descriptions** — GIF attachments and `gifv` embeds expose a bounded,
  sanitized `GIF - <name>` description in accessibility labels and hover text
  when the author did not provide one.
- **Attach button** — a primary click opens the existing file picker; secondary
  click keeps the attach-file / create-poll menu.

## Audited, already covered

WriteUpperCase, AccountPanelServerProfile, BetterFolders, BetterRoleDot,
ClearURLs and CopyUserURLs already had native or packaged implementations; they
were reviewed and left unchanged. BiggerStreamPreview is covered by the existing
native voice-roster preview and Watch Stream action. WigglyText is deferred
because the native Markdown renderer has no truthful per-glyph animation path;
see `WAVE.md`.

## Provenance and license

The applied behaviors are derived from the GPL-3.0-or-later Equicord plugins
`alwaysExpandRoles`, `betterGifAltText` and `betterUploadButton`. The native
implementation is original Serein code; no upstream React, Webpack or host
objects are copied. See the repository's third-party notices and the wave record
for exact files and omissions.

## Verification

Only inline unit tests accompany the changed parsing/limit logic. Parent runs
format, Clippy, checks, packaging and any visual verification sequentially. No
live Discord call was made.

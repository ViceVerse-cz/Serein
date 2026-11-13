# ViewRaw — safe message view

A user-invoked Serein port of the message-text portion of Equicord's **ViewRaw** plugin.
Choose **View raw message text** in a message's extension menu to display the exact selected
message text in a native panel, together with the public channel/message/author IDs supplied by
the host. No message is sent or changed.

## Safe differences

- Only `selected_message` is requested. The package cannot read credentials, raw Gateway/store
  objects, profiles, files, attachment contents or unrelated channels and servers.
- It has no network, clipboard, storage, event or host-action capability. IDs are shown only when
  the host includes `selected_message_ref`; older hosts show just the selected text.
- This is deliberately not Equicord's raw-object inspector. Displaying private host internals or
  adding a “redact secrets afterwards” option would weaken Serein's boundary.
- Raw text is split into native 4,000-byte elements. The panel displays at most 64 KiB and reports
  any omitted remainder; payloads whose JSON exceeds the 256 KiB response budget receive a fixed,
  sanitized error instead of an invalid panel.

Upstream inspiration: `src/plugins/viewRaw` in `Equicord/Equicord` at commit
`200dba124066146a8f043fde7fc8b05e3dc5e68e`. The original Vencord notice and GPLv3-or-later
terms are retained in the Rust source. Changed November 8, 2026 as a bounded, user-invoked Rust
port; no JavaScript was copied into Serein.

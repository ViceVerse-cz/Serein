# Serein GPUI experiment

Run from this worktree:

```sh
cargo run --locked -p serein-gpui -- --demo
```

The offline preview never opens the credential store or connects to Discord. It uses the
existing synthetic fixtures, including channel switching and local message sending.

To use your existing account yourself:

```sh
cargo run --locked -p serein-gpui
```

The experiment restores the exact same `cz.viceverse.serein` / `discord-session` OS credential
entry as the main desktop app. No token copying or separate login should be needed when that
entry is valid and the OS permits access. The OS may ask to allow this new executable access.
If needed, confirm account ownership and use **Sign in with Discord**. On macOS/Windows the
same ephemeral hosted-login implementation is attached to the GPUI window. New credentials
are saved only after REST authentication and a matching READY. Linux currently restores saved
logins; use the main Serein app for hosted login there. No live account validation was performed.

The native frontend includes guild and text-channel navigation, DMs, bounded message history,
older pages, gateway updates, permission-aware text sending, and a native IME/clipboard composer.
It reuses the existing model, reducer, transports, secret types, platform keystore, and palette.
[GPUI 0.2.2](https://docs.rs/gpui/0.2.2/gpui/) is pinned for this experiment; macOS builds require the Xcode Metal Toolchain. The composer adapts
its Apache-2.0 native input example. `discord-voice` remains an unconditional dependency.

This is a renderer experiment, not a complete port: rich message content is shown as plain text
or attachment labels; media, voice controls, account switching, and settings stay in the main
app. Drafts exist only in this process and are lost on exit. It does not write the main app's
SQLite caches/settings. There is no logout/credential-deletion control in this prototype.

The default `serein` executable remains the egui app. `!fast` verification uses only the offline
debug run; no release package, benchmark, full test suite, screenshots, commit, or push.

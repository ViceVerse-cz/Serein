# macOS emoji picker and dictation input

Source: crates.io `winit` 0.30.13 (Apache-2.0). Original archive SHA-256:
`a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d`. Retains the upstream
`src`, `build.rs`, README, license and manifest provenance (`Cargo.toml.orig`). Examples, tests,
docs and the upstream lockfile are dropped, and the published manifest omits their
`[[example]]`/`[[test]]` targets and dev-dependencies; Serein's workspace lockfile is used.

The only source change is in `src/platform_impl/macos/view.rs`:

- `ViewState::in_key_down` is true while `keyDown:` runs `interpretKeyEvents:`.
- `insertText:replacementRange:` without marked text, outside `keyDown:`, while the app allows
  IME and the text is not a control character, now queues `Ime::Commit(text)` (preceded by
  `Ime::Enabled` if the view had not reported IME yet). The IME state is not set to
  `Committed`, so the next key press is delivered normally.

Why: upstream only commits `insertText:` when there is marked text (an active composition).
Typed characters are fine because `keyDown:` also sends a `KeyboardInput` carrying the text,
but the macOS Character Viewer (Fn/Globe+E or Control+Command+Space), dictation and similar
input services call `insertText:` directly with no key event and no marked text, so choosing
an emoji in the system picker did nothing in Serein's composer. Insertion inside `keyDown:`
is unchanged, so typed text is not duplicated. winit 0.31.0-beta.4 (`winit-appkit`) still
drops this text.

Remove this patch when a winit release that egui-winit supports delivers `insertText:` text
that arrives outside a key event.

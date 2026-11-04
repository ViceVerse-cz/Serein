# Voice Call Popout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Pop Out button moves the live call stage, with an optional chat panel, into a separate resizable OS window that remembers where it was.

**Architecture:** `crates/ui` owns the popout state and renders through a new `MessagingUi::show_voice_popout`. `apps/desktop` hosts that call inside `ctx.show_viewport_immediate` and owns the window: geometry, fullscreen, close, and file drops. Only one window draws the stage, timeline and composer in any frame. Two new fields in `AppPreferences` persist the auto-open setting and the popout geometry.

**Tech Stack:** Rust (pinned toolchain), egui/eframe 0.36.2 (wgpu) multi-viewport, winit 0.30, `local-store` (SQLite JSON row).

**Spec:** `docs/specs/2026-11-04-voice-popout-design.md`

## Global Constraints

- Viewport id: `egui::ViewportId::from_hash_of("serein-voice-popout")`. Use `show_viewport_immediate` only; no deferred viewport.
- Popout min inner size is 480×320. The first-ever size is 960×640, centred on the main window's monitor. It has OS decorations and is not always-on-top.
- Window title: `<call channel name> — Serein`.
- Fluent keys (en-US only) and their exact text are listed in the spec under "Copy".
- No eframe persistence. Geometry is saved through the existing debounced `AppPreferences` save. `WindowGeometry` conventions: size is logical, position is physical, and the position is `None` on Wayland.
- Synthetic fixtures only. Never live Discord.
- Local machine CPU cap: `~/.cargo/config.toml` sets `build.jobs = 8` and `RUST_TEST_THREADS = 4`. Do not override it with `-j`. Run focused commands (`-p <crate>`) before `cargo xtask check`.

## Review Focus

1. Popout open while the call ends (hang up, kicked, remote end) must close the window the same frame. Owner: Task 2 test `popout_clears_when_call_ends`.
2. In the main window, the user selects another channel while the popout chat is open. The popout chat must close and the main window's composer must own the draft. Owner: Task 2 test `selecting_other_channel_returns_chat`.
3. A saved geometry on a monitor that is now disconnected must open on a visible monitor. Owner: Task 3 step `fit_popout_geometry` self-check.
4. A Watch-Stream join or a reconnect with auto-open on must **not** pop out. Owner: Task 2 test `auto_popout_only_on_join_or_accept`.
5. A file dropped on the popout while its chat is closed (no composer) must show the existing "unavailable" toast and must not upload. Owner: Task 3 smoke step.

---

### Task 1: Preferences and setting toggle

**Review:** per-task — persisted format

**Files:**
- Modify: `crates/local-store/src/lib.rs:38-166` (fields, defaults, `is_valid`)
- Modify: `crates/ui/src/lib.rs` (`MessagingUi` field `voice_popout_auto: bool`, next to `preload_on_hover` at `:260`)
- Modify: `crates/ui/src/voice.rs:2471-2548` (`voice_settings_content`: one `design::switch`)
- Modify: `crates/ui/src/settings.rs:153-155` (Voice page search keywords: "popout", "separate window", "second monitor")
- Modify: `crates/ui/locales/en-US/main.ftl` (6 keys from the spec)
- Modify: `apps/desktop/src/app_settings.rs:34-140` (`observe`/`apply` for `voice_popout_auto`; carry `voice_popout_geometry` through like `window_geometry`)
- Test: `crates/local-store/src/lib.rs` tests module

**Interfaces:**
- Produces: `AppPreferences::voice_popout_auto: bool` (default `false`), `AppPreferences::voice_popout_geometry: Option<WindowGeometry>` (default `None`), `MessagingUi::voice_popout_auto: bool` (pub).

- [ ] **Step 1: Write the failing test** `popout_preferences_round_trip_and_validate`. Saving preferences with `voice_popout_auto = true` and `voice_popout_geometry = Some(WindowGeometry { size: [960, 640], position: Some([-1920, 0]), maximized: true })` reads back equal. Setting `size: [0, 640]` makes `is_valid()` false. A row JSON without either key deserializes to the defaults.
- [ ] **Step 2:** Run `cargo test --locked -p local-store popout_preferences`. Expected: FAIL (no field).
- [ ] **Step 3:** Add the fields and defaults. `is_valid` gains `self.voice_popout_geometry.is_none_or(WindowGeometry::is_valid)`. Add the UI field, the switch (`settings-voice-popout-auto`, with its description key) and the locale keys. Add the `observe`/`apply` lines. Desktop keeps `voice_popout_geometry` in its preferences copy so `observe` does not wipe it (Task 3 writes it).
- [ ] **Step 4:** Run `cargo test --locked -p local-store popout_preferences`, then `cargo test --locked -p serein app_settings` (use the desktop package name from `apps/desktop/Cargo.toml`). Expected: PASS.
- [ ] **Step 5:** Commit `feat(voice): persist call popout preferences`.

### Task 2: Popout state, shared stage and placeholders in `crates/ui`

**Files:**
- Modify: `crates/ui/src/voice.rs` (extract `voice_stage`; Pop Out button beside `fullscreen_control` at `:3369-3390`; placeholder; popout chat bubble)
- Modify: `crates/ui/src/lib.rs` (state fields, `show_voice_popout`, chat ownership in `show` around `:5047-5222`, logout clear near `:1207`)
- Modify: `crates/ui/src/icons.rs` (Pop Out glyph next to `Fullscreen` at `:126`)
- Test: `crates/ui/examples/voice_chat.rs`

**Interfaces:**
- Consumes: `MessagingUi::voice_popout_auto` (Task 1).
- Produces (all `pub` on `MessagingUi`, used by Task 3):
  - `fn voice_popout(&self) -> Option<model::Id>`: the call channel while popped out.
  - `fn show_voice_popout(&mut self, ui: &mut egui::Ui, state: &mut State) -> Vec<Command>`: draws the stage plus the optional right-side chat panel (timeline + composer) into `ui`.
  - `fn close_voice_popout(&mut self)`: docks without navigating; also clears popout chat and popout fullscreen.
  - `fn take_voice_popout_fullscreen_request(&mut self) -> Option<bool>`.
  - `fn voice_popout_chat_open(&self) -> bool`.
- Internal: `fn voice_stage(ui: &mut egui::Ui, view: &mut MessagingUi, state: &State, channel: Id, commands: &mut Vec<Command>)` paints a full-pane stage into `ui.max_rect()` for both guild and DM calls. `voice_channel` and the popout both call it. `call_bar` keeps its top panel but calls it for the body. `call_controls` stays single-sourced.

- [ ] **Step 1: Write failing checks** in `voice_chat.rs` (same headless-frame style as the existing fullscreen checks):
  - `pop_out_button`: clicking "Pop Out" makes `voice_popout() == Some(call channel)` and exits docked fullscreen. Docked frames then show `voice-popout-docked` text and no stage tiles. "Bring back" calls the close path, and `voice_popout()` becomes `None`.
  - `popout_clears_when_call_ends`: set `state.voice.active = None` (or logout), and `voice_popout()` becomes `None` within that frame.
  - `auto_popout_only_on_join_or_accept`: with `voice_popout_auto = true`, clicking Join (guild) or Accept (incoming DM call) pops out. A Watch-Stream join or a phase change on an already active call does not. Closing the popout, then reconnecting, does not reopen it.
  - `popout_chat_owns_composer`: in `show_voice_popout`, toggle the chat bubble. A `Select { channel: call channel }` command is emitted. With `state.selected == call channel`, the main `show` renders `voice-popout-chat-elsewhere` and no composer. Typing in the popout composer changes `state.drafts[call channel]`.
  - `selecting_other_channel_returns_chat`: with the popout chat open, set `state.selected` to another channel. `voice_popout_chat_open()` becomes false and the main composer renders.
  - Fullscreen button inside `show_voice_popout`: `take_voice_popout_fullscreen_request() == Some(true)`. Escape gives `Some(false)`. `take_voice_fullscreen_request()` stays `None`.
- [ ] **Step 2:** Run `cargo run --locked -p ui --example voice_chat`. Expected: FAIL at the first new check.
- [ ] **Step 3:** Implement:
  - Fields: `voice_popout: Option<Id>`, `voice_popout_chat: bool`, `voice_popout_fullscreen: bool`, `voice_popout_fullscreen_request: Option<bool>`.
  - Auto-open: hook the Join and Accept/Start-call command emission sites, not state transitions.
  - Clear on call end: compare against `state.voice.active` at the top of `show`.
  - Chat ownership rule from the spec.
  - `voice_focus` stays global; it's safe because only one stage renders.
- [ ] **Step 4:** Run `cargo run --locked -p ui --example voice_chat` and `cargo test --locked -p ui voice`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(voice): pop out call stage state and rendering`.

### Task 3: Desktop popout window

**Files:**
- Modify: `apps/desktop/src/main.rs` (viewport call after `self.messaging.show` at `:6297`; drop handling `:6034-6068` moved into `fn accept_dropped_files(&mut self, ctx: &egui::Context, dropped: Vec<egui::DroppedFile>)` and reused)
- Modify: `apps/desktop/src/app_settings.rs` (`pub fn fit_popout_geometry(window: &winit::window::Window, saved: Option<WindowGeometry>) -> WindowGeometry`, built on `fit_window_geometry` `:281`; extend the self-check at `:300-370`)

**Interfaces:**
- Consumes: Task 1 `voice_popout_geometry`; Task 2 `voice_popout`, `show_voice_popout`, `close_voice_popout`, `take_voice_popout_fullscreen_request`.

- [ ] **Step 1: Failing self-check** in the `--demo-check-window-geometry` path:
  - `fit_popout_geometry(None)` gives 960×640 centred on the current monitor.
  - A saved position on a monitor index that no longer exists lands fully inside an available monitor.
  - `position: None` stays `None`.
- [ ] **Step 2:** Run `cargo run --locked -p <desktop> -- --demo-check-window-geometry`. Expected: FAIL.
- [ ] **Step 3:** Implement.
  - **Opening:** while `voice_popout().is_some()`, call `ctx.show_viewport_immediate`. The builder takes the title, `with_min_inner_size([480.,320.])` and `with_decorations(true)`. Position, size and maximized come from `fit_popout_geometry`, computed once when the popout opens.
  - **Inside the callback:**
    - `close_requested()`: call `close_voice_popout()` and send `ViewportCommand::Close`.
    - Fullscreen: apply the fullscreen request with `ctx.send_viewport_cmd_to(id, ViewportCommand::Fullscreen(b))`.
    - Drops: take the child's `raw.dropped_files` into `accept_dropped_files`.
    - Geometry: sample `i.viewport()` when it isn't maximized, fullscreen or minimized. Convert with `native_pixels_per_point`. Drop the position when `outer_rect` is `None`. Write it to prefs only on change, through the existing save queue.
  - **Commands:** append the popout's `Vec<Command>` to the frame's `commands`.
  - **Root-only code:** title strip and `sync_window_effects` stay root-only.
  - **Paste:** it already flows through `attachment_paste_requested`. Verify it in the smoke step.
- [ ] **Step 4:** Run the self-check (PASS).
- [ ] **Step 5: Smoke (Windows):** run with `--demo-voice-video`. Pop out, move the window to the second monitor, maximize, then fullscreen and Esc.
  - Open chat, type and send (demo); drop a file; paste an image.
  - Minimize the main window, then tray-hide it: video keeps moving.
  - Close the popout. Restart, pop out again: it opens at the saved place.
  - Drop a file with the chat closed: you get the toast and no upload.
  - Take before/after screenshots.
- [ ] **Step 6:** Commit `feat(desktop): host voice popout window`.

### Task 4: Docs and full check

**Files:**
- Modify: `docs/design.md:144-149` (Pop Out and placeholder; also fix the stale "camera/screen shown disabled" line)
- Modify: `docs/voice.md:13-24` (popout, chat-follows-popout rule, auto-open setting)
- Modify: `docs/platform-support.md:1-33` (popout geometry; Wayland has no position, and a compositor tray-hide can stall the popout)

- [ ] **Step 1:** Write the docs.
- [ ] **Step 2:** Run `cargo xtask check`. Expected: pass.
- [ ] **Step 3:** Run `cargo xtask package` (runtime change). Expected: success.
- [ ] **Step 4:** Commit `docs(voice): document call popout`, then follow `serein-delivery` for the PR.

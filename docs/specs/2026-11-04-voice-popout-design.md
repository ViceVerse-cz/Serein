# Voice call popout window — design

## Intent

Discord's call view has a **Pop Out** button that moves the call stage into its own OS window. The
owner puts that window on a second monitor and keeps the main window free to browse channels and
chat. Serein should do the same: one click moves the live call stage (participant tiles, cameras,
watched streams, control bar) into a separate, normal desktop window, with an optional chat panel
for the call's channel beside it.

Success: during a call on a dual-monitor setup the owner pops the call out, maximizes it on monitor
two, keeps using Serein normally on monitor one, and next time the popout reopens where it was.

## Goals

- **Pop Out** icon button in the call control bar, right of the existing Fullscreen button, in both
  stages: guild voice channel (`voice_channel`, `crates/ui/src/voice.rs:895`) and DM/group call
  (`call_bar`, `voice.rs:3520`). Tooltip "Pop Out".
- Popout window: native OS title bar, movable, resizable, maximizable, **not** always-on-top. Title
  `<call channel name> — Serein`. Minimum inner size 480×320.
- Popout content: the full stage exactly as docked (tiles, best-fit grid, click-to-enlarge focus,
  participant strip, hover chrome, notices, stream overlay controls, complete control bar).
- **Fullscreen inside the popout** makes the popout fullscreen on its own monitor (borderless via
  `ViewportCommand::Fullscreen(true)` sent to the popout viewport). Escape or the button exits. The
  main window is never touched. Unlike the docked button, it is available for any stage, not only a
  focused screen/stream tile.
- **Toggleable chat panel**: a chat-bubble button at the popout's top-right shows the call channel's
  text chat (message list + composer) to the right of the stage. "Chat follows popout" (below).
- Main window while popped out: the stage area shows a compact placeholder — "Call is in another
  window" with a **Bring back** button — and everything else (channel list, other channels, chat)
  works normally.
- Geometry remembered: popout inner size, outer position (when the platform reports one) and
  maximized flag persist in Serein preferences and are restored on the next popout.
- Setting **"Open calls in a separate window"** (Settings → Voice, default off). When on, every call
  that becomes active (join or accept) opens straight into the popout.
- Closing: closing the popout window docks the stage back into the main window. Ending the call
  (hang up, kicked, call ended) closes the popout. Logout closes it.

## Non-goals

- Picture-in-picture / always-on-top mini player.
- More than one popout, or popping out individual tiles/streams.
- A chat in the popout that is independent from the main window's selected channel.
- Extension SDK exposure (no new `AppView`/`HostEffect`; the feature is not author-visible).
- Popout for the camera-test / stream-preview dialogs in Settings.

## Anti-goals (failure even if it "works")

- The stage or composer rendering in **both** windows in one frame (duplicate egui ids, double
  command dispatch, a draft sent to the wrong channel).
- Video in the popout freezing or dropping rate when the main window is minimized or hidden to tray
  on Windows.
- Leaving the main window stuck fullscreen, or the popout surviving after the call ended.
- Using eframe persistence (forbidden by `tools/xtask`) or unbounded preference data.

## Constraints and decided approach

- **Window mechanism (decided):** `egui::Context::show_viewport_immediate` with
  `ViewportId::from_hash_of("serein-voice-popout")`, called once per root frame from
  `Desktop::ui` (`apps/desktop/src/main.rs`, next to `self.messaging.show(...)`). Immediate is
  chosen because its `FnMut` callback can borrow `MessagingUi` and `State` directly; deferred would
  require `Send + Sync` shared state and a second copy of the stage plumbing.
  - Verified in the pinned eframe 0.36.2 (rev `4821fc5`): when a visible immediate child exists,
    the root keeps running `App::logic`/`App::ui` even when minimized, tray-hidden
    (`ViewportCommand::Visible(false)`) or occluded (`wgpu_integration.rs` `show_ui` /
    `is_viewport_or_descendant_visible`), so the popout keeps painting on Windows.
  - Cost: each popout frame is a full root pass. Accepted; the stage already animates.
  - Worker wakes (`ctx.request_repaint()`) target ROOT, which renders the child inline — no change
    needed. Inside the popout closure, `request_repaint*` targets the popout.
- **One owner per frame:** a single `MessagingUi` flag decides where the stage renders. When popped
  out, `voice_channel`/`call_bar` draw the placeholder instead of the stage; the popout draws the
  stage. `call_controls` stays single-sourced in a shared stage function extracted from
  `voice_channel`/`call_bar`.
- **Commands:** popout UI pushes into the same `Vec<Command>` the root frame already processes.
- **Textures:** unchanged. Video `TextureId`s live in the one shared `egui_wgpu::Renderer`, so the
  popout paints them as-is. `gpu_video::collect()` stays once per root frame.
- **Chat follows popout (decided):** the timeline and composer are single-instance and bound to
  `state.selected` (`timeline.rs:2107`, send path in `client-core` uses `selected`). So:
  - Opening the popout chat panel selects the call channel (normal select command) and renders the
    existing timeline + composer inside the popout's right panel.
  - While the popout chat is open and `state.selected` is the call channel, the main window's
    central area shows a placeholder "Chat is open in the call window" instead of the
    timeline/composer, so they render once per frame.
  - If the user selects another channel in the main window, the popout chat panel closes
    automatically (main window owns chat again). Reopening it re-selects the call channel.
  - Guild calls: the call channel's text-in-voice chat. DM/group calls: the DM itself.
- **Existing main-window fullscreen** (`voice_fullscreen`, `voice.rs:65-123`): Pop Out exits it
  first; while popped out the docked fullscreen path is unreachable.
- **Window chrome:** the popout uses OS decorations. Serein's root-only custom title strip and DWM
  effects code must not run for it.
- **Persistence:** add to `local_store::AppPreferences` (`crates/local-store/src/lib.rs`):
  - `voice_popout_auto: bool` (default `false`).
  - `voice_popout_geometry: Option<WindowGeometry>` (default `None`), validated with the same rules
    as the main window geometry, and restored through the existing `fit_window_geometry` so an
    off-screen/disconnected-monitor position is corrected. `#[serde(default)]` means no migration.
  - Geometry is sampled inside the popout callback from `ui.input(|i| i.viewport())` (the root
    raw-input hook can't see child viewports) and saved through the existing debounced
    preferences save path, not every frame.
- **Platform:** Windows primary. Wayland reports no window position (restore size only) and a
  compositor-level tray hide can stall immediate viewports; documented as a limitation in
  `docs/platform-support.md`. macOS/X11 follow eframe behavior.
- Repo rules: synthetic fixtures only, no live Discord tests; few, high-signal tests.

## Copy (en-US, new Fluent keys; other locales fall back to English)

- `voice-pop-out` = Pop Out
- `voice-popout-docked` = Call is in another window
- `voice-popout-bring-back` = Bring back
- `voice-popout-chat-elsewhere` = Chat is open in the call window
- `settings-voice-popout-auto` = Open calls in a separate window
- `settings-voice-popout-auto-description` = Calls start in their own window, ready for a second monitor.

## Answers to the builder check

- **DM/group stage in the popout (my call):** both call kinds render as a full-pane stage — body
  fills the window, control bar at the bottom, like the guild `voice_channel`. The DM `call_bar`
  fractional top panel, header and drag-resize stay main-window-only. The shared stage function is
  rect-driven: `voice_stage(ui, state, call_channel, commands)` paints into `ui.max_rect()`.
- **Existing in-window voice chat split (my call):** `voice_chat_open` keeps working in the main
  window; while popped out the main window shows the stage placeholder where the stage was and the
  normal conversation beside it. Chat ownership rule: if `voice_popout_chat` is on and
  `state.selected == call channel`, the popout owns timeline + composer and the main window shows
  the chat placeholder; otherwise the main window owns them. The popout chat toggle is new state,
  not `voice_chat_open`.
- **Auto popout trigger (owner):** only on the user's own join of a voice channel or accept/start
  of a DM call. Watch-Stream joins, reconnects and phase changes never open it. Closing the popout
  keeps the call docked until the next join/accept.
- **Composer parity (owner):** full. Typing, replies, edits, emoji/GIF/sticker pickers, file
  drag-and-drop and clipboard image paste all work in the popout composer. Dropped files and paste
  events read from the popout's own raw input inside the viewport callback and are routed into the
  same `uploads` staging the root window uses.
- **Dock back (owner):** closing the popout does not navigate. The flag clears; the stage shows
  again whenever the call channel (or DM) is open in the main window.
- **Geometry plumbing (my call):** the root `winit::Window` is available in `Desktop` and its
  `available_monitors()` lists all monitors, so the saved popout geometry is fitted with the
  existing `fit_window_geometry` before the popout opens and passed to
  `ViewportBuilder::with_position/with_inner_size/with_maximized`. Sampling inside the callback
  converts `ViewportInfo::outer_rect` (points) to physical pixels with
  `native_pixels_per_point`, keeps the last normal geometry while maximized or fullscreen, and
  never stores a position on Wayland (`None`), matching `WindowGeometry` conventions.

## Left to the builder

- Icon glyph for Pop Out (match the existing `icons.rs` set; arrow-out-of-box like Discord).
- Placeholder visual details, chat panel default width (resizable side panel), exact layout of the
  chat bubble button.
- First-ever popout size: 960×640 centred on the main window's monitor.

## Verification

- Extend `crates/ui/examples/voice_chat.rs` (synthetic): Pop Out sets the popout flag and exits
  docked fullscreen; docked stage renders the placeholder while popped out; Bring back / close
  request docks it without navigating; call end and logout clear it; auto-popout fires on
  join/accept only, not on Watch-Stream join or reconnect; popout chat selects the call channel
  and the main window shows the chat placeholder; selecting another channel closes popout chat.
- `local-store` test: preferences with popout geometry round-trip; invalid geometry rejected.
- Manual Windows smoke with `--demo-voice-video`: pop out, move to second monitor, maximize,
  fullscreen, drop a file and paste an image into the popout composer, minimize and tray-hide the
  main window while video keeps moving, close, reopen at the saved place. Screenshots for the PR.
- Docs: `docs/design.md` (voice stage), `docs/voice.md` (popout + chat behavior),
  `docs/platform-support.md` (child window geometry, Wayland limits).

# Serein's egui fork

Source: GitHub `emilk/egui`, commit `cd5caec79c9e6683dd793e25b4e7b287ccdd7061` (egui 0.37.0
plus unreleased `main`), MIT OR Apache-2.0. Each crate directory is the output of
`cargo package --no-verify` for that crate after applying `patches/` on top of that commit:
it keeps upstream sources, the original manifest (`Cargo.toml.orig`) and both license files.
The packaged manifests point their egui-family dependencies at the same git revision as the
Serein workspace, and the root `[patch."https://github.com/emilk/egui"]` table maps the six
vendored crates here, so Cargo resolves one copy of each. Unpatched egui crates (`emath`,
`ecolor`, `epaint_default_fonts`, `egui_glow`) still come from the pinned git revision.
Serein's workspace `Cargo.lock` controls resolution.

## Patch series

`patches/` holds the series as `git format-patch` output, in application order. Upstream pull
requests are squashed from the PR head that was reviewed; all of them are unmerged upstream
(Preview) and may change before they land.

| Patch | Upstream | Crates | Effect |
| --- | --- | --- | --- |
| 0001 | [#8849](https://github.com/emilk/egui/pull/8849) | `egui_system_font_provider` | System fallback also finds fullwidth forms and CJK punctuation (Unicode script `Common`). |
| 0002 | [#8804](https://github.com/emilk/egui/pull/8804) | `epaint` | Fallback-font glyphs (CJK, emoji) sit on the row's shared baseline instead of their own face's. |
| 0003 | [#8768](https://github.com/emilk/egui/pull/8768) | `egui`, `eframe` | `App::repaint_on_window_event`/`repaint_on_device_event` and `RawInput::repaint_on_pointer_move` let an app skip motion-only repaints. Serein declines repaints for raw device motion only. |
| 0004 | [#8775](https://github.com/emilk/egui/pull/8775) | `egui` | Dragging an `egui::Window` title bar no longer accumulates rounding drift. |
| 0005 | [#8710](https://github.com/emilk/egui/pull/8710) | `egui-winit` | Mouse buttons are not dropped after `CursorLeft` (Windows reports it when another window appears over the pointer). |
| 0006 | [#8764](https://github.com/emilk/egui/pull/8764) | `egui` | A click elsewhere takes focus from a widget only when that widget is shown. |
| 0007 | [#8801](https://github.com/emilk/egui/pull/8801) | `egui-wgpu` | `SurfaceConfig::pre_present_notify` (default on, unchanged behavior) can be turned off for immediate viewports on Wayland. Serein keeps the default. |
| 0008 | Serein | `egui-winit` | Windows drag repaints and Wayland image paste, described below. |
| 0009 | Serein | `epaint` | Opt-in RTL shaping metadata for logical message layout, described below. |
| 0010 | [#8774](https://github.com/emilk/egui/pull/8774) | `epaint` | Invisible shapes between two images on the same texture no longer split their draw batch. |
| 0011 | [#8732](https://github.com/emilk/egui/pull/8732) | `eframe`, `egui-winit` | Every native window gets the app's initial light/dark theme before it is first shown. Supersedes #8730. |
| 0012 | [#8737](https://github.com/emilk/egui/pull/8737) | `eframe` | A root window built with `with_visible(false)` stays hidden after the first frame. Serein's root is not built hidden. |
| 0013 | [#8785](https://github.com/emilk/egui/pull/8785) | `egui` | The AccessKit root window node is named after the viewport title. |
| 0014 | [#8810](https://github.com/emilk/egui/pull/8810) | `eframe` | Child viewports get their own AccessKit adapter. |
| 0015 | [#8799](https://github.com/emilk/egui/pull/8799) | `egui` | Resizing a panel below its content's minimum no longer shrinks its clip rect past that minimum. |
| 0016 | [#8705](https://github.com/emilk/egui/pull/8705) | `egui` | Explicitly sized widgets keep their size in sizing passes, so a bottom-pivoted `Area` no longer jumps on its first frame. The PR's demo snapshot image is not carried. |
| 0017 | [#8722](https://github.com/emilk/egui/pull/8722) | `egui`, `egui-winit`, `eframe` | Native and web key events keep keypad Enter's physical key; the logical key stays `Key::Enter`. |
| 0018 | [#8766](https://github.com/emilk/egui/pull/8766) | `egui` | On macOS, Control-click is a secondary click. Serein's chat selection plugin already did this for its own input; both agree. |
| 0019 | [#5180](https://github.com/emilk/egui/pull/5180) | `egui` | Right-clicking a `TextEdit` keeps its cursor and selection (completed by 0022). |
| 0020 | [#8734](https://github.com/emilk/egui/pull/8734) | `egui` | A `ScrollArea` that cannot scroll on an axis passes that axis's scroll target to its parent. |
| 0021 | [#8814](https://github.com/emilk/egui/pull/8814) | `egui` | Requesting focus for the widget that already has it no longer interrupts IME composition. |
| 0022 | Serein | `egui` | Fixes 0019 for a right-click whose press and release arrive in one pass: it checked the button's state after the pass, so the cursor still moved. Adds `tests/text_edit_right_click.rs`; suggest upstream on #5180. |
| 0023 | Serein | `epaint`, `egui_system_font_provider` | Emoji-presentation clusters (🆕, or ♾ with U+FE0F) use a face that draws them in color instead of an earlier text font's plain glyph, and the system font provider asks for the emoji font first. Text presentation is unchanged. Fixes #913. |
| 0024 | [#8698](https://github.com/emilk/egui/pull/8698) | `epaint`, `egui` | Right-to-left runs get one glyph per char instead of `2n - 1`, so editor cursors stay inside the text (End then Ctrl+ArrowLeft in Arabic text no longer overflows). Merged with 0009's direction override. |
| 0025 | Serein | `epaint` | Laying out a multi-paragraph job one paragraph at a time keeps an empty first section, which carries `Label`'s wrap indentation, so text continuing a line after an inline widget (a heading after a channel chip) is no longer painted over that line. |

## Serein patch: RTL shaping metadata

`FontsView::layout_single_line_with_clusters` shapes one already bidi-ordered visual line with
an explicit direction per section and returns the galley plus each native shaping cluster's
UTF-8 source range and advance rect. Serein (`crates/ui/src/rtl.rs`) does bounded logical line
breaking and per-line Unicode bidi ordering, then uses that cluster map for painting, hit
testing and logical selection/copy. The single-line path skips upstream's ascending-only
continuation-glyph bookkeeping, which misplaces RTL clusters. `FontsView::layout_generation`
is a process-wide counter that changes whenever fonts are rebuilt or native galleys are
invalidated (including atlas resets), so Serein's bounded layout cache never reuses meshes with
stale atlas UVs. Default `LayoutJob` layout is unchanged. Remove this patch when upstream exposes
equivalent direction and cluster-mapping APIs with correct wrapped bidi layout. It does not
provide bidirectional `TextEdit`, IME or screen-reader parity.

## Serein patch: Windows window-drag repaints

The drag changes in `egui-winit/src/lib.rs` apply on Windows only:

- `State::on_window_event`: a new arm replaces the immediate `WindowEvent::Moved(_)` repaint
  with a delayed 500 ms repaint, so a final placement/monitor sample occurs even without
  subsequent client input.
- `State::on_mouse_motion`: raw `DeviceEvent::MouseMotion` is no longer forwarded as
  `egui::Event::MouseMoved` and never requests a repaint. Serein reads neither that event nor
  `PointerState::motion`; `Response::drag_motion` falls back to pointer deltas.

Why: dragging the custom title strip enters the OS modal move loop. Every `WM_WINDOWPOSCHANGED`
became a repaint, and raw input kept arriving during the loop: each motion event requested a
repaint, and any frame that received the forwarded events requested the next one
(`InputState::wants_repaint_after`). The loop dispatches `WM_PAINT` synchronously, so the full UI
pass ran and the `PresentMode::Fifo` present blocked on vsync before the loop could advance — the
window moved at most once per refresh and trailed the cursor. Moving a window leaves its contents
unchanged, so neither repaint was needed; with either change alone the drag stays paced at the
refresh rate. Upstream tracks the symptom as
[emilk/egui#3953](https://github.com/emilk/egui/issues/3953) (open, no fix).

## Serein patch: Wayland image paste and clipboard fixes

On Wayland, `Clipboard::get_image` first reads `image/png` through the patched smithay-clipboard
(32 MiB read, 64 MiB decode allocation) before falling back to arboard; see
`vendor/smithay-clipboard/SEREIN-PATCH.md`. Empty clipboard text counts as absent so image paste
can fall back, and image copies log success only on success. Other platform input behavior is
inherited from the pinned upstream revision.

## Updating

1. Check out the new upstream commit, apply `patches/` with `git am -3`, and drop any patch whose
   PR has merged. Refresh a PR patch from its current head when it changed upstream.
2. Run `cargo package --no-verify -p eframe -p egui -p egui-wgpu -p egui-winit
   -p egui_system_font_provider -p epaint`, extract each `.crate` here without `Cargo.lock`
   or `.cargo_vcs_info.json`, copy `LICENSE-APACHE`/`LICENSE-MIT`, and add `git`/`rev` to each
   `[*dependencies.<egui crate>]` table. Packaging keeps only `.rs` files of `epaint`, so also copy
   `src/text/test_fonts/` (fixtures for patch 0023's test).
3. Regenerate `patches/` with `git format-patch -N --zero-commit --no-signature`, update the
   revision in the root `Cargo.toml`, `THIRD_PARTY_NOTICES.md` and license provenance, and
   remove a crate from this directory and the `[patch]` table once none of its patches remain.

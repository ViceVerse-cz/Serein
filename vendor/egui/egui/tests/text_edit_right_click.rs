use egui::{
    Context, Event, Id, PointerButton, Pos2, RawInput, Rect, TextEdit, pos2,
    text_edit::TextEditState, vec2,
};

/// Runs one pass of a single-line field and returns its rect and cursor index.
/// Passes are a second apart, so consecutive clicks never count as a double-click.
fn pass(ctx: &Context, id: Id, events: Vec<Event>) -> (Rect, Option<usize>) {
    let mut text = String::from("hello world");
    let mut rect = Rect::NOTHING;
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))),
            time: Some(ctx.input(|i| i.time) + 1.0),
            events,
            ..Default::default()
        },
        |ui| rect = ui.add(TextEdit::singleline(&mut text).id(id)).rect,
    );
    output.textures_delta.clear();
    let cursor = TextEditState::load(ctx, id)
        .and_then(|state| state.cursor.char_range())
        .map(|range| range.primary.index.0);
    (rect, cursor)
}

/// Press and release in the same pass, as a quick click or synthetic input delivers them.
fn click(pos: Pos2, button: PointerButton) -> Vec<Event> {
    let button = |pressed| Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: Default::default(),
    };
    vec![Event::PointerMoved(pos), button(true), button(false)]
}

#[test]
fn a_right_click_within_one_pass_keeps_the_cursor() {
    let ctx = Context::default();
    let id = Id::unique("field");
    let (rect, _) = pass(&ctx, id, vec![]);
    let start = rect.left_center() + vec2(2.0, 0.0);
    let end = rect.right_center() - vec2(2.0, 0.0);

    pass(&ctx, id, click(end, PointerButton::Primary));
    assert_eq!(
        pass(&ctx, id, vec![]).1,
        Some(11),
        "a left click places the cursor"
    );

    pass(&ctx, id, click(start, PointerButton::Secondary));
    assert_eq!(pass(&ctx, id, vec![]).1, Some(11), "a right click keeps it");

    pass(&ctx, id, click(start, PointerButton::Primary));
    assert_eq!(pass(&ctx, id, vec![]).1, Some(0));
}

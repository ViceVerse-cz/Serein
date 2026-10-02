//! Server settings keep one modal size on every page (synthetic offline data only).
use ui::MessagingUi;

fn frame(ctx: &egui::Context, view: &mut MessagingUi, state: &mut client_core::State) {
	let output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(1280.0, 800.0),
			)),
			..Default::default()
		},
		|ui| {
			view.show(ui, state);
		},
	);
	output.drop_without_applying_deltas();
}

#[test]
fn server_settings_pages_share_one_modal_rect() {
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let mut state = test_support::chat_demo_state();
	let mut permissions = test_support::permission_snapshot(&state);
	for guild in &mut permissions.guilds {
		guild.owner = state.user.as_ref().map(|u| u.id);
	}
	state.permissions.replace(permissions).unwrap();
	let guild = state.guilds[0].id;
	let mut view = MessagingUi::default();
	let area = egui::Id::unique("server-settings");
	let mut first: Option<(&str, egui::Rect)> = None;
	for page in [
		"profile",
		"engagement",
		"emoji",
		"stickers",
		"members",
		"roles",
		"role-editor",
		"role-permissions",
		"invites",
		"integrations",
		"webhooks",
		"audit-log",
	] {
		match page {
			"profile" => drop(view.preview_server_settings(&mut state, guild)),
			"engagement" => {
				drop(view.preview_server_settings(&mut state, guild));
				view.preview_server_engagement();
			}
			_ => drop(view.preview_server_admin(&mut state, guild, page)),
		}
		for _ in 0..4 {
			frame(&ctx, &mut view, &mut state);
		}
		let rect = ctx
			.memory(|memory| memory.area_rect(area))
			.unwrap_or_else(|| panic!("{page}: server settings did not open"));
		match first {
			None => first = Some((page, rect)),
			Some((name, expected)) => assert!(
				(rect.size() - expected.size()).length() < 0.5,
				"{page} {rect:?} differs from {name} {expected:?}"
			),
		}
	}
}

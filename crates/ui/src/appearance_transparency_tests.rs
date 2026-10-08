//! Painted offline regressions for opaque controls inside a translucent native window.
use crate::{MessagingUi, design};
use client_core::State;
use egui::{Color32, Context, CornerRadius, Pos2, Rect, Shape, Vec2};
use model::Id;

const EFFECTS: [(bool, u8); 4] = [(false, 100), (true, 0), (true, 50), (true, 100)];

struct ResetAppearance;

impl ResetAppearance {
	fn new() -> Self {
		design::set_extension_theme(None);
		design::set_window_effects(false, 15, 50);
		Self
	}
}

impl Drop for ResetAppearance {
	fn drop(&mut self) {
		design::set_extension_theme(None);
		design::set_window_effects(false, 15, 50);
	}
}

#[derive(Default)]
struct Paint {
	rects: Vec<(Rect, CornerRadius, Color32)>,
	circles: Vec<(f32, Color32)>,
	text: Vec<(String, Color32)>,
}

impl Paint {
	fn add(&mut self, shape: &Shape, clip: Rect) {
		match shape {
			Shape::Rect(rect) if clip.intersects(rect.rect) => {
				self.rects.push((rect.rect, rect.corner_radius, rect.fill));
			}
			Shape::Circle(circle) if clip.contains(circle.center) => {
				self.circles.push((circle.radius, circle.fill));
			}
			Shape::Text(text) if clip.intersects(text.visual_bounding_rect()) => {
				let color = text.override_text_color.unwrap_or_else(|| {
					text.galley
						.job
						.sections
						.first()
						.map(|section| section.format.color)
						.filter(|color| *color != Color32::PLACEHOLDER)
						.unwrap_or(text.fallback_color)
				});
				self.text.push((text.galley.text().to_owned(), color));
			}
			Shape::Vec(shapes) => {
				for shape in shapes {
					self.add(shape, clip);
				}
			}
			_ => {}
		}
	}

	fn text_color(&self, label: &str) -> Color32 {
		self.text
			.iter()
			.find(|(text, _)| text == label)
			.unwrap_or_else(|| panic!("missing visible label {label}"))
			.1
	}

	fn rect_fill(&self, target: Rect) -> Color32 {
		self.rects
			.iter()
			.find(|(rect, _, _)| *rect == target)
			.unwrap_or_else(|| panic!("missing painted control at {target:?}"))
			.2
	}
}

fn context(light: bool, effects: (bool, u8)) -> Context {
	design::set_window_effects(effects.0, effects.1, 50);
	let ctx = Context::default();
	ctx.set_theme(if light {
		egui::ThemePreference::Light
	} else {
		egui::ThemePreference::Dark
	});
	design::apply(&ctx);
	ctx
}

fn render(ctx: &Context, size: Vec2, mut show: impl FnMut(&mut egui::Ui)) -> Paint {
	let mut paint = Paint::default();
	for pass in 0..3 {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
				time: Some(f64::from(pass) * 0.25),
				focused: true,
				..Default::default()
			},
			&mut show,
		);
		assert!(output.platform_output.commands.is_empty());
		paint = Paint::default();
		for shape in &output.shapes {
			paint.add(&shape.shape, shape.clip_rect);
		}
		output.drop_without_applying_deltas();
	}
	paint
}

#[derive(Debug, PartialEq, Eq)]
struct SettingsColors {
	modal: Color32,
	navigation: Option<Color32>,
	previews: Vec<Color32>,
	heading: Color32,
}

fn settings_colors(light: bool, size: Vec2, effects: (bool, u8)) -> SettingsColors {
	let ctx = context(light, effects);
	let mut state = test_support::demo_state();
	let mut view = MessagingUi::default();
	view.settings.open = true;
	let mut commands = vec![];
	let paint = render(&ctx, size, |ui| {
		view.show_settings(ui.ctx(), &mut state, &mut commands);
	});
	assert!(commands.is_empty());
	let (modal_rect, _, modal) = *paint
		.rects
		.iter()
		.find(|(rect, radius, fill)| {
			*radius == CornerRadius::same(crate::dialog::RADIUS)
				&& rect.width() > 280.0
				&& rect.height() > 240.0
				&& fill.a() == 255
		})
		.expect("Appearance modal surface must be painted");
	let wide = size.x - 32.0 >= 620.0;
	let navigation =
		wide.then(|| {
			paint
				.rects
				.iter()
				.find(|(rect, radius, fill)| {
					(200.0..=260.0).contains(&rect.width())
						&& rect.height() > 240.0
						&& (rect.left() - modal_rect.left()).abs() < 32.0
						&& (rect.top() - modal_rect.top()).abs() < 32.0
						&& radius.nw > 0 && radius.sw > 0
						&& radius.ne == 0 && radius.se == 0
						&& fill.a() == 255
				})
				.unwrap_or_else(|| {
					let candidates: Vec<_> = paint
						.rects
						.iter()
						.filter(|(rect, _, _)| rect.height() > 240.0)
						.take(8)
						.collect();
					panic!(
						"wide Appearance navigation must be painted beside {modal_rect:?}: {candidates:?}"
					)
				})
				.2
		});
	let previews: Vec<_> = paint
		.rects
		.iter()
		.filter(|(rect, radius, fill)| {
			*radius == CornerRadius::same(6)
				&& (rect.height() - 34.0).abs() < 0.1
				&& (rect.width() - 52.0).abs() < 0.1
				&& fill.a() > 0
		})
		.map(|(_, _, fill)| *fill)
		.collect();
	assert_eq!(
		previews.len(),
		3,
		"Dark, Light and System previews must be visible"
	);
	SettingsColors {
		modal,
		navigation,
		previews,
		heading: paint.text_color("Appearance"),
	}
}

#[test]
fn appearance_settings_and_mode_previews_keep_their_colors_at_every_window_opacity() {
	let _reset = ResetAppearance::new();
	for light in [false, true] {
		for size in [egui::vec2(1120.0, 760.0), egui::vec2(540.0, 760.0)] {
			let baseline = settings_colors(light, size, EFFECTS[0]);
			assert_eq!(baseline.modal.a(), 255);
			for effects in EFFECTS {
				assert_eq!(
					settings_colors(light, size, effects),
					baseline,
					"light={light}, size={size:?}, effects={effects:?}"
				);
			}
		}
	}
}

fn folder_fill(light: bool, effects: (bool, u8)) -> Color32 {
	let ctx = context(light, effects);
	let mut state = test_support::demo_state();
	state.guild_folders = Some(model::guild_folders::Settings {
		folders: vec![model::guild_folders::Folder {
			id: Some(7),
			name: Some("Synthetic folder".into()),
			color: Some(0x517fbd),
			guild_ids: vec![Id(10)],
		}],
		..Default::default()
	});
	state.folders_pending = false;
	state.folders_stale = false;
	let mut view = MessagingUi {
		expanded_folders: vec![7],
		..Default::default()
	};
	let mut commands = vec![];
	let paint = render(&ctx, egui::vec2(200.0, 400.0), |ui| {
		view.server_folders(ui, &mut state, &mut commands);
	});
	assert!(commands.is_empty());
	paint
		.rects
		.iter()
		.find(|(rect, radius, _)| *radius == CornerRadius::same(16) && rect.height() > 46.0)
		.expect("expanded folder must paint its shared background")
		.2
}

#[test]
fn expanded_server_folders_keep_their_theme_tint_at_every_window_opacity() {
	let _reset = ResetAppearance::new();
	for light in [false, true] {
		let baseline = folder_fill(light, EFFECTS[0]);
		assert_eq!(baseline.a(), 255);
		for effects in EFFECTS {
			assert_eq!(
				folder_fill(light, effects),
				baseline,
				"light={light}, effects={effects:?}"
			);
		}
	}
}

fn private_channels() -> State {
	let mut state = test_support::demo_state();
	state
		.channels
		.retain(|channel| matches!(channel.id, Id(20) | Id(25)));
	for channel in &mut state.channels {
		channel.parent_id = None;
		channel.name = if channel.kind == 2 {
			"Synthetic private voice"
		} else {
			"Synthetic private text"
		}
		.into();
	}
	state.invalidate_navigation();
	let owner = state.user.as_ref().unwrap().id;
	let mut snapshot = test_support::permission_snapshot(&state);
	for guild in &mut snapshot.guilds {
		guild.owner = Some(owner);
		for role in guild.roles.iter_mut().flatten() {
			role.bits &= !model::permissions::VIEW_CHANNEL;
		}
	}
	state.permissions.replace(snapshot).unwrap();
	for channel in [Id(20), Id(25)] {
		assert!(state.channel_access(channel).limited());
		assert!(!state.channel_access(channel).hidden());
	}
	state
}

#[derive(Debug, PartialEq, Eq)]
struct ChannelColors {
	halos: Vec<Color32>,
	text: Color32,
	voice: Color32,
}

fn channel_colors(light: bool, selected: bool, effects: (bool, u8)) -> ChannelColors {
	let ctx = context(light, effects);
	let mut state = private_channels();
	state.selected = selected.then_some(Id(20));
	let mut view = MessagingUi {
		guild: Some(Id(10)),
		..Default::default()
	};
	let paint = render(&ctx, egui::vec2(260.0, 400.0), |ui| {
		let _ = view.channel_list(ui, &mut state);
	});
	let halos: Vec<_> = paint
		.circles
		.iter()
		.filter(|(radius, _)| (*radius - 7.5).abs() < 0.01)
		.map(|(_, fill)| *fill)
		.collect();
	assert!(
		halos.is_empty(),
		"private text and voice locks must leave the backdrop visible"
	);
	ChannelColors {
		halos,
		text: paint.text_color("Synthetic private text"),
		voice: paint.text_color("Synthetic private voice"),
	}
}

#[test]
fn private_text_and_voice_channel_locks_have_no_halo_at_every_window_opacity() {
	let _reset = ResetAppearance::new();
	for light in [false, true] {
		for selected in [false, true] {
			let baseline = channel_colors(light, selected, EFFECTS[0]);
			for effects in EFFECTS {
				assert_eq!(
					channel_colors(light, selected, effects),
					baseline,
					"light={light}, selected={selected}, effects={effects:?}"
				);
			}
		}
	}
}

#[derive(Debug, PartialEq, Eq)]
struct ButtonColors {
	shared_fill: Color32,
	shared_text: Color32,
	plain_fill: Color32,
	plain_text: Color32,
}

fn button_colors(light: bool, effects: (bool, u8)) -> ButtonColors {
	let ctx = context(light, effects);
	let mut shared = Rect::NOTHING;
	let mut plain = Rect::NOTHING;
	let paint = render(&ctx, egui::vec2(400.0, 200.0), |ui| {
		let response = design::button(ui, "Shared primary", design::ButtonKind::Primary);
		assert!(response.enabled());
		shared = response.rect;
		let response = ui.button("Plain button");
		assert!(response.enabled());
		plain = response.rect;
	});
	ButtonColors {
		shared_fill: paint.rect_fill(shared),
		shared_text: paint.text_color("Shared primary"),
		plain_fill: paint.rect_fill(plain),
		plain_text: paint.text_color("Plain button"),
	}
}

fn contrast(text: Color32, background: Color32) -> f32 {
	fn luminance(color: Color32) -> f32 {
		assert_eq!(color.a(), 255, "contrast requires actual opaque paint");
		let [r, g, b, _] = color.to_srgba_unmultiplied();
		let linear = |channel: u8| {
			let value = f32::from(channel) / 255.0;
			if value <= 0.04045 {
				value / 12.92
			} else {
				((value + 0.055) / 1.055).powf(2.4)
			}
		};
		0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
	}
	let a = luminance(text);
	let b = luminance(background);
	(a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn enabled_shared_and_plain_buttons_preserve_their_fill_and_readable_text_at_every_window_opacity()
{
	let _reset = ResetAppearance::new();
	for light in [false, true] {
		let baseline = button_colors(light, EFFECTS[0]);
		assert!(contrast(baseline.shared_text, baseline.shared_fill) >= 4.5);
		assert!(contrast(baseline.plain_text, baseline.plain_fill) >= 4.5);
		for effects in EFFECTS {
			assert_eq!(
				button_colors(light, effects),
				baseline,
				"light={light}, effects={effects:?}"
			);
		}
	}
}

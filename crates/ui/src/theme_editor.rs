//! Native theme drafts. Package and image IO belongs to the desktop worker.
use crate::{ExtensionRequest, design, dialog};
use extensions::{
	Background, BackgroundFit, BackgroundTarget, ExtensionKind, Manifest, Package, Theme,
};
use std::sync::{
	Arc,
	atomic::{AtomicU64, Ordering},
};

pub(crate) struct ThemeEditor {
	pub package: Box<Package>,
	pub image: Option<Arc<egui::ColorImage>>,
	pub dirty: bool,
	pub preview: bool,
	dark: bool,
	discard: bool,
	tab: usize,
}

fn identity() -> String {
	static NEXT: AtomicU64 = AtomicU64::new(0);
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_nanos();
	format!(
		"local-theme-{now:x}-{:x}",
		NEXT.fetch_add(1, Ordering::Relaxed)
	)
}

impl ThemeEditor {
	pub fn new() -> Self {
		Self {
			package: Box::new(Package {
				manifest: Manifest {
					api_version: extensions::API_VERSION,
					id: identity(),
					name: "My theme".into(),
					version: "1.0.0".into(),
					author: String::new(),
					license: "CC0-1.0".into(),
					source: String::new(),
					kind: ExtensionKind::Theme,
					capabilities: vec![],
					actions: vec![],
				},
				theme: Some(Theme::default()),
				wasm: vec![],
				background_image: vec![],
			}),
			image: None,
			dirty: false,
			preview: false,
			dark: true,
			discard: false,
			tab: 0,
		}
	}
	pub fn duplicate(mut package: Box<Package>, image: Option<Arc<egui::ColorImage>>) -> Self {
		package.manifest.id = identity();
		package.manifest.name = format!(
			"{} copy",
			package.manifest.name.chars().take(30).collect::<String>()
		);
		Self {
			package,
			image,
			dirty: true,
			..Self::new()
		}
	}
	pub fn receive_image(&mut self, bytes: Vec<u8>, image: Arc<egui::ColorImage>) {
		self.package.background_image = bytes;
		self.image = Some(image);
		self.dirty = true;
		if let Some(theme) = self.package.theme.as_mut() {
			for palette in [&mut theme.light, &mut theme.dark] {
				palette.background.get_or_insert(Background {
					opacity: 25,
					fit: BackgroundFit::Cover,
					target: BackgroundTarget::Chat,
				});
			}
		}
	}
	pub fn preview_request(&self) -> ExtensionRequest {
		ExtensionRequest::PreviewTheme {
			theme: self.package.theme.clone().map(Box::new),
			image: self.image.clone(),
		}
	}
	/// Returns true once the draft can be discarded.
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		busy: bool,
		requests: &mut Vec<ExtensionRequest>,
	) -> bool {
		let mut close = false;
		let mut changed = false;
		ui.add_enabled_ui(!busy, |ui| {
			ui.horizontal_wrapped(|ui| {
				close = dialog::action(ui, "Back to themes", dialog::Action::Neutral).clicked();
				if self.dirty { ui.weak("Unsaved changes"); }
				if busy { ui.spinner(); }
			});
			ui.add_space(12.0);
			design::section(ui, "Your theme", None);
			design::card(ui, |ui| {
				changed |= text_field(ui, "Theme name", &mut self.package.manifest.name, 32, "My theme");
				changed |= text_field(ui, "Created by", &mut self.package.manifest.author, 32, "Your name");
			});
			ui.add_space(16.0);
			ui.horizontal_wrapped(|ui| {
				ui.label("Appearance");
				for (dark, label) in [(true, "Dark"), (false, "Light")] {
					if dialog::action(ui, label, if self.dark == dark { dialog::Action::Primary } else { dialog::Action::Outline }).clicked() { self.dark = dark; }
				}
			});
			ui.add_space(8.0);
			if dialog::action(ui, "Preview in app", dialog::Action::Outline).clicked() {
				self.preview = true;
				requests.push(self.preview_request());
			}
			design::hint(ui, "See this theme in your conversations, then return here to keep editing.");
			if design::primary_color().is_some() { ui.weak("Your custom primary color overrides this theme's accent in the app."); }
			design::divider(ui);
			ui.horizontal_wrapped(|ui| {
				for (index, label) in ["Background", "Colors", "Typography", "Details"].into_iter().enumerate() {
					if dialog::action(ui, label, if self.tab == index { dialog::Action::Primary } else { dialog::Action::Neutral }).clicked() { self.tab = index; }
				}
			});
			ui.add_space(16.0);
			let theme = self.package.theme.as_mut().expect("theme editor always holds a theme");
			let palette = if self.dark { &mut theme.dark } else { &mut theme.light };
			let base = design::builtin_colors(self.dark, design::variant());
			match self.tab {
				0 => {
					design::section(ui, "Background image", Some("Add a photo or illustration behind your messages."));
					design::card(ui, |ui| {
						ui.horizontal_wrapped(|ui| {
							if dialog::action(ui, "Choose image", dialog::Action::Outline).clicked() { requests.push(ExtensionRequest::PickThemeImage); }
							if !self.package.background_image.is_empty() && dialog::action(ui, "Remove image", dialog::Action::Neutral).clicked() {
								self.package.background_image.clear(); self.image = None; palette.background = None; changed = true;
							}
						});
						ui.weak("PNG or JPEG, up to 2 MiB and 4 million pixels.");
						if !self.package.background_image.is_empty() {
							ui.add_space(12.0);
							let background = palette.background.get_or_insert(Background { opacity: 25, fit: BackgroundFit::Cover, target: BackgroundTarget::Chat });
							changed |= row(ui, "Show image in", |ui| {
								let a = ui.selectable_value(&mut background.target, BackgroundTarget::Chat, "Message area").changed();
								a | ui.selectable_value(&mut background.target, BackgroundTarget::Window, "Whole window").changed()
							});
							changed |= row(ui, "Image opacity", |ui| ui.add(egui::Slider::new(&mut background.opacity, 0..=100).suffix("%")).changed());
							changed |= row(ui, "Image fit", |ui| {
								let a = ui.selectable_value(&mut background.fit, BackgroundFit::Cover, "Cover").changed();
								a | ui.selectable_value(&mut background.fit, BackgroundFit::Contain, "Contain").changed()
							});
							if background.target == BackgroundTarget::Window {
								ui.add_space(8.0);
								ui.weak("Lower surface opacity to reveal the image behind the app.");
								for (key, default) in [("sidebar", base.sidebar), ("chat", base.chat)] { changed |= opacity(ui, key, &mut palette.colors, default); }
							}
						}
					});
					ui.add_space(20.0);
					design::section(ui, "Window gradient", Some("Blend two colors across the app background."));
					let mut gradient = palette.backdrop.is_some();
					if design::switch(ui, "Use a gradient", None, &mut gradient).changed() {
						palette.backdrop = gradient.then(|| [hex(base.base), hex(base.chat)]); changed = true;
					}
					if let Some(stops) = &mut palette.backdrop {
						for (index, stop) in stops.iter_mut().enumerate() {
							ui.push_id(index, |ui| { changed |= row(ui, if index == 0 { "Start color" } else { "End color" }, |ui| color_input(ui, stop)); });
						}
					}
				}
				1 => {
					design::section(ui, "Colors", Some("Changes apply to the selected appearance. Reset any color to inherit the app default."));
					for (key, fallback) in colors(base) { ui.push_id(key, |ui| { changed |= color_override(ui, key, &mut palette.colors, fallback); }); }
				}
				2 => {
					design::section(ui, "Text sizes", Some("Typography and spacing apply to both light and dark appearances."));
					let s = &mut theme.style;
					for (label, value, default, min, max) in [("Message & body text", &mut s.body_size, 15, 10, 28), ("Headings", &mut s.heading_size, 20, 12, 40), ("Buttons", &mut s.button_size, 14, 10, 28), ("Small text", &mut s.small_size, 12, 10, 28), ("Code", &mut s.monospace_size, 14, 10, 28)] { changed |= metric(ui, label, value, default, min..=max); }
					ui.add_space(20.0);
					design::section(ui, "Spacing & corners", None);
					changed |= metric(ui, "Control height", &mut s.control_height, 32, 24..=56);
					changed |= pair_metric(ui, "Item spacing", &mut s.item_spacing, [8, 8]);
					changed |= pair_metric(ui, "Button padding", &mut s.button_padding, [12, 6]);
					for (label, value, default) in [("Control corners", &mut s.widget_radius, 8), ("Window corners", &mut s.window_radius, 12), ("Menu corners", &mut s.menu_radius, 12)] { changed |= metric(ui, label, value, default, 0..=24); }
				}
				_ => {
					design::section(ui, "Sharing details", Some("These details travel with your exported theme."));
					let m = &mut self.package.manifest;
					changed |= text_field(ui, "License", &mut m.license, 32, "CC0-1.0");
					changed |= text_field(ui, "Version", &mut m.version, 32, "1.0.0");
					changed |= text_field(ui, "Source URL", &mut m.source, 512, "Optional");
					ui.add_space(8.0);
					ui.weak("Use your own image or one its license allows you to share. Keep attribution when duplicating a theme.");
				}
			}
			self.dirty |= changed;
			if changed && self.preview { requests.push(self.preview_request()); }
			design::divider(ui);
			let valid = self.package.validate().is_ok();
			if !valid { ui.weak("Add a theme name and creator, and check your colors and sharing details before saving."); ui.add_space(8.0); }
			ui.horizontal_wrapped(|ui| {
				ui.add_enabled_ui(valid, |ui| {
					if dialog::action(ui, "Save and apply", dialog::Action::Primary).clicked() { requests.push(ExtensionRequest::SaveTheme { package: self.package.clone() }); }
					if dialog::action(ui, "Export theme", dialog::Action::Outline).clicked() { requests.push(ExtensionRequest::ExportTheme { package: self.package.clone() }); }
				});
				close |= dialog::action(ui, "Cancel", dialog::Action::Neutral).clicked();
			});
			ui.add_space(8.0);
			ui.weak("Emergency reset: Ctrl+Shift+F12");
		});
		if close && !busy {
			self.discard = self.dirty;
			if !self.dirty {
				return true;
			}
		}
		if self.discard {
			match dialog::Confirm::new(
				"discard-theme-draft",
				"Discard unsaved theme?",
				"Your changes have not been saved.",
			)
			.confirm_label("Discard changes")
			.cancel_label("Keep editing")
			.danger()
			.show(ui.ctx())
			{
				Some(dialog::Choice::Confirmed) => return true,
				Some(dialog::Choice::Cancelled) => self.discard = false,
				None => {}
			}
		}
		false
	}
}

fn rgba([r, g, b, a]: [u8; 4]) -> egui::Color32 {
	egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}
fn hex(color: egui::Color32) -> String {
	let [r, g, b, a] = color.to_srgba_unmultiplied();
	format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
}
fn colors(p: design::Palette) -> [(&'static str, egui::Color32); 18] {
	[
		("base", p.base),
		("sidebar", p.sidebar),
		("chat", p.chat),
		("raised", p.raised),
		("hover", p.hover),
		("selected", p.selected),
		("border", p.border),
		("text_strong", p.text_strong),
		("text", p.text),
		("muted", p.muted),
		("link", p.link),
		("accent", p.accent),
		("accent_text", p.accent_text),
		("positive", p.positive),
		("warning", p.warning),
		("danger", p.danger),
		("mention_bg", p.mention_bg),
		("mention_text", p.mention_text),
	]
}
fn color_input(ui: &mut egui::Ui, value: &mut String) -> bool {
	ui.horizontal(|ui| {
		let mut color = extensions::parse_color(value)
			.map(rgba)
			.unwrap_or(egui::Color32::TRANSPARENT);
		let mut changed = ui.color_edit_button_srgba(&mut color).changed();
		if changed {
			*value = hex(color);
		}
		changed |= ui
			.allocate_ui(egui::vec2(132.0, 0.0), |ui| {
				design::input(
					ui,
					egui::TextEdit::singleline(value)
						.char_limit(9)
						.font(egui::FontId::proportional(14.0)),
				)
				.changed()
			})
			.inner;
		if extensions::parse_color(value).is_err() {
			ui.colored_label(ui.visuals().error_fg_color, "!")
				.on_hover_text("Use #RRGGBB or #RRGGBBAA");
		}
		changed
	})
	.inner
}
fn color_label(key: &str) -> &str {
	match key {
		"base" => "Window background",
		"sidebar" => "Sidebar",
		"chat" => "Message area",
		"raised" => "Cards & message input",
		"hover" => "Hover",
		"selected" => "Selection",
		"border" => "Borders",
		"text_strong" => "Headings",
		"text" => "Body text",
		"muted" => "Secondary text",
		"link" => "Links",
		"accent" => "Accent",
		"accent_text" => "Text on accent",
		"positive" => "Success",
		"warning" => "Warning",
		"danger" => "Error & danger",
		"mention_bg" => "Mention background",
		"mention_text" => "Mention text",
		_ => key,
	}
}
/// Settings rows align values at the right; narrow pages stack instead of clipping controls.
fn row(ui: &mut egui::Ui, label: &str, controls: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
	ui.push_id(label, |ui| {
		ui.add_space(4.0);
		if ui.available_width() < 440.0 {
			ui.label(label);
			ui.horizontal_wrapped(controls).inner
		} else {
			ui.horizontal(|ui| {
				ui.label(label);
				let width = 270.0;
				ui.add_space((ui.available_width() - width).max(0.0));
				ui.allocate_ui_with_layout(
					egui::vec2(width, 0.0),
					egui::Layout::left_to_right(egui::Align::Center),
					controls,
				)
				.inner
			})
			.inner
		}
	})
	.inner
}
fn text_field(
	ui: &mut egui::Ui,
	label: &str,
	value: &mut String,
	limit: usize,
	hint: &str,
) -> bool {
	ui.push_id(label, |ui| {
		let colors = design::palette(ui);
		let label = ui.label(design::eyebrow(ui, label, colors.muted));
		let changed = design::input(
			ui,
			egui::TextEdit::singleline(value)
				.char_limit(limit)
				.font(egui::FontId::proportional(15.0))
				.hint_text(hint),
		)
		.labelled_by(label.id)
		.changed();
		ui.add_space(8.0);
		changed
	})
	.inner
}
fn color_override(
	ui: &mut egui::Ui,
	key: &str,
	map: &mut std::collections::BTreeMap<String, String>,
	fallback: egui::Color32,
) -> bool {
	row(ui, color_label(key), |ui| {
		let mut value = map.get(key).cloned().unwrap_or_else(|| hex(fallback));
		let mut changed = color_input(ui, &mut value);
		if changed {
			map.insert(key.into(), value);
		}
		if ui
			.add_enabled(map.contains_key(key), egui::Button::new("Reset"))
			.clicked()
		{
			map.remove(key);
			changed = true;
		}
		changed
	})
}
fn opacity(
	ui: &mut egui::Ui,
	key: &str,
	map: &mut std::collections::BTreeMap<String, String>,
	fallback: egui::Color32,
) -> bool {
	row(ui, &format!("{} opacity", color_label(key)), |ui| {
		let mut color = map
			.get(key)
			.and_then(|v| extensions::parse_color(v).ok())
			.unwrap_or(fallback.to_srgba_unmultiplied());
		let mut percent = f32::from(color[3]) * 100.0 / 255.0;
		if ui
			.add(egui::Slider::new(&mut percent, 0.0..=100.0).suffix("%"))
			.changed()
		{
			color[3] = (percent * 255.0 / 100.0).round() as u8;
			map.insert(key.into(), hex(rgba(color)));
			true
		} else {
			false
		}
	})
}
fn metric(
	ui: &mut egui::Ui,
	label: &str,
	value: &mut Option<u8>,
	default: u8,
	range: std::ops::RangeInclusive<u8>,
) -> bool {
	row(ui, label, |ui| {
		let mut n = value.unwrap_or(default);
		let mut changed = ui
			.add(egui::Slider::new(&mut n, range).suffix(" px"))
			.changed();
		if changed {
			*value = Some(n);
		}
		if ui
			.add_enabled(value.is_some(), egui::Button::new("Reset"))
			.clicked()
		{
			*value = None;
			changed = true;
		}
		changed
	})
}
fn pair_metric(
	ui: &mut egui::Ui,
	label: &str,
	value: &mut Option<[u8; 2]>,
	default: [u8; 2],
) -> bool {
	row(ui, label, |ui| {
		let mut pair = value.unwrap_or(default);
		let mut changed = false;
		for (index, n) in pair.iter_mut().enumerate() {
			changed |= ui
				.add(egui::DragValue::new(n).range(0..=24).prefix(if index == 0 {
					"X "
				} else {
					"Y "
				}))
				.changed();
		}
		if changed {
			*value = Some(pair);
		}
		if ui
			.add_enabled(value.is_some(), egui::Button::new("Reset"))
			.clicked()
		{
			*value = None;
			changed = true;
		}
		changed
	})
}

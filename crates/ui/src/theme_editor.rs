//! Native theme drafts. Package and image IO belongs to the desktop worker.
use crate::{ExtensionRequest, design, dialog, icons};
use extensions::{
	Background, BackgroundFit, BackgroundTarget, ExtensionKind, Manifest, Package, SectionOpacity,
	Theme,
};
use std::sync::{
	Arc,
	atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum EditorTab {
	#[default]
	Basics,
	Background,
	Colors,
	Advanced,
}
impl EditorTab {
	const ALL: [Self; 4] = [Self::Basics, Self::Background, Self::Colors, Self::Advanced];
	fn label(self) -> &'static str {
		match self {
			Self::Basics => "Basics",
			Self::Background => "Background",
			Self::Colors => "Colors",
			Self::Advanced => "Advanced",
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ImageRegion {
	TopBars,
	ServerList,
	PeopleChannels,
	#[default]
	MessageList,
	MemberList,
	InputArea,
}
impl ImageRegion {
	fn label(self) -> &'static str {
		match self {
			Self::TopBars => "Top bars",
			Self::ServerList => "Server list",
			Self::PeopleChannels => "People & channels",
			Self::MessageList => "Message list",
			Self::MemberList => "Member list",
			Self::InputArea => "Message input area",
		}
	}
	fn description(self) -> &'static str {
		match self {
			Self::TopBars => "Window title and conversation header",
			Self::ServerList => "The left server rail",
			Self::PeopleChannels => "Direct messages and channel navigation",
			Self::MessageList => "The conversation timeline",
			Self::MemberList => "The member and search pane on the right",
			Self::InputArea => "The area around the message box",
		}
	}
	fn opacity(self, sections: &mut SectionOpacity) -> &mut u8 {
		match self {
			Self::TopBars => &mut sections.top_bar,
			Self::ServerList => &mut sections.server_list,
			Self::PeopleChannels => &mut sections.channel_list,
			Self::MessageList => &mut sections.message_list,
			Self::MemberList => &mut sections.member_list,
			Self::InputArea => &mut sections.composer,
		}
	}
}

pub(crate) struct ThemeEditor {
	pub package: Box<Package>,
	pub image: Option<Arc<egui::ColorImage>>,
	pub cover: Option<Arc<egui::ColorImage>>,
	pub dirty: bool,
	pub preview: bool,
	dark: bool,
	tab: EditorTab,
	region: ImageRegion,
	discard: bool,
	show_errors: bool,
	validation_error: Option<(EditorTab, &'static str)>,
	reveal_advanced_colors: bool,
	reveal_gradient: bool,
	/// Open state of the Advanced disclosures, so they survive a repaint.
	open_colors: bool,
	open_gradient: bool,
	open_effects: bool,
	open_metrics: bool,
	thumbnail: Option<egui::TextureHandle>,
	cover_thumbnail: Option<egui::TextureHandle>,
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
	#[cfg(feature = "demo")]
	pub(crate) fn preview_tab(&mut self, label: &str) {
		if let Some(tab) = EditorTab::ALL
			.into_iter()
			.find(|tab| tab.label().eq_ignore_ascii_case(label))
		{
			self.tab = tab;
		}
	}
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
				cover_image: vec![],
			}),
			image: None,
			cover: None,
			dirty: false,
			preview: false,
			dark: true,
			tab: EditorTab::Basics,
			region: ImageRegion::MessageList,
			discard: false,
			show_errors: false,
			validation_error: None,
			reveal_advanced_colors: false,
			reveal_gradient: false,
			open_colors: false,
			open_gradient: false,
			open_effects: false,
			open_metrics: false,
			thumbnail: None,
			cover_thumbnail: None,
		}
	}
	pub fn edit(
		package: Box<Package>,
		image: Option<Arc<egui::ColorImage>>,
		cover: Option<Arc<egui::ColorImage>>,
	) -> Self {
		Self {
			package,
			image,
			cover,
			..Self::new()
		}
	}
	pub fn duplicate(
		mut package: Box<Package>,
		image: Option<Arc<egui::ColorImage>>,
		cover: Option<Arc<egui::ColorImage>>,
	) -> Self {
		package.manifest.id = identity();
		package.manifest.name = format!(
			"{} copy",
			package.manifest.name.chars().take(30).collect::<String>()
		);
		let mut editor = Self::edit(package, image, cover);
		editor.dirty = true;
		editor
	}
	pub fn receive_cover(&mut self, bytes: Vec<u8>, image: Arc<egui::ColorImage>) {
		self.package.cover_image = bytes;
		self.cover = Some(image);
		self.cover_thumbnail = None;
		self.dirty = true;
	}
	pub fn receive_image(&mut self, bytes: Vec<u8>, image: Arc<egui::ColorImage>) {
		self.package.background_image = bytes;
		self.image = Some(image);
		self.thumbnail = None;
		self.dirty = true;
		if let Some(theme) = self.package.theme.as_mut() {
			for palette in [&mut theme.light, &mut theme.dark] {
				let background = palette.background.get_or_insert(Background::default());
				background.opacity = 100;
				background.target = BackgroundTarget::Window;
				background.sections.get_or_insert_default();
			}
		}
	}
	pub fn preview_request(&self) -> ExtensionRequest {
		ExtensionRequest::PreviewTheme {
			theme: self.package.theme.clone().map(Box::new),
			image: self.image.clone(),
		}
	}
	pub fn tab_key(&self) -> u8 {
		self.tab as u8
	}
	fn ready_to_save(&self) -> bool {
		let manifest = &self.package.manifest;
		[
			&manifest.name,
			&manifest.author,
			&manifest.license,
			&manifest.version,
		]
		.into_iter()
		.all(|value| !value.trim().is_empty())
			&& self.package.validate().is_ok()
	}
	fn invalid_gradient(&self) -> bool {
		self.package.theme.as_ref().is_some_and(|theme| {
			[&theme.light, &theme.dark].into_iter().any(|palette| {
				palette.backdrop.as_ref().is_some_and(|stops| {
					stops
						.iter()
						.any(|stop| extensions::parse_color(stop).is_err())
				})
			})
		})
	}
	fn save_error(&self) -> (EditorTab, &'static str, Option<bool>, bool) {
		let manifest = &self.package.manifest;
		if manifest.name.trim().is_empty() || manifest.author.trim().is_empty() {
			return (
				EditorTab::Basics,
				"Add a theme name and creator name before saving.",
				None,
				false,
			);
		}
		if manifest.license.trim().is_empty() || manifest.version.trim().is_empty() {
			return (
				EditorTab::Advanced,
				"Add a license and version before saving.",
				None,
				false,
			);
		}
		if manifest.validate().is_err() {
			return (
				EditorTab::Advanced,
				"Check the license, version, and optional source URL.",
				None,
				false,
			);
		}
		if let Some(theme) = &self.package.theme {
			for (dark, palette) in [(false, &theme.light), (true, &theme.dark)] {
				if let Some((name, _)) = palette
					.colors
					.iter()
					.find(|(_, color)| extensions::parse_color(color).is_err())
				{
					let basic =
						["chat", "accent", "text", "muted", "sidebar"].contains(&name.as_str());
					return (
						if basic {
							EditorTab::Colors
						} else {
							EditorTab::Advanced
						},
						"Correct the highlighted color value.",
						Some(dark),
						!basic,
					);
				}
				if palette.background.is_some_and(|background| {
					background.opacity > 100
						|| background.sections.is_some_and(|sections| {
							[
								sections.top_bar,
								sections.server_list,
								sections.channel_list,
								sections.message_list,
								sections.member_list,
								sections.composer,
							]
							.into_iter()
							.any(|opacity| opacity > 100)
						})
				}) {
					return (
						EditorTab::Background,
						"Keep image and section opacity between 0% and 100%.",
						Some(dark),
						false,
					);
				}
				if palette.backdrop.as_ref().is_some_and(|stops| {
					stops
						.iter()
						.any(|stop| extensions::parse_color(stop).is_err())
				}) {
					return (
						EditorTab::Advanced,
						"Correct the highlighted gradient value.",
						Some(dark),
						false,
					);
				}
			}
		}
		(
			EditorTab::Advanced,
			"Check the remaining theme settings before saving.",
			None,
			false,
		)
	}
	/// The settings shell keeps these actions outside its scrolling content.
	pub fn toolbar(
		&mut self,
		ui: &mut egui::Ui,
		busy: bool,
		requests: &mut Vec<ExtensionRequest>,
	) -> bool {
		let mut close = false;
		ui.add_enabled_ui(!busy, |ui| {
			ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
			ui.horizontal_wrapped(|ui| {
				if dialog::action(ui, "Back", dialog::Action::Outline).clicked() {
					if self.dirty {
						self.discard = true;
					} else {
						close = true;
					}
				}
				if self.dirty {
					ui.label(
						egui::RichText::new("Unsaved changes")
							.size(12.0)
							.color(design::palette(ui).warning),
					);
				}
				if ui.max_rect().width() >= 600.0 {
					ui.add_space((ui.available_size_before_wrap().x - 278.0).max(0.0));
				}
				let valid = self
					.package
					.theme
					.as_ref()
					.is_some_and(|theme| theme.validate().is_ok());
				ui.add_enabled_ui(valid, |ui| {
					if dialog::action(ui, "Preview in app", dialog::Action::Outline).clicked() {
						self.preview = true;
						requests.push(self.preview_request());
					}
				});
				if dialog::action(ui, "Save and apply", dialog::Action::Primary).clicked() {
					self.show_errors = true;
					if self.ready_to_save() {
						self.validation_error = None;
						requests.push(ExtensionRequest::SaveTheme {
							package: self.package.clone(),
						});
					} else {
						let (tab, message, dark, reveal_colors) = self.save_error();
						self.tab = tab;
						if let Some(dark) = dark {
							self.dark = dark;
						}
						self.reveal_advanced_colors = reveal_colors;
						self.reveal_gradient = self.invalid_gradient();
						self.validation_error = Some((tab, message));
					}
				}
			});
		});
		if busy {
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 8.0;
				ui.add(egui::Spinner::new().size(14.0));
				ui.label(
					egui::RichText::new("Working…")
						.size(12.0)
						.color(design::palette(ui).muted),
				);
			});
		}
		ui.add_space(8.0);
		ui.horizontal_wrapped(|ui| {
			ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
			let labels: Vec<&str> = EditorTab::ALL.iter().map(|tab| tab.label()).collect();
			let current = EditorTab::ALL
				.iter()
				.position(|tab| *tab == self.tab)
				.unwrap_or_default();
			if let Some(index) = design::segmented(ui, &labels, current) {
				self.tab = EditorTab::ALL[index];
			}
			if self.tab != EditorTab::Basics {
				if ui.available_size_before_wrap().x >= 230.0 {
					ui.add_space((ui.available_size_before_wrap().x - 220.0).max(0.0));
				}
				appearance_switch(ui, &mut self.dark);
			}
		});
		ui.add_space(4.0);
		design::card_divider(ui);
		close
	}

	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		busy: bool,
		requests: &mut Vec<ExtensionRequest>,
	) -> bool {
		let mut changed = false;
		ui.add_enabled_ui(!busy, |ui| {
			ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
			if let Some((_, message)) = self.validation_error.filter(|(tab, _)| *tab == self.tab) {
				design::notice(ui, design::Level::Error, message);
				ui.add_space(12.0);
			}
			match self.tab {
				EditorTab::Basics => {
					design::section(
						ui,
						"Theme details",
						Some("How your theme appears in the gallery."),
					);
					let show_errors = self.show_errors;
					design::card(ui, |ui| {
						let manifest = &mut self.package.manifest;
						changed |= text_field(ui, "Theme name", &mut manifest.name, 32, "My theme");
						if show_errors && manifest.name.trim().is_empty() {
							design::notice(ui, design::Level::Error, "Theme name is required.");
						}
						changed |=
							text_field(ui, "Created by", &mut manifest.author, 32, "Your name");
						if show_errors && manifest.author.trim().is_empty() {
							design::notice(ui, design::Level::Error, "Creator name is required.");
						}
					});
					ui.add_space(20.0);
					design::section(
						ui,
						"Card cover",
						Some("Choose the image shown on your theme card in Themes."),
					);
					self.cover_card(ui, requests, &mut changed);
				}
				EditorTab::Background => {
					design::section(
						ui,
						"App background",
						Some("Use one image behind your conversations and sidebars."),
					);
					self.image_card(ui, requests, &mut changed);
					if !self.package.background_image.is_empty() {
						let theme = self
							.package
							.theme
							.as_mut()
							.expect("theme editor always holds a theme");
						let palette = if self.dark {
							&mut theme.dark
						} else {
							&mut theme.light
						};
						let background = palette.background.get_or_insert(Background {
							opacity: 100,
							sections: Some(SectionOpacity::default()),
							..Default::default()
						});
						if background.sections.is_none() {
							design::hint(ui, "This older theme uses its original image placement.");
							if dialog::action(
								ui,
								"Use image across the app",
								dialog::Action::Outline,
							)
							.clicked()
							{
								background.target = BackgroundTarget::Window;
								background.opacity = 100;
								background.sections = Some(SectionOpacity::default());
								changed = true;
							} else {
								changed |= design::slider_row(
									ui,
									"Image opacity",
									None,
									&mut background.opacity,
									0..=100,
									"%",
								)
								.changed();
							}
						}
						changed |= row(ui, "Image fit", |ui| {
							let mut changed = false;
							egui::ComboBox::from_id_salt("image-fit")
								.selected_text(match background.fit {
									BackgroundFit::Cover => "Fill area",
									BackgroundFit::Contain => "Fit entire image",
								})
								.show_ui(ui, |ui| {
									changed |= ui
										.selectable_value(
											&mut background.fit,
											BackgroundFit::Cover,
											"Fill area",
										)
										.changed();
									changed |= ui
										.selectable_value(
											&mut background.fit,
											BackgroundFit::Contain,
											"Fit entire image",
										)
										.changed();
								});
							changed
						});
						if let Some(sections) = &mut background.sections {
							ui.add_space(16.0);
							design::section(
								ui,
								"Section opacity",
								Some(
									"Select an area, then choose how much of the image shows through.",
								),
							);
							let base = design::builtin_colors(self.dark, design::variant());
							let map_colors = map_palette(base, &palette.colors);
							let fit = background.fit;
							changed |= section_map(
								ui,
								self.thumbnail.as_ref(),
								&mut self.region,
								map_colors,
								fit,
								sections,
							);
						}
					} else {
						design::hint(
							ui,
							"Choose an image to adjust the top bar, lists, and message area.",
						);
					}
				}
				EditorTab::Colors => {
					design::section(
						ui,
						"Conversation colors",
						Some("Click a swatch to choose a color, or enter its hex value."),
					);
					let theme = self
						.package
						.theme
						.as_mut()
						.expect("theme editor always holds a theme");
					let palette = if self.dark {
						&mut theme.dark
					} else {
						&mut theme.light
					};
					let base = design::builtin_colors(self.dark, design::variant());
					design::card(ui, |ui| {
						for (key, fallback) in [
							("accent", base.accent),
							("text", base.text),
							("muted", base.muted),
							("sidebar", base.sidebar),
							("chat", base.chat),
						] {
							changed |= color_override(ui, key, &mut palette.colors, fallback);
						}
					});
					if design::primary_color().is_some() {
						ui.add_space(12.0);
						design::hint(
							ui,
							"Your primary color in Appearance takes precedence over this accent.",
						);
					}
				}
				EditorTab::Advanced => {
					design::section(
						ui,
						"Advanced",
						Some("Additional colors, app controls, and sharing details."),
					);
					{
						let theme = self
							.package
							.theme
							.as_mut()
							.expect("theme editor always holds a theme");
						let palette = if self.dark {
							&mut theme.dark
						} else {
							&mut theme.light
						};
						let base = design::builtin_colors(self.dark, design::variant());
						self.open_colors |= std::mem::take(&mut self.reveal_advanced_colors);
						if design::disclosure(ui, "More colors", self.open_colors).clicked() {
							self.open_colors = !self.open_colors;
						}
						if self.open_colors {
							design::card(ui, |ui| {
								for (key, fallback) in colors(base) {
									if !["chat", "accent", "text", "muted", "sidebar"]
										.contains(&key)
									{
										changed |=
											color_override(ui, key, &mut palette.colors, fallback);
									}
								}
							});
						}
						ui.add_space(12.0);
						self.open_gradient |= std::mem::take(&mut self.reveal_gradient);
						if design::disclosure(ui, "Window gradient", self.open_gradient).clicked() {
							self.open_gradient = !self.open_gradient;
						}
						if self.open_gradient {
							design::card(ui, |ui| {
								let mut enabled = palette.backdrop.is_some();
								if design::switch(
									ui,
									"Use a gradient",
									Some("Blend two colors behind the app's surfaces."),
									&mut enabled,
								)
								.changed()
								{
									palette.backdrop =
										enabled.then(|| [hex(base.base), hex(base.chat)]);
									changed = true;
								}
								if let Some(stops) = &mut palette.backdrop {
									for (index, stop) in stops.iter_mut().enumerate() {
										changed |= row(
											ui,
											if index == 0 {
												"Start color"
											} else {
												"End color"
											},
											|ui| color_input(ui, stop),
										);
										if extensions::parse_color(stop).is_err() {
											design::notice(
												ui,
												design::Level::Error,
												"Use #RRGGBB or #RRGGBBAA.",
											);
										}
									}
								}
							});
						}
						ui.add_space(12.0);
						if design::disclosure(ui, "Window effects", self.open_effects).clicked() {
							self.open_effects = !self.open_effects;
						}
						if self.open_effects {
							design::card(ui, |ui| {
								design::hint(
									ui,
									"Requires Transparency & blur in Appearance, then an app restart.",
								);
								let style = &mut theme.style;
								let defaults = design::default_window_effects();
								let mut transparency =
									style.transparency_blur.unwrap_or(defaults.0);
								if design::switch(
									ui,
									"Transparency & blur",
									Some("Override the default Appearance setting for this theme."),
									&mut transparency,
								)
								.changed()
								{
									style.transparency_blur = Some(transparency);
									if transparency {
										style.transparency.get_or_insert(defaults.1);
										style.blur.get_or_insert(defaults.2);
										style.transparent_all.get_or_insert(defaults.3);
									}
									changed = true;
								}
								if transparency {
									let mut amount = style.transparency.unwrap_or(defaults.1);
									let mut blur = style.blur.unwrap_or(defaults.2);
									let mut all = style.transparent_all.unwrap_or(defaults.3);
									let mut effects_changed = design::slider_row(
										ui,
										"Transparency",
										None,
										&mut amount,
										0..=100,
										"%",
									)
									.changed();
									ui.add_space(8.0);
									effects_changed |= design::slider_row(
										ui,
										"Blur",
										Some(
											"Zero disables blur; the native compositor controls its exact strength.",
										),
										&mut blur,
										0..=100,
										"%",
									)
									.changed();
									ui.add_space(4.0);
									effects_changed |= design::switch(
										ui,
										"Apply to all surfaces",
										Some(
											"Include sidebars, server rail, headers, and composer.",
										),
										&mut all,
									)
									.changed();
									if effects_changed {
										style.transparency = Some(amount);
										style.blur = Some(blur);
										style.transparent_all = Some(all);
										changed = true;
									}
								}
							});
						}
						ui.add_space(12.0);
						if design::disclosure(ui, "Text, spacing & corners", self.open_metrics)
							.clicked()
						{
							self.open_metrics = !self.open_metrics;
						}
						if self.open_metrics {
							design::card(ui, |ui| {
								design::hint(
									ui,
									"These settings apply to dark and light appearances.",
								);
								let style = &mut theme.style;
								for (label, value, default, min, max) in [
									("Body text", &mut style.body_size, 15, 10, 28),
									("Headings", &mut style.heading_size, 20, 12, 40),
									("Buttons", &mut style.button_size, 14, 10, 28),
									("Small text", &mut style.small_size, 12, 10, 28),
									("Code", &mut style.monospace_size, 14, 10, 28),
									("Control height", &mut style.control_height, 32, 24, 56),
								] {
									changed |= metric(ui, label, value, default, min..=max);
								}
								changed |= pair_metric(
									ui,
									"Item spacing",
									&mut style.item_spacing,
									[8, 8],
								);
								changed |= pair_metric(
									ui,
									"Button padding",
									&mut style.button_padding,
									[12, 6],
								);
								for (label, value, default) in [
									("Control corners", &mut style.widget_radius, 8),
									("Window corners", &mut style.window_radius, 12),
									("Menu corners", &mut style.menu_radius, 12),
								] {
									changed |= metric(ui, label, value, default, 0..=24);
								}
							});
						}
					}
					ui.add_space(12.0);
					design::section(
						ui,
						"Sharing & export",
						Some(
							"The license and version are required. A source URL is optional for local themes.",
						),
					);
					design::card(ui, |ui| {
						let manifest = &mut self.package.manifest;
						changed |= text_field(ui, "License", &mut manifest.license, 32, "CC0-1.0");
						changed |= text_field(ui, "Version", &mut manifest.version, 32, "1.0.0");
						changed |=
							text_field(ui, "Source URL", &mut manifest.source, 512, "Optional");
						if self.show_errors
							&& !manifest.source.is_empty()
							&& [
								&manifest.name,
								&manifest.author,
								&manifest.license,
								&manifest.version,
							]
							.into_iter()
							.all(|value| !value.trim().is_empty())
							&& manifest.validate().is_err()
						{
							design::notice(
								ui,
								design::Level::Error,
								"Use a valid HTTPS source URL or leave this blank.",
							);
						}
						if self.show_errors
							&& (manifest.license.trim().is_empty()
								|| manifest.version.trim().is_empty())
						{
							design::notice(
								ui,
								design::Level::Error,
								"License and version are required.",
							);
						}
						design::hint(
							ui,
							"Only share images you own or have permission to use. Keep required attribution.",
						);
						ui.add_space(8.0);
						if dialog::action(ui, "Export theme", dialog::Action::Outline).clicked() {
							self.show_errors = true;
							if self.ready_to_save() {
								requests.push(ExtensionRequest::ExportTheme {
									package: self.package.clone(),
								});
							} else {
								let (tab, message, dark, reveal_colors) = self.save_error();
								self.tab = tab;
								if let Some(dark) = dark {
									self.dark = dark;
								}
								self.reveal_advanced_colors = reveal_colors;
								self.reveal_gradient = self.invalid_gradient();
								self.validation_error = Some((tab, message));
							}
						}
					});
				}
			}
			self.dirty |= changed;
			if changed {
				if self.ready_to_save()
					|| self.validation_error.is_some_and(|issue| {
						let (tab, message, _, _) = self.save_error();
						issue != (tab, message)
					}) {
					self.validation_error = None;
				}
				if self.preview {
					requests.push(self.preview_request());
				}
			}
		});
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

	fn cover_card(
		&mut self,
		ui: &mut egui::Ui,
		requests: &mut Vec<ExtensionRequest>,
		changed: &mut bool,
	) {
		if self.cover_thumbnail.is_none()
			&& let Some(image) = &self.cover
			&& image
				.size
				.iter()
				.all(|side| *side <= ui.ctx().input(|input| input.max_texture_side))
		{
			self.cover_thumbnail = Some(ui.ctx().load_texture(
				"theme-cover-thumbnail",
				image.clone(),
				egui::TextureOptions::LINEAR,
			));
		}
		design::card(ui, |ui| {
			ui.horizontal_wrapped(|ui| {
				let (rect, _) =
					ui.allocate_exact_size(egui::vec2(112.0, 63.0), egui::Sense::hover());
				let colors = design::palette(ui);
				ui.painter().rect_filled(rect, 6, colors.base);
				if let Some(texture) = &self.cover_thumbnail {
					design::paint_background_image(
						ui.painter(),
						rect,
						texture,
						Background {
							opacity: 100,
							..Default::default()
						},
					);
				} else {
					crate::icons::paint(
						ui.painter(),
						crate::icons::Icon::Image,
						rect.shrink(20.0),
						colors.muted,
					);
				}
				ui.add_space(8.0);
				ui.vertical(|ui| {
					ui.label(design::medium(
						ui,
						if self.cover.is_some() {
							"Custom cover"
						} else {
							"Automatic preview"
						},
						14.0,
					));
					ui.horizontal_wrapped(|ui| {
						if dialog::action(
							ui,
							if self.cover.is_some() {
								"Replace cover"
							} else {
								"Choose cover"
							},
							dialog::Action::Outline,
						)
						.clicked()
						{
							requests.push(ExtensionRequest::PickThemeCover);
						}
						if self.cover.is_some()
							&& dialog::action(ui, "Remove", dialog::Action::Neutral).clicked()
						{
							self.package.cover_image.clear();
							self.cover = None;
							self.cover_thumbnail = None;
							*changed = true;
						}
					});
				});
			});
		});
		design::hint(
			ui,
			"PNG or JPEG, up to 2 MiB. This image does not change the chat background.",
		);
	}

	fn image_card(
		&mut self,
		ui: &mut egui::Ui,
		requests: &mut Vec<ExtensionRequest>,
		changed: &mut bool,
	) {
		if self.thumbnail.is_none()
			&& let Some(image) = &self.image
			&& image
				.size
				.iter()
				.all(|side| *side <= ui.ctx().input(|input| input.max_texture_side))
		{
			self.thumbnail = Some(ui.ctx().load_texture(
				"theme-image-thumbnail",
				image.clone(),
				egui::TextureOptions::LINEAR,
			));
		}
		design::card(ui, |ui| {
			ui.horizontal_wrapped(|ui| {
				let (rect, _) =
					ui.allocate_exact_size(egui::vec2(96.0, 68.0), egui::Sense::hover());
				let colors = design::palette(ui);
				ui.painter().rect_filled(rect, 6, colors.base);
				if let Some(texture) = &self.thumbnail {
					design::paint_background_image(
						ui.painter(),
						rect,
						texture,
						Background {
							opacity: 100,
							..Default::default()
						},
					);
				} else {
					crate::icons::paint(
						ui.painter(),
						crate::icons::Icon::Image,
						rect.shrink(20.0),
						colors.muted,
					);
				}
				ui.add_space(8.0);
				ui.vertical(|ui| {
					let selected = self.image.is_some();
					ui.label(design::medium(
						ui,
						if selected {
							"Background image"
						} else {
							"No image selected"
						},
						14.0,
					));
					if let Some(image) = &self.image {
						design::hint(ui, &format!("{} × {} pixels", image.size[0], image.size[1]));
					}
					ui.horizontal_wrapped(|ui| {
						if dialog::action(
							ui,
							if selected {
								"Replace image"
							} else {
								"Choose image"
							},
							dialog::Action::Outline,
						)
						.clicked()
						{
							requests.push(ExtensionRequest::PickThemeImage);
						}
						if selected
							&& dialog::action(ui, "Remove", dialog::Action::Neutral).clicked()
						{
							self.package.background_image.clear();
							self.image = None;
							self.thumbnail = None;
							if let Some(theme) = self.package.theme.as_mut() {
								theme.light.background = None;
								theme.dark.background = None;
							}
							*changed = true;
						}
					});
				});
			});
		});
		design::hint(ui, "PNG or JPEG, up to 2 MiB");
	}
}

fn appearance_switch(ui: &mut egui::Ui, dark: &mut bool) {
	ui.horizontal(|ui| {
		ui.spacing_mut().item_spacing.x = 8.0;
		ui.label(
			egui::RichText::new("Editing")
				.size(12.0)
				.color(design::palette(ui).muted),
		)
		.on_hover_text("Colors and opacity are saved separately for dark and light appearance.");
		if let Some(index) = design::segmented(ui, &["Dark", "Light"], usize::from(!*dark)) {
			*dark = index == 0;
		}
	});
}

fn map_palette(
	mut base: design::Palette,
	overrides: &std::collections::BTreeMap<String, String>,
) -> design::Palette {
	let color = |key: &str, fallback| {
		overrides
			.get(key)
			.and_then(|value| extensions::parse_color(value).ok())
			.map(rgba)
			.unwrap_or(fallback)
	};
	base.base = color("base", base.base);
	base.sidebar = color("sidebar", base.sidebar);
	base.chat = color("chat", base.chat);
	base.raised = color("raised", base.raised);
	base.accent = color("accent", base.accent);
	base.text_strong = color("text_strong", base.text_strong);
	base
}

fn section_map(
	ui: &mut egui::Ui,
	texture: Option<&egui::TextureHandle>,
	selected: &mut ImageRegion,
	colors: design::Palette,
	fit: BackgroundFit,
	sections: &mut SectionOpacity,
) -> bool {
	let mut changed = false;
	if ui.available_width() >= 620.0 {
		ui.horizontal_top(|ui| {
			let width = ui.available_width() * 0.55;
			ui.allocate_ui(egui::vec2(width, 0.0), |ui| {
				section_diagram(ui, texture, selected, colors, fit, sections);
			});
			ui.add_space(12.0);
			ui.vertical(|ui| {
				changed = section_controls(ui, selected, sections);
			});
		});
	} else {
		section_diagram(ui, texture, selected, colors, fit, sections);
		ui.add_space(12.0);
		changed = section_controls(ui, selected, sections);
	}
	changed
}

fn section_controls(
	ui: &mut egui::Ui,
	selected: &mut ImageRegion,
	sections: &mut SectionOpacity,
) -> bool {
	let mut changed = false;
	design::card(ui, |ui| {
		let palette = design::palette(ui);
		ui.visuals_mut().widgets.inactive.bg_fill = palette.base;
		ui.visuals_mut().widgets.inactive.weak_bg_fill = palette.base;
		ui.visuals_mut().selection.bg_fill = palette.accent;
		ui.label(design::medium(ui, "Selected section", 13.0));
		egui::ComboBox::from_id_salt("background-section")
			.width(ui.available_width())
			.selected_text(selected.label())
			.show_ui(ui, |ui| {
				for region in [
					ImageRegion::TopBars,
					ImageRegion::ServerList,
					ImageRegion::PeopleChannels,
					ImageRegion::MessageList,
					ImageRegion::MemberList,
					ImageRegion::InputArea,
				] {
					ui.selectable_value(selected, region, region.label());
				}
			});
		design::hint(ui, selected.description());
		ui.add_space(12.0);
		changed = design::slider_row(
			ui,
			"Surface opacity",
			None,
			selected.opacity(sections),
			0..=100,
			"%",
		)
		.changed();
		design::hint(ui, "0% shows the image. 100% is a solid section color.");
	});
	changed
}

fn section_diagram(
	ui: &mut egui::Ui,
	texture: Option<&egui::TextureHandle>,
	selected: &mut ImageRegion,
	colors: design::Palette,
	fit: BackgroundFit,
	sections: &mut SectionOpacity,
) {
	let width = ui.available_width().clamp(1.0, 520.0);
	let (rect, _) =
		ui.allocate_exact_size(egui::vec2(width, width * 9.0 / 16.0), egui::Sense::hover());
	let painter = ui.painter().with_clip_rect(rect);
	painter.rect_filled(rect, 8, colors.base.to_opaque());
	if let Some(texture) = texture {
		design::paint_background_image(
			&painter,
			rect,
			texture,
			Background {
				opacity: 100,
				fit,
				..Default::default()
			},
		);
	}
	let area = |x: f32, y: f32, w: f32, h: f32| {
		egui::Rect::from_min_max(
			rect.min + egui::vec2(rect.width() * x, rect.height() * y),
			rect.min + egui::vec2(rect.width() * (x + w), rect.height() * (y + h)),
		)
	};
	let regions = [
		(ImageRegion::TopBars, area(0.0, 0.0, 1.0, 0.11), "Top", 0_u8),
		(ImageRegion::ServerList, area(0.0, 0.11, 0.09, 0.89), "S", 0),
		(
			ImageRegion::PeopleChannels,
			area(0.09, 0.11, 0.23, 0.89),
			"People",
			0,
		),
		(
			ImageRegion::TopBars,
			area(0.32, 0.11, 0.46, 0.11),
			"Header",
			1,
		),
		(
			ImageRegion::MessageList,
			area(0.32, 0.22, 0.46, 0.63),
			"Messages",
			0,
		),
		(
			ImageRegion::MemberList,
			area(0.78, 0.11, 0.22, 0.89),
			"Members",
			0,
		),
		(
			ImageRegion::InputArea,
			area(0.32, 0.85, 0.46, 0.15),
			"Input",
			0,
		),
	];
	for (region, region_rect, short_label, part) in regions {
		let surface = match region {
			ImageRegion::TopBars | ImageRegion::ServerList => colors.base,
			ImageRegion::PeopleChannels | ImageRegion::MemberList => colors.sidebar,
			ImageRegion::MessageList | ImageRegion::InputArea => colors.chat,
		};
		let [r, g, b, _] = surface.to_srgba_unmultiplied();
		let opacity = *region.opacity(sections);
		painter.rect_filled(
			region_rect.shrink(1.0),
			2,
			egui::Color32::from_rgba_unmultiplied(r, g, b, (u16::from(opacity) * 255 / 100) as u8),
		);
		let response = ui
			.interact(
				region_rect,
				ui.scope_id().with((region as u8, part)),
				egui::Sense::click(),
			)
			.on_hover_text(region.label());
		response
			.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, region.label()));
		if response.clicked()
			|| response.has_focus()
				&& ui.input(|input| {
					input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
				}) {
			*selected = region;
		}
		if response.hovered() {
			ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
		}
		let highlighted = *selected == region || response.has_focus();
		let outline = if highlighted {
			colors.accent
		} else {
			colors.border
		};
		painter.rect_stroke(
			region_rect.shrink(0.5),
			2,
			egui::Stroke::new(if highlighted { 2.0 } else { 1.0 }, outline),
			egui::StrokeKind::Inside,
		);
		if region_rect.width() >= 16.0 {
			let font = egui::FontId::proportional((rect.width() / 38.0).clamp(10.0, 13.0));
			let label = if region_rect.width() < 60.0 {
				match region {
					ImageRegion::TopBars => "T",
					ImageRegion::ServerList => "S",
					ImageRegion::PeopleChannels => "P",
					ImageRegion::MessageList => "M",
					ImageRegion::MemberList => "M",
					ImageRegion::InputArea => "I",
				}
			} else {
				short_label
			};
			let galley = painter.layout_no_wrap(label.into(), font, colors.text_strong);
			let label_rect = egui::Rect::from_center_size(
				region_rect.center(),
				galley.size() + egui::vec2(6.0, 4.0),
			);
			painter.rect_filled(label_rect, 3, colors.raised.to_opaque());
			painter.galley(
				label_rect.center() - galley.size() / 2.0,
				galley,
				colors.text_strong,
			);
		}
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
	ui.allocate_ui_with_layout(
		egui::vec2(180.0, 42.0),
		egui::Layout::left_to_right(egui::Align::Center),
		|ui| {
			let mut color = extensions::parse_color(value)
				.map(rgba)
				.unwrap_or(egui::Color32::TRANSPARENT);
			let mut changed = ui.color_edit_button_srgba(&mut color).changed();
			if changed {
				*value = hex(color);
			}
			changed |= ui
				.allocate_ui_with_layout(
					egui::vec2(132.0, 42.0),
					egui::Layout::left_to_right(egui::Align::Center),
					|ui| {
						design::input(
							ui,
							egui::TextEdit::singleline(value)
								.char_limit(9)
								.font(egui::FontId::proportional(14.0)),
						)
						.changed()
					},
				)
				.inner;
			if extensions::parse_color(value).is_err() {
				let (rect, response) =
					ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
				icons::paint(
					ui.painter(),
					icons::Icon::ShieldWarning,
					rect,
					design::palette(ui).danger,
				);
				response.on_hover_text("Use #RRGGBB or #RRGGBBAA");
			}
			changed
		},
	)
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
	settings_row(ui, label, None, controls)
}
fn settings_row(
	ui: &mut egui::Ui,
	label: &str,
	description: Option<&str>,
	controls: impl FnOnce(&mut egui::Ui) -> bool,
) -> bool {
	ui.push_id(label, |ui| {
		ui.add_space(4.0);
		let height = if description.is_some() { 54.0 } else { 42.0 };
		let heading = |ui: &mut egui::Ui| {
			ui.label(design::medium(ui, label, 14.0).color(design::palette(ui).text_strong));
			if let Some(description) = description {
				ui.add(
					egui::Label::new(
						egui::RichText::new(description)
							.size(12.0)
							.color(design::palette(ui).muted),
					)
					.wrap(),
				);
			}
		};
		if ui.available_width() < 500.0 {
			heading(ui);
			ui.allocate_ui_with_layout(
				egui::vec2(ui.available_width(), 42.0),
				egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
				controls,
			)
			.inner
		} else {
			ui.allocate_ui_with_layout(
				egui::vec2(ui.available_width(), height),
				egui::Layout::left_to_right(egui::Align::Center),
				|ui| {
					let width = 250.0;
					let label_width =
						(ui.available_width() - width - ui.spacing().item_spacing.x).max(1.0);
					ui.allocate_ui_with_layout(
						egui::vec2(label_width, height),
						egui::Layout::top_down(egui::Align::Min)
							.with_main_align(egui::Align::Center),
						|ui| {
							ui.set_min_width(label_width);
							ui.set_min_height(height);
							heading(ui);
						},
					);
					ui.allocate_ui_with_layout(
						egui::vec2(width, 42.0),
						egui::Layout::left_to_right(egui::Align::Center),
						controls,
					)
					.inner
				},
			)
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
		let label = ui.label(design::medium(ui, label, 13.0).color(colors.text_strong));
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
	let description = match key {
		"accent" => Some("Buttons, selection and highlights"),
		"text" => Some("Messages and regular labels"),
		"muted" => Some("Timestamps and supporting text"),
		"sidebar" => Some("Channel, conversation and member lists"),
		"chat" => Some("Background behind your messages"),
		_ => None,
	};
	let changed = settings_row(ui, color_label(key), description, |ui| {
		let mut value = map.get(key).cloned().unwrap_or_else(|| hex(fallback));
		let mut changed = color_input(ui, &mut value);
		if changed {
			map.insert(key.into(), value);
		}
		if map.contains_key(key)
			&& design::text_action(ui, "Reset")
				.on_hover_text("Use the default color for this appearance")
				.clicked()
		{
			map.remove(key);
			changed = true;
		}
		changed
	});
	if map
		.get(key)
		.is_some_and(|value| extensions::parse_color(value).is_err())
	{
		design::notice(ui, design::Level::Error, "Use #RRGGBB or #RRGGBBAA.");
	}
	changed
}
fn metric(
	ui: &mut egui::Ui,
	label: &str,
	value: &mut Option<u8>,
	default: u8,
	range: std::ops::RangeInclusive<u8>,
) -> bool {
	let mut changed = false;
	ui.push_id(label, |ui| {
		let mut n = value.unwrap_or(default);
		if metric_label(ui, label, value.is_some()) {
			*value = None;
			n = default;
			changed = true;
		}
		if design::slider(ui, &mut n, range, " px").changed() {
			*value = Some(n);
			changed = true;
		}
		ui.add_space(8.0);
	});
	changed
}

/// Metric title with a quiet Reset on the right; returns whether Reset was pressed.
fn metric_label(ui: &mut egui::Ui, label: &str, overridden: bool) -> bool {
	let mut reset = false;
	ui.horizontal(|ui| {
		ui.label(design::medium(ui, label, 14.0).color(design::palette(ui).text_strong));
		if overridden {
			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				reset = design::text_action(ui, "Reset")
					.on_hover_text("Use the built-in value")
					.clicked();
			});
		}
	});
	reset
}

fn pair_metric(
	ui: &mut egui::Ui,
	label: &str,
	value: &mut Option<[u8; 2]>,
	default: [u8; 2],
) -> bool {
	let mut changed = false;
	ui.push_id(label, |ui| {
		let mut pair = value.unwrap_or(default);
		if metric_label(ui, label, value.is_some()) {
			*value = None;
			pair = default;
			changed = true;
		}
		let mut edited = false;
		for (index, n) in pair.iter_mut().enumerate() {
			let axis = if index == 0 { "Horizontal" } else { "Vertical" };
			ui.label(
				egui::RichText::new(axis)
					.size(12.0)
					.color(design::palette(ui).muted),
			);
			edited |= design::slider(ui, n, 0..=24, " px").changed();
		}
		if edited {
			*value = Some(pair);
			changed = true;
		}
		ui.add_space(8.0);
	});
	changed
}

#[cfg(test)]
mod tests {
	use super::*;
	fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
		match shape {
			egui::Shape::Text(text) => {
				labels.push((text.galley.job.text.clone(), text.visual_bounding_rect()))
			}
			egui::Shape::Vec(shapes) => {
				for shape in shapes {
					collect(shape, labels);
				}
			}
			_ => {}
		}
	}

	fn map_frame(
		ctx: &egui::Context,
		region: &mut ImageRegion,
		sections: &mut SectionOpacity,
		events: Vec<egui::Event>,
	) -> Vec<(String, egui::Rect)> {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(500.0, 600.0),
				)),
				events,
				focused: true,
				..Default::default()
			},
			|ui| {
				section_map(
					ui,
					None,
					region,
					design::builtin_colors(true, design::Variant::Standard),
					BackgroundFit::Cover,
					sections,
				);
			},
		);
		let mut labels = Vec::new();
		for shape in &output.shapes {
			collect(&shape.shape, &mut labels);
		}
		output.drop_without_applying_deltas();
		labels
	}
	fn toolbar_frame(
		ctx: &egui::Context,
		editor: &mut ThemeEditor,
		events: Vec<egui::Event>,
	) -> (Vec<(String, egui::Rect)>, Vec<ExtensionRequest>) {
		let mut requests = Vec::new();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(520.0, 600.0),
				)),
				events,
				focused: true,
				..Default::default()
			},
			|ui| {
				editor.toolbar(ui, false, &mut requests);
			},
		);
		let mut labels = Vec::new();
		for shape in &output.shapes {
			collect(&shape.shape, &mut labels);
		}
		output.drop_without_applying_deltas();
		(labels, requests)
	}
	fn toolbar_click(
		ctx: &egui::Context,
		editor: &mut ThemeEditor,
		label: &str,
	) -> Vec<ExtensionRequest> {
		let (labels, _) = toolbar_frame(ctx, editor, vec![]);
		let position = labels
			.iter()
			.find(|(text, _)| text == label)
			.unwrap_or_else(|| panic!("Missing {label}"))
			.1
			.center();
		let mut requests = Vec::new();
		for pressed in [true, false] {
			requests = toolbar_frame(
				ctx,
				editor,
				vec![
					egui::Event::PointerMoved(position),
					egui::Event::PointerButton {
						pos: position,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			)
			.1;
		}
		requests
	}

	#[test]
	fn map_selects_the_member_panel_and_save_points_to_required_basics() {
		let ctx = egui::Context::default();
		ctx.set_theme(egui::ThemePreference::Dark);
		let mut editor = ThemeEditor::new();
		assert_eq!(editor.save_error().0, EditorTab::Basics);
		let mut sections = SectionOpacity::default();
		let labels = map_frame(&ctx, &mut editor.region, &mut sections, vec![]);
		let position = labels
			.iter()
			.find(|(label, _)| label == "Members")
			.unwrap()
			.1
			.center();
		for pressed in [true, false] {
			map_frame(
				&ctx,
				&mut editor.region,
				&mut sections,
				vec![
					egui::Event::PointerMoved(position),
					egui::Event::PointerButton {
						pos: position,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
		}
		assert_eq!(editor.region, ImageRegion::MemberList);
		assert_eq!(sections, SectionOpacity::default());
		// The text selector provides the same navigation as the map.
		for label in ["Member list", "Top bars"] {
			let labels = map_frame(&ctx, &mut editor.region, &mut sections, vec![]);
			let position = labels
				.iter()
				.find(|(text, _)| text == label)
				.unwrap()
				.1
				.center();
			for pressed in [true, false] {
				map_frame(
					&ctx,
					&mut editor.region,
					&mut sections,
					vec![
						egui::Event::PointerMoved(position),
						egui::Event::PointerButton {
							pos: position,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
		}
		assert_eq!(editor.region, ImageRegion::TopBars);
		assert_eq!(sections, SectionOpacity::default());
		editor.package.manifest.author = "Creator".into();
		assert!(editor.package.validate().is_ok());
	}

	#[test]
	fn tabs_and_save_keep_required_fields_easy_to_find() {
		let ctx = egui::Context::default();
		ctx.set_theme(egui::ThemePreference::Dark);
		let mut editor = ThemeEditor::new();
		assert!(toolbar_click(&ctx, &mut editor, "Background").is_empty());
		assert_eq!(editor.tab, EditorTab::Background);
		assert!(toolbar_click(&ctx, &mut editor, "Save and apply").is_empty());
		assert_eq!(editor.tab, EditorTab::Basics);
		assert!(editor.validation_error.is_some());
		editor.package.manifest.author = "  ".into();
		assert!(toolbar_click(&ctx, &mut editor, "Save and apply").is_empty());
		editor.package.manifest.author = "Creator".into();
		assert!(matches!(
			toolbar_click(&ctx, &mut editor, "Save and apply").as_slice(),
			[ExtensionRequest::SaveTheme { .. }]
		));
	}
}

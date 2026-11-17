//! Generic renderer for a plugin's declarative interface tree ([`extensions::InterfaceUi`]):
//! the host paints rows with indent, icon, tint, badge, emphasis and a selected fill, and
//! routes clicks back through the plugin's declared `interface_click` panel action, with
//! `double_clicked` marking double clicks and Enter on a focused row. The renderer knows
//! nothing about servers, voice or any particular product: the tree is data; only the
//! dock's server tiles are the host's own, painted by guild id.
use crate::{MessagingUi, design, icons};
use client_core::{Command, State, voice::Phase};
use egui::RichText;
use model::Id;

/// Tree rows: the slim lines the channel list uses.
const ROW: f32 = 22.0;
/// Horizontal step between tree levels.
const INDENT: f32 = 14.0;
/// Row glyphs.
const GLYPH: f32 = 14.0;
/// Dock tiles in the bottom control row.
const DOCK: f32 = 24.0;

/// The interface one enabled granted plugin last returned, with its owning plugin id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginInterface {
	pub plugin: String,
	pub tree: extensions::InterfaceUi,
}

/// Maps a tree row's glyph onto the icons the host already paints.
fn icon(icon: extensions::InterfaceIcon) -> icons::Icon {
	match icon {
		extensions::InterfaceIcon::Speaker => icons::Icon::Speaker,
		extensions::InterfaceIcon::MicrophoneSlash => icons::Icon::MicrophoneSlash,
		extensions::InterfaceIcon::HeadphonesSlash => icons::Icon::HeadphonesSlash,
		extensions::InterfaceIcon::ChevronDown => icons::Icon::ChevronDown,
		extensions::InterfaceIcon::ChevronRight => icons::Icon::ChevronRight,
		extensions::InterfaceIcon::Server => icons::Icon::Servers,
		extensions::InterfaceIcon::Hash => icons::Icon::Hash,
		extensions::InterfaceIcon::Gear => icons::Icon::Gear,
		extensions::InterfaceIcon::HangUp => icons::Icon::HangUp,
	}
}

/// A row's semantic tint, mapped onto the host palette.
fn tint(
	tint: Option<extensions::InterfaceTint>,
	colors: &design::Palette,
	fallback: egui::Color32,
) -> egui::Color32 {
	match tint {
		Some(extensions::InterfaceTint::Positive) => colors.positive,
		Some(extensions::InterfaceTint::Danger) => colors.danger,
		Some(extensions::InterfaceTint::Muted) => colors.muted,
		None => fallback,
	}
}

impl MessagingUi {
	/// The client surface between the title bar and the overlays while a plugin interface is
	/// active: the tree pane, with the host's call stage or a conversation beside it when the
	/// plugin asked for one. Stands in for the navigation, conversation and member panes.
	pub(super) fn show_plugin_interface(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		interface: &PluginInterface,
	) {
		// The member pane is replaced too, so its subscription closes as when the pane hides.
		if state.members.is_some() {
			commands.push(state.close_members());
		}
		let pane = egui::Frame::new()
			.fill(design::section_surface(
				ui,
				design::window_palette(ui).sidebar,
				design::ImageSection::ChannelList,
			))
			.inner_margin(0);
		let chat = match &interface.tree.window {
			Some(extensions::InterfaceWindow::Chat { channel_id }) => {
				channel_id.parse().ok().map(Id)
			}
			_ => None,
		};
		self.select_interface_chat(state, chat, commands);
		if let Some(window) = &interface.tree.window {
			egui::Panel::left("plugin-interface")
				.resizable(true)
				.show_separator_line(!design::has_window_background(ui))
				.default_size(280.0)
				.size_range(200.0..=480.0)
				.frame(pane)
				.show(ui, |ui| {
					self.plugin_interface_pane(ui, state, commands, interface)
				});
			let frame = egui::Frame::new()
				.fill(design::section_surface(
					ui,
					design::window_palette(ui).chat,
					design::ImageSection::MessageList,
				))
				.inner_margin(0);
			egui::CentralPanel::default()
				.frame(frame)
				.show(ui, |ui| match window {
					extensions::InterfaceWindow::Call => self.interface_call(ui, state, commands),
					extensions::InterfaceWindow::Chat { .. } => {
						self.interface_chat(ui, state, chat, commands)
					}
				});
		} else {
			egui::CentralPanel::default().frame(pane).show(ui, |ui| {
				self.plugin_interface_pane(ui, state, commands, interface)
			});
		}
		// The stage raises watch requests that apply after the panes, as beside the channel list.
		self.apply_stream_preview_watch(state, commands);
	}

	/// Opens the chat window's channel through the usual selection path, once per requested
	/// channel, so its history loads and access is rechecked like a channel-list click. The
	/// request counts as applied only once its channel is selected: a channel that is not
	/// loaded or viewable yet is retried on later frames, without selection side effects.
	fn select_interface_chat(
		&mut self,
		state: &mut State,
		chat: Option<Id>,
		commands: &mut Vec<Command>,
	) {
		if self.interface_chat == chat {
			return;
		}
		self.interface_chat = None;
		let Some(channel) = chat else {
			return;
		};
		if state.selected != Some(channel) {
			if !state.can_view(channel)
				|| !state
					.channel(channel)
					.is_some_and(|known| known.supports_text())
			{
				return;
			}
			if let Some(command) = state.select(channel) {
				commands.push(command);
			}
			if state.selected != Some(channel) {
				return;
			}
		}
		self.interface_chat = chat;
	}

	/// The host's native call stage for the active call. The plugin only asked for the window;
	/// which call shows stays the host's own, and only while its channel stays viewable.
	fn interface_call(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let channel = state.voice.active.as_ref().map(|call| call.channel);
		if let Some(channel) = channel
			&& state.channel(channel).is_some()
			&& state.can_view(channel)
		{
			self.voice_channel(ui, state, channel, commands);
		}
	}

	/// The selected conversation, rendered as in the usual layout, once selection has opened
	/// the requested channel. Native navigation afterwards (a link, an invite, the switcher)
	/// moves the window along to the newly selected conversation instead of blanking it. A
	/// pending request, or an inaccessible or non-text selection, leaves the window empty.
	fn interface_chat(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		chat: Option<Id>,
		commands: &mut Vec<Command>,
	) {
		if chat.is_none() || self.interface_chat != chat {
			return;
		}
		let Some(channel) = state.selected.filter(|channel| {
			state.can_view(*channel)
				&& state
					.channel(*channel)
					.is_some_and(|known| known.supports_text())
		}) else {
			return;
		};
		self.conversation(ui, state, channel, commands, None);
	}

	/// The tree pane: the generic voice-controls strip, then the plugin's rows.
	fn plugin_interface_pane(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		interface: &PluginInterface,
	) {
		egui::Panel::bottom("plugin-interface-controls")
			.show_separator_line(!design::has_window_background(ui))
			.frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 4)))
			.show(ui, |ui| {
				self.interface_controls(ui, state, commands, interface)
			});
		self.interface_tree(ui, state, interface);
	}

	/// Settings, the microphone and headphone toggles, and Disconnect during a call: the
	/// account card's controls, sending the same commands.
	fn interface_controls(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		interface: &PluginInterface,
	) {
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
			ui.spacing_mut().item_spacing.x = 2.0;
			let settings = icons::button(
				ui,
				icons::Icon::Gear,
				32.0,
				&crate::i18n::translate("user-settings"),
			);
			if settings.clicked() {
				self.settings.open = true;
				egui::Popup::close_all(ui.ctx());
			}
			self.mute_toggle_with_settings(ui, state, commands, true);
			ui.add_space(4.0);
			self.mute_toggle_with_settings(ui, state, commands, false);
			if let Some(phase) = state.voice.active.as_ref().map(|call| call.phase) {
				ui.add_space(4.0);
				let leave = ui
					.add_enabled_ui(!state.demo, |ui| {
						icons::button(
							ui,
							icons::Icon::HangUp,
							32.0,
							&crate::i18n::translate(if phase == Phase::Failed {
								"voice-voice-card-section-dismiss-call"
							} else {
								"voice-voice-card-section-disconnect"
							}),
						)
					})
					.inner;
				if leave.clicked()
					&& let Some(command) = state.leave_call()
				{
					commands.push(command);
				}
			}
			self.interface_dock(ui, state, interface);
		});
	}

	/// The plugin's dock at the left of the control row: small server tiles in dock order,
	/// then the client's own add-a-server tile when asked. Items overflow into horizontal
	/// scrolling without wrapping, and the row's height never grows.
	fn interface_dock(&mut self, ui: &mut egui::Ui, state: &State, interface: &PluginInterface) {
		if interface.tree.dock.is_empty() && !interface.tree.add_server {
			return;
		}
		let area = ui.available_rect_before_wrap();
		ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
			egui::ScrollArea::horizontal()
				.min_scrolled_width(0.0)
				.id_salt("plugin-interface-dock")
				.auto_shrink([false, false])
				.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
				.show(ui, |ui| {
					ui.set_min_height(area.height());
					ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
						ui.spacing_mut().item_spacing.x = 6.0;
						for item in &interface.tree.dock {
							self.interface_dock_item(ui, state, &interface.plugin, item);
						}
						if interface.tree.add_server {
							self.add_server_tile(ui, state, DOCK);
						}
					});
				});
		});
	}

	/// One dock item: the loaded server's icon through the rail's own painting and cache, the
	/// label's initials otherwise, an accent ring when selected, and the label on hover.
	/// Clicks, double clicks and keyboard activation route the item's id exactly like a row's.
	fn interface_dock_item(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		plugin: &str,
		item: &extensions::InterfaceDockItem,
	) {
		let unknown;
		let guild = item
			.guild_id
			.as_deref()
			.and_then(|id| id.parse().ok())
			.and_then(|id| state.guild(Id(id)));
		let guild = match guild {
			Some(known) => known,
			None => {
				unknown = model::Guild {
					default_message_notifications: None,
					stickers: None,
					emojis: None,
					id: Id(0),
					name: item.label.clone(),
					icon: None,
					banner: None,
				};
				&unknown
			}
		};
		let response =
			self.avatars
				.show_guild_dock(ui, guild, item.selected, state.demo, DOCK, &item.label);
		if item.selected {
			ui.painter().rect_stroke(
				response.rect,
				(DOCK * 0.3) as u8,
				egui::Stroke::new(2.0, design::palette(ui).accent),
				egui::StrokeKind::Inside,
			);
		}
		let double = opened(ui, &response);
		if response.clicked() {
			self.extensions
				.invoke_interface_row(ui.ctx(), state, plugin, &item.id, double);
			ui.ctx().request_repaint();
		}
	}

	/// The plugin's rows in a scrolling tree. A click on a row with an id routes to the owning
	/// plugin through its declared `interface_click` panel action. A double click, or Enter
	/// on a focused row, sets `double_clicked` (the row's primary open action, as in file
	/// managers); Space on a focused row counts as a single click. Rows without ids are inert.
	fn interface_tree(&mut self, ui: &mut egui::Ui, state: &State, interface: &PluginInterface) {
		if interface.tree.nodes.is_empty() {
			ui.add_space((ui.available_height() * 0.32).max(24.0));
			ui.vertical_centered(|ui| {
				let empty = RichText::new(crate::i18n::translate("interface-ui-empty"));
				ui.label(empty.color(design::palette(ui).muted));
			});
			return;
		}
		self.scroll
			.attach(
				ui,
				"plugin-interface-tree",
				egui::ScrollArea::vertical().auto_shrink([false, false]),
			)
			.show(ui, |ui| {
				ui.spacing_mut().item_spacing.y = 0.0;
				for (index, node) in interface.tree.nodes.iter().enumerate() {
					self.interface_row(ui, state, &interface.plugin, node, index);
				}
			});
	}

	/// One row of the plugin's tree: glyph, label and badge over an indent step, with the
	/// selected fill the channel list uses. Clicks route the row's id to the plugin.
	fn interface_row(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		plugin: &str,
		node: &extensions::InterfaceNode,
		index: usize,
	) {
		let id = row_id(plugin, node, index);
		let active = !node.id.is_empty();
		let Some((rect, response)) = tree_row(ui, ROW, id, active) else {
			return;
		};
		let colors = design::palette(ui);
		let hovered = response.hovered() || response.has_focus();
		if node.selected {
			ui.painter().rect_filled(rect, 4, colors.selected);
		} else if hovered {
			paint_hover(ui, rect);
		}
		// A double click also reports a click, as do Enter and Space on the focused row;
		// route it once, flagged.
		let double = opened(ui, &response);
		if active && response.clicked() {
			self.extensions
				.invoke_interface_row(ui.ctx(), state, plugin, &node.id, double);
			ui.ctx().request_repaint();
		}
		let row = rect.shrink2(egui::vec2(4.0, 0.0));
		let mut left = row.left() + f32::from(node.indent) * INDENT;
		let color = if node.selected || hovered || node.emphasis {
			colors.text_strong
		} else {
			colors.text
		};
		if let Some(glyph) = node.icon {
			icons::paint(
				ui.painter(),
				icon(glyph),
				glyph_rect(row, left),
				tint(node.tint, &colors, color),
			);
			left += GLYPH + 4.0;
		}
		let mut right = row.right();
		if let Some(badge) = node.badge.as_deref() {
			right = paint_badge(ui, row, right, badge) - 4.0;
		}
		let font = egui::FontId::new(
			13.0,
			if node.emphasis {
				design::semibold_family(ui.ctx())
			} else {
				design::medium_family(ui.ctx())
			},
		);
		paint_text(
			ui,
			row.with_min_x(left).with_max_x(right),
			&node.text,
			font,
			color,
		);
		response.widget_info(|| {
			egui::WidgetInfo::selected(egui::Role::Button, active, node.selected, node.text.clone())
		});
	}
}

/// A tree row's egui id. Actionable rows are keyed by plugin and row id, so focus stays on
/// the same row when others are inserted before it; inert rows never focus and keep their
/// position.
fn row_id(plugin: &str, node: &extensions::InterfaceNode, index: usize) -> egui::Id {
	if node.id.is_empty() {
		egui::Id::unique(("plugin-interface-inert-row", plugin, index))
	} else {
		egui::Id::unique(("plugin-interface-row", plugin, node.id.as_str()))
	}
}

/// Whether a row or dock click is the primary open action: a double click, or Enter on the
/// focused item (egui reports that, like Space, as a click).
fn opened(ui: &egui::Ui, response: &egui::Response) -> bool {
	response.double_clicked()
		|| (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
}

/// Allocates a full-width tree row. Rows scrolled out of view only take their space, so a long
/// tree lays out and registers just the visible ones. Inert rows never take clicks or focus.
fn tree_row(
	ui: &mut egui::Ui,
	height: f32,
	id: egui::Id,
	active: bool,
) -> Option<(egui::Rect, egui::Response)> {
	let (_, rect) = ui.allocate_space(egui::vec2(ui.available_width(), height));
	let sense = if active {
		egui::Sense::click()
	} else {
		egui::Sense::hover()
	};
	ui.is_rect_visible(rect)
		.then(|| (rect, ui.interact(rect, id, sense)))
}

/// Hover fill of a tree row; translucent over a window image, like the channel list's rows.
fn paint_hover(ui: &egui::Ui, rect: egui::Rect) {
	let fill = design::row_highlight(ui, design::palette(ui).hover, 1.0);
	ui.painter().rect_filled(rect, 4, fill);
}

/// The glyph cell starting at `left`, centred on `row`.
fn glyph_rect(row: egui::Rect, left: f32) -> egui::Rect {
	let center = egui::pos2(left + GLYPH * 0.5, row.center().y);
	egui::Rect::from_center_size(center, egui::Vec2::splat(GLYPH))
}

/// One line of `text` centred in `area`, elided when it does not fit.
fn paint_text(
	ui: &egui::Ui,
	area: egui::Rect,
	text: &str,
	font: egui::FontId,
	color: egui::Color32,
) {
	let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
	job.wrap = egui::text::TextWrapping::truncate_at_width(area.width().max(1.0));
	let galley = ui.painter().layout_job(job);
	let pos = egui::pos2(area.left(), area.center().y - galley.size().y * 0.5);
	ui.painter().galley(pos, galley, color);
}

/// A row's badge chip, ending at `right`; returns where it starts.
fn paint_badge(ui: &egui::Ui, row: egui::Rect, right: f32, badge: &str) -> f32 {
	let colors = design::palette(ui);
	let font = egui::FontId::new(10.0, design::semibold_family(ui.ctx()));
	let galley = ui
		.painter()
		.layout_no_wrap(badge.to_owned(), font, colors.accent_text);
	let chip = egui::Rect::from_min_max(
		egui::pos2(right - galley.size().x - 8.0, row.center().y - 7.0),
		egui::pos2(right, row.center().y + 7.0),
	);
	ui.painter().rect_filled(chip, 3, colors.accent);
	let text = chip.center() - galley.size() * 0.5;
	ui.painter().galley(text, galley, colors.accent_text);
	chip.left()
}

#[cfg(test)]
mod tests {
	use super::PluginInterface;
	use crate::{ExtensionEntry, ExtensionRequest, MessagingUi};
	use client_core::{Envelope, Event, State};
	use extensions::{Capability, Invocation, Surface};
	use model::{Channel, Guild, Id, User};

	fn user(id: u64, name: &str) -> User {
		User {
			kind: Default::default(),
			webhook: false,
			id: Id(id),
			name: name.into(),
			avatar: None,
			discriminator: 0,
			primary_guild: None,
			cosmetics: None,
		}
	}

	fn channel(id: u64, kind: u8, name: &str) -> Channel {
		Channel {
			icon: None,
			last_message: None,
			id: Id(id),
			guild: Some(Id(10)),
			parent_id: None,
			position: 0,
			name: name.into(),
			kind,
			recipients: vec![],
			member_list_id: None,
			message_count: None,
			user_limit: None,
			slowmode: 0,
			tags: None,
		}
	}

	/// One server with an open text channel and a voice channel.
	fn one_server() -> State {
		let mut state = State {
			demo: true,
			..State::default()
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Ready {
				permissions: Default::default(),
				user: user(1, "Interface tester"),
				guilds: vec![Guild {
					default_message_notifications: None,
					stickers: None,
					emojis: None,
					id: Id(10),
					name: "Voice club".into(),
					icon: None,
					banner: None,
				}],
				channels: vec![channel(20, 0, "plans-and-notes"), channel(30, 2, "Lobby")],
			},
		});
		let permissions = test_support::permission_snapshot(&state);
		state.permissions.replace(permissions).unwrap();
		let _ = state.select(Id(20));
		state
	}

	/// The enabled plugin the tree belongs to, declaring the `interface_click` panel action.
	fn plugin_entry() -> ExtensionEntry {
		ExtensionEntry {
			description: "Synthetic interface tree".into(),
			preview: None,
			theme_preview: None,
			cover_image: None,
			local_theme: false,
			manifest: extensions::Manifest {
				api_version: extensions::API_VERSION,
				id: "tree-plugin".into(),
				name: "Tree plugin".into(),
				version: "1.0.0".into(),
				author: "Serein contributors".into(),
				license: "MIT".into(),
				source: "https://example.org/source".into(),
				kind: extensions::ExtensionKind::Plugin,
				capabilities: vec![Capability::InterfaceUi],
				actions: vec![
					extensions::Action {
						id: "activate".into(),
						label: "Activate".into(),
						surface: Surface::Activation,
					},
					extensions::Action {
						id: extensions::INTERFACE_CLICK_ACTION.into(),
						label: "Interface row".into(),
						surface: Surface::Panel,
					},
				],
			},
			reviewed: true,
			sha256: "a".repeat(64),
			download_bytes: 1,
			enabled: true,
			cleanup_pending: false,
			update_available: false,
			update_manifest: None,
		}
	}

	fn node(id: &str, text: &str) -> extensions::InterfaceNode {
		extensions::InterfaceNode {
			id: id.into(),
			indent: 0,
			icon: None,
			text: text.into(),
			badge: None,
			emphasis: false,
			selected: false,
			tint: None,
		}
	}

	fn interface(
		nodes: Vec<extensions::InterfaceNode>,
		window: Option<extensions::InterfaceWindow>,
	) -> PluginInterface {
		PluginInterface {
			plugin: "tree-plugin".into(),
			tree: extensions::InterfaceUi {
				nodes,
				window,
				..Default::default()
			},
		}
	}

	/// Painted texts with a point inside each, and accessible names with their rects.
	type Frame = (Vec<(String, egui::Pos2)>, Vec<(String, egui::Rect)>);

	/// The client in its own context, drawn frame by frame on a steady clock.
	struct Client {
		ctx: egui::Context,
		view: MessagingUi,
		time: f64,
	}

	impl Client {
		fn new(interface: Option<PluginInterface>) -> Self {
			let ctx = egui::Context::default();
			ctx.enable_accesskit();
			let mut view = MessagingUi::default();
			view.extensions.set_entries(vec![plugin_entry()]);
			view.set_plugin_interface(interface);
			Self {
				ctx,
				view,
				time: 0.0,
			}
		}

		/// One frame with `events`: each painted text with a point inside it, and the
		/// accessible names with their rects.
		fn frame(&mut self, state: &mut State, events: Vec<egui::Event>) -> Frame {
			self.time += 0.05;
			let view = &mut self.view;
			let output = self.ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1200.0, 800.0),
					)),
					time: Some(self.time),
					events,
					..Default::default()
				},
				|ui| {
					view.show(ui, state);
				},
			);
			let mut texts = Vec::new();
			let mut stack: Vec<_> = output.shapes.iter().map(|clipped| &clipped.shape).collect();
			while let Some(shape) = stack.pop() {
				match shape {
					egui::Shape::Text(text) => {
						let point = text.pos + egui::vec2(4.0, 4.0);
						texts.push((text.galley.text().to_owned(), point));
					}
					egui::Shape::Vec(shapes) => stack.extend(shapes),
					_ => {}
				}
			}
			let names = output
				.platform_output
				.accesskit_update
				.iter()
				.flat_map(|update| update.nodes.iter())
				.filter_map(|(_, node)| {
					let bounds = node.bounds()?;
					node.label().map(|label| {
						(
							label.to_owned(),
							egui::Rect::from_min_max(
								egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
								egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
							),
						)
					})
				})
				.collect();
			output.drop_without_applying_deltas();
			(texts, names)
		}

		/// The invocations queued for the plugin so far.
		fn routed(&self) -> Vec<&Invocation> {
			self.view
				.extensions
				.requests
				.iter()
				.filter_map(|request| match request {
					ExtensionRequest::Invoke { invocation, .. } => Some(invocation),
					_ => None,
				})
				.collect()
		}
	}

	/// Checks that a granted tree stands in for the main panes: its rows, icons' tinted
	/// labels and badges render, the usual channel list and composer are gone, and the
	/// settings control stays reachable. An empty tree shows the generic placeholder instead.
	#[test]
	fn granted_tree_replaces_the_panes_and_renders_rows() {
		let mut state = one_server();
		let tree = interface(
			vec![
				extensions::InterfaceNode {
					id: "server:10".into(),
					icon: Some(extensions::InterfaceIcon::Server),
					text: "Voice club".into(),
					badge: Some("3".into()),
					emphasis: true,
					..node("server:10", "Voice club")
				},
				extensions::InterfaceNode {
					indent: 1,
					icon: Some(extensions::InterfaceIcon::MicrophoneSlash),
					tint: Some(extensions::InterfaceTint::Danger),
					..node("channel:30", "Lobby")
				},
			],
			None,
		);
		let mut client = Client::new(Some(tree));
		client.frame(&mut state, vec![]);
		let (texts, names) = client.frame(&mut state, vec![]);
		let shown = |needle: &str| texts.iter().any(|(text, _)| text.contains(needle));
		assert!(shown("Voice club"), "server row listed: {texts:?}");
		assert!(shown("Lobby"), "channel row listed: {texts:?}");
		assert!(shown("3"), "badge listed: {texts:?}");
		assert!(!shown("plans-and-notes"), "channel list hidden: {texts:?}");
		let placeholder = crate::i18n::translate_args(
			"composer-placeholder-channel",
			&[("channel", "plans-and-notes")],
		);
		assert!(!shown(&placeholder), "composer hidden: {texts:?}");
		let settings = crate::i18n::translate("user-settings");
		assert!(
			names.iter().any(|(name, _)| *name == settings),
			"settings reachable: {names:?}"
		);
		let control = |names: &[(String, egui::Rect)]| {
			names
				.iter()
				.find(|(name, _)| *name == settings)
				.map(|(_, bounds)| *bounds)
		};
		let plain = control(&names).expect("settings control");
		// The dock renders at the left of the control row: a loaded server paints its
		// initials through the rail's own painting, an unknown one falls back to the
		// label's, add-server shows only when asked, and the row's height never grows.
		let add_server = crate::i18n::translate("notifications-notification-rail-add-a-server");
		let docked = extensions::InterfaceUi {
			dock: vec![
				extensions::InterfaceDockItem {
					id: "dock:voice".into(),
					label: "Voice club".into(),
					guild_id: Some("10".into()),
					selected: true,
				},
				extensions::InterfaceDockItem {
					id: "dock:island".into(),
					label: "Island hub".into(),
					guild_id: Some("424242".into()),
					selected: false,
				},
			],
			add_server: true,
			..interface(vec![node("channel:30", "Lobby")], None).tree
		};
		client.view.set_plugin_interface(Some(PluginInterface {
			plugin: "tree-plugin".into(),
			tree: docked.clone(),
		}));
		client.frame(&mut state, vec![]);
		let (texts, names) = client.frame(&mut state, vec![]);
		assert!(
			texts.iter().any(|(text, _)| text == "Vc"),
			"loaded guild initials in the dock: {texts:?}"
		);
		let island = texts
			.iter()
			.find(|(text, _)| text == "Ih")
			.map(|(_, point)| *point)
			.expect("unknown guild falls back to the label's initials");
		assert!(
			names.iter().any(|(name, _)| *name == add_server),
			"add-server offered when asked: {names:?}"
		);
		let grown = control(&names).expect("settings control");
		assert_eq!(
			(grown.min.y, grown.height()),
			(plain.min.y, plain.height()),
			"the dock never grows the control row"
		);
		// A dock click routes the item's id through the same action as a row click.
		client.frame(&mut state, vec![egui::Event::PointerMoved(island)]);
		for pressed in [true, false] {
			client.frame(
				&mut state,
				vec![egui::Event::PointerButton {
					pos: island,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				}],
			);
		}
		let routed = client.routed();
		assert_eq!(
			routed.len(),
			1,
			"one dock click, one invocation: {routed:?}"
		);
		assert_eq!(routed[0].action, extensions::INTERFACE_CLICK_ACTION);
		assert_eq!(
			routed[0].values.get("id").map(String::as_str),
			Some("dock:island")
		);
		assert!(!routed[0].double_clicked, "first dock click is single");
		// Without the flag the dock stays but the client's add-server tile goes away.
		client.view.set_plugin_interface(Some(PluginInterface {
			plugin: "tree-plugin".into(),
			tree: extensions::InterfaceUi {
				add_server: false,
				..docked
			},
		}));
		client.frame(&mut state, vec![]);
		let (texts, names) = client.frame(&mut state, vec![]);
		assert!(
			texts.iter().any(|(text, _)| text == "Vc"),
			"dock stays without the flag: {texts:?}"
		);
		assert!(
			!names.iter().any(|(name, _)| *name == add_server),
			"add-server only on request: {names:?}"
		);
		// An empty tree keeps the panes replaced and says so generically.
		client
			.view
			.set_plugin_interface(Some(interface(vec![], None)));
		let (texts, _) = client.frame(&mut state, vec![]);
		assert!(
			texts
				.iter()
				.any(|(text, _)| *text == crate::i18n::translate("interface-ui-empty")),
			"empty placeholder: {texts:?}"
		);
		assert!(
			!shown("plans-and-notes"),
			"channel list still hidden: {texts:?}"
		);
	}

	/// Checks that a click routes the row's id to the owning plugin through its
	/// `interface_click` panel action, a double click flags `double_clicked`, rows without ids
	/// stay inert, and keyboard activation follows the focused row by its id even after a
	/// row is inserted before it: Enter opens it (`double_clicked`), Space is a single click.
	#[test]
	fn row_clicks_route_the_node_id_and_flag_double_clicks() {
		let mut state = one_server();
		let mut client = Client::new(Some(interface(
			vec![
				node("server:10", "Voice club"),
				node("channel:30", "Lobby"),
				node("", "Just a label"),
			],
			None,
		)));
		client.frame(&mut state, vec![]);
		let (texts, _) = client.frame(&mut state, vec![]);
		let lobby = texts
			.iter()
			.find(|(text, _)| text == "Lobby")
			.map(|(_, point)| *point)
			.expect("channel row");
		let label = texts
			.iter()
			.find(|(text, _)| text == "Just a label")
			.map(|(_, point)| *point)
			.expect("inert row");
		let click = |client: &mut Client, state: &mut State, pos: egui::Pos2| {
			for pressed in [true, false] {
				client.frame(
					state,
					vec![egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					}],
				);
			}
		};
		client.frame(&mut state, vec![egui::Event::PointerMoved(lobby)]);
		click(&mut client, &mut state, lobby);
		let routed = client.routed();
		assert_eq!(routed.len(), 1, "one click, one invocation: {routed:?}");
		assert_eq!(routed[0].action, extensions::INTERFACE_CLICK_ACTION);
		assert_eq!(
			routed[0].values.get("id").map(String::as_str),
			Some("channel:30")
		);
		assert!(!routed[0].double_clicked, "first click is single");
		// The second click of the pair routes the same row flagged as a double click.
		click(&mut client, &mut state, lobby);
		let routed = client.routed();
		assert_eq!(routed.len(), 2, "double click routes once more: {routed:?}");
		assert_eq!(
			routed[1].values.get("id").map(String::as_str),
			Some("channel:30")
		);
		assert!(routed[1].double_clicked, "second click is double");
		// An inert row never routes.
		client.frame(&mut state, vec![egui::Event::PointerMoved(label)]);
		click(&mut client, &mut state, label);
		assert_eq!(client.routed().len(), 2, "inert row stays inert");
		// Focus stays with the row's id when a row is inserted before it.
		let row = super::row_id("tree-plugin", &node("channel:30", "Lobby"), 1);
		client.ctx.memory_mut(|memory| memory.request_focus(row));
		client.frame(&mut state, vec![]);
		client.view.set_plugin_interface(Some(interface(
			vec![
				node("server:10", "Voice club"),
				node("participant:7", "Robin"),
				node("channel:30", "Lobby"),
				node("", "Just a label"),
			],
			None,
		)));
		client.frame(&mut state, vec![]);
		let key = |key| {
			vec![egui::Event::Key {
				key,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: egui::Modifiers::NONE,
			}]
		};
		// Enter is the row's primary open action, flagged like a double click.
		client.frame(&mut state, key(egui::Key::Enter));
		let routed = client.routed();
		assert_eq!(routed.len(), 3, "Enter routes the focused row: {routed:?}");
		assert_eq!(
			routed[2].values.get("id").map(String::as_str),
			Some("channel:30"),
			"focus followed the row, not its index"
		);
		assert!(routed[2].double_clicked, "Enter opens like a double click");
		// Space stays a single click.
		client.frame(&mut state, key(egui::Key::Space));
		let routed = client.routed();
		assert_eq!(routed.len(), 4, "Space routes the focused row: {routed:?}");
		assert_eq!(
			routed[3].values.get("id").map(String::as_str),
			Some("channel:30")
		);
		assert!(!routed[3].double_clicked, "Space is a single click");
	}

	/// Checks that clearing the interface restores the normal layout, and that without one
	/// the usual channel list shows.
	#[test]
	fn clearing_the_interface_restores_the_normal_layout() {
		let mut state = one_server();
		let mut client = Client::new(Some(interface(vec![node("channel:30", "Lobby")], None)));
		client.frame(&mut state, vec![]);
		let (texts, _) = client.frame(&mut state, vec![]);
		assert!(
			!texts
				.iter()
				.any(|(text, _)| text.contains("plans-and-notes")),
			"channel list hidden: {texts:?}"
		);
		client.view.set_plugin_interface(None);
		client.frame(&mut state, vec![]);
		let (texts, _) = client.frame(&mut state, vec![]);
		assert!(
			texts
				.iter()
				.any(|(text, _)| text.contains("plans-and-notes")),
			"normal layout restored: {texts:?}"
		);
	}

	/// Checks the window beside the tree: a call window shows the host's native call stage,
	/// a chat window selects its channel and shows that conversation with its composer, an
	/// unknown channel leaves the window empty until it loads and is then selected, native
	/// navigation moves the window to the newly selected conversation without reselecting
	/// the requested one, and no window shows the tree alone.
	#[test]
	fn window_shows_the_native_stage_or_conversation_beside_the_tree() {
		use extensions::InterfaceWindow;
		let mut state = test_support::voice_demo_state();
		// The fixture's streaming participant puts a watch control on the native stage.
		let stage = "Watch Stream";
		let composer = crate::i18n::translate_args(
			"composer-placeholder-channel",
			&[("channel", "getting-started")],
		);
		let tree = |window| Some(interface(vec![node("channel:25", "Room")], window));
		let mut client = Client::new(tree(Some(InterfaceWindow::Call)));
		client.frame(&mut state, vec![]);
		let (texts, _) = client.frame(&mut state, vec![]);
		let shown = |texts: &[(String, egui::Pos2)], needle: &str| {
			texts.iter().any(|(text, _)| text.contains(needle))
		};
		assert!(shown(&texts, stage), "stage beside the tree: {texts:?}");
		assert!(shown(&texts, "Room"), "tree still listed: {texts:?}");
		// A chat window opens its channel through selection and renders that conversation.
		client
			.view
			.set_plugin_interface(tree(Some(InterfaceWindow::Chat {
				channel_id: "20".into(),
			})));
		client.frame(&mut state, vec![]);
		let (texts, _) = client.frame(&mut state, vec![]);
		assert_eq!(
			state.selected,
			Some(Id(20)),
			"chat window selects its channel"
		);
		assert!(shown(&texts, &composer), "conversation composer: {texts:?}");
		assert!(
			!shown(&texts, stage),
			"the stage gives way to the chat: {texts:?}"
		);
		assert!(shown(&texts, "Room"), "tree still listed: {texts:?}");
		// An unknown channel shows nothing in the window and raises no error.
		client
			.view
			.set_plugin_interface(tree(Some(InterfaceWindow::Chat {
				channel_id: "424242".into(),
			})));
		client.frame(&mut state, vec![]);
		let (texts, _) = client.frame(&mut state, vec![]);
		assert!(
			!shown(&texts, &composer),
			"unknown channel shows nothing: {texts:?}"
		);
		assert!(shown(&texts, "Room"), "tree still listed: {texts:?}");
		// Once the channel loads, the pending request selects it on the next frame.
		let mut late = state.channel(Id(20)).unwrap().clone();
		late.id = Id(424242);
		late.name = "late-arrival".into();
		state.channels.push(late);
		state.revision += 1;
		state.invalidate_navigation();
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		client.frame(&mut state, vec![]);
		let (texts, _) = client.frame(&mut state, vec![]);
		assert_eq!(
			state.selected,
			Some(Id(424242)),
			"a loaded channel is retried"
		);
		let late_composer = crate::i18n::translate_args(
			"composer-placeholder-channel",
			&[("channel", "late-arrival")],
		);
		assert!(
			shown(&texts, &late_composer),
			"loaded channel's conversation: {texts:?}"
		);
		// Native navigation (a link, an invite, the switcher) keeps a conversation visible:
		// the window follows the new selection and the old request is not reselected.
		let _ = state.select(Id(21));
		client.frame(&mut state, vec![]);
		let (texts, _) = client.frame(&mut state, vec![]);
		assert_eq!(state.selected, Some(Id(21)), "native selection stays");
		let native_composer = crate::i18n::translate_args(
			"composer-placeholder-channel",
			&[("channel", "long-form")],
		);
		assert!(
			shown(&texts, &native_composer),
			"newly selected conversation shown: {texts:?}"
		);
		assert!(shown(&texts, "Room"), "tree still listed: {texts:?}");
		// Without a window only the tree shows.
		client.view.set_plugin_interface(tree(None));
		client.frame(&mut state, vec![]);
		let (texts, _) = client.frame(&mut state, vec![]);
		assert!(
			!shown(&texts, stage),
			"no stage without a window: {texts:?}"
		);
		assert!(
			!shown(&texts, &composer),
			"no chat without a window: {texts:?}"
		);
		assert!(shown(&texts, "Room"), "tree listed: {texts:?}");
	}
}

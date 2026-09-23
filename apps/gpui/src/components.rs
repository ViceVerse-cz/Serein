//! Discord message components (buttons, selects, text displays, sections, containers) through
//! `State::prepare_component`; forms and uploads stay in the main app.
use crate::Serein;
use crate::theme::{Icon, color, icon, palette, tint};
use gpui::{prelude::*, *};
use model::{Component, Id, Message};

/// Component trees are bounded by the model; this only caps how deep the GPUI view nests.
const MAX_DEPTH: u8 = 6;

fn emoji_label(component: &Component) -> Option<String> {
	let emoji = component.emoji.as_ref()?;
	match (&emoji.id, &emoji.name) {
		(None, Some(name)) => Some(name.clone()),
		(Some(_), Some(name)) => Some(format!(":{name}:")),
		_ => None,
	}
}

impl Serein {
	pub(crate) fn render_components(
		&mut self,
		message: &Message,
		cx: &mut Context<Self>,
	) -> Option<AnyElement> {
		if message.components.is_empty() {
			return None;
		}
		let mut part = 1;
		let children = message
			.components
			.clone()
			.iter()
			.map(|component| self.component(message, component, 0, &mut part, cx))
			.collect::<Vec<_>>();
		Some(
			div()
				.mt_1()
				.max_w(px(560.))
				.flex()
				.flex_col()
				.gap_2()
				.children(children)
				.into_any_element(),
		)
	}

	fn component(
		&mut self,
		message: &Message,
		component: &Component,
		depth: u8,
		part: &mut u16,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let p = palette();
		if depth > MAX_DEPTH {
			return div().into_any_element();
		}
		let mut nested = |this: &mut Self, children: &[Component], cx: &mut Context<Self>| {
			children
				.iter()
				.map(|child| this.component(message, child, depth + 1, part, cx))
				.collect::<Vec<_>>()
		};
		match component.kind {
			// Action row.
			1 => div()
				.flex()
				.flex_wrap()
				.gap_2()
				.children(nested(self, &component.components, cx))
				.into_any_element(),
			2 => self.component_button(message.id, component, cx),
			3 => self.component_select(message.id, component, cx),
			// Section: text beside an optional button or thumbnail.
			9 => {
				let texts = nested(self, &component.components, cx);
				let accessory = component
					.accessory
					.as_deref()
					.map(|accessory| self.component(message, accessory, depth + 1, part, cx));
				div()
					.flex()
					.gap_3()
					.items_start()
					.child(
						div()
							.flex_1()
							.min_w_0()
							.flex()
							.flex_col()
							.gap_1()
							.children(texts),
					)
					.children(accessory)
					.into_any_element()
			}
			// Text display.
			10 => {
				*part += 1;
				let text = component.content.clone().unwrap_or_default();
				div()
					.flex()
					.flex_col()
					.gap_1()
					.children(self.markdown(message, *part, &text, cx))
					.into_any_element()
			}
			// Separator.
			14 => div()
				.when(component.divider != Some(false), |d| {
					d.h(px(1.)).bg(color(p.border))
				})
				.my(px(if component.spacing == Some(2) {
					12.
				} else {
					6.
				}))
				.into_any_element(),
			// Container with an optional accent bar.
			17 => {
				let bar = component
					.accent_color
					.map_or(color(p.border), |value| rgb(value & 0xff_ffff));
				div()
					.rounded(px(8.))
					.bg(color(p.raised))
					.border_1()
					.border_color(color(p.border))
					.overflow_hidden()
					.flex()
					.child(div().w(px(4.)).flex_none().bg(bar))
					.child(
						div()
							.flex_1()
							.min_w_0()
							.p_3()
							.flex()
							.flex_col()
							.gap_2()
							.children(nested(self, &component.components, cx)),
					)
					.into_any_element()
			}
			11..=13 => div()
				.flex()
				.items_center()
				.gap_2()
				.text_size(px(13.))
				.text_color(color(p.muted))
				.child(icon(Icon::FileImage, px(18.), color(p.muted)))
				.child(match component.kind {
					13 => "Attached file",
					_ => "Media",
				})
				.into_any_element(),
			kind => div()
				.text_size(px(13.))
				.text_color(color(p.muted))
				.child(format!(
					"Unsupported component (type {kind}) · open in the main app"
				))
				.into_any_element(),
		}
	}

	fn component_button(
		&self,
		message: Id,
		component: &Component,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let p = palette();
		let style = component.style.unwrap_or(2);
		let label = component.label.clone().unwrap_or_default();
		let (fill, text) = match style {
			1 => (color(p.accent), color(p.accent_text)),
			3 => (color(p.positive), white().into()),
			4 => (color(p.danger), white().into()),
			_ => (color(p.selected), color(p.text_strong)),
		};
		let custom_id = component.custom_id.clone();
		let url = component.url.clone().filter(|_| style == 5);
		let disabled = component.disabled || (url.is_none() && custom_id.is_none());
		div()
			.id(ElementId::NamedInteger(
				format!("component-{}", component.id).into(),
				message.0,
			))
			.h(px(32.))
			.px_4()
			.rounded(px(6.))
			.bg(fill)
			.flex()
			.items_center()
			.gap_2()
			.text_size(px(14.))
			.font_weight(FontWeight::MEDIUM)
			.text_color(text)
			.when(disabled, |d| d.opacity(0.5))
			.when(!disabled, |d| d.cursor_pointer().hover(|d| d.opacity(0.9)))
			.children(emoji_label(component))
			.child(label)
			.when(url.is_some(), |d| d.child("↗"))
			.when(!disabled, |d| {
				d.on_click(cx.listener(move |this, _, window, cx| {
					if let Some(url) = url.clone() {
						crate::chat::confirm_open_link(url, window, cx);
					} else if let Some(custom_id) = &custom_id {
						this.use_component(message, custom_id, Vec::new(), cx);
					}
				}))
			})
			.into_any_element()
	}

	fn component_select(
		&self,
		message: Id,
		component: &Component,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let p = palette();
		let custom_id = component.custom_id.clone().unwrap_or_default();
		let open = self
			.open_select
			.as_ref()
			.is_some_and(|(id, open)| *id == message && *open == custom_id);
		let chosen = component
			.options
			.iter()
			.filter(|option| option.default)
			.map(|option| option.label.as_str())
			.collect::<Vec<_>>()
			.join(", ");
		let label = if chosen.is_empty() {
			component
				.placeholder
				.clone()
				.unwrap_or_else(|| "Make a selection".into())
		} else {
			chosen
		};
		let toggle = custom_id.clone();
		div()
			.w(px(400.))
			.max_w_full()
			.flex()
			.flex_col()
			.gap_1()
			.child(
				div()
					.id(ElementId::NamedInteger(
						format!("select-{}", component.id).into(),
						message.0,
					))
					.h(px(40.))
					.px_3()
					.rounded(px(6.))
					.bg(color(p.base))
					.border_1()
					.border_color(color(if open { p.accent } else { p.border }))
					.flex()
					.items_center()
					.justify_between()
					.when(component.disabled, |d| d.opacity(0.5))
					.when(!component.disabled, |d| {
						d.cursor_pointer()
							.on_click(cx.listener(move |this, _, _, cx| {
								this.open_select = match &this.open_select {
									Some((id, open)) if *id == message && *open == toggle => None,
									_ => Some((message, toggle.clone())),
								};
								cx.notify();
							}))
					})
					.child(
						div()
							.text_size(px(14.))
							.text_color(color(if component.options.iter().any(|o| o.default) {
								p.text_strong
							} else {
								p.muted
							}))
							.child(label),
					)
					.child(icon(Icon::CaretDown, px(14.), color(p.muted))),
			)
			.when(open, |d| {
				d.child(
					div()
						.p_1()
						.rounded(px(6.))
						.bg(color(p.base))
						.border_1()
						.border_color(color(p.border))
						.flex()
						.flex_col()
						.children(component.options.iter().enumerate().map(|(index, option)| {
							let value = option.value.clone();
							let custom_id = custom_id.clone();
							div()
								.id(("select-option", index))
								.px_2()
								.py_1()
								.rounded(px(4.))
								.cursor_pointer()
								.hover(|d| d.bg(color(p.hover)))
								.flex()
								.gap_2()
								.on_click(cx.listener(move |this, _, _, cx| {
									this.open_select = None;
									this.use_component(
										message,
										&custom_id,
										vec![value.clone()],
										cx,
									);
								}))
								.children(option.emoji.as_ref().and_then(|emoji| {
									emoji.id.is_none().then(|| emoji.name.clone()).flatten()
								}))
								.child(
									div()
										.flex()
										.flex_col()
										.child(
											div()
												.text_size(px(14.))
												.text_color(color(p.text_strong))
												.child(option.label.clone()),
										)
										.children(option.description.clone().map(|description| {
											div()
												.text_size(px(12.))
												.text_color(color(p.muted))
												.child(description)
										})),
								)
						})),
				)
			})
			.into_any_element()
	}

	fn use_component(
		&mut self,
		message: Id,
		custom_id: &str,
		values: Vec<String>,
		cx: &mut Context<Self>,
	) {
		let command = self.state.prepare_component(message, custom_id, values);
		if command.is_none() {
			self.notify_user(if self.state.demo {
				"Offline preview · interactions are not sent"
			} else {
				"This interaction is unavailable right now."
			});
		}
		self.dispatch(command);
		cx.notify();
	}

	/// Pending interaction feedback: the reducer tracks one submission at a time.
	pub(crate) fn interaction_notice(&self) -> Option<impl IntoElement> {
		let p = palette();
		(self.state.interactions.busy() || self.state.interactions.modal.is_some()).then(|| {
			div()
				.px_4()
				.pb_1()
				.text_size(px(12.))
				.text_color(color(p.muted))
				.child(if self.state.interactions.modal.is_some() {
					"This app opened a form · forms are only available in the main Serein app"
				} else {
					"Sending interaction…"
				})
				.bg(tint(p.chat, 1.))
		})
	}
}

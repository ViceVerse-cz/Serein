//! Native profile preview and grouped controls for rich-presence tools.
use crate::{
	avatars::Avatars,
	design,
	extensions_ui::{TextInputStates, render_elements},
};
use extensions::{CustomRichPresence, Element, RichPresenceKind, RichPresenceTimer};
use std::collections::BTreeMap;

fn controls(element: &Element) -> bool {
	matches!(element, Element::Row { children } if children.iter().all(|child| matches!(child, Element::Button { .. })))
}

pub(crate) fn actions(
	ui: &mut egui::Ui,
	elements: &[Element],
	action: &mut Option<String>,
	busy: bool,
) {
	ui.add_enabled_ui(!busy, |ui| {
		for element in elements {
			if let Element::Row { children } = element
				&& controls(element)
			{
				for child in children
					.iter()
					.filter(|e| matches!(e, Element::Button { id, .. } if id == "apply"))
					.chain(
						children
							.iter()
							.filter(|e| !matches!(e, Element::Button { id, .. } if id == "apply")),
					) {
					if let Element::Button { id, label } = child {
						let kind = if id == "apply" {
							design::ButtonKind::Primary
						} else if id == "stop" {
							design::ButtonKind::Neutral
						} else {
							design::ButtonKind::Outline
						};
						if design::button(ui, label, kind).clicked() {
							*action = Some(id.clone());
						}
					}
				}
			}
		}
	});
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn editor(
	ui: &mut egui::Ui,
	elements: &[Element],
	values: &mut BTreeMap<String, String>,
	action: &mut Option<String>,
	inputs: &mut TextInputStates,
	avatars: &mut Avatars,
	state: &client_core::State,
	activity_status: (&str, bool),
) {
	seed_fields(elements, values);
	let preview = elements.iter().find_map(|element| match element {
		Element::ActivityPreview { presence } => Some(presence.as_ref()),
		_ => None,
	});
	let sections: Vec<_> = elements
		.iter()
		.enumerate()
		.filter_map(|(index, e)| match e {
			Element::Heading { text } if index != 0 => Some((index, text.as_str())),
			_ => None,
		})
		.collect();
	if sections.is_empty() {
		form(ui, elements, values, action, inputs);
		preview_card(ui, preview, avatars, state, activity_status);
		return;
	}
	let first_section = sections.first().map_or(elements.len(), |(index, _)| *index);
	for element in &elements[..first_section] {
		match element {
			Element::Text { text } => design::hint(ui, text),
			Element::Heading { .. } => {}
			_ => form(ui, std::slice::from_ref(element), values, action, inputs),
		}
	}
	ui.add_space(12.0);
	let tab_id = ui.scope_id().with("presence-section");
	let mut selected = ui
		.ctx()
		.data_mut(|data| data.get_temp::<usize>(tab_id).unwrap_or(0))
		.min(sections.len().saturating_sub(1));
	let labels: Vec<_> = sections.iter().map(|(_, title)| *title).collect();
	let per_row = if ui.available_width() < 600.0 {
		3
	} else {
		labels.len().max(1)
	};
	for (row, labels) in labels.chunks(per_row).enumerate() {
		ui.push_id(row, |ui| {
			if let Some(index) = design::segmented(ui, labels, selected.wrapping_sub(row * per_row))
			{
				selected = row * per_row + index;
			}
		});
	}
	ui.ctx().data_mut(|data| data.insert_temp(tab_id, selected));
	ui.add_space(16.0);
	let Some(&(start, title)) = sections.get(selected) else {
		return;
	};
	let end = sections
		.get(selected + 1)
		.map_or(elements.len(), |(index, _)| *index);
	let fields = &elements[start + 1..end];
	if ui.available_width() >= 690.0 {
		let width = ui.available_width();
		ui.horizontal_top(|ui| {
			ui.spacing_mut().item_spacing.x = 24.0;
			ui.allocate_ui_with_layout(
				egui::vec2(width - 299.0, 0.0),
				egui::Layout::top_down(egui::Align::Min),
				|ui| {
					design::group(ui, title, |ui| form(ui, fields, values, action, inputs));
				},
			);
			ui.allocate_ui_with_layout(
				egui::vec2(275.0, 0.0),
				egui::Layout::top_down(egui::Align::Min),
				|ui| {
					preview_card(ui, preview, avatars, state, activity_status);
				},
			);
		});
	} else {
		design::group(ui, title, |ui| form(ui, fields, values, action, inputs));
		ui.add_space(20.0);
		preview_card(ui, preview, avatars, state, activity_status);
	}
}

fn form(
	ui: &mut egui::Ui,
	elements: &[Element],
	values: &mut BTreeMap<String, String>,
	action: &mut Option<String>,
	inputs: &mut TextInputStates,
) {
	for element in elements {
		if controls(element) || matches!(element, Element::ActivityPreview { .. }) {
			continue;
		}
		match element {
			Element::TextInput { id, label, value } => {
				let label = design::label(ui, label);
				let value = values.entry(id.clone()).or_insert_with(|| value.clone());
				let response = design::input(
					ui,
					egui::TextEdit::singleline(value)
						.align(egui::Align2::LEFT_CENTER)
						.id(inputs.id(id))
						.char_limit(1024),
				)
				.labelled_by(label.id);
				inputs.track(&response);
				ui.add_space(10.0);
			}
			Element::Select {
				id,
				label,
				options,
				value,
			} => {
				let label = design::label(ui, label);
				let selected = values.entry(id.clone()).or_insert_with(|| value.clone());
				egui::ComboBox::from_id_salt(id)
					.width(ui.available_width())
					.selected_text(selected.as_str())
					.show_ui(ui, |ui| {
						for option in options {
							ui.selectable_value(selected, option.clone(), option);
						}
					})
					.response
					.labelled_by(label.id);
				ui.add_space(10.0);
			}
			Element::Text { text } => {
				design::hint(ui, text);
				ui.add_space(8.0);
			}
			Element::Button { id, label } => {
				if ui
					.push_id(id, |ui| {
						design::button(ui, label, design::ButtonKind::Neutral)
					})
					.inner
					.clicked()
				{
					*action = Some(id.clone());
				}
			}
			_ => render_elements(ui, std::slice::from_ref(element), values, action, inputs),
		}
	}
}

fn preview_card(
	ui: &mut egui::Ui,
	presence: Option<&CustomRichPresence>,
	avatars: &mut Avatars,
	state: &client_core::State,
	activity_status: (&str, bool),
) {
	let colors = design::palette(ui);
	ui.label(design::eyebrow(ui, "PROFILE PREVIEW", colors.muted));
	ui.add_space(8.0);
	if let Some(presence) = presence {
		let kind = match presence.kind {
			RichPresenceKind::Playing => 0,
			RichPresenceKind::Streaming => 1,
			RichPresenceKind::Listening => 2,
			RichPresenceKind::Watching => 3,
			RichPresenceKind::Competing => 5,
		};
		let application = presence.application_id.parse::<model::Id>().ok();
		let image = |value: &Option<extensions::RichPresenceImage>| {
			value
				.as_ref()
				.and_then(|value| value.key.parse::<model::Id>().ok())
				.zip(application)
				.map(|(asset, application)| model::ActivityImage::Asset { application, asset })
		};
		let (started_at, ends_at) = match presence.timer {
			RichPresenceTimer::Custom { start, end } => (start, end),
			_ => (None, None),
		};
		let activity = model::RichActivity {
			kind,
			name: presence.name.clone(),
			details: presence.details.clone(),
			state: presence.state.clone(),
			image: image(&presence.large_image)
				.or_else(|| application.map(model::ActivityImage::Application)),
			small_image: image(&presence.small_image),
			started_at,
			ends_at,
		};
		// Drafts never start account/network work. Artwork is a local placeholder.
		crate::profiles::activity_card(ui, &activity, avatars, true, (colors.raised, colors.muted));
		summary(ui, presence);
		if presence.large_image.is_some() || presence.small_image.is_some() {
			design::hint(
				ui,
				"Artwork resolves when applied. This draft preview uses placeholders.",
			);
		}
	} else {
		design::card(ui, |ui| {
			crate::icons::inline(ui, crate::icons::Icon::GameController, 32.0, colors.muted);
			ui.label(egui::RichText::new("Your activity goes here").strong());
			ui.label("Enter an application ID and activity name, then choose Preview.");
		});
	}
	ui.add_space(12.0);
	if !state.demo {
		if !activity_status.1 {
			crate::dialog::notice(
				ui,
				crate::dialog::Level::Warning,
				"Activity sharing is off. Enable it in Settings > Registered Games to publish your presence.",
			);
		} else if !activity_status.0.is_empty() {
			design::hint(ui, activity_status.0);
		}
	}
	design::hint(
		ui,
		if state.demo {
			"Offline demo. Nothing is published."
		} else if !state.gateway_connected {
			"Offline. Your saved activity will wait for a connection."
		} else {
			"Enable activity sharing in Settings > Registered Games. Invisible hides your activity."
		},
	);
}

pub(crate) fn summary(ui: &mut egui::Ui, presence: &CustomRichPresence) {
	if let Some(party) = &presence.party {
		ui.label(format!("Party: {} / {}", party.current, party.max));
	}
	match presence.timer {
		RichPresenceTimer::Elapsed => {
			design::hint(
				ui,
				"Elapsed since starting; applying edits keeps the timer.",
			);
		}
		RichPresenceTimer::LocalDay => {
			design::hint(ui, "Timer starts at local midnight.");
		}
		_ => {}
	}
	for button in &presence.buttons {
		ui.add_enabled(
			false,
			egui::Button::new(&button.label).min_size(egui::vec2(ui.available_width(), 30.0)),
		)
		.on_disabled_hover_text(&button.url);
	}
}

// Collapsed sections still submit their draft values, including fields never focused.
fn seed_fields(elements: &[Element], values: &mut BTreeMap<String, String>) {
	for element in elements {
		match element {
			Element::TextInput { id, value, .. } | Element::Select { id, value, .. } => {
				values.entry(id.clone()).or_insert_with(|| value.clone());
			}
			Element::Checkbox { id, checked, .. } => {
				values
					.entry(id.clone())
					.or_insert_with(|| checked.to_string());
			}
			Element::Slider { id, value, .. } => {
				values
					.entry(id.clone())
					.or_insert_with(|| value.to_string());
			}
			Element::Row { children } => seed_fields(children, values),
			_ => {}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn collapsed_fields_submit_defaults_and_keep_edits() {
		let fields = vec![Element::TextInput {
			id: "image-key".into(),
			label: "Image".into(),
			value: "galaxy".into(),
		}];
		let mut values = BTreeMap::new();
		seed_fields(&fields, &mut values);
		assert_eq!(values["image-key"], "galaxy");
		values.insert("image-key".into(), "stars".into());
		seed_fields(&fields, &mut values);
		assert_eq!(values["image-key"], "stars");
	}
}

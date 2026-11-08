//! Quoted cards for Discord message links inside a message, with the message-link-previews
//! tweak. A linked message comes from the loaded history or an earlier lookup; otherwise it
//! is read once on demand and the card appears when it arrives.
use client_core::State;
use model::{Id, Message};

/// Cards shown under one message; further links stay plain links.
const PER_MESSAGE: usize = 3;
/// Characters of the linked message shown in a card.
const EXCERPT: usize = 240;

/// Shows a card for each message link in `message` whose target is readable.
pub(crate) fn show(ui: &mut egui::Ui, message: &Message, state: &State) {
	if !message.content.contains("/channels/")
		|| !crate::interface_tweaks::on(ui.ctx(), extensions::InterfaceTweak::MessageLinkPreviews)
	{
		return;
	}
	for (channel, linked) in targets(&message.content, message.id) {
		if !state.can_view(channel) {
			continue;
		}
		let found = state
			.timeline
			.get(linked)
			.or_else(|| state.lookups.message(linked))
			.filter(|found| found.channel == channel);
		match found {
			Some(found) => card(ui, found, state),
			None => crate::lookups::request(
				ui.ctx(),
				crate::lookups::Request::Message {
					channel,
					message: linked,
				},
			),
		}
	}
}

/// The distinct `(channel, message)` targets of the message links in `content`, never the
/// message itself, at most [`PER_MESSAGE`].
fn targets(content: &str, own: Id) -> Vec<(Id, Id)> {
	let mut targets = Vec::new();
	for link in crate::markdown::Formatted::parse(content).links {
		let Some(link) = crate::markdown::discord_chat_link(&link) else {
			continue;
		};
		let Some(linked) = link.message.filter(|id| *id != own) else {
			continue;
		};
		if !targets.contains(&(link.channel, linked)) {
			targets.push((link.channel, linked));
			if targets.len() == PER_MESSAGE {
				break;
			}
		}
	}
	targets
}

fn card(ui: &mut egui::Ui, found: &Message, state: &State) {
	let colors = crate::design::palette(ui);
	let name = found
		.author_nick
		.clone()
		.unwrap_or_else(|| found.author.name.clone());
	let mut text: String = found.content.chars().take(EXCERPT).collect();
	if text.len() < found.content.len() {
		text.push('…');
	}
	if text.trim().is_empty()
		&& let Some(file) = found.attachments.first()
	{
		text = file.filename.clone();
	}
	// Like replies, a blocked author's text is hidden only when that tweak hides them.
	let blocked =
		crate::interface_tweaks::on(ui.ctx(), extensions::InterfaceTweak::HiddenBlockedMessages)
			&& state.user_blocked(found.author.id) == Some(true);
	egui::Frame::new()
		.fill(colors.raised)
		.stroke(egui::Stroke::new(1.0, colors.border))
		.corner_radius(6.0)
		.inner_margin(egui::Margin::symmetric(10, 6))
		.show(ui, |ui| {
			ui.set_max_width(ui.available_width().min(520.0));
			ui.label(egui::RichText::new(name).strong().color(colors.text_strong));
			if blocked {
				ui.label(
					egui::RichText::new(crate::i18n::translate(
						"timeline-show-with-scroll-message-blocked",
					))
					.italics()
					.color(colors.muted),
				);
			} else {
				ui.add(egui::Label::new(egui::RichText::new(text).color(colors.text)).wrap());
			}
		});
}

#[cfg(test)]
mod tests {
	use super::*;

	fn texts(ctx: &egui::Context, run: impl FnMut(&mut egui::Ui)) -> Vec<String> {
		let mut output = ctx.run_ui(egui::RawInput::default(), run);
		output.textures_delta.clear();
		let mut found = vec![];
		for clipped in output.shapes {
			if let egui::Shape::Text(text) = clipped.shape {
				found.push(text.galley.job.text.clone());
			}
		}
		found
	}

	#[test]
	fn a_loaded_linked_message_shows_as_a_card_only_with_the_tweak() {
		let state = test_support::message_links_demo_state();
		let target = state
			.timeline
			.iter()
			.next()
			.expect("fixture message")
			.clone();
		let mut linking = target.clone();
		linking.id = Id(target.id.0 + 100_000);
		linking.content = format!(
			"see https://discord.com/channels/@me/{}/{}",
			target.channel.0, target.id.0
		);
		let excerpt: String = target.content.chars().take(20).collect();
		let ctx = egui::Context::default();
		let draw = |ui: &mut egui::Ui| show(ui, &linking, &state);
		assert!(
			texts(&ctx, draw)
				.iter()
				.all(|text| !text.contains(&excerpt))
		);
		crate::interface_tweaks::publish(
			&ctx,
			&std::sync::Arc::new([extensions::InterfaceTweak::MessageLinkPreviews].into()),
		);
		let draw = |ui: &mut egui::Ui| {
			crate::interface_tweaks::publish(
				ui.ctx(),
				&std::sync::Arc::new([extensions::InterfaceTweak::MessageLinkPreviews].into()),
			);
			show(ui, &linking, &state);
		};
		let shown = texts(&ctx, draw);
		assert!(
			shown.iter().any(|text| text.contains(&excerpt)),
			"{shown:?}"
		);
	}
	#[test]
	fn targets_are_distinct_bounded_and_never_the_message_itself() {
		let content = "https://discord.com/channels/1/2/3 https://discord.com/channels/1/2/3 \
			https://discord.com/channels/1/2/9 https://discord.com/channels/@me/4/5 \
			https://discord.com/channels/1/2 https://discord.com/channels/1/6/7 \
			https://discord.com/channels/1/8/10";
		assert_eq!(
			targets(content, Id(9)),
			[(Id(2), Id(3)), (Id(4), Id(5)), (Id(6), Id(7))]
		);
		assert!(targets("https://example.com/channels/1/2/3", Id(1)).is_empty());
	}
}

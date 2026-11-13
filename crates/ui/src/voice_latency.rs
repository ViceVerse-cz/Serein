use crate::{design, i18n};
use egui::{Align2, FontId, Pos2, Response, Sense, Shape, Stroke, Ui, vec2};
use std::time::Duration;

const SAMPLES: usize = 24;

/// Recent voice signaling RTT samples for the current socket, oldest first.
#[derive(Clone, Copy, Default)]
pub struct VoiceLatency {
	millis: [u16; SAMPLES],
	len: usize,
}

impl VoiceLatency {
	pub fn push(&mut self, sample: Duration) {
		if self.len == SAMPLES {
			self.millis.copy_within(1.., 0);
		} else {
			self.len += 1;
		}
		self.millis[self.len - 1] = sample.as_millis().min(u128::from(u16::MAX)) as u16;
	}

	pub(crate) fn popup(&self, trigger: Response) {
		let samples = &self.millis[..self.len];
		let Some(&last) = samples.last() else {
			let trigger = trigger.on_hover_text("—");
			show_popup(&trigger, |ui| {
				ui.label(i18n::translate("voice-connection-ping-pending"));
			});
			return;
		};
		let trigger = trigger.on_hover_ui(|ui| {
			ui.label(milliseconds(u32::from(last)));
		});
		show_popup(&trigger, |ui| {
			let last = milliseconds(u32::from(last));
			graph(ui, samples);
			let sum: u32 = samples.iter().copied().map(u32::from).sum();
			let count = samples.len() as u32;
			let average = milliseconds((sum + count / 2) / count);
			ui.label(i18n::translate_args(
				"voice-connection-average-ping",
				&[("value", &average)],
			));
			ui.label(i18n::translate_args(
				"voice-connection-last-ping",
				&[("value", &last)],
			));
			ui.label(i18n::translate("voice-connection-ping-description"));
		});
	}
}

fn milliseconds(millis: u32) -> String {
	i18n::translate_args(
		"voice-connection-ping-value",
		&[("millis", &millis.to_string())],
	)
}

fn show_popup(trigger: &Response, contents: impl FnOnce(&mut Ui)) {
	egui::Popup::menu(trigger).width(260.0).show(|ui| {
		ui.set_width(260.0);
		ui.heading(i18n::translate("voice-connection-title"));
		ui.separator();
		contents(ui);
	});
}

fn graph(ui: &mut Ui, samples: &[u16]) {
	let colors = design::palette(ui);
	let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 88.0), Sense::hover());
	let painter = ui.painter_at(rect);
	painter.rect_filled(rect, 4, colors.base);
	let plot = rect.shrink(8.0);
	let right = plot.right() - 36.0;
	let max = u32::from(samples.iter().copied().max().unwrap_or(0)).max(100);
	let max = max.div_ceil(100) * 100;
	for (fraction, label) in [(0.0, max), (0.5, max / 2), (1.0, 0)] {
		let y = egui::lerp(plot.top()..=plot.bottom(), fraction);
		painter.hline(
			plot.left()..=right,
			y,
			Stroke::new(1.0, colors.muted.gamma_multiply(0.3)),
		);
		painter.text(
			Pos2::new(plot.right(), y),
			Align2::RIGHT_CENTER,
			label.to_string(),
			FontId::proportional(10.0),
			colors.muted,
		);
	}
	let points: Vec<_> = samples
		.iter()
		.enumerate()
		.map(|(index, &millis)| {
			let x = (SAMPLES - samples.len() + index) as f32 / (SAMPLES - 1) as f32;
			let y = f32::from(millis) / max as f32;
			Pos2::new(
				egui::lerp(plot.left()..=right, x),
				plot.bottom() - y * plot.height(),
			)
		})
		.collect();
	if let [point] = points.as_slice() {
		painter.circle_filled(*point, 2.0, colors.accent);
	} else {
		painter.add(Shape::line(points, Stroke::new(1.5, colors.accent)));
	}
}

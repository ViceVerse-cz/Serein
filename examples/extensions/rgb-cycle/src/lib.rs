use serein_extension_sdk::{Element, Invocation, Output, Theme, ThemePalette};
use std::collections::BTreeMap;

/// Full hue rotation period. The host invokes `tick` at roughly
/// `extensions::TICK_MIN_INTERVAL_MS` (~250ms / ~4Hz) or slower -- it
/// never queues a second tick for this plugin before the first resolves,
/// so a slow round trip only stretches this further, never faster -- so
/// pick a period long enough that each step is a small, smooth color
/// change instead of a visible jump.
const CYCLE_MS: u64 = 6_000;

/// How a token's color is derived from the shared hue phase. Kept simple
/// on purpose: every `Surface` tone is dark and every `Text` tone is
/// bright, *regardless of hue*, so body text stays legible against its
/// background no matter where the animation currently sits -- contrast
/// comes from a fixed brightness gap, never from hue difference alone.
#[derive(Clone, Copy)]
enum Role {
	Surface { value: f64 },
	Text { value: f64 },
	Accent,
	/// `positive` / `warning` / `danger` carry meaning (success, caution,
	/// destructive) that a rotating hue can actively undermine -- a
	/// "delete" button briefly reading as green, say. Still fully
	/// user-controllable, just off by default with that risk named in
	/// the settings panel rather than silently animated.
	Status,
}

struct Token {
	/// The real theme color name from docs/theme-api.md.
	key: &'static str,
	label: &'static str,
	group: &'static str,
	role: Role,
	default_on: bool,
}

/// Every color token this plugin can drive, covering all four groups in
/// the theme API's token table, plus the backdrop gradient handled
/// separately below since it isn't a `colors` entry.
const TOKENS: &[Token] = &[
	// Surfaces
	Token { key: "base", label: "Base background", group: "Surfaces", role: Role::Surface { value: 0.12 }, default_on: true },
	Token { key: "sidebar", label: "Sidebar", group: "Surfaces", role: Role::Surface { value: 0.14 }, default_on: true },
	Token { key: "chat", label: "Chat area", group: "Surfaces", role: Role::Surface { value: 0.17 }, default_on: true },
	Token { key: "raised", label: "Raised panels", group: "Surfaces", role: Role::Surface { value: 0.21 }, default_on: true },
	Token { key: "hover", label: "Hover highlight", group: "Surfaces", role: Role::Surface { value: 0.27 }, default_on: true },
	Token { key: "selected", label: "Selected item", group: "Surfaces", role: Role::Surface { value: 0.31 }, default_on: true },
	Token { key: "border", label: "Borders", group: "Surfaces", role: Role::Surface { value: 0.36 }, default_on: true },
	// Text
	Token { key: "text_strong", label: "Strong text", group: "Text", role: Role::Text { value: 0.98 }, default_on: true },
	Token { key: "text", label: "Body text", group: "Text", role: Role::Text { value: 0.90 }, default_on: true },
	Token { key: "muted", label: "Muted text", group: "Text", role: Role::Text { value: 0.66 }, default_on: true },
	Token { key: "link", label: "Links", group: "Text", role: Role::Text { value: 0.95 }, default_on: true },
	// Actions and states
	Token { key: "accent", label: "Accent (includes accent text)", group: "Actions and states", role: Role::Accent, default_on: true },
	Token { key: "positive", label: "Positive / success -- recommended off", group: "Actions and states", role: Role::Status, default_on: false },
	Token { key: "warning", label: "Warning -- recommended off", group: "Actions and states", role: Role::Status, default_on: false },
	Token { key: "danger", label: "Danger / destructive -- recommended off", group: "Actions and states", role: Role::Status, default_on: false },
	// Mentions
	Token { key: "mention_bg", label: "Mention background", group: "Mentions", role: Role::Surface { value: 0.25 }, default_on: true },
	Token { key: "mention_text", label: "Mention text", group: "Mentions", role: Role::Text { value: 0.95 }, default_on: true },
];

const BACKDROP_KEY: &str = "backdrop";
const BACKDROP_LABEL: &str = "Window background gradient";

fn handle(input: Invocation) -> Output {
	match input.action.as_str() {
		"tick" => tick(input),
		"settings" => settings_panel(input),
		_ => Output::default(),
	}
}

// The host re-invokes this on its own schedule (see the `tick` surface in
// examples/extensions/README.md). There is no state between calls; every
// frame's color comes from `tick_ms` and the persisted settings alone.
fn tick(input: Invocation) -> Output {
	let settings = load_settings(input.storage.as_deref());
	let ms = input.tick_ms.unwrap_or(0);
	let phase = (ms % CYCLE_MS) as f64 / CYCLE_MS as f64;

	let mut colors = BTreeMap::new();
	for token in TOKENS {
		if !enabled(&settings, token.key, token.default_on) {
			continue;
		}
		let color = match token.role {
			Role::Surface { value } => hsv_to_hex(phase, 0.35, value),
			Role::Text { value } => hsv_to_hex(phase, 0.20, value),
			Role::Accent => hsv_to_hex(phase, 0.80, 0.58),
			Role::Status => hsv_to_hex(phase, 0.75, 0.55),
		};
		colors.insert(token.key.to_string(), color);
		if token.key == "accent" {
			// Pick readable accent text by hue band rather than trying to
			// compute real luminance: yellow/green/cyan hues read light,
			// so use dark text there; everywhere else use light text.
			let light_hue_band = (0.11..=0.52).contains(&phase);
			colors.insert(
				"accent_text".to_string(),
				(if light_hue_band { "#101014" } else { "#f5f7fa" }).to_string(),
			);
		}
	}

	let backdrop = enabled(&settings, BACKDROP_KEY, true).then(|| {
		[
			hsv_to_hex(phase, 0.70, 0.35),
			hsv_to_hex((phase + 0.5) % 1.0, 0.82, 0.90),
		]
	});

	let palette = ThemePalette {
		colors,
		backdrop,
		..Default::default()
	};

	Output {
		appearance: Some(Theme {
			light: palette.clone(),
			dark: palette,
			..Default::default()
		}),
		..Default::default()
	}
}

// The panel action behind "Open tool" in Settings > Extensions. Renders a
// checkbox per token, grouped to match the theme API's token table, plus
// a Save button. A save re-invokes this same action with every checkbox's
// current state in `values`; opening it fresh sends none, so persisted
// settings (or the defaults above) show through untouched.
fn settings_panel(input: Invocation) -> Output {
	let mut settings = load_settings(input.storage.as_deref());
	let saved = !input.values.is_empty();
	if saved {
		for token in TOKENS {
			if let Some(value) = input.values.get(&elem_id(token.key)) {
				settings.insert(token.key.to_string(), value == "true");
			}
		}
		if let Some(value) = input.values.get(&elem_id(BACKDROP_KEY)) {
			settings.insert(BACKDROP_KEY.to_string(), value == "true");
		}
	}

	let mut panel = vec![Element::Text {
		text: "Choose which parts of the theme animate. Changes apply on \
			the next tick (within about a tenth of a second) after Save."
			.to_string(),
	}];
	let mut last_group = "";
	for token in TOKENS {
		if token.group != last_group {
			if !last_group.is_empty() {
				panel.push(Element::Separator);
			}
			panel.push(Element::Heading {
				text: token.group.to_string(),
			});
			last_group = token.group;
		}
		panel.push(Element::Checkbox {
			id: elem_id(token.key),
			label: token.label.to_string(),
			checked: enabled(&settings, token.key, token.default_on),
		});
	}
	panel.push(Element::Separator);
	panel.push(Element::Heading {
		text: "Background".to_string(),
	});
	panel.push(Element::Checkbox {
		id: elem_id(BACKDROP_KEY),
		label: BACKDROP_LABEL.to_string(),
		checked: enabled(&settings, BACKDROP_KEY, true),
	});
	panel.push(Element::Separator);
	if saved {
		panel.push(Element::Text {
			text: "Saved.".to_string(),
		});
	}
	panel.push(Element::Button {
		id: "settings".to_string(),
		label: "Save".to_string(),
	});

	Output {
		panel,
		storage: saved.then(|| save_settings(&settings)),
		..Default::default()
	}
}

fn enabled(settings: &BTreeMap<String, bool>, key: &str, default_on: bool) -> bool {
	*settings.get(key).unwrap_or(&default_on)
}

fn load_settings(storage: Option<&str>) -> BTreeMap<String, bool> {
	storage
		.and_then(|data| serde_json::from_str(data).ok())
		.unwrap_or_default()
}

fn save_settings(settings: &BTreeMap<String, bool>) -> String {
	serde_json::to_string(settings).unwrap_or_default()
}

/// Theme color keys use `snake_case`; plugin element ids may not contain
/// underscores (see `extensions::valid_id`), so element ids swap in
/// hyphens. No token key contains a hyphen, so this round-trips cleanly.
fn elem_id(key: &str) -> String {
	key.replace('_', "-")
}

/// `h`, `s`, `v` each in `0.0..=1.0`. Returns `#RRGGBB`.
fn hsv_to_hex(h: f64, s: f64, v: f64) -> String {
	let i = (h * 6.0).floor();
	let f = h * 6.0 - i;
	let p = v * (1.0 - s);
	let q = v * (1.0 - f * s);
	let t = v * (1.0 - (1.0 - f) * s);
	let (r, g, b) = match (i as i64).rem_euclid(6) {
		0 => (v, t, p),
		1 => (q, v, p),
		2 => (p, v, t),
		3 => (p, q, v),
		4 => (t, p, v),
		_ => (v, p, q),
	};
	let to_byte = |c: f64| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
	format!("#{:02x}{:02x}{:02x}", to_byte(r), to_byte(g), to_byte(b))
}

serein_extension_sdk::export!(handle);

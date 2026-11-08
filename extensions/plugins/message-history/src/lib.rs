/*
 * Vencord, a Discord client mod
 * Copyright (c) 2023 Vendicated and contributors
 * Copyright (c) 2024 Equicord contributors
 * Copyright (c) 2026 Serein port contributors
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License as published by the Free Software
 * Foundation, either version 3 of the License, or (at your option) any later
 * version.
 *
 * This program is distributed in the hope that it will be useful, but WITHOUT
 * ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
 * FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License along with
 * this program. If not, see <https://www.gnu.org/licenses/>.
 */

//! Consent and event surface for the host-owned persistent, bounded message history.
//!
//! The bounded edit/delete store lives in `client-core` and `model`. This Wasm module never
//! receives retained history back, never writes plugin storage and never renders retained
//! bodies. Its `enable-history` activation action is the host's explicit opt-in signal; the
//! matching `message_events` grant covers the live observations. The native viewer reads the
//! host store directly.

use serein_extension_sdk::{EventInvocation, Output};

fn handle(_input: EventInvocation) -> Output {
	// Activation only expresses consent. Live observations are acknowledged without copying
	// message content into output, storage, replacement text or a panel.
	Output::default()
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{MAX_IO_BYTES, dispatch_typed, serde_json};

	#[test]
	fn activation_and_observation_retain_nothing_in_the_plugin() {
		for input in [
			serde_json::json!({ "action": "enable-history" }),
			serde_json::json!({
				"action": "observe",
				"message_event": {
					"kind": "delete",
					"channel_id": "10",
					"message_id": "20"
				}
			}),
			serde_json::json!({
				"action": "observe",
				"message_event": {
					"kind": "update",
					"channel_id": "10",
					"message_id": "20",
					"author_id": "30",
					"content": "private loaded body"
				}
			}),
		] {
			let bytes =
				dispatch_typed(serde_json::to_string(&input).unwrap().as_bytes(), handle).unwrap();
			assert!(bytes.len() <= MAX_IO_BYTES);
			let output: Output = serde_json::from_slice(&bytes).unwrap();
			assert!(output.panel.is_empty());
			assert!(output.storage.is_none());
			assert!(output.replacement.is_none());
			assert!(!output.preserve_deleted_messages);
		}
	}
}

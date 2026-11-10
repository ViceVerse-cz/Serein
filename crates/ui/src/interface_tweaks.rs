//! Interface tweaks that enabled plugins switched on, published to widgets each frame.
use extensions::InterfaceTweak;
use std::collections::BTreeSet;
use std::sync::Arc;

const KEY: &str = "interface-tweaks";

/// Publishes this frame's tweaks for [`on`] to read.
pub(crate) fn publish(ctx: &egui::Context, tweaks: &Arc<BTreeSet<InterfaceTweak>>) {
	ctx.data_mut(|data| data.insert_temp(egui::Id::unique(KEY), tweaks.clone()));
}

/// Whether an enabled plugin switched `tweak` on.
pub(crate) fn on(ctx: &egui::Context, tweak: InterfaceTweak) -> bool {
	ctx.data(|data| {
		data.get_temp::<Arc<BTreeSet<InterfaceTweak>>>(egui::Id::unique(KEY))
			.is_some_and(|tweaks| tweaks.contains(&tweak))
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Checks that tweaks set on the client reach widgets through a full frame, and that
	/// clearing them switches them off on the next frame.
	#[test]
	fn client_frame_publishes_and_clears_plugin_tweaks() {
		let ctx = egui::Context::default();
		let mut view = crate::MessagingUi::default();
		let mut state = test_support::demo_state();
		let mut frame = |view: &mut crate::MessagingUi| {
			let output = ctx.run_ui(Default::default(), |ui| {
				view.show(ui, &mut state);
			});
			output.drop_without_applying_deltas();
		};
		frame(&mut view);
		assert!(!on(&ctx, InterfaceTweak::ExpandedRoles));
		view.set_interface_tweaks(
			[InterfaceTweak::ExpandedRoles, InterfaceTweak::PlainProfiles].into(),
		);
		frame(&mut view);
		assert!(on(&ctx, InterfaceTweak::ExpandedRoles));
		assert!(on(&ctx, InterfaceTweak::PlainProfiles));
		assert!(!on(&ctx, InterfaceTweak::StaticTypingIndicator));
		view.set_interface_tweaks(BTreeSet::new());
		frame(&mut view);
		assert!(!on(&ctx, InterfaceTweak::ExpandedRoles));
	}
}

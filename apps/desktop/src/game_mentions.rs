use client_core::game_mentions::Status;
use eframe::egui;

type Loaded = (
	Result<Vec<model::GameMention>, &'static str>,
	tokio::time::Instant,
);

#[derive(Default)]
pub struct Loader {
	pending: Option<tokio::sync::oneshot::Receiver<Loaded>>,
	cooldown: Option<tokio::time::Instant>,
	scanned: Option<(u64, u64)>,
}

impl Loader {
	pub fn sync(
		&mut self,
		state: &mut client_core::State,
		runtime: &tokio::runtime::Runtime,
		ctx: &egui::Context,
		demo: bool,
	) {
		let revision = (state.generation, state.revision);
		if state.game_mentions.status == Status::Idle && self.scanned != Some(revision) {
			self.scanned = Some(revision);
			if state
				.timeline
				.iter()
				.any(|message| message.content.contains("<@$"))
			{
				state.game_mentions.request();
			}
		}
		let catalog = &mut state.game_mentions;
		if let Some(pending) = &mut self.pending {
			match pending.try_recv() {
				Ok((result, cooldown)) => {
					catalog.finish(result);
					self.cooldown = Some(cooldown);
					self.pending = None;
					ctx.request_repaint();
				}
				Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
					catalog.finish(Err("The game catalog is unavailable."));
					self.pending = None;
				}
				Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
			}
		}
		if !catalog.requested || catalog.status != Status::Idle {
			return;
		}
		catalog.requested = false;
		if demo {
			catalog.finish(Ok([
				"Dead by Daylight",
				"Red Dead Redemption 2",
				"Left 4 Dead 2",
				"Dead by Daylight: Special Edition",
				"Deadlock",
				"Dead Space",
				"Dead Cells",
				"Dead Island 2",
				"Dead as Disco",
				"Red Dead Redemption",
			]
			.into_iter()
			.enumerate()
			.map(|(i, name)| model::GameMention {
				id: model::Id(9000 + i as u64),
				name: name.into(),
				icon: None,
			})
			.collect()));
			ctx.request_repaint();
			return;
		}
		catalog.status = Status::Loading;
		let (send, receive) = tokio::sync::oneshot::channel();
		self.pending = Some(receive);
		let ctx = ctx.clone();
		let mut cooldown = self.cooldown.unwrap_or_else(tokio::time::Instant::now);
		runtime.spawn(async move {
			let result = async {
				let client = discord_api::rpc::client()?;
				let bytes = discord_api::detectable::download_list(&client, &mut cooldown).await?;
				tokio::task::spawn_blocking(move || {
					discord_api::detectable::decode_mentions(&bytes)
				})
				.await
				.map_err(|_| "The game catalog is unavailable.")?
			}
			.await;
			let _ = send.send((result, cooldown));
			ctx.request_repaint();
		});
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn game_catalog_is_lazy_and_demo_uses_no_service() {
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap();
		let mut loader = Loader::default();
		let mut state = client_core::State::default();
		let ctx = egui::Context::default();
		loader.sync(&mut state, &runtime, &ctx, true);
		assert!(state.game_mentions.status == Status::Idle);
		assert!(loader.pending.is_none());
		state.game_mentions.request();
		loader.sync(&mut state, &runtime, &ctx, true);
		assert!(state.game_mentions.status == Status::Ready);
		assert_eq!(state.game_mentions.games().len(), 10);
		assert!(state.local_game_activity.is_none());
		assert!(loader.pending.is_none());
		let revision = state.game_mentions.revision;
		loader.sync(&mut state, &runtime, &ctx, true);
		assert_eq!(state.game_mentions.revision, revision);
	}
}

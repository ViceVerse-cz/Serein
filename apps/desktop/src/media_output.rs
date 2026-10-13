//! Independent media routing. Device opening and watched-stream playout stay off rendering.
use cpal::traits::HostTrait;
use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
	mpsc::{self, SyncSender},
};
use std::time::Duration;

pub fn choose(host: &cpal::Host, selected: Option<&str>) -> Result<cpal::Device, &'static str> {
	select(
		selected,
		|selected| {
			let id = selected
				.parse()
				.map_err(|_| "Invalid media output device")?;
			Ok(host.device_by_id(&id))
		},
		|| host.default_output_device(),
	)
}
fn select<T>(
	selected: Option<&str>,
	lookup: impl FnOnce(&str) -> Result<Option<T>, &'static str>,
	default: impl FnOnce() -> Option<T>,
) -> Result<T, &'static str> {
	if let Some(selected) = selected {
		if selected.len() > 1024 {
			return Err("Invalid media output device");
		}
		return lookup(selected)?.ok_or("Selected media output is unavailable");
	}
	default().ok_or("No audio output device")
}

pub struct StreamRoute {
	pub sender: SyncSender<discord_voice::Frame>,
	selected: tokio::sync::watch::Sender<Option<String>>,
	controls: crate::video::output::Controls,
	finished: Arc<AtomicBool>,
}
impl StreamRoute {
	pub fn new(fallback: SyncSender<discord_voice::Frame>) -> Result<Self, &'static str> {
		let (sender, receive) = mpsc::sync_channel::<discord_voice::Frame>(8);
		let (selected, mut selection) = tokio::sync::watch::channel::<Option<String>>(None);
		let controls = crate::video::output::Controls {
			cancelled: Arc::new(AtomicBool::new(false)),
			paused: Arc::new(AtomicBool::new(false)),
			seek: Arc::new(AtomicU64::new(u64::MAX)),
			volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
			position: Arc::new(AtomicU64::new(0)),
			eof: Arc::new(AtomicBool::new(false)),
			failed: Arc::new(AtomicBool::new(false)),
		};
		let gate = controls.clone();
		let finished = Arc::new(AtomicBool::new(false));
		let completed = finished.clone();
		std::thread::Builder::new()
			.name("serein-stream-output".into())
			.spawn(move || {
				let mut current = None;
				let mut output = None;
				let mut paused = false;
				while !gate.cancelled.load(Ordering::Acquire) {
					let preference = selection.borrow_and_update().clone();
					let muted = gate.paused.load(Ordering::Acquire);
					let changed_device = current != preference;
					if changed_device || paused != muted {
						output = None;
						if changed_device {
							gate.failed.store(false, Ordering::Release);
						}
						current = preference;
						paused = muted;
						for _ in 0..8 {
							if receive.try_recv().is_err() {
								break;
							}
						}
					}
					let Ok(mut frame) = receive.recv_timeout(Duration::from_millis(20)) else {
						continue;
					};
					// Prefer current audio over a network burst; never replay a backlog after mute.
					for _ in 0..8 {
						match receive.try_recv() {
							Ok(newer) => frame = newer,
							Err(_) => break,
						}
					}
					// Selection can change while recv_timeout or a native open is pending.
					// Discard this packet instead of sending it through the previous route.
					if *selection.borrow() != current {
						continue;
					}
					if gate.paused.load(Ordering::Acquire) || gate.cancelled.load(Ordering::Acquire)
					{
						continue;
					}
					if current.is_none() {
						let _ = fallback.try_send(frame);
						continue;
					}
					if gate.failed.load(Ordering::Acquire) {
						continue;
					}
					if output.is_none() {
						match crate::video::output::open(
							48_000,
							current.as_deref(),
							3 * 960,
							gate.clone(),
						) {
							Ok(stream) => output = Some(stream),
							Err(_) => {
								gate.failed.store(true, Ordering::Release);
								continue;
							}
						}
					}
					if *selection.borrow() != current {
						output = None;
						continue;
					}
					let producer = &mut output.as_mut().expect("opened output").producer;
					if producer.slots() >= frame.len() {
						for sample in frame {
							let _ = producer.push([sample, sample]);
						}
					}
				}
				drop(output);
				completed.store(true, Ordering::Release);
			})
			.map_err(|_| "Could not start stream audio worker")?;
		Ok(Self {
			sender,
			selected,
			controls,
			finished,
		})
	}
	pub fn configure(&self, selected: &Option<String>, volume: u16, deafened: bool) {
		self.selected.send_if_modified(|current| {
			if current == selected {
				false
			} else {
				current.clone_from(selected);
				true
			}
		});
		self.controls.volume.store(
			(f32::from(volume.min(200)) / 100.0).to_bits(),
			Ordering::Release,
		);
		self.controls
			.paused
			.store(deafened || volume == 0, Ordering::Release);
	}
	pub fn failed(&self) -> bool {
		self.controls.failed.load(Ordering::Acquire)
	}
	pub fn cancel(&self) {
		self.controls.cancelled.store(true, Ordering::Release);
	}
	pub fn finished(&self) -> bool {
		self.finished.load(Ordering::Acquire)
	}
}
impl Drop for StreamRoute {
	fn drop(&mut self) {
		self.cancel();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn explicit_media_selection_never_falls_back_to_another_device() {
		assert_eq!(
			select::<u8>(
				Some("missing"),
				|_| Ok(None),
				|| panic!("unexpected default routing")
			),
			Err("Selected media output is unavailable")
		);
		assert_eq!(
			select(
				Some("selected"),
				|_| Ok(Some(1u8)),
				|| panic!("unexpected default routing")
			),
			Ok(1)
		);
		assert_eq!(
			select(None, |_| panic!("unexpected lookup"), || Some(2u8)),
			Ok(2)
		);
		assert!(
			select::<u8>(
				Some(&"x".repeat(1025)),
				|_| panic!("unbounded ID reached lookup"),
				|| None
			)
			.is_err()
		);
	}
	#[test]
	fn default_stream_route_forwards_without_opening_devices_and_discards_muted_audio() {
		let (fallback, receive) = mpsc::sync_channel(8);
		let route = StreamRoute::new(fallback).unwrap();
		route.configure(&None, 100, false);
		route.sender.send([0.25; 960]).unwrap();
		assert_eq!(
			receive.recv_timeout(Duration::from_secs(1)).unwrap(),
			[0.25; 960]
		);
		route.configure(&None, 100, true);
		route.sender.send([0.75; 960]).unwrap();
		assert!(receive.recv_timeout(Duration::from_millis(80)).is_err());
		route.configure(&None, 100, false);
		let mut resumed = None;
		for _ in 0..25 {
			let _ = route.sender.try_send([-0.25; 960]);
			if let Ok(frame) = receive.recv_timeout(Duration::from_millis(40)) {
				resumed = Some(frame);
				break;
			}
		}
		assert_eq!(resumed, Some([-0.25; 960]));
		let sender = route.sender.clone();
		route.cancel();
		for _ in 0..50 {
			if route.finished() {
				break;
			}
			std::thread::sleep(Duration::from_millis(20));
		}
		assert!(route.finished());
		drop(route);
		for _ in 0..50 {
			if matches!(
				sender.try_send([0.0; 960]),
				Err(mpsc::TrySendError::Disconnected(_))
			) {
				return;
			}
			std::thread::sleep(Duration::from_millis(20));
		}
		panic!("cancelled worker retained its input channel");
	}
}

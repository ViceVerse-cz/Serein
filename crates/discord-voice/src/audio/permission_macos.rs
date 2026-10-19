//! macOS authorization before CoreAudio input. No audio is captured here.
#![allow(unsafe_code)]

use super::Gate;
use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use std::{
	sync::{atomic::Ordering, mpsc},
	time::{Duration, Instant},
};

const DENIED: &str = "Microphone access denied. Allow Serein (or your terminal for cargo run) in System Settings > Privacy & Security > Microphone, then rejoin.";

pub(super) fn authorize(gate: &Gate, revision: u64) -> Result<(), &'static str> {
	// SAFETY: AVMediaTypeAudio is the framework's valid process-lifetime media constant.
	// These class methods are callable off the main thread. AVFoundation copies the
	// completion block; it owns only a bounded sender and never accesses UI or devices.
	let result = unsafe {
		let media = AVMediaTypeAudio.ok_or("macOS microphone authorization is unavailable")?;
		match AVCaptureDevice::authorizationStatusForMediaType(media) {
			AVAuthorizationStatus::Authorized => return Ok(()),
			AVAuthorizationStatus::NotDetermined => {
				let (send, receive) = mpsc::sync_channel(1);
				let completion = RcBlock::new(move |granted: Bool| {
					let _ = send.try_send(granted.as_bool());
				});
				AVCaptureDevice::requestAccessForMediaType_completionHandler(media, &completion);
				receive
			}
			_ => return Err(DENIED),
		}
	};
	wait_for_authorization(gate, revision, result)
}

fn wait_for_authorization(
	gate: &Gate,
	revision: u64,
	result: mpsc::Receiver<bool>,
) -> Result<(), &'static str> {
	wait_for_authorization_with(gate, revision, |delay| result.recv_timeout(delay))
}

fn wait_for_authorization_with(
	gate: &Gate,
	revision: u64,
	mut receive: impl FnMut(Duration) -> Result<bool, mpsc::RecvTimeoutError>,
) -> Result<(), &'static str> {
	let deadline = Instant::now() + Duration::from_secs(20);
	loop {
		if gate.stopped.load(Ordering::Acquire)
			|| !gate.ready.load(Ordering::Acquire)
			|| gate.revision.load(Ordering::Acquire) != revision
		{
			return Err("Call changed while waiting for microphone permission");
		}
		match receive(Duration::from_millis(100)) {
			Ok(true) => {
				if gate.stopped.load(Ordering::Acquire)
					|| !gate.ready.load(Ordering::Acquire)
					|| gate.revision.load(Ordering::Acquire) != revision
				{
					return Err("Call changed while waiting for microphone permission");
				}
				return Ok(());
			}
			Ok(false) => return Err(DENIED),
			Err(mpsc::RecvTimeoutError::Disconnected) => {
				return Err("macOS microphone permission request did not complete");
			}
			Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
				return Err(
					"Microphone permission timed out. Respond to the macOS prompt, then rejoin.",
				);
			}
			Err(mpsc::RecvTimeoutError::Timeout) => {}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn permission_granted_during_cancellation_rejects_the_obsolete_open() {
		for change in 0..3 {
			let gate = Gate::default();
			gate.ready.store(true, Ordering::Release);
			let revision = gate.revision.load(Ordering::Acquire);
			let result = wait_for_authorization_with(&gate, revision, |_| {
				// Inject a call change after the pre-wait check, before the grant arrives.
				match change {
					0 => gate.stopped.store(true, Ordering::Release),
					1 => gate.ready.store(false, Ordering::Release),
					_ => gate.revision.store(revision + 1, Ordering::Release),
				}
				Ok(true)
			});
			assert!(result.is_err());
		}
	}

	#[test]
	fn permission_result_and_cancellation_never_open_devices() {
		let gate = Gate::default();
		gate.ready.store(true, Ordering::Release);
		let revision = gate.revision.load(Ordering::Acquire);
		for allowed in [true, false] {
			let (send, receive) = mpsc::sync_channel(1);
			send.try_send(allowed).unwrap();
			assert_eq!(
				wait_for_authorization(&gate, revision, receive).is_ok(),
				allowed
			);
		}
		let (send, receive) = mpsc::sync_channel(1);
		send.try_send(true).unwrap();
		gate.stopped.store(true, Ordering::Release);
		assert!(wait_for_authorization(&gate, revision, receive).is_err());
	}
}

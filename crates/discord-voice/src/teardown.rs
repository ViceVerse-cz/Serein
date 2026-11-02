//! Media threads that may still be closing a hardware encoder after their owner was dropped.
//!
//! Dropping a screen or camera worker only asks its thread to stop, so quitting during a share
//! returns from `main` while GStreamer is still destroying the encoder. NVIDIA's libnvcuvid
//! registers an exit handler that crashes or deadlocks when `exit()` runs during that teardown;
//! a deadlocked process then keeps the single-instance socket and swallows later launches.
use std::{
	sync::{Condvar, Mutex},
	time::{Duration, Instant},
};

static RUNNING: Mutex<usize> = Mutex::new(0);
static ENDED: Condvar = Condvar::new();

/// Counts one media thread. Create it before spawning, so a stop issued right after the spawn
/// is still waited for, and drop it after the thread has released its pipeline.
pub(crate) struct Thread(());

impl Thread {
	pub(crate) fn start() -> Self {
		if let Ok(mut running) = RUNNING.lock() {
			*running += 1;
		}
		Self(())
	}
}

impl Drop for Thread {
	fn drop(&mut self) {
		if let Ok(mut running) = RUNNING.lock() {
			*running -= 1;
		}
		ENDED.notify_all();
	}
}

/// Waits up to `timeout` for stopped screen and camera threads to release their encoders.
/// Call after their owners are dropped and before the process exits. `false` on timeout.
pub fn wait_for_media_threads(timeout: Duration) -> bool {
	let deadline = Instant::now() + timeout;
	let Ok(mut running) = RUNNING.lock() else {
		return false;
	};
	while *running > 0 {
		let left = deadline.saturating_duration_since(Instant::now());
		if left.is_zero() {
			return false;
		}
		running = match ENDED.wait_timeout(running, left) {
			Ok((running, _)) => running,
			Err(_) => return false,
		};
	}
	true
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn waits_for_a_stopped_thread_counted_before_it_was_spawned() {
		let thread = Thread::start();
		let (release, released) = std::sync::mpsc::channel::<()>();
		let worker = std::thread::spawn(move || {
			let _thread = thread;
			let _ = released.recv();
		});
		assert!(!wait_for_media_threads(Duration::ZERO));
		drop(release);
		assert!(wait_for_media_threads(Duration::from_secs(10)));
		worker.join().unwrap();
	}
}

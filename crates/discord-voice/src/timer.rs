//! Media threads with a precise OS timer.
//!
//! Windows rounds every timed wait up to its 15.6 ms default tick, which turns 20 ms Opus
//! ticks into jitter and 1-2 ms video pacing sleeps into a frame-rate cap. Like browsers
//! during calls, raise the resolution to 1 ms only while a media loop is running.
//!
//! Windows 11 also drops that request, and may run the process on efficiency cores, once
//! every Serein window is minimized or covered (a streamer's game). Media loops opt the
//! process out of that power throttling until the last one ends.
use std::future::Future;

pub(crate) struct Resolution {
	#[cfg(target_os = "windows")]
	raised: bool,
}

#[cfg(target_os = "windows")]
static THROTTLING_OPT_OUTS: std::sync::Mutex<usize> = std::sync::Mutex::new(0);

/// Opts the process out of (`true`) or back into (`false`) Windows power throttling.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn exempt_from_power_throttling(exempt: bool) {
	use windows::Win32::System::Threading::{
		GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
		PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
		PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, SetProcessInformation,
	};
	// A zero control mask hands both policies back to the system.
	let state = PROCESS_POWER_THROTTLING_STATE {
		Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
		ControlMask: if exempt {
			PROCESS_POWER_THROTTLING_EXECUTION_SPEED
				| PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION
		} else {
			0
		},
		StateMask: 0,
	};
	// SAFETY: The pseudo-handle needs no closing; the struct outlives the call and its size
	// is passed exactly. Older Windows rejects unknown flags, which only keeps the default.
	let _ = unsafe {
		SetProcessInformation(
			GetCurrentProcess(),
			ProcessPowerThrottling,
			(&raw const state).cast(),
			size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
		)
	};
}

impl Resolution {
	pub(crate) fn acquire() -> Self {
		#[cfg(target_os = "windows")]
		if let Ok(mut count) = THROTTLING_OPT_OUTS.lock() {
			if *count == 0 {
				exempt_from_power_throttling(true);
			}
			*count += 1;
		}
		#[cfg(target_os = "windows")]
		#[allow(unsafe_code)]
		// SAFETY: A process-scoped request, balanced by timeEndPeriod in Drop on success.
		let raised = unsafe { windows::Win32::Media::timeBeginPeriod(1) } == 0;
		Self {
			#[cfg(target_os = "windows")]
			raised,
		}
	}
}

impl Drop for Resolution {
	fn drop(&mut self) {
		#[cfg(target_os = "windows")]
		if self.raised {
			#[allow(unsafe_code)]
			// SAFETY: Balances the successful timeBeginPeriod(1) in acquire.
			unsafe {
				windows::Win32::Media::timeEndPeriod(1);
			}
		}
		#[cfg(target_os = "windows")]
		if let Ok(mut count) = THROTTLING_OPT_OUTS.lock() {
			*count = count.saturating_sub(1);
			if *count == 0 {
				exempt_from_power_throttling(false);
			}
		}
	}
}

/// Run one media loop on its own thread and single-threaded runtime, so gateway, image and
/// UI tasks on the shared application runtime cannot delay audio ticks or paced video.
/// Dropping the returned future (a task abort) stops the loop at its next await.
pub(crate) async fn isolated<F, Fut>(name: &'static str, start: F) -> Result<(), &'static str>
where
	F: FnOnce() -> Fut + Send + 'static,
	Fut: Future<Output = Result<(), &'static str>>,
{
	let (mut done, result) = tokio::sync::oneshot::channel();
	std::thread::Builder::new()
		.name(name.into())
		.spawn(move || {
			let _resolution = Resolution::acquire();
			let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
				.enable_all()
				.build()
			else {
				let _ = done.send(Err("Media runtime could not start"));
				return;
			};
			let outcome = runtime.block_on(async {
				let work = start();
				tokio::pin!(work);
				tokio::select! {
					outcome = &mut work => Some(outcome),
					() = done.closed() => None,
				}
			});
			if let Some(outcome) = outcome {
				let _ = done.send(outcome);
			}
		})
		.map_err(|_| "Media thread could not start")?;
	result
		.await
		.unwrap_or(Err("Media thread stopped unexpectedly"))
}

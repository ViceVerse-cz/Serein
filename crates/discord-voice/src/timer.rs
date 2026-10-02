//! Media threads with a precise OS timer.
//!
//! Windows rounds every timed wait up to its 15.6 ms default tick, which turns 20 ms Opus
//! ticks into jitter and 1-2 ms video pacing sleeps into a frame-rate cap. Like browsers
//! during calls, raise the resolution to 1 ms only while a media loop is running.
use std::future::Future;

struct Resolution {
	#[cfg(target_os = "windows")]
	raised: bool,
}

impl Resolution {
	fn acquire() -> Self {
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

//! glibc allocator settings for a long-running client that decodes media on many threads.
//! Other platforms and C libraries keep their defaults.

/// How often freed arena memory is handed back while the client runs.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const TRIM_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Wakes the background trim thread on demand (after playback or idle artwork eviction) so
/// `malloc_trim(0)` never stalls the UI or media worker thread.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
type TrimSignal = (std::sync::Mutex<bool>, std::sync::Condvar);

#[cfg(all(target_os = "linux", target_env = "gnu"))]
static TRIM_SIGNAL: std::sync::OnceLock<std::sync::Arc<TrimSignal>> = std::sync::OnceLock::new();

/// Applied once at startup, before worker threads exist. Each GStreamer streaming thread
/// otherwise gets its own arena, and glibc's dynamic mmap threshold moves multi-megabyte
/// video frames and decoded images into those arenas once one is freed; every arena then keeps
/// tens of MB of freed frames resident. A fixed 256 KiB threshold maps video frames and medium
/// decoded images/thumbnails separately, so freeing them returns the memory immediately, and
/// four arenas bound what the remaining heaps can keep.
///
/// Smaller allocations still coalesce into free holes that glibc only returns from an arena's
/// top, so a background thread trims periodically and when signalled after large evictions.
/// Transparent huge pages are disabled for the process: with the kernel's `always` mode a single
/// live chunk keeps its whole 2 MiB page resident.
#[allow(unsafe_code)]
pub fn tune_allocator() {
	#[cfg(all(target_os = "linux", target_env = "gnu"))]
	{
		static INIT: std::sync::Once = std::sync::Once::new();
		INIT.call_once(|| {
			const M_MMAP_THRESHOLD: i32 = -3;
			const M_ARENA_MAX: i32 = -8;
			const PR_SET_THP_DISABLE: i32 = 41;
			unsafe extern "C" {
				fn mallopt(param: i32, value: i32) -> i32;
				fn prctl(option: i32, ...) -> i32;
			}
			// SAFETY: mallopt only changes allocator tuning and is called before other threads;
			// PR_SET_THP_DISABLE only sets a process flag and takes no pointers.
			unsafe {
				mallopt(M_MMAP_THRESHOLD, 256 * 1024);
				mallopt(M_ARENA_MAX, 4);
				prctl(PR_SET_THP_DISABLE, 1usize, 0usize, 0usize, 0usize);
			}
			let signal =
				std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
			let worker_signal = std::sync::Arc::clone(&signal);
			if std::thread::Builder::new()
				.name("serein-memory".into())
				.spawn(move || {
					let (lock, condvar) = &*worker_signal;
					loop {
						let requested = lock.lock().unwrap_or_else(|e| e.into_inner());
						let (mut requested, _) = condvar
							.wait_timeout_while(requested, TRIM_INTERVAL, |r| !*r)
							.unwrap_or_else(|e| e.into_inner());
						*requested = false;
						drop(requested);
						trim_now();
					}
				})
				.is_ok()
			{
				let _ = TRIM_SIGNAL.set(signal);
			}
		});
	}
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[allow(unsafe_code)]
fn trim_now() {
	unsafe extern "C" {
		fn malloc_trim(pad: usize) -> i32;
	}
	// SAFETY: glibc's malloc_trim is thread-safe and only releases free pages.
	unsafe {
		malloc_trim(0);
	}
}

/// Returns freed allocator memory to the system, such as once playback, a poster batch, or
/// idle artwork eviction ends. Elsewhere this does nothing.
pub fn release_freed_memory() {
	#[cfg(all(target_os = "linux", target_env = "gnu"))]
	{
		if let Some(signal) = TRIM_SIGNAL.get() {
			let (lock, condvar) = &**signal;
			let mut requested = lock.lock().unwrap_or_else(|e| e.into_inner());
			*requested = true;
			condvar.notify_one();
		} else {
			trim_now();
		}
	}
}

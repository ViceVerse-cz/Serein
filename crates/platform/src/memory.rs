//! glibc allocator settings for a long-running client that decodes media on many threads.
//! Other platforms and C libraries keep their defaults.

/// How often freed arena memory is handed back while the client runs.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const TRIM_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Applied once at startup, before worker threads exist. Each GStreamer streaming thread
/// otherwise gets its own arena, and glibc's dynamic mmap threshold moves multi-megabyte
/// video frames into those arenas once one is freed; every arena then keeps tens of MB of
/// freed frames resident. A fixed 1 MiB threshold maps such buffers separately, so freeing
/// them returns the memory, and four arenas bound what the remaining heaps can keep.
///
/// Allocations just under that threshold (decoded images, network and decoder buffers) still
/// coalesce into multi-megabyte free holes that glibc only returns from an arena's top, so a
/// background thread trims periodically. Transparent huge pages are disabled for the process:
/// with the kernel's `always` mode a single live chunk keeps its whole 2 MiB page resident.
#[allow(unsafe_code)]
pub fn tune_allocator() {
	#[cfg(all(target_os = "linux", target_env = "gnu"))]
	{
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
			mallopt(M_MMAP_THRESHOLD, 1024 * 1024);
			mallopt(M_ARENA_MAX, 4);
			prctl(PR_SET_THP_DISABLE, 1usize, 0usize, 0usize, 0usize);
		}
		let _ = std::thread::Builder::new()
			.name("serein-memory".into())
			.spawn(|| {
				loop {
					std::thread::sleep(TRIM_INTERVAL);
					release_freed_memory();
				}
			});
	}
}

/// Returns freed allocator memory to the system, such as once playback or a poster batch ends.
/// Elsewhere this does nothing.
#[allow(unsafe_code)]
pub fn release_freed_memory() {
	#[cfg(all(target_os = "linux", target_env = "gnu"))]
	{
		unsafe extern "C" {
			fn malloc_trim(pad: usize) -> i32;
		}
		// SAFETY: glibc's malloc_trim is thread-safe and only releases free pages.
		unsafe {
			malloc_trim(0);
		}
	}
}

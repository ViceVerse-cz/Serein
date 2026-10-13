//! glibc allocator settings for a long-running client that decodes media on many threads.
//! Other platforms and C libraries keep their defaults.

/// Applied once at startup, before worker threads exist. Each GStreamer streaming thread
/// otherwise gets its own arena, and glibc's dynamic mmap threshold moves multi-megabyte
/// video frames into those arenas once one is freed; every arena then keeps tens of MB of
/// freed frames resident. A fixed 1 MiB threshold maps such buffers separately, so freeing
/// them returns the memory, and four arenas bound what the remaining heaps can keep.
#[allow(unsafe_code)]
pub fn tune_allocator() {
	#[cfg(all(target_os = "linux", target_env = "gnu"))]
	{
		const M_MMAP_THRESHOLD: i32 = -3;
		const M_ARENA_MAX: i32 = -8;
		unsafe extern "C" {
			fn mallopt(param: i32, value: i32) -> i32;
		}
		// SAFETY: mallopt only changes allocator tuning and is called before other threads.
		unsafe {
			mallopt(M_MMAP_THRESHOLD, 1024 * 1024);
			mallopt(M_ARENA_MAX, 4);
		}
	}
}

/// Returns freed decoder memory to the system once playback or a poster batch ends.
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

//! Scheduling hints for background worker threads.

/// Lowers the calling thread's OS scheduling priority, so background work such as emoji
/// rasterisation yields to the UI thread on machines with few cores. Best effort: failures and
/// unsupported targets leave the priority unchanged.
pub fn lower_priority() {
	imp::lower_priority();
}

#[cfg(target_os = "windows")]
mod imp {
	#[allow(unsafe_code)]
	pub(super) fn lower_priority() {
		use windows::Win32::System::Threading::{
			GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_LOWEST,
		};
		// SAFETY: the pseudo-handle names the calling thread and needs no cleanup.
		let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST) };
	}
}

#[cfg(target_os = "linux")]
mod imp {
	#[allow(unsafe_code)]
	pub(super) fn lower_priority() {
		// On Linux, PRIO_PROCESS with id 0 applies to the calling thread only.
		// SAFETY: setpriority reads only its integer arguments.
		let _ = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 10) };
	}
}

#[cfg(target_os = "macos")]
mod imp {
	#[allow(unsafe_code)]
	pub(super) fn lower_priority() {
		// SAFETY: sets the calling thread's own QoS class; no pointers are passed.
		let _ =
			unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0) };
	}
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
mod imp {
	pub(super) fn lower_priority() {}
}

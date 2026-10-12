//! Audited process-resource boundary for the internal PDF helper. This is resource
//! isolation, not an OS sandbox. No UI process ever calls `constrain_helper`.
#![allow(unsafe_code)]
pub const MEMORY_BYTES: u64 = 768 * 1024 * 1024;
pub const CPU_SECONDS: u64 = 8;
#[derive(Debug)]
pub struct Unavailable;
#[cfg(not(target_os = "windows"))]
pub struct Limits;
#[cfg(target_os = "windows")]
pub struct Limits(windows::Win32::Foundation::HANDLE);
#[cfg(target_os = "windows")]
impl Drop for Limits {
	fn drop(&mut self) {
		// SAFETY: this RAII guard owns the job HANDLE returned by CreateJobObjectW.
		unsafe {
			let _ = windows::Win32::Foundation::CloseHandle(self.0);
		}
	}
}
/// Fail closed if the platform cannot establish the limits. The helper also owns
/// an independent wall watchdog, so abrupt desktop exit cannot leave it running.
pub fn constrain_helper() -> Result<Limits, Unavailable> {
	#[cfg(any(target_os = "linux", target_os = "macos"))]
	{
		#[cfg(all(target_os = "linux", not(target_env = "musl")))]
		type Resource = libc::__rlimit_resource_t;
		#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "musl")))]
		type Resource = libc::c_int;
		// SAFETY: fixed initialized rlimit values, no retained pointers. Applied only in
		// the fresh helper before it reads input; never raises an inherited hard limit.
		fn limit(resource: Resource, maximum: u64) -> Result<(), Unavailable> {
			unsafe {
				let mut current = libc::rlimit {
					rlim_cur: 0,
					rlim_max: 0,
				};
				if libc::getrlimit(resource, &mut current) != 0 {
					return Err(Unavailable);
				}
				let ceiling = (maximum as libc::rlim_t).min(current.rlim_max);
				let value = libc::rlimit {
					rlim_cur: ceiling,
					rlim_max: ceiling,
				};
				if libc::setrlimit(resource, &value) != 0 {
					return Err(Unavailable);
				}
			}
			Ok(())
		}
		limit(libc::RLIMIT_CORE, 0)?;
		limit(libc::RLIMIT_CPU, CPU_SECONDS)?;
		#[cfg(target_os = "linux")]
		{
			limit(libc::RLIMIT_AS, MEMORY_BYTES)?;
			limit(libc::RLIMIT_DATA, MEMORY_BYTES)?;
			// SAFETY: the documented integer prctl operation carries no pointers. Kill
			// the helper if its desktop parent dies; detect the pre-prctl orphan race.
			unsafe {
				if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 || libc::getppid() == 1 {
					return Err(Unavailable);
				}
			}
		}
		Ok(Limits)
	}
	#[cfg(target_os = "windows")]
	{
		use windows::Win32::System::{JobObjects::*, Threading::GetCurrentProcess};
		// SAFETY: one owned job HANDLE and initialized structure valid for the call;
		// no pointer escapes. The current helper is the sole assigned process.
		unsafe {
			let job = Limits(CreateJobObjectW(None, None).map_err(|_| Unavailable)?);
			let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
			info.BasicLimitInformation.LimitFlags =
				JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_PROCESS_TIME;
			info.BasicLimitInformation.PerProcessUserTimeLimit = (CPU_SECONDS * 10_000_000) as i64;
			info.ProcessMemoryLimit = MEMORY_BYTES as usize;
			SetInformationJobObject(
				job.0,
				JobObjectExtendedLimitInformation,
				&info as *const _ as _,
				std::mem::size_of_val(&info) as u32,
			)
			.map_err(|_| Unavailable)?;
			AssignProcessToJobObject(job.0, GetCurrentProcess()).map_err(|_| Unavailable)?;
			Ok(job)
		}
	}
	#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
	{
		Err(Unavailable)
	}
}
/// macOS has no reliable RLIMIT_AS; the parent samples resident bytes. This can
/// overshoot between polls and is not a hard allocation limit. Linux/Windows
/// enforce allocations in the helper itself instead.
pub fn memory_exceeded(pid: u32) -> Result<bool, Unavailable> {
	#[cfg(target_os = "macos")]
	{
		let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::zeroed();
		// SAFETY: libproc writes at most the supplied size into a valid POD buffer;
		// fields are read only after a complete successful write.
		let read = unsafe {
			libc::proc_pidinfo(
				pid as i32,
				libc::PROC_PIDTASKINFO,
				0,
				info.as_mut_ptr() as _,
				std::mem::size_of::<libc::proc_taskinfo>() as i32,
			)
		};
		if read != std::mem::size_of::<libc::proc_taskinfo>() as i32 {
			return Err(Unavailable);
		}
		// SAFETY: proc_pidinfo returned the complete initialized POD structure above.
		Ok(unsafe { info.assume_init() }.pti_resident_size > MEMORY_BYTES)
	}
	#[cfg(not(target_os = "macos"))]
	{
		let _ = pid;
		Ok(false)
	}
}

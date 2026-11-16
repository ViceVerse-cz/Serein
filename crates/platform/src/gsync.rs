//! NVIDIA G-SYNC detection through the graphics driver's NVAPI library.
//!
//! A G-SYNC display follows the focused window's presents, so drawing only on change swings it
//! between its refresh limits and many panels flicker. The desktop app offers its steady refresh
//! rate once when a display has G-SYNC on. FreeSync and other vendors' variable refresh rate
//! are not detected.

/// Reports from a worker thread whether an attached display has G-SYNC switched on. Other
/// platforms never report; a missing or refusing NVIDIA driver reports `false`.
pub fn detect(report: impl FnOnce(bool) + Send + 'static) {
	#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
	{
		let _ = std::thread::Builder::new()
			.name("serein-gsync".into())
			.spawn(move || report(nvapi::enabled()));
	}
	#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
	drop(report);
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[allow(unsafe_code)]
mod nvapi {
	use std::ffi::{c_char, c_void};
	use windows::{
		Win32::{
			Foundation::FreeLibrary,
			Graphics::Gdi::{
				DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICEW, EnumDisplayDevicesW,
			},
			System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
		},
		core::{s, w},
	};

	// Interface ids and the structure layout come from NVIDIA's published NVAPI SDK
	// (`nvapi_interface.h` and `nvapi.h`).
	const INITIALIZE: u32 = 0x0150_E828;
	const UNLOAD: u32 = 0xD22B_DD7E;
	const DISPLAY_ID_BY_NAME: u32 = 0xAE45_7190;
	const VRR_INFO: u32 = 0xDF8F_DA57;
	/// `bIsVRREnabled`, `bIsVRRRequested` and `bIsDisplayInVRRMode`. `bIsVRRPossible` alone is a
	/// capable display with G-SYNC switched off.
	const VRR_ON: u32 = 0b1_0101;
	/// Display adapters asked at most; Windows numbers every output of every GPU.
	const MAX_ADAPTERS: u32 = 32;

	/// `NV_GET_VRR_INFO_V1`.
	#[repr(C)]
	struct VrrInfo {
		version: u32,
		flags: u32,
		reserved: [u32; 4],
	}

	type Query = unsafe extern "C" fn(u32) -> *const c_void;
	type Call = unsafe extern "C" fn() -> i32;
	type DisplayId = unsafe extern "C" fn(*const c_char, *mut u32) -> i32;
	type GetVrrInfo = unsafe extern "C" fn(u32, *mut VrrInfo) -> i32;

	pub fn enabled() -> bool {
		// Only the driver's own copy in System32 is trusted; the search path is never consulted.
		// SAFETY: the name is a NUL-terminated literal and no file handle is passed.
		let Ok(library) =
			(unsafe { LoadLibraryExW(w!("nvapi64.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) })
		else {
			return false;
		};
		// SAFETY: `library` is the module loaded above and the name is a NUL-terminated literal.
		let found = unsafe { GetProcAddress(library, s!("nvapi_QueryInterface")) }
			// SAFETY: NVAPI documents this export as `void *nvapi_QueryInterface(NvU32)`.
			.map(|query| unsafe { std::mem::transmute::<_, Query>(query) })
			.is_some_and(any_display);
		// SAFETY: every NVAPI call has returned and `any_display` balanced its initialize with
		// an unload, so nothing of ours still runs in the module.
		let _ = unsafe { FreeLibrary(library) };
		found
	}

	fn any_display(query: Query) -> bool {
		// SAFETY: `query` is the library's lookup export; it returns null for an unknown id.
		let interface = |id| Some(unsafe { query(id) }).filter(|pointer| !pointer.is_null());
		let (Some(initialize), Some(unload), Some(display_id), Some(vrr_info)) = (
			interface(INITIALIZE),
			interface(UNLOAD),
			interface(DISPLAY_ID_BY_NAME),
			interface(VRR_INFO),
		) else {
			return false;
		};
		// SAFETY: each pointer came from the id of the function whose published signature the
		// type restates; NVAPI functions are `__cdecl` and return an `NvAPI_Status`.
		let (initialize, unload, display_id, vrr_info) = unsafe {
			(
				std::mem::transmute::<*const c_void, Call>(initialize),
				std::mem::transmute::<*const c_void, Call>(unload),
				std::mem::transmute::<*const c_void, DisplayId>(display_id),
				std::mem::transmute::<*const c_void, GetVrrInfo>(vrr_info),
			)
		};
		// SAFETY: takes no arguments; a failure means no NVIDIA GPU drives a display.
		if unsafe { initialize() } != 0 {
			return false;
		}
		let mut device = DISPLAY_DEVICEW {
			cb: size_of::<DISPLAY_DEVICEW>() as u32,
			..Default::default()
		};
		let mut found = false;
		for index in 0..MAX_ADAPTERS {
			// SAFETY: `device` is a writable structure whose `cb` holds its size.
			if !unsafe { EnumDisplayDevicesW(None, index, &mut device, 0) }.as_bool() {
				break;
			}
			if !device
				.StateFlags
				.contains(DISPLAY_DEVICE_ATTACHED_TO_DESKTOP)
			{
				continue;
			}
			// `\\.\DISPLAY1`: ASCII in a 32-unit field. The spare byte keeps it NUL-terminated.
			let mut name = [0u8; 33];
			for (byte, unit) in name.iter_mut().zip(device.DeviceName) {
				*byte = u8::try_from(unit).unwrap_or(b'?');
			}
			let mut id = 0;
			let mut info = VrrInfo {
				version: size_of::<VrrInfo>() as u32 | 1 << 16,
				flags: 0,
				reserved: [0; 4],
			};
			// SAFETY: `name` is NUL-terminated, `id` and `info` are writable, and `info` carries
			// the size and version NVAPI checks before writing to it.
			found = unsafe {
				display_id(name.as_ptr().cast(), &mut id) == 0 && vrr_info(id, &mut info) == 0
			} && info.flags & VRR_ON != 0;
			if found {
				break;
			}
		}
		// SAFETY: balances the successful initialize above.
		unsafe { unload() };
		found
	}
}

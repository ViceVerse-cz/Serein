//! Trusted, owner-installed native DSP. See docs/native-dsp.md for the C contract.
// Like permission_macos.rs, this is a narrow FFI carve-out from the crate's deny lint.
#![allow(unsafe_code)]

use libloading::Library;
use std::{ffi::c_void, path::Path, ptr::NonNull};

pub(super) const ERROR: &str = "Microphone processing failed: native DSP plugin unavailable";

// These signatures are resolved separately; no Rust-owned value crosses the ABI.
#[repr(C)]
struct Api {
	create: unsafe extern "C" fn(u32) -> *mut c_void,
	process: unsafe extern "C" fn(*mut c_void, *mut i16, usize) -> i32,
	destroy: unsafe extern "C" fn(*mut c_void),
}

pub(super) struct DspPlugin {
	api: Api,
	ctx: NonNull<c_void>,
	// Kept alive through destroy, including when session creation fails on reset.
	_library: Library,
}

impl DspPlugin {
	pub(super) fn load() -> Result<Self, &'static str> {
		let root = local_store::data_dir().map_err(|_| ERROR)?;
		Self::load_from(
			&root
				.join("plugins")
				.join(libloading::library_filename("serein_dsp")),
		)
	}

	fn load_from(path: &Path) -> Result<Self, &'static str> {
		// Never search the working directory or PATH for executable plugin code.
		if !path.is_absolute() {
			return Err(ERROR);
		}
		// SAFETY: Installation is an explicit native-code trust decision. The library must
		// implement the documented ABI, including non-unwinding initializers/functions.
		// Resolve every symbol before allocating a session; retain the library until Drop.
		unsafe {
			let library = Library::new(path).map_err(|_| ERROR)?;
			let api = Api {
				create: *library.get(b"serein_dsp_create\0").map_err(|_| ERROR)?,
				process: *library.get(b"serein_dsp_process\0").map_err(|_| ERROR)?,
				destroy: *library.get(b"serein_dsp_destroy\0").map_err(|_| ERROR)?,
			};
			let ctx = NonNull::new((api.create)(48_000)).ok_or(ERROR)?;
			Ok(Self {
				api,
				ctx,
				_library: library,
			})
		}
	}

	pub(super) fn process(&mut self, pcm: &mut [i16; 480]) -> Result<(), &'static str> {
		// SAFETY: A live session and exactly 480 writable mono samples are passed only
		// on the owning worker. The plugin may not retain pcm or access it after return.
		let status = unsafe { (self.api.process)(self.ctx.as_ptr(), pcm.as_mut_ptr(), pcm.len()) };
		if status == 0 { Ok(()) } else { Err(ERROR) }
	}
}

impl Drop for DspPlugin {
	fn drop(&mut self) {
		// SAFETY: The non-null session is exclusively owned and destroyed exactly once,
		// before Rust drops _library. The plugin must neither panic nor throw here.
		unsafe { (self.api.destroy)(self.ctx.as_ptr()) };
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn missing_or_relative_library_is_an_error() {
		assert!(DspPlugin::load_from(Path::new("serein_dsp")).is_err());
		let directory = std::env::temp_dir().join(format!(
			"serein-missing-dsp-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		// An exclusively created empty directory never loads an owner's installed plugin.
		std::fs::create_dir(&directory).unwrap();
		let result =
			DspPlugin::load_from(&directory.join(libloading::library_filename("serein_dsp")));
		std::fs::remove_dir(directory).unwrap();
		assert!(result.is_err());
	}
}

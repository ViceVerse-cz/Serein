//! Fixed-width native ABI for the actual renderer's physical GPU identity.
#![allow(unsafe_code)]

use model::{VideoAdapter, VideoAdapterIdentity};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Adapter {
	pub(crate) identity: u32,
	pub(crate) vendor_id: u32,
	pub(crate) device_id: u32,
	pub(crate) domain: u32,
	pub(crate) bus: u32,
	pub(crate) slot: u32,
	pub(crate) function: u32,
	pub(crate) value: u64,
}

impl From<VideoAdapter> for Adapter {
	fn from(value: VideoAdapter) -> Self {
		let mut result = Self {
			vendor_id: value.vendor_id,
			device_id: value.device_id,
			..Self::default()
		};
		match value.identity {
			VideoAdapterIdentity::Unidentified => {}
			VideoAdapterIdentity::WindowsLuid(value) => {
				result.identity = 1;
				result.value = value;
			}
			VideoAdapterIdentity::Pci {
				domain,
				bus,
				device,
				function,
			} => {
				result.identity = 2;
				result.domain = domain;
				result.bus = u32::from(bus);
				result.slot = u32::from(device);
				result.function = u32::from(function);
			}
			VideoAdapterIdentity::MetalRegistry(value) => {
				result.identity = 3;
				result.value = value;
			}
		}
		result
	}
}

unsafe extern "C" {
	fn serein_video_query_on_adapter(backend: i32, codec: i32, adapter: *const Adapter) -> i32;
	#[cfg(target_os = "linux")]
	fn serein_video_cuda_device(adapter: *const Adapter) -> i32;
	#[cfg(target_os = "linux")]
	fn serein_video_drm_device(
		adapter: *const Adapter,
		path: *mut std::ffi::c_char,
		capacity: usize,
	) -> i32;
}

pub(crate) fn query_on_adapter(backend: i32, codec: i32, adapter: VideoAdapter) -> i32 {
	let adapter = Adapter::from(adapter);
	// SAFETY: The native query reads this fixed-width value synchronously. It
	// creates metadata sessions only, never a capture or encoded picture.
	unsafe { serein_video_query_on_adapter(backend, codec, &adapter) }
}

#[cfg(target_os = "linux")]
pub(crate) fn cuda_device(adapter: VideoAdapter) -> Option<u32> {
	// SAFETY: The adapter remains alive for the bounded CUDA enumeration.
	u32::try_from(unsafe { serein_video_cuda_device(&Adapter::from(adapter)) }).ok()
}

#[cfg(target_os = "linux")]
pub(crate) fn drm_device(adapter: VideoAdapter) -> Option<String> {
	let mut path = [0 as std::ffi::c_char; 80];
	// SAFETY: The fixed-width adapter and writable buffer outlive this call;
	// success guarantees a terminating zero within the supplied capacity.
	let found =
		unsafe { serein_video_drm_device(&Adapter::from(adapter), path.as_mut_ptr(), path.len()) };
	if found == 0 {
		return None;
	}
	// SAFETY: The C helper's successful snprintf always terminates the buffer.
	unsafe { std::ffi::CStr::from_ptr(path.as_ptr()) }
		.to_str()
		.ok()
		.map(str::to_owned)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn physical_identity_keeps_same_vendor_adapters_distinct() {
		let first = Adapter::from(VideoAdapter {
			vendor_id: 0x10de,
			device_id: 0x2684,
			identity: VideoAdapterIdentity::Pci {
				domain: 0,
				bus: 1,
				device: 0,
				function: 0,
			},
		});
		let second = Adapter::from(VideoAdapter {
			identity: VideoAdapterIdentity::Pci {
				domain: 0,
				bus: 2,
				device: 0,
				function: 0,
			},
			vendor_id: 0x10de,
			device_id: 0x2684,
		});
		assert_eq!(first.identity, 2);
		assert_ne!(first.bus, second.bus);
		assert_eq!(Adapter::from(VideoAdapter::default()).identity, 0);
	}
}

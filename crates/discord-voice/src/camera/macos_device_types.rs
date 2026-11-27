#![allow(unsafe_code)]

use objc2_av_foundation::{
	AVCaptureDeviceType, AVCaptureDeviceTypeBuiltInWideAngleCamera,
	AVCaptureDeviceTypeDeskViewCamera,
};

enum OptionalDeviceType {
	External,
	ContinuityCamera,
}

impl OptionalDeviceType {
	fn get(self) -> Option<&'static AVCaptureDeviceType> {
		let symbol = match self {
			Self::External => c"AVCaptureDeviceTypeExternal",
			Self::ContinuityCamera => c"AVCaptureDeviceTypeContinuityCamera",
		};
		// SAFETY: These names are AVFoundation's process-lifetime NSString *const globals.
		// dlsym returns the address of the global (not the NSString) or null when unavailable.
		// AVFoundation is linked by the camera bindings and remains loaded for the process.
		unsafe {
			let slot = libc::dlsym(libc::RTLD_DEFAULT, symbol.as_ptr())
				.cast::<*const AVCaptureDeviceType>();
			slot.as_ref()?.as_ref()
		}
	}
}

#[allow(deprecated)] // The newer external device type is optional on macOS 13.
fn legacy_external_device_type() -> &'static AVCaptureDeviceType {
	// SAFETY: AVFoundation exports this immutable device type on Ventura and later.
	unsafe { objc2_av_foundation::AVCaptureDeviceTypeExternalUnknown }
}

pub(super) fn discovery_types() -> Vec<&'static AVCaptureDeviceType> {
	// SAFETY: These immutable AVFoundation device types are available on macOS 13 and later.
	let (built_in, desk_view) = unsafe {
		(
			AVCaptureDeviceTypeBuiltInWideAngleCamera,
			AVCaptureDeviceTypeDeskViewCamera,
		)
	};
	let mut types = Vec::with_capacity(4);
	types.push(built_in);
	types.push(
		OptionalDeviceType::External
			.get()
			.unwrap_or_else(legacy_external_device_type),
	);
	types.extend(OptionalDeviceType::ContinuityCamera.get());
	types.push(desk_view);
	types
}

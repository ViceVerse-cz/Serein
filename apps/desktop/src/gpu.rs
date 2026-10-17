//! GPU adapter selection and device configuration for the window surface.
//!
//! wgpu's `LowPower` hint ranks the integrated GPU above the discrete one on every platform,
//! but the display is usually wired to the discrete card. On Wayland that combination fails
//! outright: the compositor cannot import buffers from a GPU that drives no output, which
//! surfaces as `importing the supplied dmabufs failed` and then a panic inside `egui-wgpu`
//! (`The surface isn't supported by this adapter`). Rank the adapters that can actually
//! present instead, ordered by the device preference, so a preference can never pick a
//! device the window cannot use.

use eframe::wgpu;
use model::{GpuPreference, VideoAdapter, VideoAdapterIdentity};

/// Keep eframe's adapter-specific requirements while reducing DX12 allocation reserves.
pub fn setup() -> eframe::egui_wgpu::WgpuSetupCreateNew {
	let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
	let device_descriptor = setup.device_descriptor;
	setup.device_descriptor = std::sync::Arc::new(move |adapter| {
		let mut descriptor = device_descriptor(adapter);
		configure_memory(adapter.get_info().backend, &mut descriptor);
		descriptor
	});
	setup
}

fn configure_memory(backend: wgpu::Backend, descriptor: &mut wgpu::DeviceDescriptor<'_>) {
	if backend == wgpu::Backend::Dx12 {
		// Smaller allocation blocks, not a cap on texture sizes or total memory.
		descriptor.memory_hints = wgpu::MemoryHints::MemoryUsage;
	}
}

/// Sort key for an adapter; lower is better.
fn rank(info: &wgpu::AdapterInfo, prefer_integrated: bool) -> (u8, u8) {
	let device = match info.device_type {
		wgpu::DeviceType::DiscreteGpu => u8::from(prefer_integrated),
		wgpu::DeviceType::IntegratedGpu => u8::from(!prefer_integrated),
		wgpu::DeviceType::VirtualGpu => 2,
		wgpu::DeviceType::Other => 3,
		wgpu::DeviceType::Cpu => 4,
	};
	// The GL backend is the compatibility fallback; native APIs present far better.
	let backend = u8::from(info.backend == wgpu::Backend::Gl);
	(device, backend)
}

/// Whether the saved preference asks for the integrated GPU.
fn prefer_integrated(preference: GpuPreference) -> bool {
	match preference {
		GpuPreference::PowerSaving => true,
		GpuPreference::HighPerformance => false,
		// The diagnostic override only applies when the person did not choose explicitly.
		GpuPreference::Automatic => wgpu::PowerPreference::from_env()
			.is_some_and(|preference| preference == wgpu::PowerPreference::LowPower),
	}
}

/// Describes an adapter for settings and bug reports.
pub fn describe(info: &wgpu::AdapterInfo) -> String {
	format!("{} ({:?})", info.name, info.backend)
}

/// Capture the adapter actually selected by eframe, rather than re-evaluating a
/// saved preference or assuming that the first encoder of its vendor is the same GPU.
pub fn video_adapter(adapter: &wgpu::Adapter) -> VideoAdapter {
	let info = adapter.get_info();
	VideoAdapter {
		vendor_id: info.vendor,
		device_id: info.device,
		identity: physical_identity(adapter, &info),
	}
}

#[cfg(target_os = "linux")]
fn physical_identity(_: &wgpu::Adapter, info: &wgpu::AdapterInfo) -> VideoAdapterIdentity {
	// wgpu's Vulkan backend obtains this from VK_EXT_pci_bus_info on the selected
	// physical device. A GL/name/vendor fallback cannot distinguish identical cards.
	if info.backend == wgpu::Backend::Vulkan {
		pci_identity(&info.device_pci_bus_id).unwrap_or_default()
	} else {
		VideoAdapterIdentity::Unidentified
	}
}

#[cfg(any(target_os = "linux", test))]
fn pci_identity(value: &str) -> Option<VideoAdapterIdentity> {
	if !(12..=16).contains(&value.len()) || !value.is_ascii() {
		return None;
	}
	let (domain, remaining) = value.split_once(':')?;
	let (bus, remaining) = remaining.split_once(':')?;
	let (device, function) = remaining.split_once('.')?;
	if !(4..=8).contains(&domain.len())
		|| bus.len() != 2
		|| device.len() != 2
		|| function.len() != 1
	{
		return None;
	}
	if ![domain, bus, device, function]
		.into_iter()
		.all(|part| part.bytes().all(|byte| byte.is_ascii_hexdigit()))
	{
		return None;
	}
	let domain = u32::from_str_radix(domain, 16).ok()?;
	let bus = u8::from_str_radix(bus, 16).ok()?;
	let device = u8::from_str_radix(device, 16).ok()?;
	let function = u8::from_str_radix(function, 16).ok()?;
	(device <= 31 && function <= 7).then_some(VideoAdapterIdentity::Pci {
		domain,
		bus,
		device,
		function,
	})
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn physical_identity(adapter: &wgpu::Adapter, _: &wgpu::AdapterInfo) -> VideoAdapterIdentity {
	// SAFETY: HAL guards keep the selected native adapter and Vulkan instance alive.
	// Only identity properties are read; no native object is mutated or retained.
	unsafe {
		if let Some(native) = adapter.as_hal::<wgpu::hal::api::Dx12>() {
			return native.raw_adapter().GetDesc2().map_or(
				VideoAdapterIdentity::Unidentified,
				|desc| {
					VideoAdapterIdentity::WindowsLuid(
						(u64::from(desc.AdapterLuid.HighPart as u32) << 32)
							| u64::from(desc.AdapterLuid.LowPart),
					)
				},
			);
		}
		let Some(native) = adapter.as_hal::<wgpu::hal::api::Vulkan>() else {
			return VideoAdapterIdentity::Unidentified;
		};
		let shared = native.shared_instance();
		let instance = shared.raw_instance();
		let device = native.raw_physical_device();
		let device_version = instance.get_physical_device_properties(device).api_version;
		let mut identity = ash::vk::PhysicalDeviceIDProperties::default();
		let mut properties = ash::vk::PhysicalDeviceProperties2::default().push_next(&mut identity);
		if shared.instance_api_version() >= ash::vk::API_VERSION_1_1
			&& device_version >= ash::vk::API_VERSION_1_1
		{
			instance.get_physical_device_properties2(device, &mut properties);
		} else if shared
			.extensions()
			.contains(&ash::khr::get_physical_device_properties2::NAME)
			&& shared
				.extensions()
				.contains(&ash::khr::external_memory_capabilities::NAME)
		{
			// Vulkan 1.0 requires both enabled instance extensions for the ID chain.
			let query =
				ash::khr::get_physical_device_properties2::Instance::new(shared.entry(), instance);
			query.get_physical_device_properties2(device, &mut properties);
		} else {
			// Diagnostic OpenGL/older Vulkan renderers cannot safely identify a DXGI
			// adapter. Leave hardware encoding unavailable instead of guessing a GPU.
			return VideoAdapterIdentity::Unidentified;
		}
		luid_identity(identity.device_luid, identity.device_luid_valid != 0)
	}
}

#[cfg(any(target_os = "windows", test))]
fn luid_identity(bytes: [u8; 8], valid: bool) -> VideoAdapterIdentity {
	if valid {
		// Vulkan returns the native Windows LUID bytes, including the signed high
		// word's bit pattern. Both supported Windows architectures are little endian.
		VideoAdapterIdentity::WindowsLuid(u64::from_le_bytes(bytes))
	} else {
		VideoAdapterIdentity::Unidentified
	}
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn physical_identity(adapter: &wgpu::Adapter, _: &wgpu::AdapterInfo) -> VideoAdapterIdentity {
	use objc2_metal::MTLDevice;

	// SAFETY: The guard owns the HAL borrow; registryID reads the selected live
	// Metal device and no Objective-C/native handle escapes this function.
	unsafe {
		adapter
			.as_hal::<wgpu::hal::api::Metal>()
			.map_or(VideoAdapterIdentity::Unidentified, |native| {
				VideoAdapterIdentity::MetalRegistry(native.raw_device().registryID())
			})
	}
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn physical_identity(_: &wgpu::Adapter, _: &wgpu::AdapterInfo) -> VideoAdapterIdentity {
	VideoAdapterIdentity::Unidentified
}

/// Picks the adapter used for the window surface, or explains why none of them works.
pub fn select(
	preference: GpuPreference,
	adapters: &[wgpu::Adapter],
	surface: Option<&wgpu::Surface<'_>>,
) -> Result<wgpu::Adapter, String> {
	let prefer_integrated = prefer_integrated(preference);
	// An adapter without surface formats cannot configure the swapchain; egui would panic later.
	let mut usable: Vec<&wgpu::Adapter> = adapters
		.iter()
		.filter(|adapter| {
			surface.is_none_or(|surface| !surface.get_capabilities(adapter).formats.is_empty())
		})
		.collect();
	usable.sort_by_key(|adapter| rank(&adapter.get_info(), prefer_integrated));
	let Some(adapter) = usable.first() else {
		let available = adapters
			.iter()
			.map(|adapter| describe(&adapter.get_info()))
			.collect::<Vec<_>>()
			.join(", ");
		return Err(if available.is_empty() {
			"no GPU adapter found; install a Vulkan, Metal, DirectX or OpenGL driver".to_owned()
		} else {
			format!("no GPU adapter can draw this window; found {available}")
		});
	};
	eprintln!("[Serein] GPU adapter: {}", describe(&adapter.get_info()));
	Ok((*adapter).clone())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn vulkan_luid_preserves_exact_windows_identity_and_requires_driver_validity() {
		let bytes = [0x78, 0x56, 0x34, 0x12, 0xef, 0xcd, 0xab, 0x90];
		assert_eq!(
			luid_identity(bytes, true),
			VideoAdapterIdentity::WindowsLuid(0x90ab_cdef_1234_5678)
		);
		assert_eq!(
			luid_identity(bytes, false),
			VideoAdapterIdentity::Unidentified
		);
		assert_eq!(
			luid_identity([0; 8], false),
			VideoAdapterIdentity::Unidentified
		);
	}

	#[test]
	fn pci_identity_never_collapses_two_matching_gpu_models() {
		let first = pci_identity("0000:01:00.0").unwrap();
		let second = pci_identity("0000:02:00.0").unwrap();
		assert_ne!(first, second);
		assert_eq!(
			pci_identity("00010000:ff:1f.7"),
			Some(VideoAdapterIdentity::Pci {
				domain: 0x10000,
				bus: 0xff,
				device: 31,
				function: 7,
			})
		);
		for invalid in [
			"",
			"01:00.0",
			"0000:1:00.0",
			"0000:01:20.0",
			"0000:01:00.8",
			"0000:01:00.0/extra",
		] {
			assert_eq!(pci_identity(invalid), None);
		}
	}

	#[test]
	fn memory_policy_preserves_device_requirements_and_other_backends() {
		let inherited = wgpu::DeviceDescriptor {
			label: Some("inherited device"),
			required_features: wgpu::Features::TEXTURE_COMPRESSION_BC,
			required_limits: wgpu::Limits {
				max_texture_dimension_2d: 8192,
				max_bind_groups: 3,
				..Default::default()
			},
			memory_hints: wgpu::MemoryHints::Manual {
				suballocated_device_memory_block_size: 16 * 1024 * 1024..48 * 1024 * 1024,
			},
			..Default::default()
		};
		for backend in wgpu::Backend::ALL {
			let mut descriptor = inherited.clone();
			configure_memory(backend, &mut descriptor);
			assert_eq!(descriptor.label, inherited.label);
			assert_eq!(descriptor.required_features, inherited.required_features);
			assert_eq!(descriptor.required_limits, inherited.required_limits);
			if backend == wgpu::Backend::Dx12 {
				assert!(matches!(
					descriptor.memory_hints,
					wgpu::MemoryHints::MemoryUsage
				));
			} else {
				assert!(matches!(
					descriptor.memory_hints,
					wgpu::MemoryHints::Manual { suballocated_device_memory_block_size }
						if suballocated_device_memory_block_size == (16 * 1024 * 1024..48 * 1024 * 1024)
				));
			}
		}
	}

	fn info(device_type: wgpu::DeviceType, backend: wgpu::Backend) -> wgpu::AdapterInfo {
		wgpu::AdapterInfo::new(device_type, backend)
	}

	#[test]
	fn discrete_gpu_wins_by_default() {
		let discrete = info(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Vulkan);
		let integrated = info(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Vulkan);
		assert!(rank(&discrete, false) < rank(&integrated, false));
		assert!(!prefer_integrated(GpuPreference::HighPerformance));

		{
			let discrete = info(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Vulkan);
			let integrated = info(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Vulkan);
			assert!(prefer_integrated(GpuPreference::PowerSaving));
			assert!(rank(&integrated, true) < rank(&discrete, true));
		}

		{
			let vulkan = info(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Vulkan);
			let gl = info(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Gl);
			let llvmpipe = info(wgpu::DeviceType::Cpu, wgpu::Backend::Vulkan);
			assert!(rank(&vulkan, false) < rank(&gl, false));
			assert!(rank(&gl, false) < rank(&llvmpipe, false));
			assert!(rank(&gl, true) < rank(&llvmpipe, true));
		}
	}
}

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
use model::GpuPreference;

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

/// Picks the adapter used for the window surface, or explains why none of them works.
pub fn select(
	preference: GpuPreference,
	hardware_acceleration: bool,
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
	usable.sort_by_key(|adapter| {
		acceleration_rank(
			&adapter.get_info(),
			prefer_integrated,
			hardware_acceleration,
		)
	});
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
	if !hardware_acceleration && adapter.get_info().device_type != wgpu::DeviceType::Cpu {
		eprintln!("[Serein] Software rendering is unavailable; using a compatible GPU");
	}
	Ok((*adapter).clone())
}

fn acceleration_rank(
	info: &wgpu::AdapterInfo,
	prefer_integrated: bool,
	enabled: bool,
) -> (u8, u8, u8) {
	let (device, backend) = rank(info, prefer_integrated);
	(
		u8::from(!enabled && info.device_type != wgpu::DeviceType::Cpu),
		device,
		backend,
	)
}

#[cfg(test)]
mod tests {
	use super::*;

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
	fn disabled_acceleration_prefers_software_over_every_gpu() {
		let cpu = info(wgpu::DeviceType::Cpu, wgpu::Backend::Vulkan);
		for device in [
			wgpu::DeviceType::DiscreteGpu,
			wgpu::DeviceType::IntegratedGpu,
			wgpu::DeviceType::VirtualGpu,
		] {
			let gpu = info(device, wgpu::Backend::Vulkan);
			assert!(acceleration_rank(&cpu, false, false) < acceleration_rank(&gpu, false, false));
			assert!(acceleration_rank(&gpu, false, true) < acceleration_rank(&cpu, false, true));
		}
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

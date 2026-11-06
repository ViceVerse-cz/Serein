//! Rounded window corners for the undecorated Linux window with Serein's own title bar.
//!
//! egui cannot clip to a rounded shape, so after the frame is drawn a full-window pass
//! multiplies every pixel by the coverage of a rounded rectangle. Outside the shape the
//! pixel becomes fully transparent, which needs a window created with alpha.

use eframe::egui;
use eframe::egui_wgpu::{self, CallbackTrait};
use eframe::wgpu;

/// Corner radius in logical pixels, matching GTK 4 / libadwaita windows; UI zoom does not
/// scale it.
pub const RADIUS: f32 = 12.0;

const SHADER: &str = r"
struct Params { size: vec2<f32>, radius: f32, pad: f32 };
@group(0) @binding(0) var<uniform> params: Params;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
	let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
	return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
	let half_size = params.size * 0.5;
	let q = abs(frag.xy - half_size) - (half_size - vec2<f32>(params.radius));
	let sdf = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - params.radius;
	return vec4<f32>(clamp(0.5 - sdf, 0.0, 1.0));
}
";

struct Resources {
	pipeline: wgpu::RenderPipeline,
	params: wgpu::Buffer,
	bind_group: wgpu::BindGroup,
	/// Last uniform contents, so the buffer is rewritten only on resize or rescale.
	written: Option<[f32; 4]>,
}

pub struct Mask {
	format: wgpu::TextureFormat,
	radius_px: f32,
}

impl Mask {
	/// Mask covering `rect` (the whole viewport) with corners of `radius_px` physical pixels.
	pub fn callback(
		rect: egui::Rect,
		format: wgpu::TextureFormat,
		radius_px: f32,
	) -> egui::epaint::PaintCallback {
		egui_wgpu::Callback::new_paint_callback(rect, Self { format, radius_px })
	}

	fn create(&self, device: &wgpu::Device) -> Resources {
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("serein-round-corners"),
			source: wgpu::ShaderSource::Wgsl(SHADER.into()),
		});
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("serein-round-corners"),
			entries: &[wgpu::BindGroupLayoutEntry {
				binding: 0,
				visibility: wgpu::ShaderStages::FRAGMENT,
				ty: wgpu::BindingType::Buffer {
					ty: wgpu::BufferBindingType::Uniform,
					has_dynamic_offset: false,
					min_binding_size: None,
				},
				count: None,
			}],
		});
		let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("serein-round-corners"),
			bind_group_layouts: &[Some(&layout)],
			immediate_size: 0,
		});
		// Result = destination * source alpha. egui's target holds premultiplied colour, so
		// scaling every channel by coverage keeps the inside and clears the outside.
		let multiply = wgpu::BlendComponent {
			src_factor: wgpu::BlendFactor::Zero,
			dst_factor: wgpu::BlendFactor::SrcAlpha,
			operation: wgpu::BlendOperation::Add,
		};
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("serein-round-corners"),
			layout: Some(&pipeline_layout),
			vertex: wgpu::VertexState {
				module: &module,
				entry_point: Some("vs"),
				compilation_options: Default::default(),
				buffers: &[],
			},
			primitive: wgpu::PrimitiveState::default(),
			// eframe's defaults: no depth buffer and no multisampling.
			depth_stencil: None,
			multisample: wgpu::MultisampleState::default(),
			fragment: Some(wgpu::FragmentState {
				module: &module,
				entry_point: Some("fs"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState {
					format: self.format,
					blend: Some(wgpu::BlendState {
						color: multiply,
						alpha: multiply,
					}),
					write_mask: wgpu::ColorWrites::ALL,
				})],
			}),
			multiview_mask: None,
			cache: None,
		});
		let params = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("serein-round-corners"),
			size: std::mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("serein-round-corners"),
			layout: &layout,
			entries: &[wgpu::BindGroupEntry {
				binding: 0,
				resource: params.as_entire_binding(),
			}],
		});
		Resources {
			pipeline,
			params,
			bind_group,
			written: None,
		}
	}
}

impl CallbackTrait for Mask {
	fn prepare(
		&self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		screen: &egui_wgpu::ScreenDescriptor,
		_encoder: &mut wgpu::CommandEncoder,
		resources: &mut egui_wgpu::CallbackResources,
	) -> Vec<wgpu::CommandBuffer> {
		if !resources.contains::<Resources>() {
			resources.insert(self.create(device));
		}
		let state = resources.get_mut::<Resources>().expect("inserted above");
		let [width, height] = screen.size_in_pixels;
		let params = [width as f32, height as f32, self.radius_px, 0.0];
		if state.written != Some(params) {
			let mut bytes = [0; 16];
			for (chunk, value) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(params) {
				*chunk = value.to_ne_bytes();
			}
			queue.write_buffer(&state.params, 0, &bytes);
			state.written = Some(params);
		}
		Vec::new()
	}

	fn paint(
		&self,
		info: egui::PaintCallbackInfo,
		pass: &mut wgpu::RenderPass<'static>,
		resources: &egui_wgpu::CallbackResources,
	) {
		let Some(state) = resources.get::<Resources>() else {
			return;
		};
		// egui sets the viewport and scissor from point rects; cover every framebuffer pixel
		// so fractional scales cannot leave an unmasked edge row or column.
		let [width, height] = info.screen_size_px;
		pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
		pass.set_scissor_rect(0, 0, width, height);
		pass.set_pipeline(&state.pipeline);
		pass.set_bind_group(0, &state.bind_group, &[]);
		pass.draw(0..3, 0..1);
	}
}

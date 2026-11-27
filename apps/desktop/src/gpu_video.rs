//! GPU colour conversion for decoded video. Decoders hand over their native NV12/I420 planes;
//! one small render pass converts them (and applies track rotation) into an RGBA texture
//! that egui draws like any other image. Uploads move 1.5 bytes per pixel instead of 4 and
//! no per-pixel colour work runs on the CPU. Without a wgpu renderer (tests), the CPU
//! fallback in [`platform::video::Picture::to_rgba`] feeds an ordinary egui texture.
use eframe::{egui, egui_wgpu, wgpu};
use platform::video::{Layout, Picture};
use std::sync::{Arc, OnceLock};

/// The window's one wgpu device; every video texture converts through it.
static CONVERTER: OnceLock<Arc<Converter>> = OnceLock::new();

/// Builds the converter for the window's renderer; later calls keep the first one.
pub fn install(state: &egui_wgpu::RenderState) {
	CONVERTER.get_or_init(|| Converter::new(state));
}

/// Updates an existing egui RGBA texture in place on the GPU without creating a new texture,
/// bind group, or egui texture delta. Returns `false` if the texture is not yet allocated or
/// its dimensions differ, signalling the caller to fall back to a full UI pass.
pub fn update_egui_texture(id: egui::TextureId, image: &egui::ColorImage) -> bool {
	let [width, height] = image.resolution();
	if width == 0 || height == 0 || image.pixels().len() != width.saturating_mul(height) {
		return false;
	}
	let Some(converter) = CONVERTER.get() else {
		return false;
	};
	let texture = {
		let renderer = converter.renderer.read();
		let Some(texture) = renderer
			.texture(&id)
			.and_then(|entry| entry.texture.as_ref())
		else {
			return false;
		};
		if texture.width() as usize != width || texture.height() as usize != height {
			return false;
		};
		texture.clone()
	};
	converter.write(&texture, image.as_raw(), texture.width() * 4);
	true
}

/// Whether pictures are converted on the GPU; without it every frame is converted in memory.
#[cfg(target_os = "macos")]
pub fn installed() -> bool {
	CONVERTER.get().is_some()
}

/// How a decoder thread asks the window for a frame.
#[derive(Clone)]
pub struct Wake {
	ctx: egui::Context,
	picture: Option<eframe::PaintOnlyRequester>,
}

impl Wake {
	/// `picture` is the renderer's paint-only handle; without one every wake is a repaint.
	pub fn new(ctx: &egui::Context, picture: Option<eframe::PaintOnlyRequester>) -> Self {
		Self {
			ctx: ctx.clone(),
			picture,
		}
	}

	/// The ui has to change: a state, a status line, a readout that is on screen.
	pub fn ui(&self) {
		self.ctx.request_repaint();
	}

	/// Only a picture changed. The renderer presents it over the last pass's shapes
	/// without running the ui, where it can; otherwise this is an ordinary repaint.
	pub fn picture(&self) {
		self.picture_after(std::time::Duration::ZERO);
	}

	/// [`Self::picture`] for a picture that is due `delay` from now.
	pub fn picture_after(&self, delay: std::time::Duration) {
		match &self.picture {
			Some(picture) => picture.request_after(delay),
			None => self.ctx.request_repaint_after(delay),
		}
	}
}

/// A decoded picture on its way to the screen.
pub enum Frame {
	/// Planes packed in memory, uploaded to the GPU.
	Packed(Picture),
	/// The decoder's own surface, which the GPU samples in place.
	#[cfg(target_os = "macos")]
	Surface(platform::video::Surface),
}

impl Frame {
	fn display_size(&self) -> Option<(u32, u32)> {
		match self {
			Self::Packed(picture) => picture.is_valid().then(|| picture.display_size()),
			#[cfg(target_os = "macos")]
			Self::Surface(surface) => Some(surface.display_size()),
		}
	}
}

/// Frees textures retired since the last call. Call once at the start of each frame: a
/// texture dropped during a frame may still be referenced by that frame's queued shapes,
/// so its egui registration outlives the paint and is removed one frame later.
pub fn collect() {
	if let Some(converter) = CONVERTER.get() {
		let retired = std::mem::take(&mut *converter.retired.lock());
		if !retired.is_empty() {
			let mut renderer = converter.renderer.write();
			for id in retired {
				renderer.free_texture(&id);
			}
		}
	}
}

const SHADER: &str = r"
struct Params {
	rows: array<vec4<f32>, 3>,
	rotation: vec4<u32>,
};
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var planes: sampler;
@group(0) @binding(2) var luma: texture_2d<f32>;
@group(0) @binding(3) var chroma_a: texture_2d<f32>;
@group(0) @binding(4) var chroma_b: texture_2d<f32>;

struct Varyings {
	@builtin(position) position: vec4<f32>,
	@location(0) uv: vec2<f32>,
};

// One oversized triangle covers the target; `uv` is the source position after rotation.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varyings {
	let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
	var uv = corner;
	switch params.rotation.x {
		case 90u: { uv = vec2<f32>(corner.y, 1.0 - corner.x); }
		case 180u: { uv = vec2<f32>(1.0) - corner; }
		case 270u: { uv = vec2<f32>(1.0 - corner.y, corner.x); }
		default: {}
	}
	var out: Varyings;
	out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
	out.uv = uv;
	return out;
}

fn to_rgb(yuv: vec3<f32>) -> vec4<f32> {
	let rgb = vec3<f32>(
		dot(params.rows[0].xyz, yuv) + params.rows[0].w,
		dot(params.rows[1].xyz, yuv) + params.rows[1].w,
		dot(params.rows[2].xyz, yuv) + params.rows[2].w,
	);
	return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

@fragment
fn fs_nv12(in: Varyings) -> @location(0) vec4<f32> {
	let y = textureSampleLevel(luma, planes, in.uv, 0.0).r;
	let cbcr = textureSampleLevel(chroma_a, planes, in.uv, 0.0).rg;
	return to_rgb(vec3<f32>(y, cbcr));
}

@fragment
fn fs_i420(in: Varyings) -> @location(0) vec4<f32> {
	let y = textureSampleLevel(luma, planes, in.uv, 0.0).r;
	let cb = textureSampleLevel(chroma_a, planes, in.uv, 0.0).r;
	let cr = textureSampleLevel(chroma_b, planes, in.uv, 0.0).r;
	return to_rgb(vec3<f32>(y, cb, cr));
}
";

/// Shared pipelines plus the renderer handles every [`VideoTexture`] uploads through.
pub struct Converter {
	device: wgpu::Device,
	queue: wgpu::Queue,
	renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
	layout: wgpu::BindGroupLayout,
	nv12: wgpu::RenderPipeline,
	i420: wgpu::RenderPipeline,
	sampler: wgpu::Sampler,
	/// Dropped textures awaiting [`collect`]; at most one frame's worth.
	retired: egui::mutex::Mutex<Vec<egui::TextureId>>,
}

impl Converter {
	pub fn new(state: &egui_wgpu::RenderState) -> Arc<Self> {
		let device = &state.device;
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("video-yuv"),
			source: wgpu::ShaderSource::Wgsl(SHADER.into()),
		});
		let texture = |binding| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture {
				sample_type: wgpu::TextureSampleType::Float { filterable: true },
				view_dimension: wgpu::TextureViewDimension::D2,
				multisampled: false,
			},
			count: None,
		};
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("video-yuv"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
					ty: wgpu::BindingType::Buffer {
						ty: wgpu::BufferBindingType::Uniform,
						has_dynamic_offset: false,
						min_binding_size: None,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
					count: None,
				},
				texture(2),
				texture(3),
				texture(4),
			],
		});
		let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("video-yuv"),
			bind_group_layouts: &[Some(&layout)],
			immediate_size: 0,
		});
		let pipeline = |entry_point| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(entry_point),
				layout: Some(&pipeline_layout),
				vertex: wgpu::VertexState {
					module: &module,
					entry_point: Some("vs_main"),
					compilation_options: Default::default(),
					buffers: &[],
				},
				primitive: wgpu::PrimitiveState::default(),
				depth_stencil: None,
				multisample: wgpu::MultisampleState::default(),
				fragment: Some(wgpu::FragmentState {
					module: &module,
					entry_point: Some(entry_point),
					compilation_options: Default::default(),
					targets: &[Some(wgpu::ColorTargetState {
						format: wgpu::TextureFormat::Rgba8Unorm,
						blend: None,
						write_mask: wgpu::ColorWrites::ALL,
					})],
				}),
				multiview_mask: None,
				cache: None,
			})
		};
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("video-yuv"),
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			..Default::default()
		});
		Arc::new(Self {
			device: device.clone(),
			queue: state.queue.clone(),
			renderer: state.renderer.clone(),
			nv12: pipeline("fs_nv12"),
			i420: pipeline("fs_i420"),
			layout,
			sampler,
			retired: Default::default(),
		})
	}

	fn texture(
		&self,
		label: &str,
		size: (u32, u32),
		format: wgpu::TextureFormat,
		output: bool,
	) -> wgpu::Texture {
		// Output bytes stay gamma-encoded in plain `Rgba8Unorm`, exactly like egui's own
		// textures, which its shader samples without sRGB decoding.
		let usage = if output {
			wgpu::TextureUsages::TEXTURE_BINDING
				| wgpu::TextureUsages::RENDER_ATTACHMENT
				| wgpu::TextureUsages::COPY_DST
		} else {
			wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST
		};
		self.device.create_texture(&wgpu::TextureDescriptor {
			label: Some(label),
			size: wgpu::Extent3d {
				width: size.0,
				height: size.1,
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format,
			usage,
			view_formats: &[],
		})
	}

	fn params(&self) -> wgpu::Buffer {
		self.device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("video-yuv"),
			size: 64,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		})
	}

	/// Binds the luma plane and the two chroma bindings (the same view twice for NV12).
	fn bind(&self, params: &wgpu::Buffer, planes: [&wgpu::Texture; 3]) -> wgpu::BindGroup {
		let views =
			planes.map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));
		self.device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("video-yuv"),
			layout: &self.layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: params.as_entire_binding(),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::Sampler(&self.sampler),
				},
				wgpu::BindGroupEntry {
					binding: 2,
					resource: wgpu::BindingResource::TextureView(&views[0]),
				},
				wgpu::BindGroupEntry {
					binding: 3,
					resource: wgpu::BindingResource::TextureView(&views[1]),
				},
				wgpu::BindGroupEntry {
					binding: 4,
					resource: wgpu::BindingResource::TextureView(&views[2]),
				},
			],
		})
	}

	/// One render pass: the bound planes, converted and rotated, into `output`.
	#[allow(clippy::too_many_arguments)]
	fn convert(
		&self,
		name: &str,
		output: &wgpu::Texture,
		layout: Layout,
		params: &wgpu::Buffer,
		bind_group: &wgpu::BindGroup,
		color: platform::video::Color,
		rotation: u16,
	) {
		let mut bytes = [0_u8; 64];
		for (slot, value) in bytes
			.as_chunks_mut::<4>()
			.0
			.iter_mut()
			.zip(color.coefficients().into_iter().flatten())
		{
			*slot = value.to_le_bytes();
		}
		bytes[48..52].copy_from_slice(&u32::from(rotation).to_le_bytes());
		self.queue.write_buffer(params, 0, &bytes);
		let view = output.create_view(&wgpu::TextureViewDescriptor::default());
		let mut encoder = self
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(name) });
		{
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some(name),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &view,
					depth_slice: None,
					resolve_target: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
						store: wgpu::StoreOp::Store,
					},
				})],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_pipeline(match layout {
				Layout::Nv12 => &self.nv12,
				_ => &self.i420,
			});
			pass.set_bind_group(0, bind_group, &[]);
			pass.draw(0..3, 0..1);
		}
		self.queue.submit([encoder.finish()]);
	}

	fn write(&self, texture: &wgpu::Texture, data: &[u8], row_bytes: u32) {
		let size = texture.size();
		self.queue.write_texture(
			wgpu::TexelCopyTextureInfo {
				texture,
				mip_level: 0,
				origin: wgpu::Origin3d::ZERO,
				aspect: wgpu::TextureAspect::All,
			},
			data,
			wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(row_bytes),
				rows_per_image: Some(size.height),
			},
			size,
		);
	}
}

/// GPU planes for one picture size and layout, reused while the stream keeps them.
struct Planes {
	layout: Layout,
	size: (u32, u32),
	luma: wgpu::Texture,
	chroma: [wgpu::Texture; 2],
	params: wgpu::Buffer,
	bind_group: wgpu::BindGroup,
}

struct Gpu {
	converter: Arc<Converter>,
	id: egui::TextureId,
	output: wgpu::Texture,
	planes: Option<Planes>,
	/// Conversion parameters for decoder surfaces, which bring their own plane textures.
	#[cfg(target_os = "macos")]
	surface_params: Option<wgpu::Buffer>,
}

impl Drop for Gpu {
	fn drop(&mut self) {
		// egui's bind group keeps the wgpu texture alive until `collect` frees the id.
		self.converter.retired.lock().push(self.id);
	}
}

/// One on-screen video picture: the latest frame as an egui texture.
#[derive(Default)]
pub struct VideoTexture {
	gpu: Option<Gpu>,
	cpu: Option<egui::TextureHandle>,
	size: egui::Vec2,
}

impl VideoTexture {
	pub fn texture(&self) -> Option<egui::load::SizedTexture> {
		let id = match (&self.gpu, &self.cpu) {
			(Some(gpu), _) => gpu.id,
			(None, Some(cpu)) => cpu.id(),
			(None, None) => return None,
		};
		Some(egui::load::SizedTexture::new(id, self.size))
	}

	/// Whether showing `frame` keeps the texture already on screen: same id, same size.
	/// Only then can the shapes that draw it be presented again without a ui pass.
	pub fn fits(&self, frame: &Frame) -> bool {
		frame.display_size().is_some_and(|size| self.holds(size))
	}

	/// [`Self::fits`] for packed planes.
	pub fn fits_picture(&self, picture: &Picture<&[u8]>) -> bool {
		picture.is_valid() && self.holds(picture.display_size())
	}

	fn holds(&self, size: (u32, u32)) -> bool {
		self.gpu
			.as_ref()
			.is_some_and(|gpu| (gpu.output.width(), gpu.output.height()) == size)
	}

	/// Shows `frame`; returns the texture to paint, or `None` for an invalid picture.
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		name: &str,
		frame: &Frame,
	) -> Option<egui::load::SizedTexture> {
		match frame {
			Frame::Packed(picture) => self.upload(ctx, name, picture.borrow()),
			#[cfg(target_os = "macos")]
			Frame::Surface(surface) => {
				if let Some(converter) = CONVERTER.get()
					&& self.sample_surface(converter, name, surface)
				{
					let (width, height) = surface.display_size();
					self.size = egui::vec2(width as f32, height as f32);
					return self.texture();
				}
				// Not a Metal renderer, or it refused the surface: copy the planes instead.
				let picture = surface.to_picture().ok()?;
				self.upload(ctx, name, picture.borrow())
			}
		}
	}

	/// Converts a decoder surface straight from its own planes: nothing is copied.
	#[cfg(target_os = "macos")]
	fn sample_surface(
		&mut self,
		converter: &Arc<Converter>,
		name: &str,
		surface: &platform::video::Surface,
	) -> bool {
		let Some([luma, chroma]) = surface.textures(&converter.device) else {
			return false;
		};
		self.output(converter, name, surface.display_size());
		let Some(gpu) = &mut self.gpu else {
			return false;
		};
		// Packed planes of an earlier picture are not needed while surfaces arrive.
		gpu.planes = None;
		let params = gpu.surface_params.get_or_insert_with(|| converter.params());
		let bind_group = converter.bind(params, [&luma, &chroma, &chroma]);
		converter.convert(
			name,
			&gpu.output,
			Layout::Nv12,
			params,
			&bind_group,
			surface.color(),
			surface.rotation(),
		);
		true
	}

	/// Makes the RGBA output `display` pixels large, keeping the egui texture id.
	fn output(&mut self, converter: &Arc<Converter>, name: &str, display: (u32, u32)) {
		if self
			.gpu
			.as_ref()
			.is_some_and(|gpu| (gpu.output.width(), gpu.output.height()) == display)
		{
			return;
		}
		let output = converter.texture(name, display, wgpu::TextureFormat::Rgba8Unorm, true);
		let view = output.create_view(&wgpu::TextureViewDescriptor::default());
		let mut renderer = converter.renderer.write();
		match &mut self.gpu {
			Some(gpu) => {
				renderer.update_egui_texture_from_wgpu_texture(
					&converter.device,
					&view,
					wgpu::FilterMode::Linear,
					gpu.id,
				);
				gpu.output = output;
			}
			None => {
				let id = renderer.register_native_texture(
					&converter.device,
					&view,
					wgpu::FilterMode::Linear,
				);
				self.gpu = Some(Gpu {
					converter: converter.clone(),
					id,
					output,
					planes: None,
					#[cfg(target_os = "macos")]
					surface_params: None,
				});
			}
		}
	}

	/// Shows `picture`; returns the texture to paint, or `None` for an invalid picture.
	pub fn upload(
		&mut self,
		ctx: &egui::Context,
		name: &str,
		picture: Picture<&[u8]>,
	) -> Option<egui::load::SizedTexture> {
		if !picture.is_valid() {
			return None;
		}
		let (width, height) = picture.display_size();
		match CONVERTER.get() {
			Some(converter) => self.upload_gpu(converter, name, &picture),
			None => {
				let (width, height, rgba) = picture.to_rgba()?;
				let image = egui::ColorImage::from_rgba_premultiplied(
					[width as usize, height as usize],
					&rgba,
				);
				match &mut self.cpu {
					Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
					None => {
						self.cpu = Some(ctx.load_texture(name, image, egui::TextureOptions::LINEAR))
					}
				}
			}
		}
		self.size = egui::vec2(width as f32, height as f32);
		self.texture()
	}

	fn upload_gpu(&mut self, converter: &Arc<Converter>, name: &str, picture: &Picture<&[u8]>) {
		self.output(converter, name, picture.display_size());
		let Some(gpu) = &mut self.gpu else {
			return;
		};
		if picture.layout == Layout::Rgba {
			if picture.rotation == 0 {
				converter.write(&gpu.output, picture.data, picture.width * 4);
			} else if let Some((width, _, rgba)) = picture.to_rgba() {
				converter.write(&gpu.output, &rgba, width * 4);
			}
			return;
		}
		let size = (picture.width, picture.height);
		if gpu
			.planes
			.as_ref()
			.is_none_or(|planes| planes.layout != picture.layout || planes.size != size)
		{
			gpu.planes = Some(planes(converter, picture.layout, size));
		}
		let planes = gpu.planes.as_ref().expect("planes allocated");
		let (chroma_width, chroma_height) = picture.chroma_size();
		let (luma, chroma) = picture.data.split_at((size.0 * size.1) as usize);
		converter.write(&planes.luma, luma, size.0);
		match picture.layout {
			Layout::Nv12 => converter.write(&planes.chroma[0], chroma, chroma_width * 2),
			_ => {
				let (cb, cr) = chroma.split_at((chroma_width * chroma_height) as usize);
				converter.write(&planes.chroma[0], cb, chroma_width);
				converter.write(&planes.chroma[1], cr, chroma_width);
			}
		}
		converter.convert(
			name,
			&gpu.output,
			picture.layout,
			&planes.params,
			&planes.bind_group,
			picture.color,
			picture.rotation,
		);
	}
}

fn planes(converter: &Converter, layout: Layout, size: (u32, u32)) -> Planes {
	let chroma_size = (size.0.div_ceil(2), size.1.div_ceil(2));
	let luma = converter.texture("video-luma", size, wgpu::TextureFormat::R8Unorm, false);
	let chroma = match layout {
		Layout::Nv12 => {
			let pairs = converter.texture(
				"video-chroma",
				chroma_size,
				wgpu::TextureFormat::Rg8Unorm,
				false,
			);
			[pairs.clone(), pairs]
		}
		_ => [
			converter.texture("video-cb", chroma_size, wgpu::TextureFormat::R8Unorm, false),
			converter.texture("video-cr", chroma_size, wgpu::TextureFormat::R8Unorm, false),
		],
	};
	let params = converter.params();
	let bind_group = converter.bind(&params, [&luma, &chroma[0], &chroma[1]]);
	Planes {
		layout,
		size,
		luma,
		chroma,
		params,
		bind_group,
	}
}

//! One explicit icon selection; bounded local decoding never uploads or retains a file path.
use eframe::egui;
use image::GenericImageView;
use model::Id;
use std::{
	io::{Cursor, Read},
	path::Path,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
};

type Scope = (u64, Id, u64);
type Selected = Result<Option<(String, egui::ColorImage)>, &'static str>;
const MAX_INPUT: usize = 8 * 1024 * 1024;
struct Choosing {
	scope: Scope,
	result: mpsc::Receiver<Selected>,
	cancelled: Arc<AtomicBool>,
}
#[derive(Default)]
pub struct GroupIcon {
	choosing: Option<Choosing>,
}
impl GroupIcon {
	pub fn start(
		&mut self,
		scope: Scope,
		runtime: &tokio::runtime::Handle,
		context: &egui::Context,
		parent: Arc<winit::window::Window>,
		title: &'static str,
		output_edge: u32,
	) -> Result<(), &'static str> {
		if self.choosing.is_some() {
			return Err("Close the previous image picker first");
		}
		let dialog = platform::save::icon_source(parent, title);
		let (send, result) = mpsc::sync_channel(1);
		let cancelled = Arc::new(AtomicBool::new(false));
		let flag = cancelled.clone();
		let context = context.clone();
		runtime.spawn(async move {
			let result = match dialog.await {
				Some(path) if !flag.load(Ordering::Acquire) => {
					let decode_flag = flag.clone();
					tokio::task::spawn_blocking(move || {
						if decode_flag.load(Ordering::Acquire) {
							Ok(None)
						} else {
							read(&path, output_edge).map(Some)
						}
					})
					.await
					.unwrap_or(Err("Image preparation interrupted; choose it again"))
				}
				_ => Ok(None),
			};
			let _ = send.send(if flag.load(Ordering::Acquire) {
				Ok(None)
			} else {
				result
			});
			context.request_repaint();
		});
		self.choosing = Some(Choosing {
			scope,
			result,
			cancelled,
		});
		Ok(())
	}
	pub fn cancel(&self) {
		if let Some(choosing) = &self.choosing {
			choosing.cancelled.store(true, Ordering::Release);
		}
	}
	pub fn poll(&mut self, state: &client_core::State) -> Option<(Scope, Selected)> {
		self.poll_scoped(state.generation, |id| state.is_group_dm(id))
	}
	pub fn poll_server(&mut self, state: &client_core::State) -> Option<(Scope, Selected)> {
		self.poll_scoped(state.generation, |id| {
			state.server_settings.guild == Some(id) && state.can_manage_guild(id)
		})
	}
	pub(crate) fn poll_scoped(
		&mut self,
		generation: u64,
		valid: impl FnOnce(Id) -> bool,
	) -> Option<(Scope, Selected)> {
		let choosing = self.choosing.as_ref()?;
		if choosing.scope.0 != generation || !valid(choosing.scope.1) {
			self.cancel();
		}
		let result = match choosing.result.try_recv() {
			Ok(result) => result,
			Err(mpsc::TryRecvError::Empty) => return None,
			Err(mpsc::TryRecvError::Disconnected) => {
				Err("Image preparation interrupted; choose it again")
			}
		};
		let choosing = self.choosing.take()?;
		(!choosing.cancelled.load(Ordering::Acquire)).then_some((choosing.scope, result))
	}
}
impl Drop for GroupIcon {
	fn drop(&mut self) {
		self.cancel();
	}
}

fn read(path: &Path, output_edge: u32) -> Result<(String, egui::ColorImage), &'static str> {
	let metadata =
		std::fs::symlink_metadata(path).map_err(|_| "Could not open the chosen image")?;
	if !metadata.is_file() || metadata.file_type().is_symlink() {
		return Err("Choose a regular image file");
	}
	if metadata.len() == 0 || metadata.len() > MAX_INPUT as u64 {
		return Err("Choose an image up to 8 MB");
	}
	let file = std::fs::File::open(path).map_err(|_| "Could not open the chosen image")?;
	let mut bytes = Vec::with_capacity(metadata.len() as usize);
	file.take(MAX_INPUT as u64 + 1)
		.read_to_end(&mut bytes)
		.map_err(|_| "Could not read the chosen image")?;
	decode(&bytes, output_edge)
}

fn decode(bytes: &[u8], output_edge: u32) -> Result<(String, egui::ColorImage), &'static str> {
	decode_image(bytes, output_edge, true)
}

pub(crate) fn decode_image(
	bytes: &[u8],
	output_edge: u32,
	square: bool,
) -> Result<(String, egui::ColorImage), &'static str> {
	let (png, preview) = decode_png(bytes, output_edge, square, 256 * 1024)?;
	let uri = discord_protocol::group_actions::icon_data_uri(&png)
		.ok_or("Prepared icon exceeds 256 KB; choose a simpler image")?;
	Ok((uri, preview))
}

pub(crate) fn decode_png(
	bytes: &[u8],
	output_edge: u32,
	square: bool,
	max_output: usize,
) -> Result<(Vec<u8>, egui::ColorImage), &'static str> {
	if bytes.len() > MAX_INPUT {
		return Err("Choose an image up to 8 MB");
	}
	let mut reader = image::ImageReader::new(Cursor::new(bytes))
		.with_guessed_format()
		.map_err(|_| "Choose a PNG, JPEG, GIF or WebP image")?;
	let mut limits = image::Limits::default();
	limits.max_image_width = Some(4096);
	limits.max_image_height = Some(4096);
	limits.max_alloc = Some(64 * 1024 * 1024);
	reader.limits(limits);
	let image = reader
		.decode()
		.map_err(|_| "Image is unsupported or too large; use at most 4096 × 4096 pixels")?;
	let edge = image.width().min(image.height());
	let crop = if square {
		image.view(
			(image.width() - edge) / 2,
			(image.height() - edge) / 2,
			edge,
			edge,
		)
	} else {
		image.view(0, 0, image.width(), image.height())
	};
	let image = image::DynamicImage::ImageRgba8(image::imageops::thumbnail(
		&*crop,
		crop.width().min(output_edge.clamp(1, 512)),
		crop.height().min(output_edge.clamp(1, 512)),
	));
	let mut png = Cursor::new(Vec::new());
	image
		.write_to(&mut png, image::ImageFormat::Png)
		.map_err(|_| "Could not prepare the icon")?;
	if png.get_ref().len() > max_output {
		return Err("Prepared image is too large; choose a simpler image");
	}
	let pixels = image.into_rgba8();
	let preview = egui::ColorImage::from_rgba_unmultiplied(
		[pixels.width() as usize, pixels.height() as usize],
		pixels.as_raw(),
	);
	Ok((png.into_inner(), preview))
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn icon_decode_is_square_bounded_and_rejects_invalid_input() {
		let mut png = Cursor::new(Vec::new());
		image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
			512,
			320,
			image::Rgba([255, 0, 0, 255]),
		))
		.write_to(&mut png, image::ImageFormat::Png)
		.unwrap();
		let (uri, preview) = decode(png.get_ref(), 256).unwrap();
		assert_eq!(preview.size, [256, 256]);
		assert!(uri.starts_with("data:image/png;base64,"));
		assert!(decode(b"invalid", 256).is_err());
		assert!(decode(&vec![0; MAX_INPUT + 1], 256).is_err());
		png = Cursor::new(Vec::new());
		image::DynamicImage::new_rgba8(4097, 1)
			.write_to(&mut png, image::ImageFormat::Png)
			.unwrap();
		assert!(decode(png.get_ref(), 256).is_err());
	}
	#[test]
	fn stale_picker_retains_single_slot_until_completion_and_discards_result() {
		let (send, result) = mpsc::sync_channel(1);
		let mut picker = GroupIcon {
			choosing: Some(Choosing {
				scope: (1, Id(1), 1),
				result,
				cancelled: Arc::new(AtomicBool::new(false)),
			}),
		};
		let state = client_core::State::default();
		assert!(picker.poll(&state).is_none());
		assert!(
			picker
				.choosing
				.as_ref()
				.unwrap()
				.cancelled
				.load(Ordering::Acquire)
		);
		send.send(Err("old result")).unwrap();
		assert!(picker.poll(&state).is_none());
		assert!(picker.choosing.is_none());
		let mut state = test_support::chat_demo_state();
		let mut group = state.channels[0].clone();
		group.id = Id(987);
		group.guild = None;
		group.kind = 3;
		state.channels.push(group);
		state.invalidate_navigation();
		let scope = (state.generation, Id(987), 2);
		let (send, result) = mpsc::sync_channel(1);
		picker.choosing = Some(Choosing {
			scope,
			result,
			cancelled: Arc::new(AtomicBool::new(false)),
		});
		send.send(Ok(None)).unwrap();
		let (received_scope, result) = picker.poll(&state).unwrap();
		assert_eq!(received_scope, scope);
		assert!(result.unwrap().is_none());
	}
}

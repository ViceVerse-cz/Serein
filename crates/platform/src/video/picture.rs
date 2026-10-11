//! Decoded pictures in the decoder's own layout. Backends hand over planar YUV whenever the
//! native decoder produces it, so the renderer can convert colour on the GPU; [`Picture::to_rgba`]
//! is the CPU fallback for posters, tests and renderers without a GPU converter. Both paths
//! use [`Color::coefficients`], so they agree on every pixel up to rounding.

/// Plane layout of [`Picture::data`]; every plane is tightly packed, row after row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
	/// One plane of 4-byte RGBA pixels (alpha is always opaque).
	Rgba,
	/// Full-size luma, then half-size interleaved Cb/Cr pairs.
	Nv12,
	/// Full-size luma, then half-size Cb, then half-size Cr.
	I420,
}

/// YCbCr-to-RGB matrix the stream was encoded with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
	Bt601,
	Bt709,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
	pub matrix: Matrix,
	/// Full (0-255) instead of limited (16-235 luma, 16-240 chroma) sample range.
	pub full_range: bool,
}

impl Color {
	/// The WebRTC default for camera and Go Live H.264: limited-range BT.601.
	pub const WEBRTC: Self = Self {
		matrix: Matrix::Bt601,
		full_range: false,
	};

	/// Untagged streams follow the usual convention: HD frames are BT.709, SD BT.601.
	pub fn untagged(height: u32, full_range: bool) -> Self {
		Self {
			matrix: if height >= 720 {
				Matrix::Bt709
			} else {
				Matrix::Bt601
			},
			full_range,
		}
	}

	/// Rows of `rgb = row.xyz · [y, cb, cr] + row.w` for normalized (0-1) plane samples.
	pub fn coefficients(self) -> [[f32; 4]; 3] {
		let (kr, kb) = match self.matrix {
			Matrix::Bt601 => (0.299, 0.114),
			Matrix::Bt709 => (0.2126, 0.0722),
		};
		let kg = 1.0 - kr - kb;
		let (luma_scale, luma_offset, chroma_scale, chroma_offset) = if self.full_range {
			(1.0, 0.0, 1.0, -128.0 / 255.0)
		} else {
			(255.0 / 219.0, -16.0 / 219.0, 255.0 / 224.0, -128.0 / 224.0)
		};
		let cr_r = 2.0 * (1.0 - kr);
		let cb_b = 2.0 * (1.0 - kb);
		let cb_g = 2.0 * kb * (1.0 - kb) / kg;
		let cr_g = 2.0 * kr * (1.0 - kr) / kg;
		[
			[
				luma_scale,
				0.0,
				cr_r * chroma_scale,
				luma_offset + cr_r * chroma_offset,
			],
			[
				luma_scale,
				-cb_g * chroma_scale,
				-cr_g * chroma_scale,
				luma_offset - (cb_g + cr_g) * chroma_offset,
			],
			[
				luma_scale,
				cb_b * chroma_scale,
				0.0,
				luma_offset + cb_b * chroma_offset,
			],
		]
	}
}

/// One decoded picture. `data` owns or borrows the planes described by `layout`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture<D = Vec<u8>> {
	pub width: u32,
	pub height: u32,
	pub layout: Layout,
	/// Ignored for [`Layout::Rgba`].
	pub color: Color,
	/// Clockwise quarter turns the display applies: 0, 90, 180 or 270.
	pub rotation: u16,
	pub data: D,
}

impl Layout {
	/// Exact byte length of a `width` x `height` picture, or `None` on overflow.
	pub fn len(self, width: u32, height: u32) -> Option<usize> {
		let (width, height) = (width as usize, height as usize);
		let luma = width.checked_mul(height)?;
		let chroma = width.div_ceil(2).checked_mul(height.div_ceil(2))?;
		match self {
			Self::Rgba => luma.checked_mul(4),
			Self::Nv12 | Self::I420 => luma.checked_add(chroma.checked_mul(2)?),
		}
	}
}

impl<D> Picture<D> {
	/// The same picture description over other plane storage.
	pub fn with_data<E>(&self, data: E) -> Picture<E> {
		Picture {
			width: self.width,
			height: self.height,
			layout: self.layout,
			color: self.color,
			rotation: self.rotation,
			data,
		}
	}
}

impl<D: AsRef<[u8]>> Picture<D> {
	/// Dimensions, rotation and plane length agree with each other.
	pub fn is_valid(&self) -> bool {
		self.width > 0
			&& self.height > 0
			&& matches!(self.rotation, 0 | 90 | 180 | 270)
			&& self.layout.len(self.width, self.height) == Some(self.data.as_ref().len())
	}

	/// Size after rotation, as shown on screen.
	pub fn display_size(&self) -> (u32, u32) {
		if matches!(self.rotation, 90 | 270) {
			(self.height, self.width)
		} else {
			(self.width, self.height)
		}
	}

	/// Chroma plane size for the planar layouts.
	pub fn chroma_size(&self) -> (u32, u32) {
		(self.width.div_ceil(2), self.height.div_ceil(2))
	}

	pub fn borrow(&self) -> Picture<&[u8]> {
		Picture {
			width: self.width,
			height: self.height,
			layout: self.layout,
			color: self.color,
			rotation: self.rotation,
			data: self.data.as_ref(),
		}
	}

	/// Converts and rotates to packed display-orientation RGBA: (width, height, pixels).
	pub fn to_rgba(&self) -> Option<(u32, u32, Vec<u8>)> {
		if !self.is_valid() {
			return None;
		}
		let data = self.data.as_ref();
		let (width, height) = (self.width as usize, self.height as usize);
		let rgba = match self.layout {
			Layout::Rgba => data.to_vec(),
			Layout::Nv12 | Layout::I420 => {
				let rows = self.color.coefficients();
				let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
				let (luma, chroma) = data.split_at(width * height);
				let (first, second) = chroma.split_at(chroma_width * chroma_height);
				let mut rgba = vec![0; width * height * 4];
				for (y, row) in rgba.chunks_exact_mut(width * 4).enumerate() {
					for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
						let chroma_at = (y / 2) * chroma_width + x / 2;
						let (cb, cr) = match self.layout {
							Layout::Nv12 => (chroma[chroma_at * 2], chroma[chroma_at * 2 + 1]),
							_ => (first[chroma_at], second[chroma_at]),
						};
						let sample = [
							f32::from(luma[y * width + x]) / 255.0,
							f32::from(cb) / 255.0,
							f32::from(cr) / 255.0,
						];
						let channel = |row: [f32; 4]| {
							let value = row[0] * sample[0]
								+ row[1] * sample[1] + row[2] * sample[2]
								+ row[3];
							(value * 255.0).round().clamp(0.0, 255.0) as u8
						};
						*pixel = [channel(rows[0]), channel(rows[1]), channel(rows[2]), 255];
					}
				}
				rgba
			}
		};
		Some(super::rotate_rgba(
			&rgba,
			self.width,
			self.height,
			u32::from(self.rotation),
		))
	}
}

/// Copies one plane out of a strided buffer into `out`, `row_bytes` per row.
pub fn pack_plane(
	out: &mut Vec<u8>,
	source: &[u8],
	stride: usize,
	row_bytes: usize,
	rows: usize,
) -> Option<()> {
	if stride < row_bytes || rows == 0 {
		return (rows == 0).then_some(());
	}
	let needed = stride.checked_mul(rows - 1)?.checked_add(row_bytes)?;
	let source = source.get(..needed)?;
	if stride == row_bytes {
		out.extend_from_slice(source);
	} else {
		for row in 0..rows {
			out.extend_from_slice(&source[row * stride..][..row_bytes]);
		}
	}
	Some(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn nv12(y: u8, cb: u8, cr: u8, color: Color) -> Picture {
		Picture {
			width: 2,
			height: 2,
			layout: Layout::Nv12,
			color,
			rotation: 0,
			data: vec![y, y, y, y, cb, cr],
		}
	}

	#[test]
	fn limited_and_full_range_reach_black_and_white() {
		let limited = Color::WEBRTC;
		let full = Color {
			matrix: Matrix::Bt709,
			full_range: true,
		};
		let pixel = |picture: Picture| picture.to_rgba().unwrap().2[..4].to_vec();
		assert_eq!(pixel(nv12(16, 128, 128, limited)), [0, 0, 0, 255]);
		assert_eq!(pixel(nv12(235, 128, 128, limited)), [255, 255, 255, 255]);
		assert_eq!(pixel(nv12(0, 128, 128, full)), [0, 0, 0, 255]);
		assert_eq!(pixel(nv12(255, 128, 128, full)), [255, 255, 255, 255]);
		// Limited-range BT.601 red (81, 90, 240) and BT.709 red (63, 102, 240).
		let red = pixel(nv12(81, 90, 240, limited));
		assert!(red[0] >= 253 && red[1] <= 2 && red[2] <= 2, "{red:?}");
		let red = pixel(nv12(
			63,
			102,
			240,
			Color {
				matrix: Matrix::Bt709,
				full_range: false,
			},
		));
		assert!(red[0] >= 253 && red[1] <= 2 && red[2] <= 2, "{red:?}");
	}

	#[test]
	fn planar_layouts_agree_and_rotate() {
		// 3x1 with odd width: two chroma columns, one chroma row.
		let nv12 = Picture {
			width: 3,
			height: 1,
			layout: Layout::Nv12,
			color: Color::WEBRTC,
			rotation: 90,
			data: vec![16, 126, 235, 128, 128, 90, 240],
		};
		let i420 = Picture {
			layout: Layout::I420,
			data: vec![16, 126, 235, 128, 90, 128, 240],
			..nv12.clone()
		};
		assert!(nv12.is_valid() && i420.is_valid());
		assert_eq!(nv12.display_size(), (1, 3));
		let (width, height, rgba) = nv12.to_rgba().unwrap();
		assert_eq!((width, height), (1, 3));
		assert_eq!(i420.to_rgba().unwrap().2, rgba);
		assert_eq!(&rgba[..4], &[0, 0, 0, 255]);
		// Rotation never changes plane validation; bad lengths and turns do.
		assert!(
			!Picture {
				rotation: 45,
				..nv12.clone()
			}
			.is_valid()
		);
		assert!(
			!Picture {
				data: vec![0; 6],
				..nv12
			}
			.is_valid()
		);
		assert_eq!(Layout::Rgba.len(u32::MAX, u32::MAX), None);
	}

	#[test]
	fn strided_planes_pack_tightly() {
		let mut out = Vec::new();
		pack_plane(&mut out, &[1, 2, 0, 3, 4], 3, 2, 2).unwrap();
		assert_eq!(out, [1, 2, 3, 4]);
		assert!(pack_plane(&mut out, &[1, 2, 0, 3], 3, 2, 2).is_none());
		assert!(pack_plane(&mut out, &[1, 2], 1, 2, 1).is_none());
	}
}

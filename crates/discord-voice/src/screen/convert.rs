//! One-pass letterboxed BGRA scaling and BT.601 limited-range 4:2:0 conversion.
//!
//! Screen sources rarely match the stream preset (a 1440p monitor streamed at 1080p), so
//! scaling, color conversion and chroma packing happen together in integer arithmetic,
//! split into row bands across a few threads, and written straight into the encoder's
//! input buffer. Coefficients match openh264's converter, so both encoders look the same.
use super::{RawFrame, validate_frame};

/// Bilinear source tap: byte offsets of the two neighbours and the 8-bit blend weight.
#[derive(Clone, Copy)]
struct Tap {
	near: usize,
	far: usize,
	weight: u32,
}

/// Destination axis mapping into a source axis, with letterbox bounds.
struct Axis {
	taps: Vec<Tap>,
	start: usize,
	end: usize,
}

impl Axis {
	fn new(source: usize, scaled: usize, destination: usize, step: usize) -> Self {
		let start = (destination - scaled) / 2;
		let taps = (0..scaled)
			.map(|index| {
				let position = ((index as f64 + 0.5) * source as f64 / scaled as f64 - 0.5)
					.clamp(0.0, (source - 1) as f64);
				let near = position as usize;
				Tap {
					near: near * step,
					far: (near + 1).min(source - 1) * step,
					weight: ((position - near as f64) * 256.0) as u32,
				}
			})
			.collect();
		Self {
			taps,
			start,
			end: start + scaled,
		}
	}

	fn tap(&self, index: usize) -> Option<Tap> {
		(self.start..self.end)
			.contains(&index)
			.then(|| self.taps[index - self.start])
	}
}

/// Fit `frame` into `width`x`height` 4:2:0 `output` (exactly `width * height * 3 / 2` bytes).
pub(crate) fn bgra_to_i420(
	frame: &RawFrame,
	width: usize,
	height: usize,
	output: &mut [u8],
) -> Result<(), &'static str> {
	validate_frame(frame)?;
	if width == 0
		|| height == 0
		|| width > model::voice_settings::VideoResolution::MAX_WIDTH as usize
		|| height > model::voice_settings::VideoResolution::MAX_HEIGHT as usize
		|| !width.is_multiple_of(2)
		|| !height.is_multiple_of(2)
		|| output.len() != width * height * 3 / 2
	{
		return Err("Invalid screen encoder picture size");
	}
	let (source_width, source_height) = (frame.width as usize, frame.height as usize);
	let scale = (width as f64 / source_width as f64).min(height as f64 / source_height as f64);
	let scaled_width = ((source_width as f64 * scale).round() as usize).clamp(1, width);
	let scaled_height = ((source_height as f64 * scale).round() as usize).clamp(1, height);
	let columns = Axis::new(source_width, scaled_width, width, 4);
	let rows = Axis::new(source_height, scaled_height, height, frame.stride);

	let (luma, chroma_planes) = output.split_at_mut(width * height);
	let bands = std::thread::available_parallelism()
		.map_or(1, |count| count.get())
		.clamp(1, 4)
		.min(height / 64)
		.max(1);
	// Even row counts keep every chroma row inside one band.
	let band_rows = height.div_ceil(bands).next_multiple_of(2);
	let (mut u, mut v) = chroma_planes.split_at_mut(width * height / 4);
	std::thread::scope(|scope| {
		let mut luma = luma;
		let mut first = 0;
		while first < height {
			let count = band_rows.min(height - first);
			let (band_luma, rest) = luma.split_at_mut(count * width);
			luma = rest;
			let chroma_bytes = count / 2 * width / 2;
			let (band_u, rest_u) = std::mem::take(&mut u).split_at_mut(chroma_bytes);
			let (band_v, rest_v) = std::mem::take(&mut v).split_at_mut(chroma_bytes);
			(u, v) = (rest_u, rest_v);
			let job = Band {
				source: &frame.data,
				columns: &columns,
				rows: &rows,
				width,
				first,
				luma: band_luma,
				u: band_u,
				v: band_v,
			};
			if first + count >= height {
				job.run();
			} else {
				scope.spawn(move || job.run());
			}
			first += count;
		}
	});
	Ok(())
}

struct Band<'a> {
	source: &'a [u8],
	columns: &'a Axis,
	rows: &'a Axis,
	width: usize,
	first: usize,
	luma: &'a mut [u8],
	u: &'a mut [u8],
	v: &'a mut [u8],
}

impl Band<'_> {
	fn run(self) {
		let width = self.width;
		let mut top = vec![[0u32; 3]; width];
		let mut bottom = vec![[0u32; 3]; width];
		for pair in 0..self.luma.len() / width / 2 {
			let row = self.first + pair * 2;
			self.sample(row, &mut top);
			self.sample(row + 1, &mut bottom);
			let offset = pair * 2 * width;
			for (x, rgb) in top.iter().enumerate() {
				self.luma[offset + x] = luma(*rgb);
			}
			for (x, rgb) in bottom.iter().enumerate() {
				self.luma[offset + width + x] = luma(*rgb);
			}
			let chroma_row = pair * width / 2;
			for x in 0..width / 2 {
				let mut sum = [0u32; 3];
				for rgb in [top[2 * x], top[2 * x + 1], bottom[2 * x], bottom[2 * x + 1]] {
					for (total, channel) in sum.iter_mut().zip(rgb) {
						*total += channel;
					}
				}
				let (u, v) = chroma(sum.map(|total| (total + 2) / 4));
				self.u[chroma_row + x] = u;
				self.v[chroma_row + x] = v;
			}
		}
	}

	/// One destination row as RGB, black outside the letterboxed picture.
	fn sample(&self, row: usize, out: &mut [[u32; 3]]) {
		let Some(y) = self.rows.tap(row) else {
			out.fill([0; 3]);
			return;
		};
		let (near, far) = (&self.source[y.near..], &self.source[y.far..]);
		for (column, rgb) in out.iter_mut().enumerate() {
			let Some(x) = self.columns.tap(column) else {
				*rgb = [0; 3];
				continue;
			};
			for (channel, value) in rgb.iter_mut().enumerate() {
				// BGRA: red is byte 2, green 1, blue 0.
				let byte = 2 - channel;
				let lerp = |line: &[u8]| {
					let a = u32::from(line[x.near + byte]);
					if x.weight == 0 {
						return a << 8;
					}
					a * (256 - x.weight) + u32::from(line[x.far + byte]) * x.weight
				};
				let upper = lerp(near);
				*value = if y.weight == 0 {
					(upper + 128) >> 8
				} else {
					(upper * (256 - y.weight) + lerp(far) * y.weight + 32_768) >> 16
				};
			}
		}
	}
}

fn luma([r, g, b]: [u32; 3]) -> u8 {
	(((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8
}

fn chroma([r, g, b]: [u32; 3]) -> (u8, u8) {
	let (r, g, b) = (r as i32, g as i32, b as i32);
	let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
	let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
	(u.clamp(0, 255) as u8, v.clamp(0, 255) as u8)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn solid(width: u32, height: u32, stride: usize, bgra: [u8; 4]) -> RawFrame {
		let mut data = vec![0xee; stride * height as usize];
		for row in data.chunks_exact_mut(stride) {
			for pixel in row[..width as usize * 4].as_chunks_mut::<4>().0 {
				pixel.copy_from_slice(&bgra);
			}
		}
		RawFrame {
			width,
			height,
			stride,
			data,
		}
	}

	#[test]
	fn scales_letterboxes_and_matches_bt601_limited_range() {
		// Padded 4:3 red source into 16:9: red picture with black pillars.
		let frame = solid(64, 48, 64 * 4 + 12, [0, 0, 255, 255]);
		let (width, height) = (128, 72);
		let mut out = vec![0; width * height * 3 / 2];
		bgra_to_i420(&frame, width, height, &mut out).unwrap();
		let (y, uv) = out.split_at(width * height);
		assert_eq!(y[36 * width + 64], 82);
		assert_eq!(y[36 * width], 16);
		assert_eq!(
			(uv[18 * 64 + 32], uv[width * height / 4 + 18 * 64 + 32]),
			(90, 240)
		);
		assert_eq!((uv[18 * 64], uv[width * height / 4 + 18 * 64]), (128, 128));
		// Identity size keeps a sharp one-pixel edge.
		let mut frame = solid(4, 2, 16, [0, 0, 0, 255]);
		frame.data[4..8].copy_from_slice(&[255, 255, 255, 255]);
		let mut out = vec![0; 12];
		bgra_to_i420(&frame, 4, 2, &mut out).unwrap();
		assert_eq!(&out[..4], &[16, 235, 16, 16]);
		assert!(bgra_to_i420(&frame, 3, 2, &mut out).is_err());
		assert!(bgra_to_i420(&frame, 4, 2, &mut out[..11]).is_err());

		{
			let (width, height) = (320, 180);
			let mut frame = solid(333, 250, 333 * 4, [0; 4]);
			for (index, byte) in frame.data.iter_mut().enumerate() {
				*byte = (index * 31 % 251) as u8;
			}
			let mut banded = vec![0; width * height * 3 / 2];
			bgra_to_i420(&frame, width, height, &mut banded).unwrap();
			let columns = Axis::new(333, 240, width, 4);
			let rows = Axis::new(250, 180, height, frame.stride);
			let mut single = vec![0; width * height * 3 / 2];
			let (luma, uv) = single.split_at_mut(width * height);
			let (u, v) = uv.split_at_mut(width * height / 4);
			Band {
				source: &frame.data,
				columns: &columns,
				rows: &rows,
				width,
				first: 0,
				luma,
				u,
				v,
			}
			.run();
			assert!(banded == single);
		}
	}
}

//! Offline decoder check: rotate only the synthetic MOV's track matrix, never its pixels.
//! Run: cargo run --locked -p serein --example video_orientation
#[cfg(target_os = "windows")]
fn main() {
	use platform::video::{Decoder, Sample};
	let original = include_bytes!("../tests/fixtures/video.mov");
	let decode = |bytes: Vec<u8>| {
		let mut decoder = Decoder::open(Box::new(std::io::Cursor::new(bytes))).unwrap();
		let Some(Sample::Video {
			width,
			height,
			rgba,
			..
		}) = decoder.read_video().unwrap()
		else {
			panic!("Missing synthetic video frame");
		};
		(width as usize, height as usize, rgba)
	};
	let (w, h, original_rgba) = decode(original.to_vec());
	// This test pattern has purple at the top left and red at the bottom left.
	// Check upright pixels too: comparing rotated frames alone would miss a shared flip.
	assert!(
		original_rgba[2] > 220 && original_rgba[0] < 180,
		"Top left must be purple"
	);
	let bottom = (h - 1) * w * 4;
	assert!(
		original_rgba[bottom] > 220 && original_rgba[bottom + 2] < 20,
		"Bottom left must be red"
	);
	let track = original
		.windows(4)
		.enumerate()
		.find_map(|(offset, tag)| {
			(tag == b"tkhd"
				&& offset >= 4
				&& offset + 88 <= original.len()
				&& original[offset + 80..offset + 84] == (320_u32 << 16).to_be_bytes())
			.then(|| offset - 4)
		})
		.expect("Synthetic video track");
	const ONE: i32 = 1 << 16;
	for (turn, matrix) in [
		(0, [ONE, 0, 0, 0, ONE, 0, 0, 0, 1 << 30]),
		(90, [0, ONE, 0, -ONE, 0, 0, 0, 0, 1 << 30]),
		(180, [-ONE, 0, 0, 0, -ONE, 0, 0, 0, 1 << 30]),
		(270, [0, -ONE, 0, ONE, 0, 0, 0, 0, 1 << 30]),
	] {
		let mut bytes = original.to_vec();
		for (word, value) in bytes[track + 48..track + 84]
			.as_chunks_mut::<4>()
			.0
			.iter_mut()
			.zip(matrix)
		{
			word.copy_from_slice(&value.to_be_bytes());
		}
		let (width, height, rgba) = decode(bytes);
		assert_eq!(
			(width, height),
			if turn == 90 || turn == 270 {
				(h, w)
			} else {
				(w, h)
			}
		);
		let mut error = 0_u64;
		for y in 0..h {
			for x in 0..w {
				// MOV's display matrix maps these coordinates clockwise.
				let (dx, dy) = match turn {
					90 => (h - 1 - y, x),
					180 => (w - 1 - x, h - 1 - y),
					270 => (y, w - 1 - x),
					_ => (x, y),
				};
				let source = (y * w + x) * 4;
				let target = (dy * width + dx) * 4;
				for channel in 0..3 {
					error +=
						u64::from(rgba[target + channel].abs_diff(original_rgba[source + channel]));
				}
			}
		}
		// Separate native conversions can round chroma differently; a flipped/rotated
		// test pattern differs far beyond this small mean RGB tolerance.
		let mean_error = error as f64 / (w * h * 3) as f64;
		assert!(
			mean_error < 3.0,
			"{turn}-degree orientation: mean RGB error {mean_error}"
		);
	}
	println!("Native Windows MOV orientation passed: upright rows and 0, 90, 180 and 270 degrees.");
}

#[cfg(not(target_os = "windows"))]
fn main() {
	eprintln!("This check exercises the Windows Media Foundation rotation convention.");
}

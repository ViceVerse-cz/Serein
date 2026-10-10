use ecolor::Color32;
use std::{hint::black_box, time::Instant};
fn main() {
	let rgba: Vec<u8> = (0..1920 * 1080)
		.flat_map(|i| [i as u8, (i / 8) as u8, 128, 255])
		.collect();
	let mut pixels = vec![Color32::BLACK; 1920 * 1080];
	for _ in 0..3 {
		for mode in 0..3 {
			let start = Instant::now();
			for _ in 0..300 {
				let rgba = black_box(&rgba);
				match mode {
					0 => {
						pixels.clear();
						pixels.extend(
							rgba.as_chunks::<4>()
								.0
								.iter()
								.map(|&[r, g, b, a]| Color32::from_rgba_unmultiplied(r, g, b, a)),
						);
					}
					1 => {
						pixels.resize(rgba.len() / 4, Color32::BLACK);
						if rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255) {
							bytemuck::cast_slice_mut(&mut pixels).copy_from_slice(rgba);
						} else {
							panic!();
						}
					}
					_ => {
						pixels.resize(rgba.len() / 4, Color32::BLACK);
						bytemuck::cast_slice_mut(&mut pixels).copy_from_slice(rgba);
						for pixel in &mut pixels {
							if pixel.a() != 255 {
								let [r, g, b, a] = pixel.to_array();
								*pixel = Color32::from_rgba_unmultiplied(r, g, b, a);
							}
						}
					}
				}
				black_box(&pixels);
			}
			println!(
				"mode={mode} ms={:.3}",
				start.elapsed().as_secs_f64() * 1000.
			);
		}
	}
}

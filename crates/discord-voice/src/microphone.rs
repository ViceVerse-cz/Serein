//! Microphone Opus settings that follow the user's quality choice and the call channel.
use model::voice_settings::MicrophoneQuality;
use opus2::{Bitrate, Channels, Encoder, Signal};

/// Loss Opus plans for when in-band FEC is on; low enough to keep speech quality.
const EXPECTED_LOSS_PERCENT: i32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Encoding {
	quality: MicrophoneQuality,
	bitrate: u32,
}
impl Encoding {
	pub(crate) fn new(quality: MicrophoneQuality, channel_bitrate: Option<u32>) -> Self {
		Self {
			quality,
			bitrate: quality.bitrate(channel_bitrate),
		}
	}

	/// Configure an encoder created for stereo 48 kHz. Opus accepts these between frames, so a
	/// call can switch without a new encoder.
	pub(crate) fn apply(self, encoder: &mut Encoder) -> Result<(), &'static str> {
		let maximum = self.quality == MicrophoneQuality::Maximum;
		// Within 8_000..=510_000, so the cast is exact.
		encoder
			.set_bitrate(Bitrate::Bits(self.bitrate as i32))
			.map_err(|_| "Opus bitrate configuration failed")?;
		// Maximum keeps upstream's constant 1,275-byte packets; the channel mode varies the
		// rate with the signal, as Discord's client does, and stays near the target.
		encoder
			.set_vbr(!maximum)
			.map_err(|_| "Opus VBR configuration failed")?;
		encoder
			.set_vbr_constraint(!maximum)
			.map_err(|_| "Opus VBR constraint configuration failed")?;
		encoder
			.set_signal(if maximum { Signal::Music } else { Signal::Auto })
			.map_err(|_| "Opus signal configuration failed")?;
		// Capture is mono. Dual-mono stereo only pays off at the maximum rate.
		encoder
			.set_force_channels(Some(if maximum {
				Channels::Stereo
			} else {
				Channels::Mono
			}))
			.map_err(|_| "Opus channel configuration failed")?;
		encoder
			.set_inband_fec(!maximum)
			.map_err(|_| "Opus FEC configuration failed")?;
		encoder
			.set_packet_loss_perc(if maximum { 0 } else { EXPECTED_LOSS_PERCENT })
			.map_err(|_| "Opus packet-loss configuration failed")?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use opus2::{Application, Decoder};

	fn encoder() -> Encoder {
		Encoder::new(48_000, Channels::Stereo, Application::Audio).unwrap()
	}

	/// Encodes 20 frames of a steady tone and returns the last 10 packets, after rate control
	/// has settled.
	fn packets(encoder: &mut Encoder) -> Vec<Vec<u8>> {
		let input = std::array::from_fn::<_, 1920, _>(|i| ((i / 2) as f32 * 0.06).sin() * 0.2);
		let mut encoded = [0; 1275];
		(0..20)
			.map(|_| {
				let length = encoder.encode_float(&input, &mut encoded).unwrap();
				encoded[..length].to_vec()
			})
			.skip(10)
			.collect()
	}

	fn packet(encoding: Encoding) -> Vec<u8> {
		let mut encoder = encoder();
		encoding.apply(&mut encoder).unwrap();
		packets(&mut encoder).pop().unwrap()
	}

	fn average(packets: &[Vec<u8>]) -> usize {
		packets.iter().map(Vec::len).sum::<usize>() / packets.len()
	}

	#[test]
	fn channel_mode_sends_mono_near_the_channel_rate_and_maximum_keeps_stereo_cbr() {
		let channel = packet(Encoding::new(MicrophoneQuality::Channel, Some(64_000)));
		assert_eq!(
			opus2::packet::get_nb_channels(&channel).unwrap(),
			Channels::Mono
		);
		// 64 kb/s is 160 bytes per 20 ms frame; constrained VBR stays well below the maximum.
		assert!(channel.len() < 400, "{} bytes", channel.len());
		let maximum = packet(Encoding::new(MicrophoneQuality::Maximum, Some(64_000)));
		assert_eq!(
			opus2::packet::get_nb_channels(&maximum).unwrap(),
			Channels::Stereo
		);
		assert_eq!(maximum.len(), 1275);
		// Peers decode either form with the same stereo or mono decoder.
		let mut decoder = Decoder::new(48_000, Channels::Mono).unwrap();
		let mut out = [0.0; 960];
		for packet in [channel, maximum] {
			assert_eq!(decoder.decode_float(&packet, &mut out, false).unwrap(), 960);
		}
	}

	/// A call keeps one encoder, so quality and channel-bitrate changes must apply in place.
	#[test]
	fn one_encoder_switches_modes_and_bitrates_during_a_call() {
		let mut encoder = encoder();
		let mut decoder = Decoder::new(48_000, Channels::Stereo).unwrap();
		let mut out = [0.0; 1920];
		let mut step = |encoding: Encoding| {
			encoding.apply(&mut encoder).unwrap();
			let packets = packets(&mut encoder);
			for packet in &packets {
				assert_eq!(decoder.decode_float(packet, &mut out, false).unwrap(), 960);
			}
			packets
		};
		let low = step(Encoding::new(MicrophoneQuality::Channel, Some(32_000)));
		let high = step(Encoding::new(MicrophoneQuality::Channel, Some(128_000)));
		let maximum = step(Encoding::new(MicrophoneQuality::Maximum, Some(128_000)));
		let back = step(Encoding::new(MicrophoneQuality::Channel, Some(64_000)));

		for packet in low.iter().chain(&high).chain(&back) {
			assert_eq!(
				opus2::packet::get_nb_channels(packet).unwrap(),
				Channels::Mono
			);
		}
		// 32 and 128 kb/s are 80 and 320 bytes per 20 ms frame.
		assert!(
			average(&high) > 2 * average(&low),
			"{} vs {} bytes",
			average(&high),
			average(&low)
		);
		for packet in &maximum {
			assert_eq!(
				opus2::packet::get_nb_channels(packet).unwrap(),
				Channels::Stereo
			);
			assert_eq!(packet.len(), 1275);
		}
		assert!(average(&back) < 400, "{} bytes", average(&back));
	}

	#[test]
	fn a_known_channel_bitrate_changes_the_encoding() {
		assert_ne!(
			Encoding::new(MicrophoneQuality::Channel, None),
			Encoding::new(MicrophoneQuality::Channel, Some(96_000))
		);
		assert_eq!(
			Encoding::new(MicrophoneQuality::Maximum, None),
			Encoding::new(MicrophoneQuality::Maximum, Some(96_000))
		);
	}
}

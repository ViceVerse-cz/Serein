//! Bounded MPEG-4 / QuickTime demuxer for the macOS decoder. Only the `moov` sample tables
//! are read into memory; media bytes stay in the caller's seekable stream. Every table length
//! is capped, every offset is range-checked and nothing here can allocate from a size field
//! alone. Fragmented files, external data references and unknown codecs are rejected.
use super::{INVALID, MAX_BYTES, MAX_SECONDS, ReadSeek, TOO_LONG, UNSUPPORTED};
use std::io::SeekFrom;

const MAX_MOOV: u64 = 32 * 1024 * 1024;
const MAX_TOP_LEVEL_BOXES: usize = 256;
const MAX_SAMPLES: usize = 1_500_000;
const MAX_TABLE_ENTRIES: usize = MAX_SAMPLES;
const MAX_PARAMETER_SETS: usize = 64;
const MAX_PARAMETER_SET_BYTES: usize = 64 * 1024;

pub(super) enum VideoCodec {
	H264 {
		nal_length: u8,
		parameter_sets: Vec<Vec<u8>>,
	},
	Hevc {
		nal_length: u8,
		parameter_sets: Vec<Vec<u8>>,
	},
}

pub(super) enum AudioCodec {
	/// MPEG-4 AAC with its AudioSpecificConfig.
	Aac { config: Vec<u8> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SampleEntry {
	pub offset: u64,
	pub size: u32,
	/// Decode time in track ticks.
	pub dts: u64,
	/// Presentation time in track ticks (decode time plus composition offset).
	pub pts: i64,
	pub sync: bool,
}

pub(super) struct Track<C> {
	pub codec: C,
	pub timescale: u32,
	pub samples: Vec<SampleEntry>,
}
impl<C> Track<C> {
	pub fn seconds(&self, ticks: i64) -> f64 {
		ticks as f64 / f64::from(self.timescale.max(1))
	}
	pub fn ticks(&self, seconds: f64) -> i64 {
		(seconds * f64::from(self.timescale)).round() as i64
	}
}

pub(super) struct VideoTrack {
	pub track: Track<VideoCodec>,
	pub width: u32,
	pub height: u32,
	pub rotation: u32,
}

pub(super) struct AudioTrack {
	pub track: Track<AudioCodec>,
	pub sample_rate: u32,
	pub channels: u16,
}

pub(super) struct Movie {
	pub duration: f64,
	pub video: VideoTrack,
	pub audio: Option<AudioTrack>,
}

/// Locate and parse the movie header. `length` is the total stream length in bytes.
pub(super) fn parse(source: &mut dyn ReadSeek, length: u64) -> Result<Movie, &'static str> {
	let mut offset = 0_u64;
	let mut moov = None;
	for _ in 0..MAX_TOP_LEVEL_BOXES {
		if offset + 8 > length {
			break;
		}
		source.seek(SeekFrom::Start(offset)).map_err(|_| INVALID)?;
		let mut header = [0_u8; 16];
		source.read_exact(&mut header[..8]).map_err(|_| INVALID)?;
		let mut size = u64::from(u32::from_be_bytes([
			header[0], header[1], header[2], header[3],
		]));
		let kind = [header[4], header[5], header[6], header[7]];
		let mut header_len = 8_u64;
		if size == 1 {
			source.read_exact(&mut header[8..16]).map_err(|_| INVALID)?;
			size = u64::from_be_bytes(header[8..16].try_into().map_err(|_| INVALID)?);
			header_len = 16;
		} else if size == 0 {
			size = length - offset;
		}
		if size < header_len || offset + size > length {
			return Err(INVALID);
		}
		match &kind {
			b"moov" => {
				let body = size - header_len;
				if body > MAX_MOOV {
					return Err(INVALID);
				}
				let mut bytes = vec![0; body as usize];
				source.read_exact(&mut bytes).map_err(|_| INVALID)?;
				moov = Some(bytes);
				break;
			}
			b"moof" | b"sidx" | b"mvex" => return Err(UNSUPPORTED),
			b"ftyp" | b"mdat" | b"free" | b"skip" | b"wide" | b"uuid" | b"meta" | b"pdin"
			| b"udta" => {}
			_ if offset == 0 => return Err(UNSUPPORTED),
			_ => {}
		}
		offset += size;
	}
	let moov = moov.ok_or(UNSUPPORTED)?;
	parse_moov(&moov, length)
}

fn parse_moov(moov: &[u8], length: u64) -> Result<Movie, &'static str> {
	let mut movie_timescale = 0_u32;
	let mut movie_duration = 0_u64;
	let mut video = None;
	let mut audio = None;
	for (kind, body) in Boxes::new(moov) {
		match &kind {
			b"mvhd" => {
				let (timescale, duration) = parse_header_times(body)?;
				movie_timescale = timescale;
				movie_duration = duration;
			}
			b"mvex" => return Err(UNSUPPORTED),
			b"trak" => {
				if let Some(track) = parse_trak(body, length)? {
					match track {
						Parsed::Video(track) if video.is_none() => video = Some(track),
						Parsed::Audio(track) if audio.is_none() => audio = Some(track),
						_ => {}
					}
				}
			}
			_ => {}
		}
	}
	let video = video.ok_or(UNSUPPORTED)?;
	if movie_timescale == 0 {
		return Err(INVALID);
	}
	let mut duration = movie_duration as f64 / f64::from(movie_timescale);
	// Some muxers leave the movie duration at zero; fall back to the video track's span.
	if duration <= 0.0 {
		duration = video
			.track
			.samples
			.last()
			.map(|last| video.track.seconds(last.pts) + video.track.seconds(1))
			.unwrap_or(0.0);
	}
	if !duration.is_finite() || duration <= 0.0 {
		return Err(INVALID);
	}
	if duration > MAX_SECONDS {
		return Err(TOO_LONG);
	}
	Ok(Movie {
		duration,
		video,
		audio,
	})
}

enum Parsed {
	Video(VideoTrack),
	Audio(AudioTrack),
}

fn parse_trak(trak: &[u8], length: u64) -> Result<Option<Parsed>, &'static str> {
	let mut rotation = 0;
	let mut handler = [0_u8; 4];
	let mut timescale = 0_u32;
	let mut stbl = None;
	for (kind, body) in Boxes::new(trak) {
		match &kind {
			b"tkhd" => rotation = parse_tkhd_rotation(body)?,
			b"mdia" => {
				for (kind, body) in Boxes::new(body) {
					match &kind {
						b"mdhd" => timescale = parse_header_times(body)?.0,
						b"hdlr" => {
							handler = body
								.get(8..12)
								.and_then(|h| h.try_into().ok())
								.ok_or(INVALID)?;
						}
						b"minf" => {
							for (kind, body) in Boxes::new(body) {
								match &kind {
									b"stbl" => stbl = Some(body),
									b"dinf" => check_self_contained(body)?,
									_ => {}
								}
							}
						}
						_ => {}
					}
				}
			}
			_ => {}
		}
	}
	let Some(stbl) = stbl else {
		return Ok(None);
	};
	let is_video = &handler == b"vide";
	let is_audio = &handler == b"soun";
	if !is_video && !is_audio {
		return Ok(None);
	}
	if timescale == 0 {
		return Err(INVALID);
	}
	let tables = parse_stbl(stbl)?;
	let samples = build_samples(&tables, length)?;
	if samples.is_empty() {
		return Err(INVALID);
	}
	let stsd = tables.stsd.ok_or(INVALID)?;
	if is_video {
		let (codec, width, height) = parse_visual_entry(stsd)?;
		return Ok(Some(Parsed::Video(VideoTrack {
			track: Track {
				codec,
				timescale,
				samples,
			},
			width,
			height,
			rotation,
		})));
	}
	let Some((codec, sample_rate, channels)) = parse_audio_entry(stsd)? else {
		return Err(UNSUPPORTED);
	};
	Ok(Some(Parsed::Audio(AudioTrack {
		track: Track {
			codec,
			timescale,
			samples,
		},
		sample_rate,
		channels,
	})))
}

/// Data references must point at this file; URL/URN entries would mean external media.
fn check_self_contained(dinf: &[u8]) -> Result<(), &'static str> {
	for (kind, body) in Boxes::new(dinf) {
		if &kind == b"dref" {
			let entries = Boxes::new(body.get(8..).unwrap_or(&[]));
			for (kind, body) in entries {
				let flags = body.get(1..4).ok_or(INVALID)?;
				let self_contained = flags[2] & 1 == 1;
				if !self_contained || (&kind != b"url " && &kind != b"alis" && &kind != b"urn ") {
					return Err(UNSUPPORTED);
				}
			}
		}
	}
	Ok(())
}

/// Shared mvhd/mdhd layout: returns (timescale, duration).
fn parse_header_times(body: &[u8]) -> Result<(u32, u64), &'static str> {
	let version = *body.first().ok_or(INVALID)?;
	match version {
		0 => Ok((read_u32(body, 12)?, u64::from(read_u32(body, 16)?))),
		1 => Ok((read_u32(body, 20)?, read_u64(body, 24)?)),
		_ => Err(UNSUPPORTED),
	}
}

fn parse_tkhd_rotation(body: &[u8]) -> Result<u32, &'static str> {
	let version = *body.first().ok_or(INVALID)?;
	let matrix = match version {
		0 => 40,
		1 => 52,
		_ => return Err(UNSUPPORTED),
	};
	let fixed = |index: usize| read_u32(body, matrix + index * 4).map(|v| v as i32);
	let (a, b, c, d) = (fixed(0)?, fixed(1)?, fixed(3)?, fixed(4)?);
	const ONE: i32 = 1 << 16;
	const NEG: i32 = -ONE;
	Ok(match (a, b, c, d) {
		(ONE, 0, 0, ONE) => 0,
		(0, ONE, NEG, 0) => 90,
		(NEG, 0, 0, NEG) => 180,
		(0, NEG, ONE, 0) => 270,
		_ => return Err(UNSUPPORTED),
	})
}

#[derive(Default)]
struct Tables<'a> {
	stsd: Option<&'a [u8]>,
	stts: Vec<(u32, u32)>,
	ctts: Vec<(u32, i32)>,
	stsc: Vec<(u32, u32)>,
	sizes: Sizes,
	chunks: Vec<u64>,
	sync: Option<Vec<u32>>,
}
#[derive(Default)]
enum Sizes {
	#[default]
	Missing,
	Fixed(u32, u32),
	Table(Vec<u32>),
}

fn parse_stbl(stbl: &[u8]) -> Result<Tables<'_>, &'static str> {
	let mut tables = Tables::default();
	for (kind, body) in Boxes::new(stbl) {
		match &kind {
			b"stsd" => tables.stsd = Some(body),
			b"stts" => {
				tables.stts = read_entries(body, 8, |b, at| {
					Ok((read_u32(b, at)?, read_u32(b, at + 4)?))
				})?;
			}
			b"ctts" => {
				tables.ctts = read_entries(body, 8, |b, at| {
					Ok((read_u32(b, at)?, read_u32(b, at + 4)? as i32))
				})?;
			}
			b"stsc" => {
				tables.stsc = read_entries(body, 12, |b, at| {
					Ok((read_u32(b, at)?, read_u32(b, at + 4)?))
				})?;
			}
			b"stsz" => {
				let fixed = read_u32(body, 4)?;
				let count = read_u32(body, 8)?;
				tables.sizes = if fixed != 0 {
					Sizes::Fixed(fixed, count)
				} else {
					Sizes::Table(read_entries(&body[4..], 4, read_u32)?)
				};
			}
			b"stz2" => return Err(UNSUPPORTED),
			b"stco" => {
				tables.chunks = read_entries(body, 4, |b, at| read_u32(b, at).map(u64::from))?;
			}
			b"co64" => tables.chunks = read_entries(body, 8, read_u64)?,
			b"stss" => tables.sync = Some(read_entries(body, 4, read_u32)?),
			_ => {}
		}
	}
	Ok(tables)
}

/// Full-box table: version/flags (4), entry count (4), fixed-size entries.
fn read_entries<T>(
	body: &[u8],
	entry_size: usize,
	read: impl Fn(&[u8], usize) -> Result<T, &'static str>,
) -> Result<Vec<T>, &'static str> {
	let count = read_u32(body, 4)? as usize;
	if count > MAX_TABLE_ENTRIES || body.len() < 8 + count * entry_size {
		return Err(INVALID);
	}
	(0..count).map(|i| read(body, 8 + i * entry_size)).collect()
}

fn build_samples(tables: &Tables<'_>, length: u64) -> Result<Vec<SampleEntry>, &'static str> {
	let count = match &tables.sizes {
		Sizes::Missing => return Err(INVALID),
		Sizes::Fixed(_, count) => *count as usize,
		Sizes::Table(sizes) => sizes.len(),
	};
	if count == 0 || count > MAX_SAMPLES || tables.chunks.is_empty() || tables.stsc.is_empty() {
		return Err(INVALID);
	}
	let size_of = |index: usize| -> Result<u32, &'static str> {
		let size = match &tables.sizes {
			Sizes::Fixed(size, _) => *size,
			Sizes::Table(sizes) => *sizes.get(index).ok_or(INVALID)?,
			Sizes::Missing => return Err(INVALID),
		};
		if size as usize > MAX_BYTES {
			return Err(INVALID);
		}
		Ok(size)
	};
	let mut samples = Vec::with_capacity(count);
	// Chunk runs: entry i applies from first_chunk(i) until first_chunk(i + 1).
	let mut index = 0;
	'chunks: for (run, &(first_chunk, per_chunk)) in tables.stsc.iter().enumerate() {
		let next_first = tables
			.stsc
			.get(run + 1)
			.map_or(tables.chunks.len() as u32 + 1, |next| next.0);
		if first_chunk == 0 || next_first <= first_chunk || per_chunk == 0 {
			return Err(INVALID);
		}
		for chunk in first_chunk..next_first {
			let mut offset = *tables.chunks.get(chunk as usize - 1).ok_or(INVALID)?;
			for _ in 0..per_chunk {
				if index == count {
					break 'chunks;
				}
				let size = size_of(index)?;
				if offset
					.checked_add(u64::from(size))
					.is_none_or(|end| end > length)
				{
					return Err(INVALID);
				}
				samples.push(SampleEntry {
					offset,
					size,
					dts: 0,
					pts: 0,
					sync: true,
				});
				offset += u64::from(size);
				index += 1;
			}
		}
	}
	if samples.len() != count {
		return Err(INVALID);
	}
	let mut dts = 0_u64;
	let mut index = 0;
	for &(run, delta) in &tables.stts {
		for _ in 0..run {
			let Some(sample) = samples.get_mut(index) else {
				break;
			};
			sample.dts = dts;
			sample.pts = i64::try_from(dts).map_err(|_| INVALID)?;
			dts = dts.checked_add(u64::from(delta)).ok_or(INVALID)?;
			index += 1;
		}
	}
	if index != count {
		return Err(INVALID);
	}
	let mut index = 0;
	for &(run, offset) in &tables.ctts {
		for _ in 0..run {
			let Some(sample) = samples.get_mut(index) else {
				break;
			};
			sample.pts = sample.pts.checked_add(i64::from(offset)).ok_or(INVALID)?;
			index += 1;
		}
	}
	if !tables.ctts.is_empty() && index != count {
		return Err(INVALID);
	}
	if let Some(sync) = &tables.sync {
		for sample in &mut samples {
			sample.sync = false;
		}
		for &number in sync {
			let Some(sample) = number
				.checked_sub(1)
				.and_then(|i| samples.get_mut(i as usize))
			else {
				return Err(INVALID);
			};
			sample.sync = true;
		}
		if !samples[0].sync {
			return Err(INVALID);
		}
	}
	Ok(samples)
}

fn parse_visual_entry(stsd: &[u8]) -> Result<(VideoCodec, u32, u32), &'static str> {
	let entries = Boxes::new(stsd.get(8..).ok_or(INVALID)?);
	for (kind, body) in entries {
		// Visual sample entry: 6 reserved, data-reference index, 16 predefined/reserved bytes,
		// width, height, then resolution/frame count/name/depth fields before child boxes.
		let width = u32::from(read_u16(body, 24)?);
		let height = u32::from(read_u16(body, 26)?);
		let children = body.get(78..).ok_or(INVALID)?;
		let codec = match &kind {
			b"avc1" | b"avc3" => {
				let avcc = find_box(children, b"avcC").ok_or(UNSUPPORTED)?;
				parse_avcc(avcc)?
			}
			b"hvc1" | b"hev1" => {
				let hvcc = find_box(children, b"hvcC").ok_or(UNSUPPORTED)?;
				parse_hvcc(hvcc)?
			}
			b"encv" => return Err(UNSUPPORTED),
			_ => continue,
		};
		super::check_dimensions(width, height)?;
		return Ok((codec, width, height));
	}
	Err(UNSUPPORTED)
}

fn parse_avcc(avcc: &[u8]) -> Result<VideoCodec, &'static str> {
	if *avcc.first().ok_or(INVALID)? != 1 {
		return Err(UNSUPPORTED);
	}
	let nal_length = (avcc.get(4).ok_or(INVALID)? & 3) + 1;
	let mut parameter_sets = Vec::new();
	let mut at = 6;
	let sps_count = usize::from(avcc.get(5).ok_or(INVALID)? & 0x1f);
	at = read_parameter_sets(avcc, at, sps_count, &mut parameter_sets)?;
	let pps_count = usize::from(*avcc.get(at).ok_or(INVALID)?);
	read_parameter_sets(avcc, at + 1, pps_count, &mut parameter_sets)?;
	if sps_count == 0 || pps_count == 0 {
		return Err(UNSUPPORTED);
	}
	Ok(VideoCodec::H264 {
		nal_length,
		parameter_sets,
	})
}

fn parse_hvcc(hvcc: &[u8]) -> Result<VideoCodec, &'static str> {
	if *hvcc.first().ok_or(INVALID)? != 1 {
		return Err(UNSUPPORTED);
	}
	let nal_length = (hvcc.get(21).ok_or(INVALID)? & 3) + 1;
	let arrays = usize::from(*hvcc.get(22).ok_or(INVALID)?);
	let mut parameter_sets = Vec::new();
	let mut at = 23;
	for _ in 0..arrays {
		let count = usize::from(read_u16(hvcc, at + 1)?);
		at = read_parameter_sets(hvcc, at + 3, count, &mut parameter_sets)?;
	}
	if parameter_sets.len() < 3 {
		return Err(UNSUPPORTED);
	}
	Ok(VideoCodec::Hevc {
		nal_length,
		parameter_sets,
	})
}

fn read_parameter_sets(
	bytes: &[u8],
	mut at: usize,
	count: usize,
	out: &mut Vec<Vec<u8>>,
) -> Result<usize, &'static str> {
	for _ in 0..count {
		let len = usize::from(read_u16(bytes, at)?);
		let set = bytes.get(at + 2..at + 2 + len).ok_or(INVALID)?;
		if set.is_empty() || set.len() > MAX_PARAMETER_SET_BYTES || out.len() == MAX_PARAMETER_SETS
		{
			return Err(INVALID);
		}
		out.push(set.to_vec());
		at += 2 + len;
	}
	Ok(at)
}

/// Only the first sample description counts; other audio codecs (including encrypted
/// `enca` entries) are reported as unsupported rather than played silently.
fn parse_audio_entry(stsd: &[u8]) -> Result<Option<(AudioCodec, u32, u16)>, &'static str> {
	let Some((kind, body)) = Boxes::new(stsd.get(8..).ok_or(INVALID)?).next() else {
		return Ok(None);
	};
	if &kind != b"mp4a" {
		return Err(UNSUPPORTED);
	}
	let version = read_u16(body, 8)?;
	let mut channels = read_u16(body, 16)?;
	let mut sample_rate = read_u32(body, 24)? >> 16;
	let children_at = match version {
		0 => 28,
		1 => 44,
		2 => 64,
		_ => return Err(UNSUPPORTED),
	};
	let children = body.get(children_at..).ok_or(INVALID)?;
	let esds = find_box(children, b"esds")
		.or_else(|| find_box(children, b"wave").and_then(|wave| find_box(wave, b"esds")))
		.ok_or(UNSUPPORTED)?;
	let config = parse_esds(esds)?;
	if let Some((rate, count)) = audio_specific_config(&config) {
		sample_rate = rate;
		channels = count;
	}
	if !(8000..=96_000).contains(&sample_rate) || !(1..=2).contains(&channels) {
		return Err(UNSUPPORTED);
	}
	Ok(Some((AudioCodec::Aac { config }, sample_rate, channels)))
}

/// MPEG-4 elementary stream descriptor: the ES_Descriptor wraps a DecoderConfigDescriptor
/// whose DecoderSpecificInfo carries the AudioSpecificConfig for MPEG-4 AAC.
fn parse_esds(esds: &[u8]) -> Result<Vec<u8>, &'static str> {
	let mut at = 4;
	let (tag, size, header) = descriptor(esds, at)?;
	if tag != 0x03 {
		return Err(UNSUPPORTED);
	}
	at += header;
	let end = at
		.checked_add(size)
		.filter(|end| *end <= esds.len())
		.ok_or(INVALID)?;
	let flags = *esds.get(at + 2).ok_or(INVALID)?;
	at += 3;
	if flags & 0x80 != 0 {
		at += 2;
	}
	if flags & 0x40 != 0 {
		at += 1 + usize::from(*esds.get(at).ok_or(INVALID)?);
	}
	if flags & 0x20 != 0 {
		at += 2;
	}
	while at < end {
		let (tag, size, header) = descriptor(esds, at)?;
		at += header;
		let body = esds.get(at..at + size).ok_or(INVALID)?;
		if tag == 0x04 {
			let object_type = *body.first().ok_or(INVALID)?;
			// 0x40 MPEG-4 Audio, 0x66-0x68 MPEG-2 AAC profiles.
			if !matches!(object_type, 0x40 | 0x66 | 0x67 | 0x68) {
				return Err(UNSUPPORTED);
			}
			let mut inner = 13;
			while inner < body.len() {
				let (tag, size, header) = descriptor(body, inner)?;
				inner += header;
				let info = body.get(inner..inner + size).ok_or(INVALID)?;
				if tag == 0x05 {
					if info.len() < 2 || info.len() > 64 {
						return Err(UNSUPPORTED);
					}
					return Ok(info.to_vec());
				}
				inner += size;
			}
			return Err(UNSUPPORTED);
		}
		at += size;
	}
	Err(UNSUPPORTED)
}

/// Returns (tag, payload size, header length) for a descriptor with an expandable size field.
fn descriptor(bytes: &[u8], at: usize) -> Result<(u8, usize, usize), &'static str> {
	let tag = *bytes.get(at).ok_or(INVALID)?;
	let mut size = 0_usize;
	let mut header = 1;
	for _ in 0..4 {
		let byte = *bytes.get(at + header).ok_or(INVALID)?;
		header += 1;
		size = (size << 7) | usize::from(byte & 0x7f);
		if byte & 0x80 == 0 {
			return Ok((tag, size, header));
		}
	}
	Err(INVALID)
}

/// Decode the leading AudioSpecificConfig fields: (sample rate, channel count). AAC-LC only;
/// object types with SBR/PS extensions are rejected because the decoder cannot upsample them.
fn audio_specific_config(config: &[u8]) -> Option<(u32, u16)> {
	const RATES: [u32; 13] = [
		96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025,
		8_000, 7_350,
	];
	let mut bits = Bits::new(config);
	let mut object_type = bits.read(5)?;
	if object_type == 31 {
		object_type = 32 + bits.read(6)?;
	}
	if !matches!(object_type, 1..=4) {
		return None;
	}
	let index = bits.read(4)? as usize;
	let rate = if index == 15 {
		bits.read(24)?
	} else {
		*RATES.get(index)?
	};
	let channels = bits.read(4)? as u16;
	Some((rate, channels))
}

struct Bits<'a> {
	bytes: &'a [u8],
	position: usize,
}
impl<'a> Bits<'a> {
	fn new(bytes: &'a [u8]) -> Self {
		Self { bytes, position: 0 }
	}
	fn read(&mut self, count: usize) -> Option<u32> {
		let mut value = 0_u32;
		for _ in 0..count {
			let byte = *self.bytes.get(self.position / 8)?;
			let bit = (byte >> (7 - self.position % 8)) & 1;
			value = (value << 1) | u32::from(bit);
			self.position += 1;
		}
		Some(value)
	}
}

/// Iterates sibling boxes inside a parent body. Malformed sizes end iteration.
struct Boxes<'a> {
	bytes: &'a [u8],
	at: usize,
}
impl<'a> Boxes<'a> {
	fn new(bytes: &'a [u8]) -> Self {
		Self { bytes, at: 0 }
	}
}
impl<'a> Iterator for Boxes<'a> {
	type Item = ([u8; 4], &'a [u8]);
	fn next(&mut self) -> Option<Self::Item> {
		let header = self.bytes.get(self.at..self.at + 8)?;
		let mut size = u32::from_be_bytes(header[..4].try_into().ok()?) as usize;
		let kind: [u8; 4] = header[4..8].try_into().ok()?;
		let mut header_len = 8;
		if size == 1 {
			let large = self.bytes.get(self.at + 8..self.at + 16)?;
			size = usize::try_from(u64::from_be_bytes(large.try_into().ok()?)).ok()?;
			header_len = 16;
		} else if size == 0 {
			size = self.bytes.len() - self.at;
		}
		if size < header_len {
			return None;
		}
		let body = self.bytes.get(self.at + header_len..self.at + size)?;
		self.at += size;
		Some((kind, body))
	}
}

fn find_box<'a>(bytes: &'a [u8], wanted: &[u8; 4]) -> Option<&'a [u8]> {
	Boxes::new(bytes).find_map(|(kind, body)| (&kind == wanted).then_some(body))
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, &'static str> {
	bytes
		.get(at..at + 2)
		.and_then(|b| b.try_into().ok())
		.map(u16::from_be_bytes)
		.ok_or(INVALID)
}
fn read_u32(bytes: &[u8], at: usize) -> Result<u32, &'static str> {
	bytes
		.get(at..at + 4)
		.and_then(|b| b.try_into().ok())
		.map(u32::from_be_bytes)
		.ok_or(INVALID)
}
fn read_u64(bytes: &[u8], at: usize) -> Result<u64, &'static str> {
	bytes
		.get(at..at + 8)
		.and_then(|b| b.try_into().ok())
		.map(u64::from_be_bytes)
		.ok_or(INVALID)
}

#[cfg(test)]
mod tests {
	use super::*;
	const FIXTURE: &[u8] = include_bytes!("../../../../apps/desktop/tests/fixtures/video.mov");

	#[test]
	fn truncated_avcc_header_is_rejected() {
		let header = [1, 0x64, 0, 0x1f, 0xff];
		for length in 0..=header.len() {
			assert!(parse_avcc(&header[..length]).is_err());
		}
	}

	#[test]
	fn fixture_tables_are_complete_and_bounded() {
		let mut cursor = std::io::Cursor::new(FIXTURE);
		let movie = parse(&mut cursor, FIXTURE.len() as u64).unwrap();
		assert!((2.9..3.1).contains(&movie.duration));
		let video = &movie.video;
		assert_eq!((video.width, video.height, video.rotation), (320, 180, 0));
		assert_eq!(video.track.samples.len(), 72);
		assert!(matches!(
			video.track.codec,
			VideoCodec::H264 { nal_length: 4, ref parameter_sets } if parameter_sets.len() == 2
		));
		assert!(video.track.samples[0].sync);
		// Decode order is monotonic; every sample lies inside the file.
		for pair in video.track.samples.windows(2) {
			assert!(pair[0].dts < pair[1].dts);
		}
		assert!(
			video
				.track
				.samples
				.iter()
				.all(|s| s.offset + u64::from(s.size) <= FIXTURE.len() as u64)
		);
		// libx264 defaults use B-frames, so presentation order differs from decode order.
		assert!(
			video
				.track
				.samples
				.windows(2)
				.any(|pair| pair[0].pts > pair[1].pts)
		);
		let audio = movie.audio.as_ref().unwrap();
		assert_eq!((audio.sample_rate, audio.channels), (48_000, 1));
		assert!(audio.track.samples.len() > 100);
		assert!(matches!(audio.track.codec, AudioCodec::Aac { ref config } if config.len() >= 2));

		{
			// AAC-LC, 48 kHz (index 3), mono.
			assert_eq!(audio_specific_config(&[0x11, 0x88]), Some((48_000, 1)));
			// HE-AAC (object type 5) is rejected.
			assert_eq!(audio_specific_config(&[0x2b, 0x92, 0x08, 0x00]), None);
			assert_eq!(
				descriptor(&[0x03, 0x80, 0x80, 0x80, 0x05], 0),
				Ok((3, 5, 5))
			);
			assert!(descriptor(&[0x03, 0x80, 0x80, 0x80, 0x80, 0x05], 0).is_err());
		}
	}

	#[test]
	fn rejects_external_or_fragmented_files() {
		let mut bogus = FIXTURE.to_vec();
		let moov = bogus.windows(4).position(|w| w == b"moov").unwrap() - 4;
		bogus[moov + 4..moov + 8].copy_from_slice(b"moof");
		let mut cursor = std::io::Cursor::new(&bogus);
		assert!(matches!(
			parse(&mut cursor, bogus.len() as u64),
			Err(error) if error == UNSUPPORTED
		));
		let mut truncated = std::io::Cursor::new(&FIXTURE[..64]);
		assert!(parse(&mut truncated, 64).is_err());
		let mut junk = std::io::Cursor::new(vec![0_u8; 64]);
		assert!(parse(&mut junk, 64).is_err());
	}
}

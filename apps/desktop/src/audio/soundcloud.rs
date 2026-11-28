//! SoundCloud track playback through the service's public web API (unofficial). A Discord embed
//! names the track in its hosted player's address; the track's stream is AAC in fragmented MP4,
//! delivered as HLS segments. Requests identify Serein and carry no cookie, account or Discord
//! identity; every response is bounded and nothing is written to disk.
use super::*;
use crate::audio::streaming::Feed;
use symphonia::core::{
	audio::{Channels, Position},
	codecs::audio::{
		AudioCodecParameters, AudioDecoder, AudioDecoderOptions, well_known::CODEC_ID_AAC,
	},
	errors::Error as DecodeError,
	packet::Packet,
	units::{Duration as Frames, Timestamp},
};
use tokio::sync::Notify;

/// Longest track the player streams.
pub(super) const MAX_TRACK_SECONDS: u64 = 2 * 60 * 60;
const REFUSED: &str = "SoundCloud refused this track; try again later";
const UNAVAILABLE: &str = "SoundCloud is not responding; try again";
const UNSUPPORTED: &str = "This SoundCloud track cannot be played here";
const CANCELLED: &str = "Cancelled";
const TOO_LONG: &str = "SoundCloud preview limit: 2 hours";
const HOME: &str = "https://soundcloud.com/";
const USER_AGENT: &str = concat!("Serein/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_TRACK: f64 = MAX_TRACK_SECONDS as f64;
const MAX_PAGE: usize = 1024 * 1024;
/// Script bundles scanned, last first, for the public client id the web player uses.
const MAX_SCRIPTS: usize = 8;
const MAX_SCRIPT: usize = 8 * 1024 * 1024;
const MAX_JSON: usize = 256 * 1024;
const MAX_PLAYLIST: usize = 512 * 1024;
const MAX_SEGMENTS: usize = 20_000;
const MAX_SEGMENT: usize = 8 * 1024 * 1024;

/// Why a track stopped. Fixed local labels only; never remote text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
	/// The service rejected the client id: it may have been retired.
	Rejected,
	/// The service answered but would not serve the track.
	Refused,
	Unavailable,
	Unsupported,
	Cancelled,
}
impl Failure {
	fn label(self) -> &'static str {
		match self {
			Self::Rejected | Self::Refused => REFUSED,
			Self::Unavailable => UNAVAILABLE,
			Self::Unsupported => UNSUPPORTED,
			Self::Cancelled => CANCELLED,
		}
	}
}

/// The failure an HTTP status reports: 401 and 403 reject the public client id; a missing,
/// withdrawn, rate-limited or blocked track is refused.
fn status_failure(status: u16) -> Option<Failure> {
	match status {
		401 | 403 => Some(Failure::Rejected),
		404 | 410 | 429 | 451 => Some(Failure::Refused),
		_ => None,
	}
}

/// The one track address the player requests: `https://api.soundcloud.com/tracks/<id>`.
pub(super) fn track_url(value: &str) -> Option<url::Url> {
	let url = url::Url::parse(value).ok()?;
	let mut path = url.path_segments()?;
	let track = path.next() == Some("tracks")
		&& path.next().is_some_and(|id| {
			!id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
		}) && path.next().is_none();
	(track
		&& url.scheme() == "https"
		&& url.host_str() == Some("api.soundcloud.com")
		&& url.port().is_none()
		&& url.username().is_empty()
		&& url.password().is_none()
		&& url.query().is_none()
		&& url.fragment().is_none())
	.then_some(url)
}

/// Only SoundCloud's own API, script and media hosts are contacted, whatever a response says.
fn allowed(url: &url::Url) -> bool {
	url.scheme() == "https"
		&& url.port().is_none()
		&& url.username().is_empty()
		&& url.password().is_none()
		&& url.host_str().is_some_and(|host| {
			let host = host.to_ascii_lowercase();
			["soundcloud.com", "sndcdn.com", "soundcloud.cloud"]
				.iter()
				.any(|suffix| {
					host == *suffix
						|| host
							.strip_suffix(suffix)
							.is_some_and(|rest| rest.ends_with('.'))
				})
		})
}

fn client() -> Result<wreq::Client, Failure> {
	wreq::Client::builder()
		.https_only(true)
		.no_proxy()
		.redirect(wreq::redirect::Policy::none())
		.user_agent(USER_AGENT)
		.timeout(TIMEOUT)
		.connect_timeout(Duration::from_secs(5))
		.build()
		.map_err(|_| Failure::Unavailable)
}

/// One bounded GET; a replaced clip or a pending seek ends the transfer.
async fn fetch(
	client: &wreq::Client,
	url: &url::Url,
	limit: usize,
	gate: &Gate,
	generation: u64,
) -> Result<Vec<u8>, Failure> {
	if !allowed(url) {
		return Err(Failure::Unsupported);
	}
	let transfer = async {
		let response = client
			.get(url.as_str())
			.send()
			.await
			.map_err(|_| Failure::Unavailable)?;
		if let Some(failure) = status_failure(response.status().as_u16()) {
			return Err(failure);
		}
		if response.status() != wreq::StatusCode::OK
			|| response
				.content_length()
				.is_some_and(|len| len > limit as u64)
		{
			return Err(Failure::Unavailable);
		}
		let mut bytes = Vec::new();
		let mut body = std::pin::pin!(response.bytes_stream());
		while let Some(chunk) = futures_util::StreamExt::next(&mut body)
			.await
			.transpose()
			.map_err(|_| Failure::Unavailable)?
		{
			if chunk.len() > limit - bytes.len() {
				return Err(Failure::Unavailable);
			}
			bytes.extend_from_slice(&chunk);
			if !gate.current(generation) {
				return Err(Failure::Cancelled);
			}
		}
		Ok(bytes)
	};
	let cancelled = async {
		while gate.current(generation) && gate.seek_millis.load(Ordering::Acquire) == NO_SEEK {
			tokio::time::sleep(Duration::from_millis(20)).await;
		}
	};
	tokio::select! { biased;
		_ = cancelled => Err(Failure::Cancelled),
		result = transfer => result,
	}
}

/// The public client id the web player sends, found in the home page's script bundles. It changes
/// rarely, so it is read once per session; a refused track request clears it for one retry.
static CLIENT_ID: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn client_id_cell() -> std::sync::MutexGuard<'static, Option<String>> {
	CLIENT_ID
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Asset script addresses in the page, last first: the bundle carrying the id is a late one.
fn script_urls(page: &str) -> Vec<url::Url> {
	const PREFIX: &str = "https://a-v2.sndcdn.com/assets/";
	let mut urls = Vec::new();
	let mut rest = page;
	while urls.len() < 16
		&& let Some(start) = rest.find(PREFIX)
	{
		let tail = &rest[start..];
		let end = tail
			.find(|c: char| c == '"' || c.is_whitespace())
			.unwrap_or(tail.len());
		if let Ok(url) = url::Url::parse(&tail[..end])
			&& url.path().ends_with(".js")
		{
			urls.push(url);
		}
		rest = &tail[end..];
	}
	urls.reverse();
	urls.truncate(MAX_SCRIPTS);
	urls
}

fn client_id_in(script: &[u8]) -> Option<String> {
	const KEY: &[u8] = b"client_id:\"";
	let start = script.windows(KEY.len()).position(|w| w == KEY)? + KEY.len();
	let length = script[start..]
		.iter()
		.take(64)
		.position(|b| !b.is_ascii_alphanumeric())?;
	(length > 0).then(|| String::from_utf8_lossy(&script[start..start + length]).into_owned())
}

async fn discover_client_id(
	client: &wreq::Client,
	gate: &Gate,
	generation: u64,
) -> Result<String, Failure> {
	let home = url::Url::parse(HOME).map_err(|_| Failure::Unavailable)?;
	let page = fetch(client, &home, MAX_PAGE, gate, generation).await?;
	for url in script_urls(&String::from_utf8_lossy(&page)) {
		let script = fetch(client, &url, MAX_SCRIPT, gate, generation).await?;
		if let Some(id) = client_id_in(&script) {
			return Ok(id);
		}
	}
	Err(Failure::Unavailable)
}

#[derive(Debug)]
struct Segment {
	url: url::Url,
	seconds: f64,
}
#[derive(Debug)]
struct Playlist {
	init: url::Url,
	segments: Vec<Segment>,
	seconds: f64,
}

/// `URI="…"` of an HLS attribute list, resolved like a segment address.
fn attribute_uri(attributes: &str, base: &url::Url) -> Result<url::Url, Failure> {
	let start = attributes.find("URI=\"").ok_or(Failure::Unsupported)? + 5;
	let end = start + attributes[start..].find('"').ok_or(Failure::Unsupported)?;
	base.join(&attributes[start..end])
		.map_err(|_| Failure::Unsupported)
}

/// A finished media playlist of unencrypted whole segments with one init segment. Master
/// playlists, encryption, byte ranges and live or sliding windows are refused.
fn playlist(text: &str, base: &url::Url) -> Result<Playlist, Failure> {
	let bad = Failure::Unsupported;
	if !text.starts_with("#EXTM3U") {
		return Err(bad);
	}
	let mut init = None;
	let mut segments: Vec<Segment> = Vec::new();
	let mut seconds = 0.0;
	let mut pending = None;
	let mut ended = false;
	for line in text.lines().map(str::trim) {
		if line.is_empty() {
			continue;
		}
		if [
			"#EXT-X-STREAM-INF",
			"#EXT-X-KEY",
			"#EXT-X-BYTERANGE",
			"#EXT-X-GAP",
			"#EXT-X-PART",
			"#EXT-X-PRELOAD-HINT",
			"#EXT-X-SKIP",
			"#EXT-X-DISCONTINUITY",
		]
		.iter()
		.any(|tag| line.starts_with(tag))
		{
			return Err(bad);
		}
		if let Some(attributes) = line.strip_prefix("#EXT-X-MAP:") {
			// An initialization section inside a larger file is not a whole resource to fetch.
			if attributes
				.split(',')
				.any(|attribute| attribute.trim_start().starts_with("BYTERANGE="))
			{
				return Err(bad);
			}
			let map = attribute_uri(attributes, base)?;
			// A map applies to the segments after it; one description serves the whole track.
			if init.is_some_and(|previous| previous != map) {
				return Err(bad);
			}
			init = Some(map);
			continue;
		}
		if line == "#EXT-X-ENDLIST" {
			ended = true;
			continue;
		}
		if let Some(value) = line.strip_prefix("#EXTINF:") {
			let duration: f64 = value
				.split(',')
				.next()
				.unwrap_or("")
				.trim()
				.parse()
				.map_err(|_| bad)?;
			if !(0.0..=60.0).contains(&duration) {
				return Err(bad);
			}
			pending = Some(duration);
			continue;
		}
		if line.starts_with('#') {
			continue;
		}
		let duration = pending.take().ok_or(bad)?;
		let url = base.join(line).map_err(|_| bad)?;
		if segments.len() >= MAX_SEGMENTS || !allowed(&url) {
			return Err(Failure::Unavailable);
		}
		seconds += duration;
		segments.push(Segment {
			url,
			seconds: duration,
		});
	}
	let init = init.ok_or(bad)?;
	if !ended || pending.is_some() || !allowed(&init) {
		return Err(bad);
	}
	if seconds > MAX_TRACK {
		return Err(Failure::Unavailable);
	}
	Ok(Playlist {
		init,
		segments,
		seconds,
	})
}

/// The track's AAC HLS stream: the best AAC transcoding the service offers, and the
/// authorization its player sends for it.
fn transcoding(track: &serde_json::Value) -> Option<(url::Url, Option<String>)> {
	let mut best: Option<(u8, url::Url)> = None;
	for entry in track.pointer("/media/transcodings")?.as_array()? {
		let Some(format) = entry.get("format") else {
			continue;
		};
		if format.get("protocol").and_then(|p| p.as_str()) != Some("hls")
			|| !format
				.get("mime_type")
				.and_then(|m| m.as_str())
				.is_some_and(|m| m.starts_with("audio/mp4"))
		{
			continue;
		}
		// Only the service's own host may name a stream; an entry without such an address
		// is no candidate, whatever its preset.
		let Some(stream) = entry
			.get("url")
			.and_then(|u| u.as_str())
			.and_then(|u| url::Url::parse(u).ok())
			.filter(|url| url.host_str() == Some("api-v2.soundcloud.com") && allowed(url))
		else {
			continue;
		};
		let rank = u8::from(entry.get("preset").and_then(|p| p.as_str()) != Some("aac_160k"));
		if best.as_ref().is_none_or(|(current, _)| rank < *current) {
			best = Some((rank, stream));
		}
	}
	let (_, stream) = best?;
	Some((
		stream,
		track
			.get("track_authorization")
			.and_then(|a| a.as_str())
			.map(str::to_owned),
	))
}

/// Reads the track, its playlist and the playlist's init segment. `None` when the service
/// rejected the client id, which may have been retired.
async fn resolve(
	client: &wreq::Client,
	track: &url::Url,
	client_id: &str,
	gate: &Gate,
	generation: u64,
) -> Result<Option<(Playlist, Vec<u8>)>, Failure> {
	let id = track
		.path_segments()
		.and_then(|mut path| path.nth(1))
		.ok_or(Failure::Unsupported)?;
	let api = url::Url::parse(&format!(
		"https://api-v2.soundcloud.com/tracks/{id}?client_id={client_id}"
	))
	.map_err(|_| Failure::Unavailable)?;
	let body = match fetch(client, &api, MAX_JSON, gate, generation).await {
		Ok(body) => body,
		Err(Failure::Rejected) => return Ok(None),
		Err(error) => return Err(error),
	};
	let track: serde_json::Value =
		serde_json::from_slice(&body).map_err(|_| Failure::Unsupported)?;
	let (mut stream, authorization) = transcoding(&track).ok_or(Failure::Unsupported)?;
	{
		// Values are appended as encoded query pairs, never spliced into the address.
		let mut pairs = stream.query_pairs_mut();
		pairs.append_pair("client_id", client_id);
		if let Some(authorization) = authorization {
			pairs.append_pair("track_authorization", &authorization);
		}
	}
	let body = fetch(client, &stream, MAX_JSON, gate, generation).await?;
	let stream: serde_json::Value =
		serde_json::from_slice(&body).map_err(|_| Failure::Unsupported)?;
	let address = stream
		.get("url")
		.and_then(|u| u.as_str())
		.ok_or(Failure::Unsupported)?;
	let playlist_url = url::Url::parse(address).map_err(|_| Failure::Unsupported)?;
	let body = fetch(client, &playlist_url, MAX_PLAYLIST, gate, generation).await?;
	let playlist = playlist(&String::from_utf8_lossy(&body), &playlist_url)?;
	let init = fetch(client, &playlist.init, MAX_JSON, gate, generation).await?;
	Ok(Some((playlist, init)))
}

/// Bounded ISO base media file parsing: an init segment's AAC description and a fragment's
/// sample locations.
mod mp4 {
	use super::UNSUPPORTED;
	const MAX_BOXES: usize = 64;
	const MAX_SAMPLES: usize = 65_536;

	fn u16be(b: &[u8]) -> u16 {
		u16::from_be_bytes([b[0], b[1]])
	}
	fn u32be(b: &[u8]) -> u32 {
		u32::from_be_bytes([b[0], b[1], b[2], b[3]])
	}
	fn i32be(b: &[u8]) -> i32 {
		i32::from_be_bytes([b[0], b[1], b[2], b[3]])
	}
	/// One parsed box: its type, its offset in `buf` and its body.
	type Parsed<'a> = (&'a str, usize, &'a [u8]);
	/// Boxes of `buf`.
	fn boxes(buf: &[u8]) -> Result<Vec<Parsed<'_>>, &'static str> {
		let mut out = Vec::new();
		let mut at = 0;
		while at < buf.len() && out.len() < MAX_BOXES {
			let header = buf.get(at..at + 8).ok_or(UNSUPPORTED)?;
			let mut size = u32be(header) as usize;
			let mut skip = 8;
			if size == 1 {
				size = usize::try_from(u64::from_be_bytes(
					buf.get(at + 8..at + 16)
						.ok_or(UNSUPPORTED)?
						.try_into()
						.map_err(|_| UNSUPPORTED)?,
				))
				.map_err(|_| UNSUPPORTED)?;
				skip = 16;
			}
			let end = at
				.checked_add(size)
				.filter(|end| size >= skip && *end <= buf.len())
				.ok_or(UNSUPPORTED)?;
			out.push((
				std::str::from_utf8(&header[4..8]).map_err(|_| UNSUPPORTED)?,
				at,
				buf.get(at + skip..end).ok_or(UNSUPPORTED)?,
			));
			at = end;
		}
		Ok(out)
	}
	fn child<'a>(buf: &'a [u8], name: &str) -> Result<&'a [u8], &'static str> {
		boxes(buf)?
			.into_iter()
			.find(|(kind, _, _)| *kind == name)
			.map(|(_, _, body)| body)
			.ok_or(UNSUPPORTED)
	}

	pub(super) struct Init {
		pub rate: u32,
		pub channels: u16,
		/// The AudioSpecificConfig the AAC decoder is opened with.
		pub config: Vec<u8>,
	}

	/// Walks the descriptor tree of an `esds` box for the decoder-specific information.
	fn specific_config(descriptors: &[u8], depth: usize) -> Result<Vec<u8>, &'static str> {
		if depth > 4 {
			return Err(UNSUPPORTED);
		}
		let mut at = 0;
		while at < descriptors.len() {
			let tag = descriptors[at];
			let mut length = 0usize;
			at += 1;
			for _ in 0..4 {
				let byte = *descriptors.get(at).ok_or(UNSUPPORTED)?;
				at += 1;
				length = length.checked_shl(7).ok_or(UNSUPPORTED)? | usize::from(byte & 0x7f);
				if byte & 0x80 == 0 {
					break;
				}
			}
			let end = at
				.checked_add(length)
				.filter(|end| *end <= descriptors.len())
				.ok_or(UNSUPPORTED)?;
			let body = &descriptors[at..end];
			match tag {
				// ES_Descriptor: its identifier and flags, with their optional fields, come first.
				0x03 => {
					let flags = *body.first().ok_or(UNSUPPORTED)?;
					let mut skip = 3 + if flags & 0x80 != 0 { 2 } else { 0 };
					if flags & 0x40 != 0 {
						skip += 1 + usize::from(*body.get(3).ok_or(UNSUPPORTED)?);
					}
					return specific_config(body.get(skip..).ok_or(UNSUPPORTED)?, depth + 1);
				}
				// DecoderConfigDescriptor carries the DecoderSpecificInfo after thirteen bytes.
				0x04 => return specific_config(body.get(13..).ok_or(UNSUPPORTED)?, depth + 1),
				0x05 => return Ok(body.to_vec()),
				_ => {}
			}
			at = end;
		}
		Err(UNSUPPORTED)
	}

	pub(super) fn init_segment(bytes: &[u8]) -> Result<Init, &'static str> {
		let moov = child(bytes, "moov")?;
		let mdia = child(child(moov, "trak")?, "mdia")?;
		let stbl = child(child(mdia, "minf")?, "stbl")?;
		// stsd: version and flags, then the entry count, then the entries.
		let mp4a = child(child(stbl, "stsd")?.get(8..).ok_or(UNSUPPORTED)?, "mp4a")?;
		let fields = mp4a.get(..28).ok_or(UNSUPPORTED)?;
		let (channels, rate) = (u16be(&fields[16..18]), u32be(&fields[24..28]) >> 16);
		let esds = child(mp4a.get(28..).ok_or(UNSUPPORTED)?, "esds")?;
		let config = specific_config(esds.get(4..).ok_or(UNSUPPORTED)?, 0)?;
		if channels == 0
			|| !(8000..=96_000).contains(&rate)
			|| config.is_empty()
			|| config.len() > 16
		{
			return Err(UNSUPPORTED);
		}
		Ok(Init {
			rate,
			channels,
			config,
		})
	}

	/// Byte ranges of the AAC packets in one fragment. Each run's data is addressed from its
	/// `moof`, the default base, unless the track declares another.
	pub(super) fn fragment(bytes: &[u8]) -> Result<Vec<(usize, usize)>, &'static str> {
		let mut samples: Vec<(usize, usize)> = Vec::new();
		for (kind, moof, body) in boxes(bytes)? {
			if kind != "moof" {
				continue;
			}
			for (kind, _, traf) in boxes(body)? {
				if kind != "traf" {
					continue;
				}
				let tfhd = child(traf, "tfhd")?;
				let flags = u32be(tfhd.get(..4).ok_or(UNSUPPORTED)?) & 0xff_ffff;
				let mut at = 8;
				let mut field = |width: usize| -> Result<u32, &'static str> {
					let value = tfhd.get(at..at + width).ok_or(UNSUPPORTED)?;
					at += width;
					Ok(u32be(value))
				};
				if flags & 0x01 != 0 {
					field(8)?;
				}
				if flags & 0x02 != 0 {
					field(4)?;
				}
				if flags & 0x08 != 0 {
					field(4)?;
				}
				let default_size = if flags & 0x10 != 0 { field(4)? } else { 0 };
				if flags & 0x20 != 0 {
					field(4)?;
				}
				for (kind, _, trun) in boxes(traf)? {
					if kind != "trun" {
						continue;
					}
					let flags = u32be(trun.get(..4).ok_or(UNSUPPORTED)?) & 0xff_ffff;
					let count = u32be(trun.get(4..8).ok_or(UNSUPPORTED)?) as usize;
					let mut at = 8;
					if flags & 0x001 == 0 {
						// Without a data offset the run continues the previous one.
						return Err(UNSUPPORTED);
					}
					let offset =
						usize::try_from(i64::from(i32be(trun.get(at..at + 4).ok_or(UNSUPPORTED)?)))
							.map_err(|_| UNSUPPORTED)?;
					at += 4;
					if flags & 0x004 != 0 {
						at += 4;
					}
					if samples.len() + count > MAX_SAMPLES {
						return Err(UNSUPPORTED);
					}
					let mut sample = moof + offset;
					for _ in 0..count {
						if flags & 0x100 != 0 {
							at += 4;
						}
						let size = if flags & 0x200 != 0 {
							let size = u32be(trun.get(at..at + 4).ok_or(UNSUPPORTED)?) as usize;
							at += 4;
							size
						} else {
							default_size as usize
						};
						if flags & 0x400 != 0 {
							at += 4;
						}
						if flags & 0x800 != 0 {
							at += 4;
						}
						let end = sample
							.checked_add(size)
							.filter(|end| size > 0 && *end <= bytes.len())
							.ok_or(UNSUPPORTED)?;
						samples.push((sample, size));
						sample = end;
					}
				}
			}
		}
		if samples.is_empty() {
			Err(UNSUPPORTED)
		} else {
			Ok(samples)
		}
	}
}

#[cfg(test)]
mod media_tests {
	use super::mp4;

	/// `trun` flags: a data offset, with or without per-sample sizes.
	const SIZES: u32 = 0x000_201;
	const OFFSET_ONLY: u32 = 0x000_001;

	fn u16(value: u16) -> [u8; 2] {
		value.to_be_bytes()
	}
	fn u32(value: u32) -> [u8; 4] {
		value.to_be_bytes()
	}
	fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
		let mut out = u32(body.len() as u32 + 8).to_vec();
		out.extend_from_slice(kind);
		out.extend_from_slice(body);
		out
	}
	/// A fragment: `moof(traf(tfhd, trun))` then an `mdat` of nine payload bytes, the run's
	/// data offset pointing at that payload. `tfhd` sets the default sample size and makes
	/// the `moof` the data base, as the service's streams do.
	fn fragment(default_size: u32, flags: u32, samples: &[u32]) -> Vec<u8> {
		let mut tfhd = 0x0002_0010_u32.to_be_bytes().to_vec();
		tfhd.extend_from_slice(&u32(1));
		tfhd.extend_from_slice(&u32(default_size));
		let mut trun = flags.to_be_bytes().to_vec();
		trun.extend_from_slice(&u32(samples.len() as u32));
		trun.extend_from_slice(&[0; 4]);
		if flags & 0x200 != 0 {
			for size in samples {
				trun.extend_from_slice(&u32(*size));
			}
		}
		let traf = boxed(
			b"traf",
			&[boxed(b"tfhd", &tfhd), boxed(b"trun", &trun)].concat(),
		);
		let moof = boxed(b"moof", &traf);
		let mut bytes = moof.clone();
		bytes.extend_from_slice(&boxed(b"mdat", &[0xaa; 9]));
		let at = moof.len() - trun.len() - 8 + 16;
		bytes[at..at + 4].copy_from_slice(&(moof.len() as u32 + 8).to_be_bytes());
		bytes
	}
	/// The init segment's `moov` for AAC with `config` as its AudioSpecificConfig.
	fn moov(channels: u16, config: &[u8]) -> Vec<u8> {
		let mut mp4a = vec![0; 16];
		mp4a.extend_from_slice(&u16(channels));
		mp4a.extend_from_slice(&u16(16));
		mp4a.extend_from_slice(&[0; 4]);
		mp4a.extend_from_slice(&u32(44_100 << 16));
		// ES_Descriptor of 22 bytes: the identifier, its flags, then the decoder configuration.
		let mut descriptors = vec![0x03, 0x16, 0, 1, 0, 0x04, 0x11, 0x40, 0x15];
		descriptors.extend_from_slice(&[0; 11]);
		descriptors.push(0x05);
		descriptors.push(config.len() as u8);
		descriptors.extend_from_slice(config);
		let mut esds = vec![0; 4];
		esds.extend_from_slice(&descriptors);
		mp4a.extend_from_slice(&boxed(b"esds", &esds));
		let mut stsd = vec![0; 8];
		stsd.extend_from_slice(&boxed(b"mp4a", &mp4a));
		let stbl = boxed(b"stbl", &boxed(b"stsd", &stsd));
		boxed(
			b"moov",
			&boxed(b"trak", &boxed(b"mdia", &boxed(b"minf", &stbl))),
		)
	}

	#[test]
	fn fragments_yield_their_packets_and_init_segments_the_aac_description() {
		let bytes = fragment(3, SIZES, &[2, 3, 4]);
		let offset = bytes.len() - 9;
		assert_eq!(
			mp4::fragment(&bytes).unwrap(),
			[(offset, 2), (offset + 2, 3), (offset + 5, 4)]
		);
		// Runs without per-sample sizes use the header's default.
		let bytes = fragment(3, OFFSET_ONLY, &[0, 0]);
		let offset = bytes.len() - 9;
		assert_eq!(
			mp4::fragment(&bytes).unwrap(),
			[(offset, 3), (offset + 3, 3)]
		);
		let init = mp4::init_segment(&moov(2, &[0x12, 0x10])).unwrap();
		assert_eq!((init.rate, init.channels), (44_100, 2));
		assert_eq!(init.config, [0x12, 0x10]);
		assert_eq!(
			mp4::init_segment(&moov(1, &[0x13, 0x08])).unwrap().channels,
			1
		);
	}

	#[test]
	fn damaged_or_unbounded_media_is_refused() {
		assert!(mp4::fragment(&[0; 7]).is_err());
		assert!(mp4::init_segment(b"ftyp").is_err());
		assert!(mp4::init_segment(&moov(0, &[0x12, 0x10])).is_err());
		// A box that runs past its buffer.
		let mut short = 64_u32.to_be_bytes().to_vec();
		short.extend_from_slice(b"moof");
		assert!(mp4::fragment(&short).is_err());
		// A sample larger than the fragment's data, and a sample of no size.
		assert!(mp4::fragment(&fragment(64, SIZES, &[64])).is_err());
		assert!(mp4::fragment(&fragment(0, OFFSET_ONLY, &[0])).is_err());
		// Sizes without a data offset, and more samples than a fragment may hold.
		assert!(mp4::fragment(&fragment(3, 0x200, &[1])).is_err());
		assert!(mp4::fragment(&fragment(3, OFFSET_ONLY, &vec![0; 65_537])).is_err());
	}
}

fn decoder(config: &mp4::Init) -> Result<Box<dyn AudioDecoder>, Failure> {
	let positions = Position::from_count(u32::from(config.channels)).ok_or(Failure::Unsupported)?;
	let mut parameters = AudioCodecParameters::new();
	parameters
		.for_codec(CODEC_ID_AAC)
		.with_sample_rate(config.rate)
		.with_channels(Channels::Positioned(positions))
		.with_extra_data(config.config.clone().into_boxed_slice());
	let mut options = AudioDecoderOptions::default();
	options.gapless = false;
	symphonia::default::get_codecs()
		.make_audio_decoder(&parameters, &options)
		.map_err(|_| Failure::Unsupported)
}

/// The segment holding `target` and the frames before it. HLS durations for AAC count whole
/// 1024-sample packets, so each segment's frame count is exact.
fn locate(playlist: &Playlist, target: Duration, rate: u32) -> Option<(usize, u64)> {
	let mut elapsed = 0.0;
	let mut frames = 0_u64;
	for (index, segment) in playlist.segments.iter().enumerate() {
		if elapsed + segment.seconds > target.as_secs_f64() {
			return Some((index, frames));
		}
		elapsed += segment.seconds;
		frames += (segment.seconds * f64::from(rate) / 1024.0).round() as u64 * 1024;
	}
	None
}

type Prefetch = tokio::task::JoinHandle<Result<Vec<u8>, Failure>>;

/// Downloads the segment on the runtime while the previous one plays.
fn prefetch(
	runtime: &tokio::runtime::Handle,
	client: &wreq::Client,
	url: &url::Url,
	gate: &Arc<Gate>,
	generation: u64,
) -> Prefetch {
	let (client, url, gate) = (client.clone(), url.clone(), gate.clone());
	runtime.spawn(async move { fetch(&client, &url, MAX_SEGMENT, &gate, generation).await })
}

/// The segments of one pass in playlist order, each requested while the previous one decodes.
struct Segments<S: Fn(usize) -> Prefetch> {
	/// Index of the next segment to hand out, and one past the pass's last one.
	next: usize,
	end: usize,
	request: S,
	pending: Option<Prefetch>,
}
impl<S: Fn(usize) -> Prefetch> Segments<S> {
	fn new(first: usize, end: usize, request: S) -> Self {
		Self {
			next: first,
			end,
			request,
			pending: None,
		}
	}
	/// The next segment with its playlist index, waited for; `None` once the pass is complete.
	fn take(
		&mut self,
		runtime: &tokio::runtime::Handle,
	) -> Result<Option<(usize, Vec<u8>)>, Failure> {
		if self.next >= self.end {
			return Ok(None);
		}
		let Self {
			next,
			end,
			request,
			pending,
		} = self;
		let index = *next;
		*next += 1;
		let ready = pending.take().unwrap_or_else(|| request(index));
		let bytes = runtime
			.block_on(ready)
			.map_err(|_| Failure::Unavailable)??;
		// The segment after this one downloads while this one decodes.
		*pending = (*next < *end).then(|| request(*next));
		Ok(Some((index, bytes)))
	}
}

/// Streams the track through the shared PCM hand-off. A seek restarts from the segment holding
/// the target; the next segment downloads while the current one plays.
pub(super) fn play(
	request: &Request,
	gate: &Arc<Gate>,
	wake: &Arc<Notify>,
	runtime: &tokio::runtime::Handle,
	publish: &impl Fn(Status),
) -> Result<(), &'static str> {
	let Some(track) = &request.soundcloud else {
		return Err(INVALID);
	};
	let generation = request.generation;
	let client = client().map_err(Failure::label)?;
	// The cached client id may have been retired: only its rejection reads a new one. Any
	// other answer, a track the service will not serve or stream included, is final.
	let found = runtime
		.block_on(async {
			// The guard is released here: the resolve below awaits.
			let cached = client_id_cell().clone();
			if let Some(id) = cached {
				match resolve(&client, track, &id, gate, generation).await {
					Ok(Some(found)) => return Ok(found),
					Ok(None) => {}
					Err(error) => return Err(error),
				}
			}
			if !gate.current(generation) {
				return Err(Failure::Cancelled);
			}
			let id = discover_client_id(&client, gate, generation).await?;
			let found = resolve(&client, track, &id, gate, generation)
				.await?
				.ok_or(Failure::Refused)?;
			*client_id_cell() = Some(id);
			Ok(found)
		})
		.map_err(Failure::label)?;
	let (playlist, init) = found;
	let config = mp4::init_segment(&init).map_err(|_| UNSUPPORTED)?;
	let mut decoder = decoder(&config).map_err(Failure::label)?;
	let rate = config.rate;
	let duration = Duration::from_secs_f64(playlist.seconds);
	let mut target: u64 = 0;
	loop {
		if !gate.current(generation) {
			return Ok(());
		}
		gate.failed.store(false, Ordering::Release);
		gate.buffering.store(true, Ordering::Release);
		publish(Status {
			state: State::Loading,
			position: Duration::from_millis(target),
			duration,
		});
		let Some((start, before)) = locate(&playlist, Duration::from_millis(target), rate) else {
			// The target is past the end of the track.
			publish(Status {
				state: State::Ended,
				position: duration,
				duration,
			});
			return Ok(());
		};
		let mut feed = Feed::new(
			gate, wake, runtime, publish, generation, target, before, duration,
		);
		let result = decode(
			&playlist,
			&config,
			&mut decoder,
			&client,
			&mut feed,
			gate,
			runtime,
			start,
		);
		if result.is_ok() {
			feed.finish()?;
		}
		drop(feed);
		if !gate.current(generation) {
			return Ok(());
		}
		let seek = gate.seek_millis.swap(NO_SEEK, Ordering::AcqRel);
		if seek != NO_SEEK {
			target = seek;
			continue;
		}
		result?;
		publish(Status {
			state: State::Ended,
			position: duration,
			duration,
		});
		return Ok(());
	}
}

#[allow(clippy::too_many_arguments)]
fn decode<P: Fn(Status)>(
	playlist: &Playlist,
	config: &mp4::Init,
	decoder: &mut Box<dyn AudioDecoder>,
	client: &wreq::Client,
	feed: &mut Feed<'_, P>,
	gate: &Arc<Gate>,
	runtime: &tokio::runtime::Handle,
	start: usize,
) -> Result<(), &'static str> {
	decoder.reset();
	let generation = feed.generation();
	let request = |index: usize| {
		prefetch(
			runtime,
			client,
			&playlist.segments[index].url,
			gate,
			generation,
		)
	};
	let mut segments = Segments::new(start, playlist.segments.len(), request);
	// Audio past the limit never reaches the output, whatever the segments advertise.
	let limit = MAX_TRACK_SECONDS * u64::from(config.rate);
	let origin = feed.origin;
	let before = feed.total_frames;
	let (mut buffer, mut stereo) = (Vec::new(), Vec::new());
	while let Some((_, bytes)) = segments.take(runtime).map_err(Failure::label)? {
		if !feed.current() {
			return Err(CANCELLED);
		}
		for (offset, len) in mp4::fragment(&bytes)? {
			let packet = Packet::new(
				0,
				Timestamp::new((origin + feed.total_frames) as i64),
				Frames::new(1024),
				bytes[offset..offset + len].to_vec(),
			);
			let decoded = match decoder.decode(&packet) {
				Ok(decoded) => decoded,
				// A damaged packet is skipped; a decoder that lost its configuration is fatal.
				Err(DecodeError::DecodeError(_)) => continue,
				Err(_) => return Err(INVALID),
			};
			let channels = decoded.spec().channels().count();
			if decoded.spec().rate() != config.rate || !(1..=2).contains(&channels) {
				return Err(UNSUPPORTED);
			}
			if feed.origin + feed.total_frames + decoded.frames() as u64 > limit {
				return Err(TOO_LONG);
			}
			buffer.resize(decoded.samples_interleaved(), 0.0);
			decoded.copy_to_slice_interleaved(&mut buffer);
			let samples = if channels == 1 {
				stereo.clear();
				stereo.extend(buffer.iter().flat_map(|sample| [*sample; 2]));
				&stereo[..]
			} else {
				&buffer[..]
			};
			feed.push(
				samples,
				2,
				config.rate,
				Some(Duration::from_secs_f64(playlist.seconds)),
			)?;
		}
	}
	// A pass that decoded packets but no audio did not play a track.
	if feed.total_frames == before {
		return Err(UNSUPPORTED);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn url(value: &str) -> url::Url {
		url::Url::parse(value).unwrap()
	}

	#[test]
	fn only_the_track_api_address_and_soundcloud_hosts_are_requested() {
		let track = "https://api.soundcloud.com/tracks/2420480568";
		assert_eq!(track_url(track).map(|u| u.to_string()), Some(track.into()));
		for other in [
			"http://api.soundcloud.com/tracks/1",
			"https://api.soundcloud.com.evil.test/tracks/1",
			"https://user@api.soundcloud.com/tracks/1",
			"https://api.soundcloud.com/tracks/1?client_id=x",
			"https://api.soundcloud.com/tracks/soundcloud:tracks:1",
			"https://api.soundcloud.com/tracks/1/streams",
			"https://api.soundcloud.com/users/1",
			"https://api.soundcloud.com:444/tracks/1",
			"https://api.soundcloud.com/tracks/",
		] {
			assert!(track_url(other).is_none(), "{other}");
		}
		for host in [
			"api-v2.soundcloud.com",
			"a-v2.sndcdn.com",
			"playback.media-streaming.soundcloud.cloud",
		] {
			assert!(allowed(&url(&format!("https://{host}/x"))), "{host}");
		}
		for other in [
			"https://soundcloud.com.evil.test/",
			"https://sndcdn.com.evil.test/",
			"http://a-v2.sndcdn.com/assets/0.js",
			"https://user@a-v2.sndcdn.com/assets/0.js",
			"https://127.0.0.1/",
			"https://localhost/",
		] {
			assert!(!allowed(&url(other)), "{other}");
		}
	}

	#[test]
	fn the_client_id_is_read_from_the_last_script_bundles_first() {
		let page = "<script src=\"https://a-v2.sndcdn.com/assets/0-a.js\"></script>\
			<script src=\"https://a-v2.sndcdn.com/assets/1-b.js\"></script> \
			src=\"https://a-v2.sndcdn.com/assets/2-c.css\" https://a-v2.sndcdn.com/assets/3-d.js?x\"";
		let names: Vec<_> = script_urls(page)
			.iter()
			.map(|u| u.path().rsplit('/').next().unwrap().to_owned())
			.collect();
		assert_eq!(names, ["3-d.js", "1-b.js", "0-a.js"]);
		assert_eq!(
			client_id_in(b"x,y={client_id:\"abcDEF123\",z:1}"),
			Some("abcDEF123".into())
		);
		assert_eq!(client_id_in(b"client_id:\"\""), None);
		assert_eq!(client_id_in(b"client_id: abc"), None);
	}

	#[test]
	fn a_finished_media_playlist_resolves_its_addresses_and_bounds_the_track() {
		let base = url("https://media.soundcloud.cloud/a/playlist.m3u8?Policy=p");
		let text = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-MEDIA-SEQUENCE:0\n\
			#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MAP:URI=\"init.mp4?Policy=p\"\n\
			#EXTINF:10.007800,\ndata000.m4s?Policy=p\n#EXTINF:0.899705,\n/data001.m4s?Policy=p\n#EXT-X-ENDLIST\n";
		let playlist = playlist(text, &base).unwrap();
		assert_eq!(playlist.segments.len(), 2);
		assert!((playlist.seconds - 10.907505).abs() < 1e-9);
		assert_eq!(
			playlist.init.as_str(),
			"https://media.soundcloud.cloud/a/init.mp4?Policy=p"
		);
		assert_eq!(
			playlist.segments[1].url.as_str(),
			"https://media.soundcloud.cloud/data001.m4s?Policy=p"
		);
		// 44.1 kHz: 431 whole packets in the first segment.
		assert_eq!(locate(&playlist, Duration::ZERO, 44_100), Some((0, 0)));
		assert_eq!(
			locate(&playlist, Duration::from_millis(10_500), 44_100),
			Some((1, 441_344))
		);
		assert_eq!(locate(&playlist, Duration::from_secs(60), 44_100), None);
	}

	#[test]
	fn playlists_the_player_cannot_stream_are_refused() {
		let base = url("https://media.soundcloud.cloud/a/p.m3u8");
		let body = "#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:10,\n0.m4s\n";
		for refused in [
			"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv.m3u8\n".to_owned(),
			format!("#EXTM3U\n{body}"),
			format!("#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n{body}#EXT-X-ENDLIST\n"),
			"#EXTM3U\n#EXTINF:10,\n#EXT-X-BYTERANGE:5@0\n0.m4s\n#EXT-X-ENDLIST\n".to_owned(),
			format!("#EXTM3U\n{body}#EXTINF:10,\n"),
			"#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:10,\nhttps://evil.test/0.m4s\n#EXT-X-ENDLIST\n"
				.to_owned(),
			format!("not a playlist\n{body}#EXT-X-ENDLIST\n"),
			"#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:nan,\n0.m4s\n#EXT-X-ENDLIST\n".to_owned(),
			format!("#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"0@100\"\n{body}#EXT-X-ENDLIST\n"),
			"#EXTM3U\n#EXT-X-MAP:URI=\"a.mp4\"\n#EXTINF:10,\n0.m4s\n#EXT-X-MAP:URI=\"b.mp4\"\n#EXTINF:10,\n1.m4s\n#EXT-X-ENDLIST\n"
				.to_owned(),
		] {
			assert!(playlist(&refused, &base).is_err(), "{refused}");
		}
		// The same map declared again is still one description.
		assert!(playlist(
			"#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:10,\n0.m4s\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:10,\n1.m4s\n#EXT-X-ENDLIST\n",
			&base
		)
		.is_ok());
		// Longer than the player's limit.
		let long = format!(
			"#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n{}#EXT-X-ENDLIST\n",
			"#EXTINF:60,\n0.m4s\n".repeat(MAX_TRACK_SECONDS as usize / 60 + 1)
		);
		assert_eq!(playlist(&long, &base).unwrap_err(), Failure::Unavailable);
	}

	#[test]
	fn the_best_aac_stream_is_chosen() {
		let track = serde_json::json!({"track_authorization": "auth+1=", "media": {"transcodings": [
			{"url": "https://api-v2.soundcloud.com/media/1/a/stream/hls", "preset": "abr_sq",
			 "format": {"protocol": "hls", "mime_type": "audio/mpegurl"}},
			{"url": "https://api-v2.soundcloud.com/media/1/c/stream/progressive", "preset": "mp3",
			 "format": {"protocol": "progressive", "mime_type": "audio/mpeg"}},
			{"url": "https://api-v2.soundcloud.com/media/1/d/stream/hls", "preset": "aac_96k",
			 "format": {"protocol": "hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\""}},
			{"url": "https://api-v2.soundcloud.com/media/1/b/stream/hls", "preset": "aac_160k",
			 "format": {"protocol": "hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\""}}
		]}});
		let (stream, authorization) = transcoding(&track).unwrap();
		assert!(stream.as_str().ends_with("/1/b/stream/hls"));
		assert_eq!(authorization.as_deref(), Some("auth+1="));
		// Only the service's own host may name a stream.
		let elsewhere = serde_json::json!({"media": {"transcodings": [
			{"url": "https://evil.test/1/stream/hls", "preset": "aac_160k",
			 "format": {"protocol": "hls", "mime_type": "audio/mp4"}}
		]}});
		assert!(transcoding(&elsewhere).is_none());
		assert!(transcoding(&serde_json::json!({})).is_none());
		// A preferred entry without a usable address loses to one that has it.
		let unusable = serde_json::json!({"media": {"transcodings": [
			{"url": "https://evil.test/1/stream/hls", "preset": "aac_160k",
			 "format": {"protocol": "hls", "mime_type": "audio/mp4"}},
			{"url": "https://api-v2.soundcloud.com/media/1/d/stream/hls", "preset": "aac_96k",
			 "format": {"protocol": "hls", "mime_type": "audio/mp4"}}
		]}});
		assert!(
			transcoding(&unusable)
				.unwrap()
				.0
				.as_str()
				.ends_with("/1/d/stream/hls")
		);
	}

	#[test]
	fn a_pass_hands_out_its_segments_in_order_and_downloads_one_ahead() {
		// `Handle::block_on` drives spawned downloads only on a runtime with its own workers,
		// as the player's is.
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let requested = Arc::new(std::sync::Mutex::new(Vec::new()));
		let log = requested.clone();
		let handle = runtime.handle().clone();
		// Every segment answers with its own index, so a misaligned download shows.
		let request = move |index: usize| {
			log.lock().unwrap().push(index);
			handle.spawn(async move { Ok::<_, Failure>(vec![index as u8; 4]) })
		};
		fn take<S: Fn(usize) -> Prefetch>(
			segments: &mut Segments<S>,
			runtime: &tokio::runtime::Handle,
		) -> Option<(usize, Vec<u8>)> {
			segments.take(runtime).unwrap()
		}
		let mut segments = Segments::new(0, 3, request);
		let handle = runtime.handle();
		assert_eq!(take(&mut segments, handle), Some((0, vec![0; 4])));
		// The next segment was requested as this one was handed out.
		assert_eq!(*requested.lock().unwrap(), [0, 1]);
		assert_eq!(take(&mut segments, handle), Some((1, vec![1; 4])));
		assert_eq!(*requested.lock().unwrap(), [0, 1, 2]);
		assert_eq!(take(&mut segments, handle), Some((2, vec![2; 4])));
		assert_eq!(*requested.lock().unwrap(), [0, 1, 2]);
		assert_eq!(take(&mut segments, handle), None);
		// A pass that starts at a later segment hands out the rest the same way.
		let mut later = Segments::new(1, 3, |index| {
			runtime.spawn(async move { Ok::<_, Failure>(vec![index as u8; 4]) })
		});
		assert_eq!(take(&mut later, handle), Some((1, vec![1; 4])));
		assert_eq!(take(&mut later, handle), Some((2, vec![2; 4])));
		assert_eq!(take(&mut later, handle), None);
	}

	#[test]
	fn only_a_rejected_client_id_is_rediscovered() {
		// A rejected id is replaced; a missing, withdrawn, rate-limited or blocked track keeps
		// its own answer instead of starting a rediscovery that could mask it.
		for status in [401, 403] {
			assert_eq!(status_failure(status), Some(Failure::Rejected), "{status}");
		}
		for status in [404, 410, 429, 451] {
			assert_eq!(status_failure(status), Some(Failure::Refused), "{status}");
		}
		assert_eq!(status_failure(200), None);
	}
}

//! Discord device state kept across launches, as a browser profile keeps it: the cookies the
//! API sets (Discord's `__dcfduid`/`__sdcfduid` device IDs and Cloudflare's) and the installation
//! ID Discord issues. Without them every launch looks like a new device with no history. Yapfast
//! and SakuraCord keep both; how the service weighs them is unverified.
//!
//! Only `https://discord.com` cookies are taken and sent. Session cookies stay in memory;
//! persistent ones and the installation ID are saved in one private, bounded file that can
//! be deleted at any time. They are device state, not credentials: the account token is never
//! written here.
use std::{
	path::{Path, PathBuf},
	sync::{Arc, Mutex, OnceLock},
	time::{Duration, SystemTime, UNIX_EPOCH},
};
use wreq::{
	Uri, Version,
	cookie::{CookieStore, Cookies, IntoCookie},
	header::HeaderValue,
};

const HOST: &str = "discord.com";
/// Discord and Cloudflare set a handful; anything past this is not a browser profile's worth.
const MAX_COOKIES: usize = 16;
/// Name plus value, the limit browsers apply to one cookie.
const MAX_COOKIE_BYTES: usize = 4096;
/// A saved file larger than this is ignored rather than parsed.
const MAX_FILE_BYTES: u64 = 128 * 1024;
/// Chromium's cap on a cookie's lifetime.
const MAX_LIFETIME: Duration = Duration::from_secs(400 * 24 * 60 * 60);
/// A saved expiry is rewritten once it has moved this far; a smaller refresh is not worth a write.
const EXPIRY_SLACK: u64 = 24 * 60 * 60;

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Cookie {
	name: String,
	value: String,
	path: String,
	/// Unix seconds; a session cookie has none and is never saved.
	expires: Option<u64>,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Saved {
	installation: Option<String>,
	cookies: Vec<Cookie>,
}

#[derive(Default)]
struct State {
	file: Option<PathBuf>,
	/// Oldest first; a cookie whose value changes counts as new, as in Chromium.
	cookies: Vec<Cookie>,
	/// The persistent cookies as the file last held them.
	saved: Vec<Cookie>,
}

/// The process's one device; every REST client shares it, so a proxy change keeps it.
pub(crate) fn store() -> &'static Arc<Device> {
	static DEVICE: OnceLock<Arc<Device>> = OnceLock::new();
	DEVICE.get_or_init(Arc::default)
}

#[derive(Default)]
pub(crate) struct Device {
	state: Mutex<State>,
	/// Held across one file write, so saves land in the order their snapshots were taken and
	/// no request waits on the disk while holding the cookies.
	writing: Mutex<()>,
}

/// Loads the device saved in `file` and saves it there from now on. A missing, oversized
/// or unreadable file starts a new device.
pub fn keep_in(file: PathBuf) {
	let saved = std::fs::metadata(&file)
		.ok()
		.filter(|meta| meta.len() <= MAX_FILE_BYTES)
		.and_then(|_| std::fs::read(&file).ok())
		.and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
		.unwrap_or_default();
	if let Some(installation) = &saved.installation {
		client_core::fingerprint::set_installation_id(installation);
	}
	let now = unix_now();
	let Ok(mut state) = store().state.lock() else {
		return;
	};
	state.cookies = saved
		.cookies
		.into_iter()
		.filter(|cookie| valid(cookie) && cookie.expires.is_some_and(|at| at > now))
		.take(MAX_COOKIES)
		.collect();
	state.saved = state.cookies.clone();
	state.file = Some(file);
}

/// Keeps the installation ID Discord issued and saves it with the device.
pub(crate) fn set_installation(id: &str) {
	if client_core::fingerprint::set_installation_id(id) {
		store().save();
	}
}

impl Device {
	/// Saves the persistent cookies and the installation ID. The snapshot is taken under the
	/// cookie lock and written outside it.
	fn save(&self) {
		let Ok(_writing) = self.writing.lock() else {
			return;
		};
		let (file, cookies) = {
			let Ok(state) = self.state.lock() else {
				return;
			};
			let Some(file) = state.file.clone() else {
				return;
			};
			(file, persistent(&state.cookies).cloned().collect())
		};
		let saved = Saved {
			installation: client_core::fingerprint::installation_id(),
			cookies,
		};
		if write_private(&file, &saved)
			&& let Ok(mut state) = self.state.lock()
		{
			state.saved = saved.cookies;
		}
	}
}

impl CookieStore for Device {
	fn set_cookies(&self, headers: &mut dyn Iterator<Item = &HeaderValue>, uri: &Uri) {
		if !is_discord(uri) {
			return;
		}
		let Ok(mut state) = self.state.lock() else {
			return;
		};
		let now = unix_now();
		state
			.cookies
			.retain(|cookie| cookie.expires.is_none_or(|at| at > now));
		for header in headers {
			let Some(cookie) = header.to_str().ok().and_then(<&str>::into_cookie) else {
				continue;
			};
			// A cookie for another site, or a subdomain only, is not this API's to keep.
			if cookie
				.domain()
				.is_some_and(|domain| !domain.eq_ignore_ascii_case(HOST))
			{
				continue;
			}
			let expires = match (cookie.max_age(), cookie.expires()) {
				(Some(age), _) => Some(now.saturating_add(age.min(MAX_LIFETIME).as_secs())),
				(None, Some(at)) => Some(
					at.duration_since(UNIX_EPOCH)
						.map_or(0, |at| at.as_secs())
						.min(now.saturating_add(MAX_LIFETIME.as_secs())),
				),
				(None, None) => None,
			};
			let path = cookie
				.path()
				.filter(|path| path.starts_with('/'))
				.unwrap_or_else(|| default_path(uri.path()));
			let previous = state
				.cookies
				.iter()
				.position(|kept| kept.name == cookie.name() && kept.path == path);
			// An expiry in the past is how a server deletes a cookie, whatever value comes with it.
			if expires.is_some_and(|at| at <= now) {
				if let Some(at) = previous {
					state.cookies.remove(at);
				}
				continue;
			}
			let entry = Cookie {
				name: cookie.name().to_owned(),
				value: cookie.value().to_owned(),
				path: path.to_owned(),
				expires,
			};
			if !valid(&entry) {
				continue;
			}
			// A refreshed cookie keeps its place in the order; a new value takes the newest.
			if let Some(kept) = previous
				.and_then(|at| state.cookies.get_mut(at))
				.filter(|kept| kept.value == entry.value)
			{
				*kept = entry;
				continue;
			}
			if let Some(at) = previous {
				state.cookies.remove(at);
			} else if state.cookies.len() >= MAX_COOKIES {
				// Full: the oldest session cookie makes room, otherwise the oldest of all.
				let oldest = state
					.cookies
					.iter()
					.position(|kept| kept.expires.is_none())
					.unwrap_or(0);
				state.cookies.remove(oldest);
			}
			state.cookies.push(entry);
		}
		let stale = state.file.is_some() && stale(&state.saved, &state.cookies);
		drop(state);
		if stale {
			self.save();
		}
	}

	fn cookies(&self, uri: &Uri, version: Version) -> Cookies {
		if !is_discord(uri) {
			return Cookies::Empty;
		}
		let Ok(state) = self.state.lock() else {
			return Cookies::Empty;
		};
		let now = unix_now();
		let path = uri.path();
		let mut matching = state
			.cookies
			.iter()
			.filter(|cookie| cookie.expires.is_none_or(|at| at > now))
			.filter(|cookie| path_matches(path, &cookie.path))
			.collect::<Vec<_>>();
		// Chromium's order: longest path first, then oldest. The sort keeps the stored order.
		matching.sort_by_key(|cookie| std::cmp::Reverse(cookie.path.len()));
		let pairs = matching
			.into_iter()
			.map(|cookie| format!("{}={}", cookie.name, cookie.value));
		// Chromium sends one field per cookie over HTTP/2 and HTTP/3, and one joined field before.
		if matches!(version, Version::HTTP_2 | Version::HTTP_3) {
			let fields = pairs
				.filter_map(|pair| HeaderValue::from_str(&pair).ok())
				.collect::<Vec<_>>();
			if fields.is_empty() {
				return Cookies::Empty;
			}
			return Cookies::Uncompressed(fields);
		}
		let joined = pairs.collect::<Vec<_>>().join("; ");
		if joined.is_empty() {
			return Cookies::Empty;
		}
		HeaderValue::from_str(&joined).map_or(Cookies::Empty, Cookies::Compressed)
	}
}

fn is_discord(uri: &Uri) -> bool {
	uri.scheme_str() == Some("https")
		&& uri
			.host()
			.is_some_and(|host| host.eq_ignore_ascii_case(HOST))
}

/// RFC 6265 path matching.
fn path_matches(request: &str, cookie: &str) -> bool {
	request == cookie
		|| request.starts_with(cookie)
			&& (cookie.ends_with('/') || request[cookie.len()..].starts_with('/'))
}

/// RFC 6265 default path for a cookie that names none: the request path up to its last `/`.
fn default_path(request: &str) -> &str {
	match request.rfind('/') {
		Some(end) if end > 0 && request.starts_with('/') => &request[..end],
		_ => "/",
	}
}

fn valid(cookie: &Cookie) -> bool {
	let token = |text: &str| {
		text.bytes()
			.all(|b| b.is_ascii_graphic() && !matches!(b, b';' | b',' | b'"' | b'\\'))
	};
	// RFC 6265 allows one pair of quotes around a value; they are stored and sent back as given.
	let value = cookie
		.value
		.strip_prefix('"')
		.and_then(|value| value.strip_suffix('"'))
		.unwrap_or(&cookie.value);
	!cookie.name.is_empty()
		&& cookie.name.len() + cookie.value.len() <= MAX_COOKIE_BYTES
		&& !cookie.name.contains('=')
		&& token(&cookie.name)
		&& token(value)
		&& cookie.path.len() <= 256
		&& cookie.path.starts_with('/')
		&& token(&cookie.path)
}

/// The cookies the file holds.
fn persistent(cookies: &[Cookie]) -> impl Iterator<Item = &Cookie> {
	cookies.iter().filter(|cookie| cookie.expires.is_some())
}

/// Whether the file no longer holds `cookies`: a saved cookie came, went or changed its value,
/// or its expiry moved by more than [`EXPIRY_SLACK`].
fn stale(saved: &[Cookie], cookies: &[Cookie]) -> bool {
	let mut kept = persistent(cookies);
	let same = saved.iter().all(|old| {
		kept.next().is_some_and(|new| {
			old.name == new.name
				&& old.path == new.path
				&& old.value == new.value
				&& old
					.expires
					.zip(new.expires)
					.is_some_and(|(old, new)| old.abs_diff(new) <= EXPIRY_SLACK)
		})
	});
	!same || kept.next().is_some()
}

/// Replaces `file` whole with `saved`, readable only by this user. The temporary file is
/// always created new, so it never follows a link or keeps another file's permissions.
fn write_private(file: &Path, saved: &Saved) -> bool {
	let Ok(bytes) = serde_json::to_vec(saved) else {
		return false;
	};
	let partial = file.with_extension("json.partial");
	// What an interrupted save left behind; removing it never follows a link either.
	let _ = std::fs::remove_file(&partial);
	let mut options = std::fs::OpenOptions::new();
	options.write(true).create_new(true);
	#[cfg(unix)]
	std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
	let written = options
		.open(&partial)
		.and_then(|mut out| {
			std::io::Write::write_all(&mut out, &bytes)?;
			out.sync_all()
		})
		.and_then(|()| std::fs::rename(&partial, file));
	if written.is_err() {
		let _ = std::fs::remove_file(&partial);
		return false;
	}
	// The rename itself survives a crash once its directory is synced; best effort.
	#[cfg(unix)]
	if let Some(directory) = file.parent() {
		let _ = std::fs::File::open(directory).and_then(|directory| directory.sync_all());
	}
	true
}

fn unix_now() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |now| now.as_secs())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn set(device: &Device, path: &str, cookies: &[&str]) {
		let headers = cookies
			.iter()
			.map(|cookie| HeaderValue::from_str(cookie).unwrap())
			.collect::<Vec<_>>();
		let uri = format!("https://discord.com{path}").parse().unwrap();
		device.set_cookies(&mut headers.iter(), &uri);
	}

	fn sent(device: &Device, url: &str, version: Version) -> Vec<String> {
		let fields = match device.cookies(&url.parse().unwrap(), version) {
			Cookies::Compressed(field) => vec![field],
			Cookies::Uncompressed(fields) => fields,
			_ => Vec::new(),
		};
		fields
			.iter()
			.map(|field| field.to_str().unwrap().to_owned())
			.collect()
	}

	#[test]
	fn cookies_keep_browser_scope_order_and_deletion() {
		let device = Device::default();
		set(
			&device,
			"/api/v10/users/@me",
			&[
				"root=1; Path=/",
				"scoped=2",
				"quoted=\"v\"; Path=/",
				"foreign=3; Domain=example.com",
				"spaced=a b; Path=/",
			],
		);
		// A cookie naming no path belongs to the directory that set it.
		assert_eq!(
			sent(
				&device,
				"https://discord.com/api/v10/users/7",
				Version::HTTP_2
			),
			["scoped=2", "root=1", "quoted=\"v\""]
		);
		assert_eq!(
			sent(
				&device,
				"https://discord.com/api/v10/channels/7",
				Version::HTTP_11
			),
			["root=1; quoted=\"v\""]
		);
		for other in ["http://discord.com/api", "https://cdn.discord.com/api"] {
			assert!(sent(&device, other, Version::HTTP_2).is_empty());
		}
		// A deletion needs no storable value of its own.
		set(
			&device,
			"/api/v10/users/@me",
			&[
				"quoted=\"\"; Path=/; Max-Age=0",
				"root=a b; Path=/; Max-Age=0",
			],
		);
		assert_eq!(
			sent(
				&device,
				"https://discord.com/api/v10/users/7",
				Version::HTTP_2
			),
			["scoped=2"]
		);
	}

	#[test]
	fn a_full_device_evicts_and_saves_privately() {
		let directory = std::env::temp_dir().join(format!(
			"serein-device-{}-{}",
			std::process::id(),
			unix_now()
		));
		std::fs::create_dir(&directory).unwrap();
		let file = directory.join("discord-device.json");
		let device = Device::default();
		device.state.lock().unwrap().file = Some(file.clone());
		// A planted link in the temporary file's place must not redirect the write.
		#[cfg(unix)]
		{
			std::fs::write(directory.join("victim"), "untouched").unwrap();
			std::os::unix::fs::symlink(
				directory.join("victim"),
				directory.join("discord-device.json.partial"),
			)
			.unwrap();
		}
		let saved = || -> Saved { serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap() };

		set(&device, "/api", &["keep=1; Path=/; Max-Age=1000"]);
		let sessions = (0..MAX_COOKIES - 1)
			.map(|index| format!("s{index}=1; Path=/"))
			.collect::<Vec<_>>();
		set(
			&device,
			"/api",
			&sessions.iter().map(String::as_str).collect::<Vec<_>>(),
		);
		set(&device, "/api", &["new=1; Path=/; Max-Age=1000"]);
		let held = sent(&device, "https://discord.com/api", Version::HTTP_2);
		assert_eq!(held.len(), MAX_COOKIES);
		assert!(held.contains(&"keep=1".to_owned()) && !held.contains(&"s0=1".to_owned()));
		assert_eq!(
			saved()
				.cookies
				.iter()
				.map(|cookie| cookie.name.as_str())
				.collect::<Vec<_>>(),
			["keep", "new"]
		);
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			assert_eq!(
				std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
				0o600
			);
			assert_eq!(
				std::fs::read_to_string(directory.join("victim")).unwrap(),
				"untouched"
			);
		}

		// A small refresh is not rewritten; one past the slack is.
		let first = saved().cookies[0].expires.unwrap();
		set(&device, "/api", &["keep=1; Path=/; Max-Age=2000"]);
		assert_eq!(saved().cookies[0].expires, Some(first));
		set(&device, "/api", &["keep=1; Path=/; Max-Age=200000"]);
		assert!(saved().cookies[0].expires.unwrap() > first + EXPIRY_SLACK);

		std::fs::remove_dir_all(directory).unwrap();
	}
}

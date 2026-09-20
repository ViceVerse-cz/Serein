//! One human-completed invite challenge; never persisted or solved automatically.
use std::{
	fmt,
	time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub const LIFETIME: Duration = Duration::from_secs(300);
#[derive(Clone)]
pub struct Challenge {
	sitekey: String,
	rqdata: Zeroizing<Option<String>>,
	rqtoken: Zeroizing<Option<String>>,
	session_id: Zeroizing<Option<String>>,
	invisible: bool,
}
impl Challenge {
	pub fn new(
		sitekey: String,
		rqdata: Option<String>,
		rqtoken: Option<String>,
		session_id: Option<String>,
		invisible: bool,
	) -> Option<Self> {
		let rqdata = Zeroizing::new(rqdata);
		let rqtoken = Zeroizing::new(rqtoken);
		let session_id = Zeroizing::new(session_id);
		if sitekey.is_empty()
			|| sitekey.capacity() > 128
			|| !sitekey
				.bytes()
				.all(|b| b.is_ascii_alphanumeric() || b == b'-')
			|| !valid_optional(&rqdata, 4096)
			|| !valid_optional(&rqtoken, 2048)
			|| !valid_optional(&session_id, 512)
		{
			return None;
		}
		Some(Self {
			sitekey,
			rqdata,
			rqtoken,
			session_id,
			invisible,
		})
	}
	pub fn sitekey(&self) -> &str {
		&self.sitekey
	}
	pub fn rqdata(&self) -> Option<&str> {
		self.rqdata.as_deref()
	}
	pub fn invisible(&self) -> bool {
		self.invisible
	}
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.sitekey.capacity()
			+ [&self.rqdata, &self.rqtoken, &self.session_id]
				.iter()
				.map(|v| v.as_ref().map_or(0, String::capacity))
				.sum::<usize>()
	}
}
fn valid_optional(value: &Option<String>, max: usize) -> bool {
	value
		.as_ref()
		.is_none_or(|v| v.capacity() <= max && valid(v, max))
}
fn valid(value: &str, max: usize) -> bool {
	!value.is_empty() && value.len() <= max && value.bytes().all(|b| (32..=126).contains(&b))
}
impl fmt::Debug for Challenge {
	/// Redacted debug output; never prints challenge material.
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("Challenge([REDACTED])")
	}
}
pub struct Solution(Zeroizing<String>);
impl Solution {
	pub fn new(value: String) -> Option<Self> {
		let value = Zeroizing::new(value);
		(value.capacity() <= 8192 && valid(&value, 8192)).then_some(Self(value))
	}
}
impl fmt::Debug for Solution {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("Solution([REDACTED])")
	}
}
/// Constructed only after matching the outstanding state request; consumed once.
pub struct Retry {
	pub(crate) target: Target,
	pub(crate) request: u64,
	pub(crate) challenge: Challenge,
	pub(crate) solution: Solution,
	pub(crate) expires: Instant,
}
/// Identity of the single write a solved challenge may resume.
#[derive(Clone, PartialEq, Eq)]
pub enum Target {
	Invite { code: String },
	Friend { user: model::Id },
	Username { username: String },
}
/// The user-visible flow a pending challenge belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Verification {
	Invite { request: u64 },
	Friend { request: u64 },
}
impl Verification {
	/// The request sequence this pending challenge belongs to.
	pub fn request(self) -> u64 {
		match self {
			Self::Invite { request } | Self::Friend { request } => request,
		}
	}
}
impl Retry {
	/// Whether a solved challenge may resume this exact target and request.
	pub fn matches(&self, target: &Target, request: u64) -> bool {
		self.target == *target && self.request == request && !self.expired()
	}
	/// Whether a solved challenge may resume this specific write target.
	pub fn matches_target(&self, target: &Target) -> bool {
		self.target == *target && !self.expired()
	}
	/// Whether the challenge's five-minute lifetime has elapsed.
	pub fn expired(&self) -> bool {
		Instant::now() >= self.expires
	}
	pub fn passcode(&self) -> &str {
		&self.solution.0
	}
	pub fn rqtoken(&self) -> Option<&str> {
		self.challenge.rqtoken.as_deref()
	}
	pub fn session_id(&self) -> Option<&str> {
		self.challenge.session_id.as_deref()
	}
}

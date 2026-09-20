use chacha20poly1305::{
	KeyInit, XChaCha20Poly1305, XNonce,
	aead::{Aead, Payload},
};
use davey::{DaveSession, ProposalsOperationType, SigningKeyPair};
use openmls::prelude::{
	ContentType, MlsMessageBodyIn, MlsMessageIn, Proposal, ProposalOrRefType, Sender,
	SenderExtensionIndex,
	tls_codec::{DeserializeBytes, VLBytes},
};
use std::{num::NonZeroU16, sync::Arc};
use zeroize::Zeroize;

pub(crate) const MODE: &str = "aead_xchacha20_poly1305_rtpsize";
pub(crate) const MAX_PACKET: usize = 4096;
pub(crate) const MAX_SIGNAL: usize = 64 * 1024;
pub(crate) use client_core::voice::MAX_PARTICIPANTS;

/// Ephemeral DAVE identity shared by the call and its Go Live streams.
///
/// Discord requires one identity keypair across simultaneous media sessions in
/// a voice channel. It is never persisted or formatted for diagnostics.
pub struct Identity(SigningKeyPair);
impl Identity {
	pub fn generate() -> Arc<Self> {
		Arc::new(Self(SigningKeyPair::generate()))
	}
}
impl Drop for Identity {
	fn drop(&mut self) {
		self.0.private.zeroize();
		self.0.public.zeroize();
	}
}

pub(crate) struct Encryption {
	cipher: XChaCha20Poly1305,
	counter: u32,
}
/// One authenticated, transport-decrypted RTP packet; the payload is still DAVE ciphertext.
pub struct Rtp {
	pub ssrc: u32,
	pub sequence: u16,
	pub timestamp: u32,
	pub marker: bool,
	pub payload_type: u8,
	pub payload: Vec<u8>,
}
impl Encryption {
	pub fn new(key: &[u8; 32]) -> Self {
		Self {
			cipher: XChaCha20Poly1305::new(key.into()),
			counter: 0,
		}
	}
	pub fn seal(&mut self, header: &[u8; 12], frame: &[u8]) -> Result<Vec<u8>, &'static str> {
		self.seal_packet(header, frame)
	}
	/// Discord's unofficial RFC 8285 extension 9 marks context audio as soundshare.
	/// The extension preamble is authenticated; its four-byte body is encrypted.
	pub fn seal_soundshare(
		&mut self,
		header: &[u8; 12],
		frame: &[u8],
	) -> Result<Vec<u8>, &'static str> {
		if frame.len() > MAX_PACKET - 40 {
			return Err("Stream audio packet exceeds transport limit");
		}
		let mut extended = [0; 16];
		extended[..12].copy_from_slice(header);
		extended[0] |= 0x10;
		extended[12..].copy_from_slice(&[0xbe, 0xde, 0, 1]);
		let mut payload = Vec::with_capacity(4 + frame.len());
		payload.extend([0x90, 0x04, 0, 0]);
		payload.extend_from_slice(frame);
		self.seal_packet(&extended, &payload)
	}
	fn seal_packet(&mut self, header: &[u8], frame: &[u8]) -> Result<Vec<u8>, &'static str> {
		self.counter = self
			.counter
			.checked_add(1)
			.ok_or("Voice transport nonce exhausted; rejoin the call")?;
		let mut nonce = [0; 24];
		nonce[..4].copy_from_slice(&self.counter.to_be_bytes());
		let data = self
			.cipher
			.encrypt(
				XNonce::from_slice(&nonce),
				Payload {
					msg: frame,
					aad: header,
				},
			)
			.map_err(|_| "Voice transport encryption failed")?;
		let mut packet = Vec::with_capacity(header.len() + data.len() + 4);
		packet.extend_from_slice(header);
		packet.extend_from_slice(&data);
		packet.extend_from_slice(&nonce[..4]);
		Ok(packet)
	}
	/// Encrypt one RTCP packet: the eight-byte header stays clear as associated data, the
	/// rest is sealed and the four-byte nonce trails, as in the `rtpsize` RTP framing.
	pub fn seal_rtcp(&mut self, header: &[u8; 8], body: &[u8]) -> Result<Vec<u8>, &'static str> {
		self.seal_packet(header, body)
	}
	/// Authenticate RTCP feedback before honoring a PLI for our video SSRC.
	/// As with `seal_rtcp`, only the first eight bytes remain clear on the wire.
	pub fn requests_keyframe(&self, packet: &[u8], video_ssrc: u32) -> bool {
		if video_ssrc == 0
			|| !(32..=MAX_PACKET).contains(&packet.len())
			|| packet[0] >> 6 != 2
			|| !(192..=223).contains(&packet[1])
		{
			return false;
		}
		let mut nonce = [0; 24];
		nonce[..4].copy_from_slice(&packet[packet.len() - 4..]);
		let Ok(body) = self.cipher.decrypt(
			XNonce::from_slice(&nonce),
			Payload {
				msg: &packet[8..packet.len() - 4],
				aad: &packet[..8],
			},
		) else {
			return false;
		};
		let mut compound = Vec::with_capacity(8 + body.len());
		compound.extend_from_slice(&packet[..8]);
		compound.extend_from_slice(&body);
		let mut remaining = compound.as_slice();
		let mut requested = false;
		while !remaining.is_empty() {
			if remaining.len() < 4 || remaining[0] >> 6 != 2 {
				return false;
			}
			let size = (usize::from(u16::from_be_bytes([remaining[2], remaining[3]])) + 1) * 4;
			if size > remaining.len() {
				return false;
			}
			let mut content = &remaining[..size];
			if content[0] & 0x20 != 0 {
				let padding = usize::from(content[size - 1]);
				if size != remaining.len() || padding == 0 || padding > size - 4 {
					return false;
				}
				content = &content[..size - padding];
			}
			// RFC 4585: PSFB/FMT=1 has a media SSRC and no FCI payload.
			if content[1] == 206 && content[0] & 0x1f == 1 {
				if content.len() != 12 {
					return false;
				}
				requested |= content[8..12] == video_ssrc.to_be_bytes();
			}
			remaining = &remaining[size..];
		}
		requested
	}
	/// Authenticate and decrypt one Opus (120), H264 (101) or H264 RTX (102) RTP packet.
	pub fn open(&self, packet: &[u8]) -> Option<Rtp> {
		if packet.len() < 32
			|| packet.len() > MAX_PACKET
			|| packet[0] >> 6 != 2
			|| !matches!(packet[1] & 0x7f, 120 | 101 | 102)
		{
			return None;
		}
		let csrc_end = 12 + usize::from(packet[0] & 15) * 4;
		let extended = packet[0] & 0x10 != 0;
		let header_len = csrc_end + if extended { 4 } else { 0 };
		if header_len + 20 > packet.len() {
			return None;
		}
		let mut nonce = [0; 24];
		nonce[..4].copy_from_slice(&packet[packet.len() - 4..]);
		let mut frame = self
			.cipher
			.decrypt(
				XNonce::from_slice(&nonce),
				Payload {
					msg: &packet[header_len..packet.len() - 4],
					aad: &packet[..header_len],
				},
			)
			.ok()?;
		if extended {
			let extension_len = usize::from(u16::from_be_bytes(
				packet[csrc_end + 2..csrc_end + 4].try_into().ok()?,
			)) * 4;
			if extension_len > frame.len() {
				return None;
			}
			frame.drain(..extension_len);
		}
		if packet[0] & 0x20 != 0 {
			let padding = usize::from(*frame.last()?);
			if padding == 0 || padding > frame.len() {
				return None;
			}
			frame.truncate(frame.len() - padding);
		}
		Some(Rtp {
			ssrc: u32::from_be_bytes(packet[8..12].try_into().ok()?),
			sequence: u16::from_be_bytes(packet[2..4].try_into().ok()?),
			timestamp: u32::from_be_bytes(packet[4..8].try_into().ok()?),
			marker: packet[1] & 0x80 != 0,
			payload_type: packet[1] & 0x7f,
			payload: frame,
		})
	}
}

pub(crate) struct Dave {
	pub session: DaveSession,
	own: u64,
	peer: Option<u64>,
	participants: Vec<u64>,
	/// Clients the voice server announced as connected; a DM peer is allowed before it arrives.
	announced: Vec<u64>,
	pub waiting: bool,
	channel: u64,
	pub pending: Option<u16>,
	pub ready: bool,
	pub resets: u8,
	epochs: u16,
	identity: Arc<Identity>,
	pending_commit: Option<Vec<u8>>,
}
impl Dave {
	#[cfg(test)]
	pub fn new(own: u64, peer: Option<u64>, channel: u64) -> Result<Self, &'static str> {
		Self::with_identity(own, peer, channel, Identity::generate())
	}
	pub fn with_identity(
		own: u64,
		peer: Option<u64>,
		channel: u64,
		identity: Arc<Identity>,
	) -> Result<Self, &'static str> {
		Ok(Self {
			session: DaveSession::new(NonZeroU16::new(1).unwrap(), own, channel, Some(&identity.0))
				.map_err(|_| "DAVE initialization failed")?,
			own,
			peer,
			participants: std::iter::once(own).chain(peer).collect(),
			announced: vec![own],
			waiting: false,
			channel,
			pending: None,
			ready: false,
			resets: 0,
			epochs: 0,
			identity,
			pending_commit: None,
		})
	}
	pub fn contains(&self, user: u64) -> bool {
		self.participants.contains(&user)
	}
	/// Only this device remains announced in the call.
	pub fn alone(&self) -> bool {
		self.announced.len() == 1
	}
	/// A sole announced member with no pending transition has nobody to negotiate with.
	/// Discord does not always announce a transition for a fresh sole member or after
	/// departures; waiting must not depend on it.
	pub fn should_wait_for_peer(&self) -> bool {
		!self.ready
			&& !self.waiting
			&& self.pending.is_none()
			&& self.alone()
			&& self.session.group().is_some()
	}
	pub fn is_group_member(&self, user: u64) -> bool {
		self.session
			.get_user_ids()
			.is_some_and(|ids| ids.contains(&user))
	}
	pub fn connect(&mut self, users: &[u64]) -> Result<bool, &'static str> {
		if users.len() > MAX_PARTICIPANTS
			|| users.iter().any(|user| {
				*user == 0
					|| self
						.peer
						.is_some_and(|peer| *user != self.own && *user != peer)
			}) {
			return Err("Voice participants do not match this call");
		}
		let mut next = self.participants.clone();
		for user in users {
			if !next.contains(user) {
				if next.len() == MAX_PARTICIPANTS {
					return Err("Voice channel exceeds the 64 participant limit");
				}
				next.push(*user);
			}
		}
		let mut announced = self.announced.clone();
		for user in users {
			if !announced.contains(user) {
				announced.push(*user);
			}
		}
		let changed = next != self.participants || announced != self.announced;
		if changed {
			self.participants = next;
			self.announced = announced;
			self.ready = false;
			self.waiting = false;
		}
		Ok(changed)
	}
	/// A departing member, including a DM peer: the call continues and waits for them to rejoin.
	pub fn disconnect(&mut self, user: u64) -> Result<bool, &'static str> {
		if user == self.own {
			return Err("Discord removed this device from the call");
		}
		let before = self.participants.len() + self.announced.len();
		self.participants.retain(|id| *id != user);
		self.announced.retain(|id| *id != user);
		let changed = before != self.participants.len() + self.announced.len();
		if changed {
			self.ready = false;
			self.waiting = false;
		}
		Ok(changed)
	}
	/// Epoch zero has no media ratchets in Davey. Remain joined without opening audio devices.
	pub fn wait_for_peer(&mut self) -> Result<(), &'static str> {
		self.validate_group()?;
		if !self.alone() || self.session.epoch().is_none_or(|epoch| epoch.as_u64() != 0) {
			return Err("Unexpected sole-member DAVE transition");
		}
		self.pending = None;
		self.ready = false;
		self.waiting = true;
		Ok(())
	}
	/// Safely transition a sole member into waiting mode, reinitializing the group
	/// if it still contains departed peers or has advanced beyond epoch zero.
	pub fn enter_sole_member_waiting(&mut self) -> Result<(), &'static str> {
		if !self.alone() {
			return Err("Cannot enter sole member waiting with peers announced");
		}
		if self.session.epoch().is_none_or(|epoch| epoch.as_u64() != 0)
			|| self.validate_group().is_err()
		{
			self.reinitialize()?;
		}
		self.wait_for_peer()
	}
	pub fn reset(&mut self) -> Result<(), &'static str> {
		self.resets += 1;
		if self.resets > 3 {
			return Err("DAVE recovery limit reached; rejoin the call");
		}
		self.reinitialize()
	}
	pub fn reinitialize(&mut self) -> Result<(), &'static str> {
		self.transition_budget()?;
		self.ready = false;
		self.waiting = false;
		self.pending = None;
		self.pending_commit = None;
		self.session
			.reinit(
				NonZeroU16::new(1).unwrap(),
				self.own,
				self.channel,
				Some(&self.identity.0),
			)
			.map_err(|_| "DAVE reset failed")
	}
	pub fn key_package(&mut self) -> Result<Vec<u8>, &'static str> {
		// Match libdave and discord.py-self: opcode followed by the raw TLS KeyPackage.
		// The whitepaper's MLSMessage wrapper differs from these reference send paths.
		let mut out = vec![26];
		out.extend(
			self.session
				.create_key_package()
				.map_err(|_| "DAVE key package failed")?,
		);
		Ok(out)
	}
	pub fn proposals(&mut self, payload: &[u8]) -> Result<Option<Vec<u8>>, &'static str> {
		if payload.len() > MAX_SIGNAL {
			return Err("DAVE proposals exceed the signaling budget");
		}
		// DAVE initial group creation ignores proposals until the external sender
		// and protocol context have established our local group.
		if self.session.group().is_none() {
			return Ok(None);
		}
		let (&operation, data) = payload.split_first().ok_or("Truncated DAVE proposal")?;
		let operation = match operation {
			0 => ProposalsOperationType::APPEND,
			1 => ProposalsOperationType::REVOKE,
			_ => return Err("Unsupported DAVE proposal operation"),
		};
		if operation == ProposalsOperationType::APPEND {
			let wire: VLBytes = VLBytes::tls_deserialize_exact_bytes(data)
				.map_err(|_| "Invalid DAVE proposal vector")?;
			let mut remaining = wire.as_slice();
			let mut count = 0;
			while !remaining.is_empty() {
				count += 1;
				if count > MAX_PARTICIPANTS * 2 {
					return Err("Too many DAVE proposals");
				}
				let (message, rest) = MlsMessageIn::tls_deserialize_bytes(remaining)
					.map_err(|_| "Invalid MLS proposal message")?;
				remaining = rest;
				let MlsMessageBodyIn::PublicMessage(public) = message.extract() else {
					return Err("DAVE requires external public proposals");
				};
				if *public.sender() != Sender::External(SenderExtensionIndex::new(0))
					|| public.content_type() != ContentType::Proposal
				{
					return Err("DAVE proposal was not sent by the external sender");
				}
			}
		}
		let result = self
			.session
			.process_proposals(operation, data, Some(&self.participants))
			.map_err(|_| "DAVE proposal validation failed")?;
		if let Some(group) = self.session.group()
			&& (group.pending_proposals().count() > MAX_PARTICIPANTS * 2
				|| group.pending_proposals().any(|p| {
					!matches!(p.proposal(), Proposal::Add(_) | Proposal::Remove(_))
						|| *p.sender() != Sender::External(SenderExtensionIndex::new(0))
						|| p.proposal_or_ref_type() != ProposalOrRefType::Reference
				})) {
			return Err("Disallowed DAVE proposal");
		}
		match result {
			Some(result) => {
				self.pending_commit = Some(result.commit.clone());
				let mut frame = vec![28];
				frame.extend(result.commit);
				if let Some(welcome) = result.welcome {
					frame.extend(welcome);
				}
				if frame.len() > MAX_SIGNAL {
					return Err("DAVE response exceeds call budget");
				}
				Ok(Some(frame))
			}
			None => {
				self.pending_commit = None;
				Ok(None)
			}
		}
	}
	pub fn group_changed(&mut self, opcode: u8, payload: &[u8]) -> Result<u16, &'static str> {
		if payload.len() < 3 || payload.len() > MAX_SIGNAL {
			return Err("Truncated DAVE group transition");
		}
		self.ready = false;
		self.waiting = false;
		self.transition_budget()?;
		let transition = u16::from_be_bytes([payload[0], payload[1]]);
		if opcode == 29 {
			if self
				.session
				.epoch()
				.is_some_and(|epoch| epoch.as_u64() == 0)
				&& self.pending_commit.as_deref() != Some(&payload[2..])
			{
				return Err("Initial DAVE commit differs from the locally proposed commit");
			}
			self.session
				.process_commit(&payload[2..])
				.map_err(|_| "DAVE commit validation failed")?;
		} else {
			self.session
				.process_welcome(&payload[2..])
				.map_err(|_| "DAVE welcome validation failed")?;
		}
		self.validate_group()?;
		self.pending_commit = None;
		self.pending = Some(transition);
		if transition == 0 {
			self.execute(transition)?;
		}
		Ok(transition)
	}
	fn transition_budget(&mut self) -> Result<(), &'static str> {
		self.epochs = self
			.epochs
			.checked_add(1)
			.ok_or("Call key transition budget exhausted")?;
		// ponytail: cap long-lived MLS storage at 1024 transitions; rejoin creates a fresh provider.
		if self.epochs > 1024 {
			return Err("Call key transition budget exhausted; rejoin the call");
		}
		Ok(())
	}
	fn validate_group(&self) -> Result<(), &'static str> {
		let group = self.session.group().ok_or("DAVE group is missing")?;
		let ids = self
			.session
			.get_user_ids()
			.ok_or("DAVE members are missing")?;
		if group.group_id().as_slice() != self.channel.to_be_bytes()
			|| ids.is_empty()
			|| ids.len() > MAX_PARTICIPANTS
			|| !ids.contains(&self.own)
			|| ids.iter().any(|id| !self.contains(*id))
			|| ids.iter().enumerate().any(|(i, id)| ids[..i].contains(id))
			|| (self.peer.is_some() && ids.len() > 2)
		{
			return Err("DAVE group does not match the authenticated call participants");
		}
		Ok(())
	}
	pub fn execute(&mut self, id: u16) -> Result<(), &'static str> {
		if self.pending != Some(id) || !self.session.is_ready() {
			return Err("Unexpected DAVE encryption transition");
		}
		self.validate_group()?;
		self.pending = None;
		self.ready = true;
		Ok(())
	}
}

impl Drop for Dave {
	fn drop(&mut self) {
		let _ = self.session.reset();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use openmls::prelude::{OpenMlsProvider, ProtocolVersion};
	#[test]
	fn proposals_before_local_group_are_ignored_without_weakening_later_validation() {
		let server = crate::test_mls::Delivery::new();
		let mut alice = Dave::new(1, Some(2), 3).unwrap();
		let mut bob = Dave::new(2, Some(1), 3).unwrap();
		bob.session.set_external_sender(&server.external).unwrap();
		let early = server.add_proposal(&bob, &alice.key_package().unwrap());
		assert!(alice.session.group().is_none());
		assert!(alice.proposals(&early).unwrap().is_none());
		assert!(alice.pending_commit.is_none());
		assert!(!alice.ready);
		assert!(alice.proposals(&vec![0; MAX_SIGNAL + 1]).is_err());
		alice.session.set_external_sender(&server.external).unwrap();
		assert!(alice.proposals(&[2]).is_err()); // Established context still validates the operation.
		let (commit, welcome) = server.add(&mut bob, &alice.key_package().unwrap());
		bob.group_changed(29, &[&[0, 0], commit.as_slice()].concat())
			.unwrap();
		alice
			.group_changed(30, &[&[0, 0], welcome.as_slice()].concat())
			.unwrap();
		assert!(alice.ready);
		assert_eq!(
			alice.session.voice_privacy_code(),
			bob.session.voice_privacy_code()
		);
	}
	#[test]
	fn rtcp_keyframe_requests_require_authentication_and_matching_ssrc() {
		fn sealed(crypto: &mut Encryption, clear: &[u8]) -> Vec<u8> {
			crypto
				.seal_rtcp(clear[..8].try_into().unwrap(), &clear[8..])
				.unwrap()
		}
		let mut crypto = Encryption::new(&[7; 32]);
		let pli = [0x81, 206, 0, 2, 0, 0, 0, 9, 0, 0, 0, 42];
		let packet = sealed(&mut crypto, &pli);
		assert!(crypto.requests_keyframe(&packet, 42));
		assert!(!crypto.requests_keyframe(&packet, 41));
		assert!(!crypto.requests_keyframe(&packet, 0));
		assert!(!Encryption::new(&[8; 32]).requests_keyframe(&packet, 42));
		for i in 0..packet.len() {
			let mut corrupt = packet.clone();
			corrupt[i] ^= 0x40;
			assert!(!crypto.requests_keyframe(&corrupt, 42));
			assert!(!crypto.requests_keyframe(&packet[..i], 42));
		}
		assert!(!crypto.requests_keyframe(&vec![0; MAX_PACKET + 1], 42));

		// The server may bundle receiver reports before the PLI in one authenticated packet.
		let mut compound = vec![0x80, 201, 0, 1, 0, 0, 0, 9];
		compound.extend(pli);
		let packet = sealed(&mut crypto, &compound);
		assert!(crypto.requests_keyframe(&packet, 42));
		compound.extend([0x80, 201, 0]); // A valid PLI cannot hide a truncated trailing packet.
		let packet = sealed(&mut crypto, &compound);
		assert!(!crypto.requests_keyframe(&packet, 42));

		let mut malformed = pli;
		malformed[3] = 3; // Authenticated but claims more bytes than are present.
		let packet = sealed(&mut crypto, &malformed);
		assert!(!crypto.requests_keyframe(&packet, 42));
		malformed[3] = 1; // Authenticated but missing the required media SSRC.
		let packet = sealed(&mut crypto, &malformed);
		assert!(!crypto.requests_keyframe(&packet, 42));

		let mut padded = pli.to_vec();
		padded[0] |= 0x20;
		padded[3] = 3;
		padded.extend([0, 0, 0, 4]);
		let packet = sealed(&mut crypto, &padded);
		assert!(crypto.requests_keyframe(&packet, 42));
		padded[15] = 0;
		let packet = sealed(&mut crypto, &padded);
		assert!(!crypto.requests_keyframe(&packet, 42));
		padded[15] = 13; // Padding may not consume the RTCP header.
		let packet = sealed(&mut crypto, &padded);
		assert!(!crypto.requests_keyframe(&packet, 42));
	}
	#[test]
	fn soundshare_extension_is_authenticated_encrypted_and_stripped_before_dave() {
		let mut crypto = Encryption::new(&[7; 32]);
		let header = [0x80, 120, 0, 1, 0, 0, 0, 1, 0, 0, 0, 9];
		let frame = b"synthetic DAVE ciphertext";
		let packet = crypto.seal_soundshare(&header, frame).unwrap();
		assert_eq!(packet[0], 0x90);
		assert_eq!(&packet[1..12], &header[1..]);
		assert_eq!(&packet[12..16], &[0xbe, 0xde, 0, 1]);
		let mut nonce = [0; 24];
		nonce[..4].copy_from_slice(&packet[packet.len() - 4..]);
		let clear = crypto
			.cipher
			.decrypt(
				XNonce::from_slice(&nonce),
				Payload {
					aad: &packet[..16],
					msg: &packet[16..packet.len() - 4],
				},
			)
			.unwrap();
		assert_eq!(&clear[..4], &[0x90, 0x04, 0, 0]);
		assert_eq!(&clear[4..], frame);
		assert_eq!(crypto.open(&packet).unwrap().payload, frame);
		for i in 0..packet.len() {
			let mut corrupt = packet.clone();
			corrupt[i] ^= 0x40;
			assert!(crypto.open(&corrupt).is_none());
		}
		assert!(
			crypto
				.seal_soundshare(&header, &vec![0; MAX_PACKET])
				.is_err()
		);
	}
	#[test]
	fn rtp_authentication_bounds_and_nonce_exhaustion() {
		assert_eq!(
			tracing::level_filters::STATIC_MAX_LEVEL,
			tracing::level_filters::LevelFilter::OFF
		);
		let mut crypto = Encryption::new(&[7; 32]);
		let header = [0x80, 120, 0, 1, 0, 0, 0, 1, 0, 0, 0, 9];
		let packet = crypto
			.seal(&header, b"synthetic encrypted DAVE frame")
			.unwrap();
		let opened = crypto.open(&packet).unwrap();
		assert_eq!(
			(
				opened.ssrc,
				opened.sequence,
				opened.payload_type,
				opened.marker
			),
			(9, 1, 120, false)
		);
		assert_eq!(opened.payload, b"synthetic encrypted DAVE frame".to_vec());
		let mut video = header;
		video[1] = 101 | 0x80;
		let sealed = crypto.seal(&video, b"h264").unwrap();
		let opened = crypto.open(&sealed).unwrap();
		assert!(opened.marker && opened.payload_type == 101 && opened.timestamp == 1);
		let mut other = header;
		other[1] = 96;
		let sealed = crypto.seal(&other, b"x").unwrap();
		assert!(crypto.open(&sealed).is_none());
		for i in 0..packet.len() {
			let mut corrupt = packet.clone();
			corrupt[i] ^= 0x40;
			assert!(crypto.open(&corrupt).is_none());
		}
		for i in 0..packet.len() {
			assert!(crypto.open(&packet[..i]).is_none());
		}
		// Authenticated RTP extension preamble remains clear; its body is encrypted and stripped.
		let mut extension_header = header.to_vec();
		extension_header[0] = 0x90;
		extension_header.extend([0xbe, 0xde, 0, 1]);
		let nonce = [0u8; 24];
		let encrypted = crypto
			.cipher
			.encrypt(
				XNonce::from_slice(&nonce),
				Payload {
					msg: &[1, 2, 3, 4, 9, 8, 7],
					aad: &extension_header,
				},
			)
			.unwrap();
		let mut extension_packet = extension_header;
		extension_packet.extend(encrypted);
		extension_packet.extend([0; 4]);
		assert_eq!(
			crypto.open(&extension_packet).unwrap().payload,
			vec![9, 8, 7]
		);
		crypto.counter = u32::MAX;
		assert!(crypto.seal(&header, b"x").is_err());
		let mut dave = Dave::new(1, Some(2), 3).unwrap();
		assert!(!dave.ready);
		assert!(dave.execute(0).is_err());
		let package = dave.key_package().unwrap();
		assert_eq!(&package[..5], &[26, 0, 1, 0, 2]);
		let package =
			openmls::prelude::KeyPackageIn::tls_deserialize_exact_bytes(&package[1..]).unwrap();
		let provider = openmls_rust_crypto::OpenMlsRustCrypto::default();
		package
			.validate(provider.crypto(), ProtocolVersion::Mls10)
			.unwrap();
		assert!(dave.group_changed(30, &[0, 0, 0]).is_err());
		assert!(!dave.ready);
	}
	#[test]
	fn sole_member_departure_waiting_and_group_membership() {
		let server = crate::test_mls::Delivery::new();
		let mut alice = Dave::new(1, Some(2), 3).unwrap();
		let mut bob = Dave::new(2, Some(1), 3).unwrap();
		alice.session.set_external_sender(&server.external).unwrap();
		bob.session.set_external_sender(&server.external).unwrap();

		alice.connect(&[2]).unwrap();
		bob.connect(&[1]).unwrap();
		assert!(!alice.is_group_member(2));
		let (commit, welcome) = server.add(&mut alice, &bob.key_package().unwrap());
		alice
			.group_changed(29, &[&[0, 0], commit.as_slice()].concat())
			.unwrap();
		bob.group_changed(30, &[&[0, 0], welcome.as_slice()].concat())
			.unwrap();
		assert!(alice.ready && bob.ready);
		assert!(alice.is_group_member(2));
		assert!(!alice.is_group_member(999));

		// When not alone, enter_sole_member_waiting fails closed.
		assert!(alice.enter_sole_member_waiting().is_err());

		// Bob departs: Alice is now alone and was in an established epoch > 0 group.
		assert!(alice.disconnect(2).unwrap());
		assert!(alice.alone());
		assert!(alice.should_wait_for_peer());

		// enter_sole_member_waiting reinitializes the epoch > 0 group and enters waiting cleanly.
		alice.enter_sole_member_waiting().unwrap();
		assert!(alice.waiting);
		assert!(!alice.ready);
	}
}

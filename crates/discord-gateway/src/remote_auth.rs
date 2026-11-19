//! QR sign-in (unofficial, live-unverified): Discord's remote-auth v2 socket, as its login
//! page uses it. The socket shows one code and reports what the owner's phone does with it;
//! it never carries the account token. An approved login yields a ticket, which REST trades
//! for a token encrypted to this session's one-use RSA key. Wire shapes follow SakuraCord's
//! `DiscordRemoteAuthManager` (commit aa8a3da8) and discord-userdoccers.
use btls::{
	pkey::Private,
	rsa::{Padding, Rsa},
};
use client_core::auth::SessionSecret;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::time::{Instant, sleep_until, timeout};
use tokio_tungstenite::tungstenite::{Message as Frame, protocol::WebSocketConfig};
use zeroize::Zeroizing;

const URL: &str = "wss://remote-auth-gateway.discord.gg/?v=2";
/// Every message is a small JSON object; the largest holds one RSA block in base64.
const MAX_FRAME: usize = 16 * 1024;
const KEY_BITS: u32 = 2048;
const HASH: usize = 32;
/// Discord closes an unscanned code after about two minutes; this bounds a silent socket.
const MAX_SESSION: Duration = Duration::from_secs(300);

/// Unofficial: the close code for a code nobody scanned in time.
const EXPIRED_CLOSE: u16 = 4003;

/// Where a session is in its handshake.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
	Connected,
	Greeted,
	Proved,
	Shown,
	Scanned,
}

/// The account that scanned the code, as the phone reported it. Display only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
	pub id: model::Id,
	pub name: String,
	pub discriminator: String,
	pub avatar: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
	/// The address to show as a QR code.
	Code(String),
	/// A phone scanned the code and is asking its owner to confirm.
	Scanned(Option<Account>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
	/// The phone's owner declined.
	Cancelled,
	/// Nobody scanned the code before Discord closed it.
	Expired,
	Failed(&'static str),
}

/// One QR sign-in attempt and the key its answers are encrypted to.
pub struct RemoteAuth {
	key: Rsa<Private>,
}
impl RemoteAuth {
	/// Generates the session key. CPU-bound for a moment; call it off the async runtime.
	pub fn new() -> Result<Self, Stop> {
		Rsa::generate(KEY_BITS)
			.map(|key| Self { key })
			.map_err(|_| Stop::Failed("Could not create a key for QR sign-in."))
	}
	/// Runs the socket until the phone approves, returning the ticket for the REST exchange.
	pub async fn approve(
		&self,
		mut progress: impl FnMut(Progress) + Send,
	) -> Result<Zeroizing<String>, Stop> {
		const LOST: Stop = Stop::Failed("QR sign-in lost its connection.");
		const INVALID: Stop =
			Stop::Failed("Discord sent a QR sign-in message Serein does not understand.");
		#[derive(serde::Deserialize)]
		struct Packet {
			op: String,
			heartbeat_interval: Option<u64>,
			timeout_ms: Option<u64>,
			encrypted_nonce: Option<String>,
			fingerprint: Option<String>,
			encrypted_user_payload: Option<String>,
			ticket: Option<String>,
		}
		let public = self.key.public_key_to_der().map_err(|_| INVALID)?;
		let config = WebSocketConfig::default()
			.max_message_size(Some(MAX_FRAME))
			.max_frame_size(Some(MAX_FRAME))
			.write_buffer_size(0)
			.max_write_buffer_size(MAX_FRAME);
		let mut socket = timeout(
			Duration::from_secs(15),
			client_core::fingerprint::websocket(URL, config),
		)
		.await
		.map_err(|_| LOST)?
		.map_err(|_| Stop::Failed("Could not reach Discord's QR sign-in service."))?;
		let mut deadline = Instant::now() + MAX_SESSION;
		let mut heartbeat: Option<(Duration, Instant)> = None;
		let mut acknowledged = true;
		// Each step is taken once and in order; anything else ends the session.
		let mut phase = Phase::Connected;
		loop {
			let beat = heartbeat.map_or(deadline, |(_, at)| at.min(deadline));
			let frame = tokio::select! {
				frame = socket.next() => frame,
				() = sleep_until(beat) => {
					if Instant::now() >= deadline {
						return Err(if phase == Phase::Shown {
							Stop::Expired
						} else {
							Stop::Failed("QR sign-in was not confirmed in time.")
						});
					}
					let Some((interval, _)) = heartbeat else {
						return Err(LOST);
					};
					// A socket that stopped answering would otherwise keep a dead code on screen.
					if !std::mem::take(&mut acknowledged) {
						return Err(LOST);
					}
					send(&mut socket, serde_json::json!({"op": "heartbeat"})).await?;
					heartbeat = Some((interval, Instant::now() + interval));
					continue;
				}
			};
			let text = match frame {
				Some(Ok(Frame::Text(text))) => text,
				Some(Ok(Frame::Ping(_) | Frame::Pong(_))) => continue,
				// Unofficial: Discord closes a code nobody scanned with 4003. Any other end,
				// or one after a scan, is a lost connection and is not renewed on its own.
				Some(Ok(Frame::Close(Some(close))))
					if phase == Phase::Shown && u16::from(close.code) == EXPIRED_CLOSE =>
				{
					return Err(Stop::Expired);
				}
				_ => return Err(LOST),
			};
			let packet: Packet = serde_json::from_str(text.as_str()).map_err(|_| INVALID)?;
			match (packet.op.as_str(), phase) {
				("hello", Phase::Connected) => {
					let interval = packet
						.heartbeat_interval
						.filter(|ms| (1000..=120_000).contains(ms))
						.map(Duration::from_millis)
						.ok_or(INVALID)?;
					// Discord's own lifetime for the code, never longer than the local cap.
					if let Some(lifetime) = packet.timeout_ms {
						deadline = deadline
							.min(Instant::now() + Duration::from_millis(lifetime.max(1000)));
					}
					send(
						&mut socket,
						serde_json::json!({"op": "init", "encoded_public_key": btls::base64::encode_block(&public)}),
					)
					.await?;
					heartbeat = Some((interval, Instant::now() + interval));
					phase = Phase::Greeted;
				}
				("heartbeat_ack", _) => acknowledged = true,
				("nonce_proof", Phase::Greeted) => {
					let nonce = packet
						.encrypted_nonce
						.and_then(|nonce| self.decrypt(&nonce))
						.ok_or(INVALID)?;
					send(
						&mut socket,
						serde_json::json!({"op": "nonce_proof", "nonce": base64_url(&nonce)}),
					)
					.await?;
					phase = Phase::Proved;
				}
				("pending_remote_init", Phase::Proved) => {
					// The fingerprint is the SHA-256 of the key sent in `init` (unofficial;
					// matches the discord-userdoccers example), which ties the code on screen
					// to this session's key. Both values are public.
					let expected = base64_url(&btls::sha::sha256(&public));
					if packet.fingerprint.as_deref() != Some(expected.as_str()) {
						return Err(Stop::Failed(
							"Discord's QR code did not match this sign-in's key.",
						));
					}
					progress(Progress::Code(format!("https://discord.com/ra/{expected}")));
					phase = Phase::Shown;
				}
				("pending_ticket", Phase::Shown) => {
					let account = packet
						.encrypted_user_payload
						.and_then(|payload| self.decrypt(&payload))
						.and_then(|payload| account(&payload));
					progress(Progress::Scanned(account));
					phase = Phase::Scanned;
				}
				("pending_login", Phase::Scanned) => {
					let ticket = Zeroizing::new(packet.ticket.ok_or(INVALID)?);
					if ticket.is_empty()
						|| ticket.len() > 4096
						|| !ticket.bytes().all(|b| b.is_ascii_graphic())
					{
						return Err(INVALID);
					}
					let _ = timeout(Duration::from_secs(2), socket.close(None)).await;
					return Ok(ticket);
				}
				("cancel", _) => return Err(Stop::Cancelled),
				// A step out of order is not followed.
				(
					"hello"
					| "nonce_proof"
					| "pending_remote_init"
					| "pending_ticket"
					| "pending_login",
					_,
				) => return Err(INVALID),
				// Anything newer than this client.
				_ => {}
			}
		}
	}
	/// Opens the token the ticket exchange returned.
	pub fn open_token(&self, encrypted: &str) -> Option<SessionSecret> {
		let token = self.decrypt(encrypted)?;
		let token = Zeroizing::new(String::from_utf8(token.to_vec()).ok()?);
		SessionSecret::from_owner_input(token.trim().to_owned()).ok()
	}
	/// RSA-OAEP with SHA-256, as WebCrypto's `RSA-OAEP` does it on Discord's login page. The
	/// private-key operation is BoringSSL's; its safe wrapper only offers OAEP with SHA-1, so
	/// the padding is checked here. Only Discord, over TLS, supplies ciphertexts, and a
	/// failure tells it nothing it did not choose.
	fn decrypt(&self, base64: &str) -> Option<Zeroizing<Vec<u8>>> {
		let size = self.key.size() as usize;
		if base64.len() > size * 2 {
			return None;
		}
		let cipher = btls::base64::decode_block(base64.trim()).ok()?;
		// Exactly one modulus: anything longer or shorter is not an RSA block for this key.
		(cipher.len() == size).then_some(())?;
		let mut block = Zeroizing::new(vec![0_u8; size]);
		let written = self
			.key
			.private_decrypt(&cipher, &mut block, Padding::NONE)
			.ok()?;
		(written == size).then_some(())?;
		unpad(&block)
	}
}

/// Unpadded URL-safe base64, as the remote-auth socket spells hashes and proofs.
fn base64_url(bytes: &[u8]) -> String {
	btls::base64::encode_block(bytes)
		.trim_end_matches('=')
		.replace('+', "-")
		.replace('/', "_")
}

async fn send<S>(socket: &mut S, packet: serde_json::Value) -> Result<(), Stop>
where
	S: SinkExt<Frame> + Unpin,
{
	timeout(
		Duration::from_secs(5),
		socket.send(Frame::Text(packet.to_string().into())),
	)
	.await
	.ok()
	.and_then(Result::ok)
	.ok_or(Stop::Failed("QR sign-in lost its connection."))
}

/// `id:discriminator:avatar:username`, as the phone's client reports the scanning account.
fn account(payload: &[u8]) -> Option<Account> {
	let text = std::str::from_utf8(payload).ok()?;
	let mut parts = text.splitn(4, ':');
	let id = parts.next()?.parse::<u64>().ok().filter(|id| *id != 0)?;
	let discriminator = parts.next()?;
	let avatar = parts.next()?;
	let name = parts.next()?;
	let plain =
		|value: &str, max: usize| value.len() <= max && !value.chars().any(char::is_control);
	if name.is_empty()
		|| !plain(name, 128)
		|| !plain(discriminator, 8)
		|| avatar.len() > 64
		|| !avatar
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b == b'_')
	{
		return None;
	}
	Some(Account {
		id: model::Id(id),
		name: name.to_owned(),
		discriminator: discriminator.to_owned(),
		avatar: (!avatar.is_empty() && avatar != "0").then(|| avatar.to_owned()),
	})
}

/// Removes EME-OAEP padding (RFC 8017 §7.1.2, SHA-256 for both the label hash and MGF1,
/// empty label) from one raw RSA block.
fn unpad(block: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
	if block.len() < 2 * HASH + 2 || block[0] != 0 {
		return None;
	}
	let (masked_seed, masked_data) = block[1..].split_at(HASH);
	let mut seed = Zeroizing::new(masked_seed.to_vec());
	mask(&mut seed, masked_data);
	let mut data = Zeroizing::new(masked_data.to_vec());
	mask(&mut data, &seed);
	let (label, message) = data.split_at(HASH);
	if !btls::memcmp::eq(label, &btls::sha::sha256(&[])) {
		return None;
	}
	let start = message.iter().position(|byte| *byte != 0)?;
	(message[start] == 1).then(|| Zeroizing::new(message[start + 1..].to_vec()))
}

/// XORs `target` with MGF1-SHA-256 of `seed`.
fn mask(target: &mut [u8], seed: &[u8]) {
	let mut input = Zeroizing::new(Vec::with_capacity(seed.len() + 4));
	for (counter, chunk) in target.chunks_mut(HASH).enumerate() {
		input.clear();
		input.extend_from_slice(seed);
		input.extend_from_slice(&(counter as u32).to_be_bytes());
		for (byte, mask) in chunk.iter_mut().zip(btls::sha::sha256(&input)) {
			*byte ^= mask;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	/// A throwaway 2048-bit key (PKCS#1 DER) made for this test; it protects nothing.
	const KEY: &str = concat!(
		"MIIEpAIBAAKCAQEAqulaWYz4JqzZRBu0iB/HPVn0z4Qiyn+2SFMeZs07B8kELd0Gez2lnrUe4fNdjZomik6FJzUf",
		"4BnZgT9ELglVFysyzkOd6+OmzrTaP00jkXeIkFxhe2bLMIh/iew7beRry+316TgkUu1z1R/026X3A3Rhqn5+0wCC",
		"zvaOHULrxQLaJY9lBn3EXU/zVzlwcavntC/469RSEBM6Ir6lrg0abXUhYPgNt0KwtF6A+hPpypQdM2t9ojbSdIqX",
		"8PqU0tI3rJYAYjZv8XHJj6xoP/ylyIR+WoIqGX+4tMCj1VaBOMu7fhEbuiooIZ/q2Ou9SxqfkHNXfMKK9nXpxLpx",
		"HZLTbwIDAQABAoIBAA1qiPmbm8Upb1qt1BOVbwXtYE7D4BlGQjBWOf424Qng6S+SeJZHBg+5xwVUncTmU10rAb/M",
		"p0ZnsQ+4wCje+at6bCAN5cElGyyBvNoYO5EJMZKndc2CibOzG1Pf0+Bx/FZDRTj7+7n4ws15lzaAxi2ivqHsEReO",
		"JDaL2ycH4EsUJ8mL0snjXlIPxqrW9NM5MSBN5X7+bJdeLcHHxnMeXRuGUAueoy+jGdWZ+upRCFKGORchnPIGHM+p",
		"IwpBKoq1C6c1sBH6K9uZQPdP4uaPKz/3rNu40sJNIVGKMVUESIWhC45Zo8hlNl3yOsBcBZ6WMRi7eAstVnYWZSwA",
		"kBbEKkUCgYEA4wGH0oXG/wak1vGTfwJV9UBMzLlQ4avYqSVJNDPhlRX/DjMOpYuAl4EYnqSLPcDA9mGfbvurP7PL",
		"2z9iPVZXFzpX1GvjB2CW9dvhP/FBVOI32jYXunQ+Iy3aTmAxAaaeUhQWj7qaRVS7mrCsb4vNJZEUIM70xXYGIMPG",
		"DAums0UCgYEAwL2wUrKRSi5uXXrXMvy7t7V/T3vkLAEQlgSfdKrST7qQ5KFmZL+i39CUW5bMlw+YTXEmPy+8X3wN",
		"Jt25PpY0GT4zKRFcACbn/FrAhdAVkZVQzhJeOHQSdCDWapiSOeJEC1HI4L2z02qB/6m2ztL+K//kI4NpnmrfPScv",
		"W6xrnSMCgYBXuUmxkzz5ZKni3mwvUsnBAt4q5xpYez44703AvKPaJyvvYs+wLZ697YeEFvYwIQLws30XOG5PKb4H",
		"thDSv0DcmxFB54dOXIc574S2XxbuJJWM4AZ/aVhx49BPJWEK0EayVUz6shPX4wy0lL2uRYo7kDsFCrUQcbGgi8dG",
		"9GGxOQKBgQCvTRfM73qzxK0+upZ2WmYFvb8QNPefikxilcvN2fw05dQZDi9KHP5voI7/CpYEIvv8G5iUyb8D/Zkc",
		"q3dr1EOIXgk8JfzOapR+8q++RTcI1YzMEOq/X9rU4nUCOMhuezw2K2mP9xpShMG+HccWL9Nbg38lScXLfxqRzU0G",
		"jTjHjwKBgQDNb1hj/dcL/TQ9LZkK4UvrEUNhEAOWJgEn8S3xeAdglkwI6qz6W+xOgfF0ey4yPKlNXODlWrRHU1+4",
		"MwN8DfnY+PG6ZE+8qudGsG6ML65P54KtFLt6OzHpecRZIaBJWxycsT1WBeaFqUT/zM3z6OfjErOKa87cEUpvCDJK",
		"nxxZdg==",
	);
	/// `openssl pkeyutl -encrypt` of the message below to that key with OAEP, SHA-256 for
	/// both the label hash and MGF1, as WebCrypto's RSA-OAEP produces it.
	const CIPHER: &str = concat!(
		"W0+r0ZqQAuuhdZ359Sb+bqCJzfQ/XcV5VfUq7JjpLp+5hWq9O7upV+p9dD5UEyjxLArzCGUc1WW5gKy283rmPbDQ",
		"qjizVds4Zh57r4ENPWlEnubXrUVCA5SaDu+s8xL0LmJ9WXPE26yYwEtVxD5pnWgdxFNKvj6R9flMzypQj4wVoa0D",
		"gDHEt0DHx99g38Jer8vyUuMJPfrTG2KYsulcwZEFyvw9BeJtenzaoCSjx8pj7gcju6e/G21bNd25e3SPGZkVY1nX",
		"I2no3srIUm1pTl1rxTrI3P7F101miXUDryGKvuFOjXpewuRlZ03W01C5X4dEwOIPVx9E0Bjy4y3EhA==",
	);
	#[test]
	fn opens_openssl_oaep_sha256_and_rejects_altered_ciphertext() {
		let key = btls::base64::decode_block(KEY).unwrap();
		let auth = RemoteAuth {
			key: Rsa::private_key_from_der(&key).unwrap(),
		};
		assert_eq!(
			auth.decrypt(CIPHER).as_deref().map(Vec::as_slice),
			Some(&b"synthetic-qr-token.not-a-credential"[..])
		);
		assert_eq!(
			auth.open_token(CIPHER).unwrap().expose(),
			"synthetic-qr-token.not-a-credential"
		);
		// One changed character decrypts to a block whose padding no longer checks out.
		let altered = CIPHER.replacen('A', "B", 1);
		assert_ne!(altered, CIPHER);
		assert!(auth.decrypt(&altered).is_none());
		assert!(auth.decrypt(&CIPHER[..CIPHER.len() - 8]).is_none());
		assert!(auth.decrypt(&"A".repeat(4096)).is_none());
	}
	#[test]
	fn scanned_account_keeps_colons_in_the_name_and_rejects_malformed_payloads() {
		let parsed = account(b"42:0:abc123:name:with:colons").unwrap();
		assert_eq!(parsed.id, model::Id(42));
		assert_eq!(parsed.name, "name:with:colons");
		assert_eq!(parsed.avatar.as_deref(), Some("abc123"));
		assert_eq!(account(b"42:0::plain").unwrap().avatar, None);
		let malformed: [&[u8]; 7] = [
			b"42:0:abc",
			b"0:0::name",
			b"x:0::name",
			b"42:0:../../etc:name",
			b"42:0::line\nbreak",
			b"42:0::",
			&[0xff, b':', b'0', b':', b':', b'n'],
		];
		for payload in malformed {
			assert!(account(payload).is_none(), "{payload:?}");
		}
	}
}

//! Offline compatibility peer. Built only in temporary workspaces by run.py.
use davey::{DaveSession, MediaType, ProposalsOperationType, SigningKeyPair};
use openmls::prelude::tls_codec::Serialize as _;
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use serde_json::{Value, json};
use std::io::{self, BufRead, Read, Write};
use std::num::NonZeroU16;

const LIMIT: u64 = 64 * 1024;
fn bytes(v: &Value, key: &str) -> Vec<u8> {
	v[key]
		.as_array()
		.unwrap()
		.iter()
		.map(|b| u8::try_from(b.as_u64().unwrap()).unwrap())
		.collect()
}
struct Peer {
	session: Option<DaveSession>,
	server: SignatureKeyPair,
}
impl Peer {
	fn request(&mut self, v: Value) -> Result<Value, Box<dyn std::error::Error>> {
		if v["op"] == "external" {
			let external = ExternalSender::new(
				self.server.public().to_vec().into(),
				BasicCredential::new(b"offline legacy delivery".to_vec()).into(),
			);
			return Ok(json!({"bytes": external.tls_serialize_detached()?}));
		}
		if v["op"] == "init" {
			let key = SigningKeyPair::generate();
			let mut session = DaveSession::new(
				NonZeroU16::new(1).unwrap(),
				v["user"].as_u64().unwrap(),
				3,
				Some(&key),
			)?;
			session.set_external_sender(&bytes(&v, "external"))?;
			self.session = Some(session);
			return Ok(json!({"ok":true}));
		}
		let s = self.session.as_mut().unwrap();
		Ok(match v["op"].as_str().unwrap() {
			"package" => json!({"bytes":s.create_key_package()?}),
			"proposal" => {
				let provider = OpenMlsRustCrypto::default();
				let package = KeyPackageIn::tls_deserialize_exact_bytes(&bytes(&v, "bytes"))?
					.validate(provider.crypto(), ProtocolVersion::Mls10)?;
				let group = s.group().unwrap();
				let proposal = ExternalProposal::new_add::<OpenMlsRustCrypto>(
					package,
					group.group_id().clone(),
					group.epoch(),
					&self.server,
					SenderExtensionIndex::new(0),
				)?
				.tls_serialize_detached()?;
				json!({"bytes":VLBytes::new(proposal).tls_serialize_detached()?})
			}
			"proposals" => {
				let result = s
					.process_proposals(
						ProposalsOperationType::APPEND,
						&bytes(&v, "bytes"),
						Some(&[1, 2]),
					)?
					.unwrap();
				json!({"commit":result.commit,"welcome":result.welcome.unwrap()})
			}
			"commit" => {
				s.process_commit(&bytes(&v, "bytes"))?;
				json!({"ok":true})
			}
			"welcome" => {
				s.process_welcome(&bytes(&v, "bytes"))?;
				json!({"ok":true})
			}
			"encrypt" => json!({"bytes":s.encrypt_opus(&bytes(&v,"bytes"))?.into_owned()}),
			"decrypt" => {
				json!({"bytes":s.decrypt(v["user"].as_u64().unwrap(),MediaType::AUDIO,&bytes(&v,"bytes"))?})
			}
			"privacy" => {
				json!({"code":s.voice_privacy_code().unwrap(),"epoch":s.epoch().unwrap().as_u64()})
			}
			_ => return Err("unsupported offline peer operation".into()),
		})
	}
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
	let mut peer = Peer {
		session: None,
		server: SignatureKeyPair::new(SignatureScheme::ECDSA_SECP256R1_SHA256)?,
	};
	let stdin = io::stdin();
	let mut reader = stdin.lock();
	let stdout = io::stdout();
	let mut writer = stdout.lock();
	loop {
		let mut line = Vec::new();
		let n = (&mut reader).take(LIMIT + 1).read_until(b'\n', &mut line)?;
		if n == 0 {
			break;
		}
		if n as u64 > LIMIT || line.last() != Some(&b'\n') {
			return Err("offline peer request exceeds limit".into());
		}
		let request: Value = serde_json::from_slice(&line)?;
		let reply = peer
			.request(request)
			.unwrap_or_else(|e| json!({"error":e.to_string()}));
		serde_json::to_writer(&mut writer, &reply)?;
		writeln!(writer)?;
		writer.flush()?;
	}
	Ok(())
}

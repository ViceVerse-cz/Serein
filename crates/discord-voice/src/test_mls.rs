//! Synthetic local MLS delivery service; never connects to Discord.
use crate::crypto::Dave;
use openmls::prelude::tls_codec::Serialize as _;
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;

pub struct Delivery {
	signer: SignatureKeyPair,
	pub external: Vec<u8>,
}
impl Delivery {
	pub fn new() -> Self {
		let signer = SignatureKeyPair::new(SignatureScheme::ECDSA_SECP256R1_SHA256).unwrap();
		let external = ExternalSender::new(
			signer.public().to_vec().into(),
			BasicCredential::new(b"local test voice server".to_vec()).into(),
		)
		.tls_serialize_detached()
		.unwrap();
		Self { signer, external }
	}
	pub fn add_proposal(&self, creator: &Dave, key_package: &[u8]) -> Vec<u8> {
		assert_eq!(key_package[0], 26);
		let package = KeyPackageIn::tls_deserialize_exact_bytes(&key_package[1..]).unwrap();
		let provider = OpenMlsRustCrypto::default();
		let package = package
			.validate(provider.crypto(), ProtocolVersion::Mls10)
			.unwrap();
		let group = creator.session.group().unwrap();
		let proposal = ExternalProposal::new_add::<OpenMlsRustCrypto>(
			package,
			group.group_id().clone(),
			group.epoch(),
			&self.signer,
			SenderExtensionIndex::new(0),
		)
		.unwrap()
		.tls_serialize_detached()
		.unwrap();
		let mut payload = vec![0];
		payload.extend(VLBytes::new(proposal).tls_serialize_detached().unwrap());
		payload
	}
	pub fn remove_proposal(&self, creator: &Dave, user: u64) -> Vec<u8> {
		let group = creator.session.group().unwrap();
		let member = group
			.members()
			.find(|member| member.credential.serialized_content() == user.to_be_bytes())
			.unwrap();
		let proposal = ExternalProposal::new_remove::<OpenMlsRustCrypto>(
			member.index,
			group.group_id().clone(),
			group.epoch(),
			&self.signer,
			SenderExtensionIndex::new(0),
		)
		.unwrap()
		.tls_serialize_detached()
		.unwrap();
		let mut payload = vec![0];
		payload.extend(VLBytes::new(proposal).tls_serialize_detached().unwrap());
		payload
	}
	pub fn add(&self, creator: &mut Dave, key_package: &[u8]) -> (Vec<u8>, Vec<u8>) {
		let payload = self.add_proposal(creator, key_package);
		let result = creator.proposals(&payload).unwrap().unwrap();
		Self::split(&result)
	}
	pub fn split(result: &[u8]) -> (Vec<u8>, Vec<u8>) {
		assert_eq!(result[0], 28);
		let (_, welcome) = MlsMessageIn::tls_deserialize_bytes(&result[1..]).unwrap();
		let commit_len = result.len() - 1 - welcome.len();
		(result[1..1 + commit_len].to_vec(), welcome.to_vec())
	}
}
#[test]
fn dave_two_parties_encrypt_decrypt_reject_tampering_and_transition_gate() {
	let server = Delivery::new();
	let mut alice = Dave::new(1, Some(2), 3).unwrap();
	let mut bob = Dave::new(2, Some(1), 3).unwrap();
	alice.session.set_external_sender(&server.external).unwrap();
	bob.session.set_external_sender(&server.external).unwrap();
	let package = bob.key_package().unwrap();
	let (commit, welcome) = server.add(&mut alice, &package);
	let mut committed = vec![0, 5];
	committed.extend(commit);
	let mut welcomed = vec![0, 5];
	welcomed.extend(welcome);
	assert_eq!(alice.group_changed(29, &committed).unwrap(), 5);
	assert_eq!(bob.group_changed(30, &welcomed).unwrap(), 5);
	assert!(!alice.ready);
	assert!(!bob.ready);
	alice.execute(5).unwrap();
	bob.execute(5).unwrap();
	let encrypted = alice
		.session
		.encrypt_opus(b"synthetic opus bytes")
		.unwrap()
		.into_owned();
	assert_ne!(encrypted, b"synthetic opus bytes");
	let mut corrupt = encrypted.clone();
	corrupt[0] ^= 1;
	assert!(
		bob.session
			.decrypt(1, davey::MediaType::AUDIO, &corrupt)
			.is_err()
	);
	assert_eq!(
		bob.session
			.decrypt(1, davey::MediaType::AUDIO, &encrypted)
			.unwrap(),
		b"synthetic opus bytes"
	);
	assert!(
		bob.session
			.decrypt(1, davey::MediaType::AUDIO, &encrypted)
			.is_err()
	);
	assert_eq!(
		alice.session.voice_privacy_code(),
		bob.session.voice_privacy_code()
	);
	bob.reset().unwrap();
	assert!(!bob.ready);
	assert!(bob.session.encrypt_opus(b"private voice").is_err());
}

#[test]
fn welcome_cannot_add_an_unexpected_dm_peer() {
	let server = Delivery::new();
	let mut alice = Dave::new(1, Some(2), 3).unwrap();
	let mut bob = Dave::new(2, Some(99), 3).unwrap();
	alice.session.set_external_sender(&server.external).unwrap();
	bob.session.set_external_sender(&server.external).unwrap();
	let package = bob.key_package().unwrap();
	let (_, welcome) = server.add(&mut alice, &package);
	let mut welcomed = vec![0, 0];
	welcomed.extend(welcome);
	assert!(bob.group_changed(30, &welcomed).is_err());
	assert!(!bob.ready);
	assert!(bob.execute(0).is_err());
}

#[test]
fn guild_three_party_join_remove_and_empty_room_fail_closed() {
	let server = Delivery::new();
	let mut alice = Dave::new(1, None, 3).unwrap();
	let mut bob = Dave::new(2, None, 3).unwrap();
	let mut charlie = Dave::new(4, None, 3).unwrap();
	for member in [&mut alice, &mut bob, &mut charlie] {
		member
			.session
			.set_external_sender(&server.external)
			.unwrap();
		member.wait_for_peer().unwrap();
		assert!(member.waiting);
		assert!(!member.ready);
		assert!(member.session.encrypt_opus(b"never plaintext").is_err());
		member.connect(&[1, 2, 4]).unwrap();
		assert!(!member.waiting);
		assert!(member.wait_for_peer().is_err());
	}
	let (commit, welcome) = server.add(&mut alice, &bob.key_package().unwrap());
	let transition = |bytes: &[u8], id: u16| [id.to_be_bytes().as_slice(), bytes].concat();
	alice.group_changed(29, &transition(&commit, 0)).unwrap();
	bob.group_changed(30, &transition(&welcome, 0)).unwrap();
	let package = charlie.key_package().unwrap();
	let proposal = server.add_proposal(&alice, &package);
	let result = alice.proposals(&proposal).unwrap().unwrap();
	bob.proposals(&proposal).unwrap();
	let (commit, welcome) = Delivery::split(&result);
	alice.group_changed(29, &transition(&commit, 7)).unwrap();
	bob.group_changed(29, &transition(&commit, 7)).unwrap();
	charlie.group_changed(30, &transition(&welcome, 7)).unwrap();
	assert!(alice.ready && bob.ready && !charlie.ready);
	for member in [&mut alice, &mut bob, &mut charlie] {
		member.execute(7).unwrap();
		assert_eq!(member.session.get_user_ids().unwrap().len(), 3);
	}
	assert_eq!(
		alice.session.voice_privacy_code(),
		bob.session.voice_privacy_code()
	);
	assert_eq!(
		alice.session.voice_privacy_code(),
		charlie.session.voice_privacy_code()
	);
	let audio = alice
		.session
		.encrypt_opus(b"three party audio")
		.unwrap()
		.into_owned();
	assert_eq!(
		bob.session
			.decrypt(1, davey::MediaType::AUDIO, &audio)
			.unwrap(),
		b"three party audio"
	);
	assert_eq!(
		charlie
			.session
			.decrypt(1, davey::MediaType::AUDIO, &audio)
			.unwrap(),
		b"three party audio"
	);
	let proposal = server.remove_proposal(&alice, 4);
	alice.disconnect(4).unwrap();
	bob.disconnect(4).unwrap();
	assert!(!alice.ready);
	let result = alice.proposals(&proposal).unwrap().unwrap();
	bob.proposals(&proposal).unwrap();
	let (commit, welcome) = Delivery::split(&result);
	assert!(welcome.is_empty());
	alice.group_changed(29, &transition(&commit, 8)).unwrap();
	bob.group_changed(29, &transition(&commit, 8)).unwrap();
	alice.execute(8).unwrap();
	bob.execute(8).unwrap();
	let audio = alice
		.session
		.encrypt_opus(b"removed user cannot hear")
		.unwrap()
		.into_owned();
	assert!(
		charlie
			.session
			.decrypt(1, davey::MediaType::AUDIO, &audio)
			.is_err()
	);
	assert_eq!(
		bob.session
			.decrypt(1, davey::MediaType::AUDIO, &audio)
			.unwrap(),
		b"removed user cannot hear"
	);
	alice.disconnect(2).unwrap();
	for _ in 0..5 {
		alice.reinitialize().unwrap();
	}
	assert_eq!(alice.resets, 0);
	alice.wait_for_peer().unwrap();
	assert!(alice.waiting);
	assert!(!alice.ready);
	assert!(alice.connect(&(10..74).collect::<Vec<_>>()).is_err());
	assert!(alice.connect(&[0]).is_err());
	assert!(!alice.contains(10));
	let mut dm = Dave::new(1, Some(2), 3).unwrap();
	assert!(dm.connect(&[4]).is_err());
	// The DM peer hanging up leaves this side alone in the call, like Discord.
	assert!(dm.disconnect(2).unwrap());
	assert!(!dm.contains(2));
	assert!(!dm.ready);
	assert!(dm.disconnect(1).is_err());
	assert!(dm.connect(&[2]).unwrap());
	assert!(dm.contains(2));
}

#[test]
fn guild_welcome_requires_announced_members_and_rejects_duplicate_credentials() {
	let server = Delivery::new();
	let mut alice = Dave::new(1, None, 3).unwrap();
	let mut bob = Dave::new(2, None, 3).unwrap();
	for member in [&mut alice, &mut bob] {
		member
			.session
			.set_external_sender(&server.external)
			.unwrap();
	}
	alice.connect(&[2]).unwrap();
	let (commit, welcome) = server.add(&mut alice, &bob.key_package().unwrap());
	alice
		.group_changed(29, &[&[0, 0], commit.as_slice()].concat())
		.unwrap();
	// The welcome is cryptographically valid but Alice was never announced to Bob.
	assert!(
		bob.group_changed(30, &[&[0, 0], welcome.as_slice()].concat())
			.is_err()
	);
	assert!(!bob.ready);
	let mut duplicate = Dave::new(2, None, 3).unwrap();
	let (commit, _) = server.add(&mut alice, &duplicate.key_package().unwrap());
	assert!(
		alice
			.group_changed(29, &[&[0, 0], commit.as_slice()].concat())
			.is_err()
	);
	assert!(!alice.ready);
}

#[test]
fn protocol_transitions_execute_independently_of_mls_and_do_not_clobber_pending() {
	let server = Delivery::new();
	let mut alice = Dave::new(1, None, 3).unwrap();
	alice.session.set_external_sender(&server.external).unwrap();
	alice.connect(&[1, 2]).unwrap();

	// Case 1: Opcode 21 / 22 protocol transition arrives before MLS welcome / commit.
	// In multi-user channels, Discord negotiates protocol version 1 and executes it before MLS.
	assert!(!alice.session.is_ready());
	alice.prepare_protocol(1);
	assert_eq!(alice.pending_protocol, Some(1));
	assert_eq!(alice.pending, None);
	assert!(!alice.ready);

	// Spurious execution ID fails.
	assert_eq!(
		alice.execute(99),
		Err("Unexpected DAVE encryption transition")
	);

	// Protocol transition executes successfully without requiring MLS session readiness.
	alice.execute(1).unwrap();
	assert_eq!(alice.pending_protocol, None);
	assert_eq!(alice.pending, None);
	assert!(!alice.ready);

	// Case 2: Protocol transition and MLS transition interleaved.
	let mut bob = Dave::new(2, None, 3).unwrap();
	bob.session.set_external_sender(&server.external).unwrap();
	bob.connect(&[1, 2]).unwrap();
	let (commit, welcome) = server.add(&mut alice, &bob.key_package().unwrap());
	let transition = |bytes: &[u8], id: u16| [id.to_be_bytes().as_slice(), bytes].concat();

	// Bob receives MLS welcome with transition id 5.
	bob.group_changed(30, &transition(&welcome, 5)).unwrap();
	assert_eq!(bob.pending, Some(5));
	assert!(bob.session.is_ready());
	assert!(!bob.ready);

	// Opcode 21 arrives for Bob with protocol transition id 2.
	// It must NOT clobber Bob's pending MLS transition.
	bob.prepare_protocol(2);
	assert_eq!(bob.pending_protocol, Some(2));
	assert_eq!(bob.pending, Some(5));

	// Executing protocol transition clears pending_protocol without clearing pending MLS.
	bob.execute(2).unwrap();
	assert_eq!(bob.pending_protocol, None);
	assert_eq!(bob.pending, Some(5));
	assert!(!bob.ready);

	// Executing MLS transition clears pending and sets ready.
	bob.execute(5).unwrap();
	assert_eq!(bob.pending, None);
	assert!(bob.ready);

	// Alice processes and executes the commit to establish the group with Bob.
	alice.group_changed(29, &transition(&commit, 5)).unwrap();
	alice.execute(5).unwrap();

	// Case 3: Protocol transition prepared first, then MLS transition prepared.
	let mut charlie = Dave::new(4, None, 3).unwrap();
	charlie
		.session
		.set_external_sender(&server.external)
		.unwrap();
	charlie.connect(&[1, 2, 4]).unwrap();
	charlie.prepare_protocol(10);
	assert_eq!(charlie.pending_protocol, Some(10));
	assert_eq!(charlie.pending, None);

	alice.connect(&[1, 2, 4]).unwrap();
	let proposal = server.add_proposal(&alice, &charlie.key_package().unwrap());
	let result = alice.proposals(&proposal).unwrap().unwrap();
	let (_, welcome) = Delivery::split(&result);
	charlie
		.group_changed(30, &transition(&welcome, 11))
		.unwrap();
	assert_eq!(charlie.pending_protocol, Some(10));
	assert_eq!(charlie.pending, Some(11));
	assert!(!charlie.ready);

	// Executing MLS transition first validates the group, but ready waits for protocol transition.
	charlie.execute(11).unwrap();
	assert_eq!(charlie.pending, None);
	assert_eq!(charlie.pending_protocol, Some(10));
	assert!(!charlie.ready);

	// Executing protocol transition completes readiness.
	charlie.execute(10).unwrap();
	assert_eq!(charlie.pending_protocol, None);
	assert!(charlie.ready);
}

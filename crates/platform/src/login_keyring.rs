//! One bounded native credential job plus one queued request. A stalled OS provider
//! cannot hold the plaintext worker or its cleanup hostage. Late native results are
//! discarded; fallback authority markers keep late writes from becoming authoritative.
use crate::CredentialError;
use client_core::auth::SessionSecret;
use std::sync::{
	Arc, OnceLock,
	atomic::{AtomicBool, Ordering},
	mpsc,
};
use std::time::Duration;

enum Operation {
	Load(String),
	Save(String, SessionSecret),
	Forget(String),
}
type Result = std::result::Result<Option<SessionSecret>, CredentialError>;
type Job = (Operation, mpsc::SyncSender<Result>, Arc<AtomicBool>);
static WORKER: OnceLock<mpsc::SyncSender<Job>> = OnceLock::new();
fn start_worker(execute: impl Fn(Operation) -> Result + Send + 'static) -> mpsc::SyncSender<Job> {
	let (send, receive) = mpsc::sync_channel::<Job>(1);
	std::thread::spawn(move || {
		while let Ok((operation, reply, cancelled)) = receive.recv() {
			if cancelled.load(Ordering::Acquire) {
				continue;
			}
			let _ = reply.send(execute(operation));
		}
	});
	send
}
fn worker() -> &'static mpsc::SyncSender<Job> {
	WORKER.get_or_init(|| {
		start_worker(|operation| match operation {
			Operation::Load(name) => crate::load_entry(&name),
			Operation::Save(name, secret) => crate::save_entry(&name, &secret).map(|()| None),
			Operation::Forget(name) => crate::forget_entry(&name).map(|()| None),
		})
	})
}
fn request_with(send: &mpsc::SyncSender<Job>, operation: Operation, timeout: Duration) -> Result {
	let (reply, receive) = mpsc::sync_channel(1);
	let cancelled = Arc::new(AtomicBool::new(false));
	send.try_send((operation, reply, cancelled.clone()))
		.map_err(|_| CredentialError::Unavailable)?;
	let result = receive.recv_timeout(timeout);
	if result.is_err() {
		cancelled.store(true, Ordering::Release);
	}
	result.unwrap_or(Err(CredentialError::TimedOut))
}
fn request(operation: Operation) -> Result {
	request_with(worker(), operation, Duration::from_secs(5))
}
pub(crate) fn load(name: &str) -> Result {
	request(Operation::Load(name.to_owned()))
}
pub(crate) fn save(name: &str, secret: &SessionSecret) -> std::result::Result<(), CredentialError> {
	let secret = SessionSecret::from_owner_input(secret.expose().to_owned())
		.map_err(|_| CredentialError::Invalid)?;
	request(Operation::Save(name.to_owned(), secret)).map(|_| ())
}
pub(crate) fn forget(name: &str) -> std::result::Result<(), CredentialError> {
	request(Operation::Forget(name.to_owned())).map(|_| ())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn stalled_native_work_is_bounded_and_expired_queued_writes_are_skipped() {
		let (release, gate) = mpsc::sync_channel(1);
		let (started, starting) = mpsc::sync_channel(1);
		let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
		let count = calls.clone();
		let send = start_worker(move |_| {
			if count.fetch_add(1, Ordering::SeqCst) == 0 {
				started.send(()).unwrap();
				gate.recv().unwrap();
			}
			Ok(None)
		});
		let (reply, _) = mpsc::sync_channel(1);
		send.send((
			Operation::Load("first".into()),
			reply,
			Arc::new(AtomicBool::new(false)),
		))
		.unwrap();
		starting.recv_timeout(Duration::from_secs(1)).unwrap();
		assert_eq!(
			request_with(
				&send,
				Operation::Forget("expired".into()),
				Duration::from_millis(1)
			)
			.err(),
			Some(CredentialError::TimedOut)
		);
		assert_eq!(
			request_with(
				&send,
				Operation::Load("full".into()),
				Duration::from_millis(1)
			)
			.err(),
			Some(CredentialError::Unavailable)
		);
		release.send(()).unwrap();
		let (reply, result) = mpsc::sync_channel(1);
		// Blocking admission is test-only: waits for the expired request to be skipped.
		send.send((
			Operation::Load("last".into()),
			reply,
			Arc::new(AtomicBool::new(false)),
		))
		.unwrap();
		assert!(result.recv_timeout(Duration::from_secs(1)).unwrap().is_ok());
		assert_eq!(calls.load(Ordering::SeqCst), 2);
	}
}

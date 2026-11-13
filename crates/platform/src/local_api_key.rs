//! The local API's access key lives only in the OS credential store, never in preferences.
//!
//! One worker runs the native calls in order and every call is bounded: a provider that
//! never answers (a keyring waiting for its owner, a stopped Secret Service) costs the
//! caller five seconds, not the listener or the app's exit. A call whose caller gave up
//! while it was still queued is skipped, so a key that has since been reset is not written
//! late.
use crate::{ACCOUNT, CredentialError, entry, forget_entry};
use std::sync::{
	Arc, OnceLock,
	atomic::{AtomicBool, Ordering},
	mpsc,
};
use std::time::Duration;
use tokio::sync::oneshot;
use zeroize::Zeroizing;

/// 32 random bytes as lowercase hex.
pub const KEY_CHARS: usize = 64;
const TIMEOUT: Duration = Duration::from_secs(5);

type Reply = Result<Option<Zeroizing<String>>, CredentialError>;
type Job = (
	Box<dyn FnOnce() -> Reply + Send>,
	oneshot::Sender<Reply>,
	Arc<AtomicBool>,
);
static WORKER: OnceLock<mpsc::SyncSender<Job>> = OnceLock::new();

/// One call in flight plus one queued; a third is refused instead of piling up.
fn start_worker() -> mpsc::SyncSender<Job> {
	let (send, receive) = mpsc::sync_channel::<Job>(1);
	std::thread::spawn(move || {
		while let Ok((work, reply, expired)) = receive.recv() {
			if !expired.load(Ordering::Acquire) {
				let _ = reply.send(work());
			}
		}
	});
	send
}

/// Marks a request expired once its caller stops waiting, by deadline or by being dropped.
struct Expire(Arc<AtomicBool>);
impl Drop for Expire {
	fn drop(&mut self) {
		self.0.store(true, Ordering::Release);
	}
}

async fn request_with(
	send: &mpsc::SyncSender<Job>,
	work: impl FnOnce() -> Reply + Send + 'static,
	timeout: Duration,
) -> Reply {
	let (reply, answer) = oneshot::channel();
	let expired = Arc::new(AtomicBool::new(false));
	send.try_send((Box::new(work), reply, expired.clone()))
		.map_err(|_| CredentialError::Unavailable)?;
	let _expire = Expire(expired);
	match tokio::time::timeout(timeout, answer).await {
		Ok(Ok(result)) => result,
		Ok(Err(_)) => Err(CredentialError::Unavailable),
		Err(_) => Err(CredentialError::TimedOut),
	}
}

async fn request(work: impl FnOnce() -> Reply + Send + 'static) -> Reply {
	request_with(WORKER.get_or_init(start_worker), work, TIMEOUT).await
}

pub fn valid(key: &str) -> bool {
	key.len() == KEY_CHARS
		&& key
			.bytes()
			.all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}
fn name() -> String {
	format!("{ACCOUNT}.local-api")
}
/// The saved key. An entry that is not a key reads as absent, so the caller replaces it.
pub async fn load() -> Result<Option<Zeroizing<String>>, CredentialError> {
	request(|| match entry(&name())?.get_password() {
		Ok(value) => Ok(Some(Zeroizing::new(value)).filter(|key| valid(key))),
		Err(keyring_core::Error::NoEntry) => Ok(None),
		Err(_) => Err(CredentialError::Unavailable),
	})
	.await
}
pub async fn save(key: &str) -> Result<(), CredentialError> {
	if !valid(key) {
		return Err(CredentialError::Invalid);
	}
	let key = Zeroizing::new(key.to_owned());
	request(move || {
		entry(&name())?
			.set_password(&key)
			.map(|()| None)
			.map_err(|_| CredentialError::Unavailable)
	})
	.await
	.map(|_| ())
}
pub async fn forget() -> Result<(), CredentialError> {
	request(|| forget_entry(&name()).map(|()| None))
		.await
		.map(|_| ())
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::AtomicUsize;

	#[tokio::test]
	async fn a_stalled_store_is_bounded_and_an_expired_queued_write_never_runs() {
		let send = start_worker();
		let (release, gate) = mpsc::sync_channel::<()>(1);
		let (started, starting) = mpsc::sync_channel(1);
		let (reply, _) = oneshot::channel();
		let stalled: Job = (
			Box::new(move || {
				started.send(()).unwrap();
				gate.recv().unwrap();
				Ok(None)
			}),
			reply,
			Arc::new(AtomicBool::new(false)),
		);
		send.send(stalled).unwrap();
		starting.recv_timeout(Duration::from_secs(5)).unwrap();

		// Queued behind the stalled call, this write expires before the store is free.
		let writes = Arc::new(AtomicUsize::new(0));
		let count = writes.clone();
		let late = move || {
			count.fetch_add(1, Ordering::SeqCst);
			Ok(None)
		};
		assert_eq!(
			request_with(&send, late, Duration::from_millis(1))
				.await
				.err(),
			Some(CredentialError::TimedOut)
		);
		// It still holds the one queue slot, so the next call is refused at once.
		assert_eq!(
			request_with(&send, || Ok(None), Duration::from_millis(1))
				.await
				.err(),
			Some(CredentialError::Unavailable)
		);

		release.send(()).unwrap();
		let (reply, answer) = oneshot::channel();
		// Blocking admission is test-only: it waits for the expired write to be skipped.
		send.send((
			Box::new(|| Ok(None)),
			reply,
			Arc::new(AtomicBool::new(false)),
		))
		.unwrap();
		assert_eq!(
			tokio::time::timeout(Duration::from_secs(5), answer)
				.await
				.unwrap()
				.unwrap(),
			Ok(None)
		);
		assert_eq!(writes.load(Ordering::SeqCst), 0);
	}
}

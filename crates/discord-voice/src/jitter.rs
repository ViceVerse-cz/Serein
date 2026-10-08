//! Fixed 40ms startup with a bounded reorder window for 2.5?120ms Opus packets.
const SLOTS: usize = 8;
#[derive(Default)]
pub(crate) struct Jitter {
	packets: Vec<(u16, Vec<u8>)>,
	next: Option<u16>,
	wait: u8,
	missing: u8,
}
impl Jitter {
	pub fn clear(&mut self) {
		*self = Self::default();
	}
	pub fn push(&mut self, sequence: u16, opus: Vec<u8>) {
		if opus.len() > 1275 {
			return;
		}
		if self.next.is_none() {
			self.next = Some(sequence);
			self.wait = 2;
		}
		let distance = sequence.wrapping_sub(self.next.unwrap());
		if distance >= 32768 {
			return;
		}
		if distance >= SLOTS as u16 {
			self.packets.clear();
			self.next = Some(sequence);
			self.wait = 2;
			self.missing = 0;
		}
		if self.packets.len() < SLOTS && !self.packets.iter().any(|(id, _)| *id == sequence) {
			self.packets.push((sequence, opus));
		}
	}
	/// An empty packet requests Opus packet-loss concealment, at most three consecutive packets.
	pub fn pop(&mut self) -> Option<Vec<u8>> {
		if self.wait > 0 {
			// Short packets can fill the fixed window before 40ms. Start before
			// the next arrival would repeatedly reset a full window.
			if self.packets.len() < SLOTS {
				self.wait -= 1;
				return None;
			}
			self.wait = 0;
		}
		let next = self.next?;
		self.next = Some(next.wrapping_add(1));
		if let Some(index) = self.packets.iter().position(|(id, _)| *id == next) {
			self.missing = 0;
			return Some(self.packets.swap_remove(index).1);
		}
		self.missing += 1;
		if self.missing > 3 {
			// Stop concealing the gap, but retain speech that already arrived after it.
			if let Some((index, (sequence, _))) = self
				.packets
				.iter()
				.enumerate()
				.min_by_key(|(_, (sequence, _))| sequence.wrapping_sub(next))
			{
				self.next = Some(sequence.wrapping_add(1));
				self.missing = 0;
				return Some(self.packets.swap_remove(index).1);
			}
			self.clear();
			return None;
		}
		Some(Vec::new())
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn burst_loss_preserves_buffered_recovery_and_resets_on_far_jump() {
		for first in [0u16, u16::MAX - 2] {
			let mut jitter = Jitter::default();
			jitter.push(first, vec![0]);
			for offset in [7, 5, 6] {
				jitter.push(first.wrapping_add(offset), vec![offset as u8]);
			}
			assert!(jitter.pop().is_none());
			assert!(jitter.pop().is_none());
			assert_eq!(jitter.pop(), Some(vec![0]));
			for _ in 0..3 {
				assert_eq!(jitter.pop(), Some(vec![]));
			}
			for offset in [5, 6, 7] {
				assert_eq!(jitter.pop(), Some(vec![offset]));
			}
			for _ in 0..2 {
				assert_eq!(jitter.pop(), Some(vec![]));
			}
			jitter.push(first.wrapping_add(100), vec![100]);
			assert_eq!(jitter.missing, 0);
			assert!(jitter.pop().is_none());
			assert!(jitter.pop().is_none());
			assert_eq!(jitter.pop(), Some(vec![100]));
		}
	}
	#[test]
	fn bounded_reordering_loss_duplicates_and_wrap() {
		let mut jitter = Jitter::default();
		jitter.push(u16::MAX, vec![1]);
		jitter.push(1, vec![3]);
		jitter.push(0, vec![2]);
		jitter.push(0, vec![9]);
		assert!(jitter.pop().is_none());
		assert!(jitter.pop().is_none());
		assert_eq!(jitter.pop(), Some(vec![1]));
		assert_eq!(jitter.pop(), Some(vec![2]));
		assert_eq!(jitter.pop(), Some(vec![3]));
		for _ in 0..3 {
			assert_eq!(jitter.pop(), Some(vec![]));
		}
		assert!(jitter.pop().is_none());
		assert!(jitter.pop().is_none());
		for sequence in 0..1000 {
			jitter.push(sequence, vec![1; 1275]);
			assert!(jitter.packets.len() <= SLOTS);
		}
		jitter.push(1000, vec![0; 1276]);
		assert!(jitter.packets.len() <= SLOTS);
	}
}

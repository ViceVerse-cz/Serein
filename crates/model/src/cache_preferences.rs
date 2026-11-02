//! Device-local disk budgets for downloaded images, chosen from bounded presets.
use std::time::Duration;

const MIB: u64 = 1024 * 1024;
/// Selectable budgets for avatars, previews, stickers and other conversation artwork.
pub const MEDIA_CACHE_MIB: [u32; 5] = [256, 512, 1024, 2048, 4096];
/// Selectable budgets for small artwork reused across conversations.
pub const ARTWORK_CACHE_MIB: [u32; 4] = [64, 128, 256, 512];
/// Selectable retention for cached images that are not shown again.
pub const CACHE_RETENTION_DAYS: [u16; 5] = [7, 30, 90, 180, 365];

/// The two image caches: conversation media, and small artwork reused everywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachePartition {
	Media = 0,
	Artwork = 1,
}

/// Measured disk use of one cache partition.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheUsage {
	pub bytes: u64,
	pub files: usize,
}

/// Bytes and file count one disk cache may hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskLimit {
	pub bytes: u64,
	pub files: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CachePreferences {
	/// Avatars, banners, previews, stickers and other conversation artwork.
	pub media_mib: u32,
	/// Emoji, badges, server tags, role icons, avatar decorations and nameplates. Kept apart
	/// so large media cannot evict artwork that appears in almost every conversation.
	pub artwork_mib: u32,
	/// Cached images unused for this many days are removed.
	pub retention_days: u16,
}
impl Default for CachePreferences {
	fn default() -> Self {
		Self {
			media_mib: 1024,
			artwork_mib: 128,
			retention_days: 90,
		}
	}
}
impl CachePreferences {
	pub fn is_valid(&self) -> bool {
		MEDIA_CACHE_MIB.contains(&self.media_mib)
			&& ARTWORK_CACHE_MIB.contains(&self.artwork_mib)
			&& CACHE_RETENTION_DAYS.contains(&self.retention_days)
	}
	/// Average media files are about 256 KiB, so 1 GiB keeps the historical 4,096 files.
	pub fn media_limit(&self) -> DiskLimit {
		let bytes = u64::from(self.media_mib) * MIB;
		DiskLimit {
			bytes,
			files: (bytes / (256 * 1024)) as usize,
		}
	}
	/// Shared artwork is small: 128 MiB holds up to 4,096 files.
	pub fn artwork_limit(&self) -> DiskLimit {
		let bytes = u64::from(self.artwork_mib) * MIB;
		DiskLimit {
			bytes,
			files: (bytes / (32 * 1024)) as usize,
		}
	}
	pub fn retention(&self) -> Duration {
		Duration::from_secs(u64::from(self.retention_days) * 24 * 60 * 60)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn presets_bound_bytes_files_and_age() {
		let defaults = CachePreferences::default();
		assert!(defaults.is_valid());
		assert_eq!(
			defaults.media_limit(),
			DiskLimit {
				bytes: 1 << 30,
				files: 4096
			}
		);
		assert_eq!(defaults.artwork_limit().files, 4096);
		assert_eq!(defaults.retention(), Duration::from_secs(90 * 86_400));
		let largest = CachePreferences {
			media_mib: 4096,
			artwork_mib: 512,
			retention_days: 365,
		};
		assert!(largest.is_valid());
		assert_eq!(largest.media_limit().files, 16_384);
		assert_eq!(largest.artwork_limit().files, 16_384);
		for invalid in [
			CachePreferences {
				media_mib: 0,
				..defaults
			},
			CachePreferences {
				artwork_mib: 100_000,
				..defaults
			},
			CachePreferences {
				retention_days: 0,
				..defaults
			},
		] {
			assert!(!invalid.is_valid(), "{invalid:?}");
		}
	}
}

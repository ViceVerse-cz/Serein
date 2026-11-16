//! Shared texture pages for emoji rasters.
//!
//! Small textures each cost a padded GPU allocation, so rasters share up to [`PAGES`] pages of
//! [`PAGE`]² pixels. Each page is a quad-tree buddy allocator over power-of-two square blocks.
//! A raster sits one pixel in from its block's corner, and its whole block is uploaded, so a
//! transparent guard surrounds it and linear filtering never samples a neighbour or old pixels.
use egui::{ColorImage, Context, TextureHandle};
use std::collections::BTreeSet;

/// Page edge in pixels: 4 MiB of RGBA each.
pub(super) const PAGE: u32 = 1024;
/// Most pages alive at once (16 MiB).
pub(super) const PAGES: usize = 4;
const MIN_BLOCK: u32 = 16;
/// Block orders: 16, 32, ... 1024 pixels.
const ORDERS: usize = 7;

fn block(order: usize) -> u32 {
	MIN_BLOCK << order
}

/// Order of the smallest block holding an `edge` raster and its one-pixel guard.
fn order(edge: u32) -> usize {
	let size = (edge + 2).max(MIN_BLOCK).next_power_of_two();
	(size.trailing_zeros() - MIN_BLOCK.trailing_zeros()) as usize
}

/// RGBA bytes uploaded and reserved for an `edge` raster.
pub(super) fn block_bytes(edge: u32) -> usize {
	let size = block(order(edge)) as usize;
	size * size * 4
}

/// Free blocks by order, as top-left corners; ordered sets keep allocation deterministic.
struct Buddy {
	free: [BTreeSet<(u32, u32)>; ORDERS],
}

impl Buddy {
	fn new() -> Self {
		let mut free: [BTreeSet<(u32, u32)>; ORDERS] = Default::default();
		free[ORDERS - 1].insert((0, 0));
		Self { free }
	}
	/// A free block of `order`, splitting a larger one into quadrants when needed.
	fn alloc(&mut self, order: usize) -> Option<(u32, u32)> {
		let from = (order..ORDERS).find(|order| !self.free[*order].is_empty())?;
		let (x, y) = self.free[from].pop_first()?;
		for split in (order..from).rev() {
			let size = block(split);
			self.free[split].extend([(x + size, y), (x, y + size), (x + size, y + size)]);
		}
		Some((x, y))
	}
	/// Returns a block, merging it with its three free buddies as far up as possible.
	fn free(&mut self, (mut x, mut y): (u32, u32), mut order: usize) {
		while order + 1 < ORDERS {
			let size = block(order);
			let parent = block(order + 1);
			let (px, py) = (x & !(parent - 1), y & !(parent - 1));
			let quadrants = [
				(px, py),
				(px + size, py),
				(px, py + size),
				(px + size, py + size),
			];
			let buddies = quadrants.into_iter().filter(|corner| *corner != (x, y));
			if !buddies
				.clone()
				.all(|corner| self.free[order].contains(&corner))
			{
				break;
			}
			for corner in buddies {
				self.free[order].remove(&corner);
			}
			(x, y, order) = (px, py, order + 1);
		}
		self.free[order].insert((x, y));
	}
	fn empty(&self) -> bool {
		self.free[ORDERS - 1].contains(&(0, 0))
	}
}

/// Where one raster lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Slot {
	page: usize,
	x: u32,
	y: u32,
	order: usize,
	edge: u32,
}

impl Slot {
	/// Texture coordinates of exactly the raster's `edge` × `edge` pixels.
	pub(super) fn uv(&self) -> egui::Rect {
		let page = PAGE as f32;
		egui::Rect::from_min_size(
			egui::pos2((self.x + 1) as f32 / page, (self.y + 1) as f32 / page),
			egui::Vec2::splat(self.edge as f32 / page),
		)
	}
}

struct Page {
	texture: TextureHandle,
	buddy: Buddy,
}

#[derive(Default)]
pub(super) struct Pages {
	pages: [Option<Page>; PAGES],
}

impl Pages {
	/// A block for an `edge` raster in an existing page, else in a new page while fewer than
	/// [`PAGES`] exist.
	pub(super) fn alloc(&mut self, ctx: &Context, edge: u32) -> Option<Slot> {
		let order = order(edge);
		let place = |page: usize, (x, y): (u32, u32)| Slot {
			page,
			x,
			y,
			order,
			edge,
		};
		for (index, page) in self.pages.iter_mut().enumerate() {
			if let Some(page) = page
				&& let Some(corner) = page.buddy.alloc(order)
			{
				return Some(place(index, corner));
			}
		}
		let index = self.pages.iter().position(Option::is_none)?;
		let texture = ctx.load_texture(
			"emoji-page",
			ColorImage::filled([PAGE as usize; 2], egui::Color32::TRANSPARENT),
			egui::TextureOptions::LINEAR,
		);
		let mut page = Page {
			texture,
			buddy: Buddy::new(),
		};
		let corner = page.buddy.alloc(order)?;
		self.pages[index] = Some(page);
		Some(place(index, corner))
	}
	/// Writes `image` into its slot, with the rest of the block transparent.
	pub(super) fn upload(&mut self, slot: Slot, image: &ColorImage) {
		let Some(page) = &mut self.pages[slot.page] else {
			return;
		};
		let size = block(slot.order) as usize;
		let mut padded = ColorImage::filled([size, size], egui::Color32::TRANSPARENT);
		let [width, height] = image.resolution();
		for row in 0..height.min(size - 2) {
			let source = &image.pixels()[row * width..row * width + width.min(size - 2)];
			let start = (row + 1) * size + 1;
			padded.pixels_mut()[start..start + source.len()].copy_from_slice(source);
		}
		page.texture.set_partial(
			[slot.x as usize, slot.y as usize],
			padded,
			egui::TextureOptions::LINEAR,
		);
	}
	pub(super) fn free(&mut self, slot: Slot) {
		if let Some(page) = &mut self.pages[slot.page] {
			page.buddy.free((slot.x, slot.y), slot.order);
		}
	}
	/// Drops pages holding no rasters; true when any was dropped.
	pub(super) fn release_empty(&mut self) -> bool {
		let mut released = false;
		for page in &mut self.pages {
			if page.as_ref().is_some_and(|page| page.buddy.empty()) {
				*page = None;
				released = true;
			}
		}
		released
	}
	pub(super) fn texture(&self, slot: Slot) -> Option<egui::TextureId> {
		Some(self.pages[slot.page].as_ref()?.texture.id())
	}
	#[cfg(test)]
	pub(super) fn count(&self) -> usize {
		self.pages.iter().flatten().count()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Allocated blocks never overlap, and freeing everything merges back to one free page.
	#[test]
	fn buddy_blocks_split_merge_and_never_overlap() {
		let mut buddy = Buddy::new();
		let mut live: Vec<((u32, u32), usize)> = Vec::new();
		let mut seed = 0x9e37_79b9_u32;
		let mut next = || {
			seed ^= seed << 13;
			seed ^= seed >> 17;
			seed ^= seed << 5;
			seed
		};
		for _ in 0..4000 {
			if live.is_empty() || next() % 3 != 0 {
				let order = next() as usize % 5;
				if let Some(corner) = buddy.alloc(order) {
					live.push((corner, order));
				}
			} else {
				let (corner, order) = live.swap_remove(next() as usize % live.len());
				buddy.free(corner, order);
			}
			let area: u32 = live.iter().map(|(_, order)| block(*order).pow(2)).sum();
			assert!(area <= PAGE * PAGE);
		}
		for (index, ((x, y), order)) in live.iter().enumerate() {
			let size = block(*order);
			assert!(x % size == 0 && y % size == 0 && x + size <= PAGE && y + size <= PAGE);
			for ((ox, oy), other) in &live[index + 1..] {
				let other = block(*other);
				let apart =
					x + size <= *ox || ox + other <= *x || y + size <= *oy || oy + other <= *y;
				assert!(apart, "blocks overlap");
			}
		}
		for (corner, order) in live {
			buddy.free(corner, order);
		}
		assert!(buddy.empty());
		assert!(buddy.free[..ORDERS - 1].iter().all(BTreeSet::is_empty));
	}

	#[test]
	fn pages_are_bounded_and_released_when_empty() {
		let ctx = Context::default();
		let mut pages = Pages::default();
		// A 254 px raster and its guard fill a 256 px block: 16 per page.
		let slots: Vec<_> = std::iter::from_fn(|| pages.alloc(&ctx, 254)).collect();
		assert_eq!(slots.len(), PAGES * 16);
		assert_eq!(pages.count(), PAGES);
		assert!(pages.alloc(&ctx, 16).is_none());
		for slot in &slots[..16] {
			pages.free(*slot);
		}
		assert!(pages.release_empty());
		assert_eq!(pages.count(), PAGES - 1);
		for slot in &slots[16..] {
			pages.free(*slot);
		}
		pages.release_empty();
		assert_eq!(pages.count(), 0);
	}

	#[test]
	fn a_raster_maps_to_exactly_its_pixels_inside_its_block() {
		let ctx = Context::default();
		let mut pages = Pages::default();
		for edge in [16, 30, 31, 37, 64, 254, 256] {
			let slot = pages.alloc(&ctx, edge).unwrap();
			let size = block(slot.order);
			let pixels = slot.uv().size() * PAGE as f32;
			assert_eq!(pixels, egui::Vec2::splat(edge as f32));
			let min = slot.uv().min * PAGE as f32;
			assert_eq!(min, egui::pos2((slot.x + 1) as f32, (slot.y + 1) as f32));
			assert!(edge + 2 <= size, "a guard pixel surrounds the raster");
		}
	}
}

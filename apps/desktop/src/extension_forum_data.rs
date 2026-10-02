//! Loaded thread metadata only; no discovery, requests, or tags absent from core state.
use crate::extension_app::{name, push};
use client_core::State;
use extensions::{ForumDataSnapshot, ForumPostSnapshot, MAX_FORUM_DATA_BYTES, MAX_FORUM_POSTS};
use model::Freshness;

pub fn snapshot(state: &State) -> Option<ForumDataSnapshot> {
	if !crate::extension_app::available(state)
		|| !state.gateway_connected
		|| state.freshness != Freshness::Fresh
	{
		return None;
	}
	let selected = state.selected?;
	if !state.can_view(selected) || !state.can_read_history(selected) {
		return None;
	}
	let current = state.channel(selected)?;
	let guild = current.guild?;
	let parent = match current.kind {
		0 | 5 | 15 | 16 => current,
		10..=12 => state.channel(current.parent_id?)?,
		_ => return None,
	};
	if parent.guild != Some(guild)
		|| !matches!(parent.kind, 0 | 5 | 15 | 16)
		|| !state.can_view(parent.id)
		|| !state.can_read_history(parent.id)
	{
		return None;
	}
	let archive = state
		.archives
		.as_ref()
		.filter(|view| {
			view.parent == parent.id
				&& view.guild == guild
				&& !view.loading
				&& view.error.is_none()
				&& state.can_archive(parent.id, view.kind)
		})
		.and_then(|view| view.page.as_ref());
	let mut group = ForumDataSnapshot {
		channel_id: selected.0.to_string(),
		guild_id: guild.0.to_string(),
		parent_id: parent.id.0.to_string(),
		posts: Vec::new(),
		truncated: state.posts.parent != Some(parent.id)
			|| state.posts.loading
			|| state.posts.more
			|| state.posts.error.is_some()
			|| archive.is_some_and(|page| page.next.is_some()),
	};
	let mut budget = MAX_FORUM_DATA_BYTES - 512;
	for post in state.channels.iter().filter(|post| {
		post.guild == Some(guild)
			&& post.parent_id == Some(parent.id)
			&& matches!(post.kind, 10..=12)
			&& state.can_view(post.id)
			&& state.can_read_history(post.id)
	}) {
		let id = post.id.0.to_string();
		if group.posts.iter().any(|known| known.id == id) {
			continue;
		}
		let details = state.post_details(post.id);
		let archived =
			archive.is_some_and(|page| page.threads.iter().any(|thread| thread.id == post.id));
		let item = ForumPostSnapshot {
			id,
			name: name(&post.name),
			kind: post.kind,
			message_count: post.message_count,
			owner_id: details
				.and_then(|details| details.owner)
				.filter(|id| id.0 != 0)
				.map(|id| id.0.to_string()),
			archived: details
				.map(|details| details.archived)
				.or(archived.then_some(true)),
			locked: details.map(|details| details.locked),
			pinned: details.map(|details| details.pinned),
			followed: details.map(|details| details.followed),
		};
		if !push(&mut group.posts, item, &mut budget, MAX_FORUM_POSTS) {
			group.truncated = true;
			break;
		}
	}
	group.validate().ok()?;
	Some(group)
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::Id;
	#[test]
	fn forum_data_contains_only_loaded_readable_children_and_known_flags() {
		let mut state = test_support::demo_state();
		let parent = state.channel(state.selected.unwrap()).unwrap().clone();
		state
			.channels
			.retain(|post| post.parent_id != Some(parent.id) || !matches!(post.kind, 10..=12));
		state.permissions.clear_cache();
		for id in 1000..1012 {
			let mut post = parent.clone();
			post.id = Id(id);
			post.parent_id = Some(parent.id);
			post.kind = 11;
			post.name = "Synthetic post".into();
			state.channels.push(post);
		}
		let first = state
			.channels
			.iter()
			.find(|post| post.id == Id(1000))
			.unwrap()
			.clone();
		let mut not_resident = first.clone();
		not_resident.id = Id(9000);
		state.archives = Some(client_core::archives::View {
			parent: parent.id,
			guild: parent.guild.unwrap(),
			kind: model::archives::Kind::Public,
			before: None,
			request: 1,
			loading: false,
			error: None,
			page: Some(model::archives::Page {
				threads: vec![first, not_resident],
				next: None,
			}),
		});
		let group = snapshot(&state).unwrap();
		assert_eq!(group.posts.len(), MAX_FORUM_POSTS);
		assert!(group.truncated);
		assert_eq!(group.posts[0].archived, Some(true));
		assert!(
			group
				.posts
				.iter()
				.all(|post| post.followed.is_none() && post.locked.is_none())
		);
		assert!(group.posts.iter().all(|post| post.id != "9000"));
		assert!(serde_json::to_vec(&group).unwrap().len() <= MAX_FORUM_DATA_BYTES);
		state
			.channels
			.iter_mut()
			.find(|post| post.id == Id(1000))
			.unwrap()
			.guild = Some(Id(999));
		state.permissions.clear_cache();
		assert!(
			snapshot(&state)
				.unwrap()
				.posts
				.iter()
				.all(|post| post.id != "1000")
		);
	}
}

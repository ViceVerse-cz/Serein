use crate::{Id, User};
pub const MAX_MENTIONS: usize = 100;

pub fn mention_bytes(users: &Vec<User>) -> usize {
	users.capacity() * size_of::<User>() + users.iter().map(User::heap_bytes).sum::<usize>()
}
pub fn valid_mentions(users: &[User]) -> bool {
	users.len() <= MAX_MENTIONS
		&& users.iter().all(|u| {
			u.id.0 != 0
				&& u.name.len() <= 512
				&& u.discriminator <= 9999
				&& u.avatar.as_deref().is_none_or(crate::valid_avatar_hash)
		})
}
/// Return one exact user mention prefix, excluding roles and mass mentions.
pub fn user_mention_prefix(text: &str) -> Option<(Id, usize)> {
	let rest = text.strip_prefix("<@")?;
	let digits = rest.strip_prefix('!').unwrap_or(rest);
	let end = digits.find('>')?;
	if end > 20 {
		return None;
	}
	Some((
		digits[..end].parse().ok()?,
		text.len() - digits.len() + end + 1,
	))
}
/// Return one exact role mention prefix with a nonzero, bounded snowflake.
pub fn role_mention_prefix(text: &str) -> Option<(Id, usize)> {
	let rest = text.strip_prefix("<@&")?;
	let end = rest.find('>')?;
	Some((rest[..end].parse().ok()?, end + 4))
}
pub fn mentioned_role_ids(content: &str) -> Vec<Id> {
	let mut ids = Vec::new();
	for (start, _) in content.match_indices("<@&") {
		if let Some((id, _)) = role_mention_prefix(&content[start..])
			&& !ids.contains(&id)
		{
			if ids.len() == MAX_MENTIONS {
				break;
			}
			ids.push(id);
		}
	}
	ids
}
/// Return one exact mass-mention prefix without matching longer words.
pub fn mass_mention_prefix(text: &str) -> Option<usize> {
	["@everyone", "@here"].into_iter().find_map(|mention| {
		let rest = text.strip_prefix(mention)?;
		rest.chars()
			.next()
			.is_none_or(|c| !c.is_alphanumeric() && c != '_')
			.then_some(mention.len())
	})
}
pub fn has_mass_mention(content: &str) -> bool {
	content
		.match_indices('@')
		.any(|(start, _)| mass_mention_prefix(&content[start..]).is_some())
}
/// Return one exact channel reference prefix, using the same nonzero snowflake bounds.
pub fn channel_mention_prefix(text: &str) -> Option<(Id, usize)> {
	let rest = text.strip_prefix("<#")?;
	let end = rest.find('>')?;
	Some((rest[..end].parse().ok()?, end + 3))
}
/// Return one exact `<t:seconds[:style]>` timestamp reference: its instant, style and length.
///
/// Discord accepts a single style letter from `tTdDfFR` and falls back to `f` when omitted;
/// anything else stays literal text.
pub fn timestamp_prefix(text: &str) -> Option<(i64, u8, usize)> {
	let rest = text.strip_prefix("<t:")?;
	let end = rest.find('>')?;
	let body = &rest[..end];
	if body.len() > 24 {
		return None;
	}
	let (digits, style) = match body.split_once(':') {
		Some((digits, style)) => {
			let mut letters = style.chars();
			let style = letters.next()?;
			if letters.next().is_some() || !matches!(style, 't' | 'T' | 'd' | 'D' | 'f' | 'F' | 'R')
			{
				return None;
			}
			(digits, style as u8)
		}
		None => (body, b'f'),
	};
	if digits.is_empty()
		|| !digits
			.bytes()
			.enumerate()
			.all(|(i, b)| b.is_ascii_digit() || (i == 0 && b == b'-'))
	{
		return None;
	}
	Some((digits.parse().ok()?, style, end + 4))
}
pub fn mentioned_user_ids(content: &str) -> Vec<Id> {
	let mut ids = Vec::new();
	for (start, _) in content.match_indices("<@") {
		if let Some((id, _)) = user_mention_prefix(&content[start..])
			&& !ids.contains(&id)
		{
			if ids.len() == MAX_MENTIONS {
				break;
			}
			ids.push(id);
		}
	}
	ids
}
/// Shared wire/cache array bound, enforced before parsing an excess item.
pub fn deserialize_mentions<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de>>(
	deserializer: D,
) -> Result<Vec<T>, D::Error> {
	struct Visitor<T>(std::marker::PhantomData<T>);
	impl<'de, T: serde::Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
		type Value = Vec<T>;
		fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.write_str("at most 100 mentioned users")
		}
		fn visit_seq<A: serde::de::SeqAccess<'de>>(
			self,
			mut sequence: A,
		) -> Result<Self::Value, A::Error> {
			let mut users = Vec::new();
			for _ in 0..MAX_MENTIONS {
				match sequence.next_element()? {
					Some(user) => users.push(user),
					None => return Ok(users),
				}
			}
			if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
				return Err(serde::de::Error::custom("mention limit"));
			}
			Ok(users)
		}
	}
	deserializer.deserialize_seq(Visitor(std::marker::PhantomData))
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn timestamp_references_accept_only_known_styles() {
		assert_eq!(
			timestamp_prefix("<t:1700000000:R> suffix"),
			Some((1_700_000_000, b'R', 16))
		);
		assert_eq!(timestamp_prefix("<t:0>"), Some((0, b'f', 5)));
		assert_eq!(timestamp_prefix("<t:-100:D>"), Some((-100, b'D', 10)));
		for text in [
			"<t:1700000000:z>",
			"<t:1700000000:RR>",
			"<t:1700000000:>",
			"<t:>",
			"<t:12abc:t>",
			"<t:1 2:t>",
			"<t:1700000000",
			"<t:100000000000000000000000000:t>",
			"t:1700000000:R>",
		] {
			assert_eq!(timestamp_prefix(text), None, "{text}");
		}
	}
	#[test]
	fn exact_user_mentions_are_bounded_and_never_roles_or_everyone() {
		assert!(has_mass_mention("hello @everyone and @here"));
		assert!(!has_mass_mention("hello @everyone_else"));
		assert_eq!(
			channel_mention_prefix("<#18446744073709551615> suffix"),
			Some((Id(u64::MAX), 23))
		);
		for text in [
			"<#0>",
			"<#>",
			"<#-1>",
			"<#+2>",
			"<# 2>",
			"<#18446744073709551616>",
			"<#000000000000000000001>",
			"<#2",
			"<@2>",
			"<#٢>",
		] {
			assert_eq!(channel_mention_prefix(text), None);
		}
		assert!(mentioned_user_ids("<#42> <#9>").is_empty());
		assert_eq!(
			mentioned_user_ids("<@42> <@!42> <@9> <@&5> @everyone @here <@0> <@+2> <@ 2>"),
			vec![Id(42), Id(9)]
		);
		assert_eq!(
			user_mention_prefix("<@!18446744073709551615> suffix"),
			Some((Id(u64::MAX), 24))
		);
		assert_eq!(
			mentioned_user_ids(&(1..=200).map(|id| format!("<@{id}>")).collect::<String>()).len(),
			100
		);
	}
}

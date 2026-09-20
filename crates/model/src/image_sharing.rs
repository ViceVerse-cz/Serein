use crate::Id;

/// Explicitly stages public artwork as a normal attachment, without sending a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageShare {
	Emoji { id: Id, animated: bool },
	Sticker { id: Id, format_type: u8 },
}

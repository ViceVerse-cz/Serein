use crate::Id;

/// Public artwork selected for an ordinary image attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageShare {
	Emoji { id: Id, animated: bool },
	Sticker { id: Id, format_type: u8 },
}

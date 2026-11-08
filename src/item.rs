//! Identity of entries in a folder.

/// Stable identity of one item while it remains in the folder.
#[derive(
	Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ItemId(u64);

impl ItemId {
	pub fn new(raw: u64) -> Self {
		Self(raw)
	}

	pub fn get(self) -> u64 {
		self.0
	}
}

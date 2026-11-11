//! Identity and description of entries in a folder.

use std::path::PathBuf;

/// Stable identity of one item while it remains in the folder.
#[derive(
	Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ItemId(u64);

impl ItemId {
	pub fn new(raw: u64) -> Self {
		Self(raw)
	}

	/// Derive the identity from the item's path, so an id survives a rescan
	/// while the path remains in the folder.
	pub fn for_path(path: &std::path::Path) -> Self {
		let mut hash = 0xcbf2_9ce4_8422_2325u64;
		for byte in path.as_os_str().as_encoded_bytes() {
			hash ^= *byte as u64;
			hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
		}
		Self(hash)
	}

	pub fn get(self) -> u64 {
		self.0
	}
}

#[derive(
	Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum FsKind {
	File,
	Directory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ItemKind {
	Directory,
	File,
	Application,
	Link,
	Button,
	Toggle,
	Number,
	Radio,
}

impl ItemKind {
	pub fn from_fs(kind: FsKind) -> Self {
		match kind {
			FsKind::Directory => Self::Directory,
			FsKind::File => Self::File,
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IconRef {
	Theme(String),
	Path(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
	pub id: ItemId,
	pub name: String,
	pub title: String,
	pub comment: String,
	pub path: PathBuf,
	/// The entry that supplied this item's metadata, when there is one.
	pub entry_path: Option<PathBuf>,
	/// The target of a `Type=Link` entry.
	pub target: Option<PathBuf>,
	pub fs_kind: FsKind,
	pub kind: ItemKind,
	pub mime: Option<String>,
	pub icon: IconRef,
	/// Bare label art from a generated descriptor's `[Topscreen] IconArt=`.
	/// The 3D banner uses this instead of the file view's card drawing.
	pub model_icon: Option<PathBuf>,
	pub is_symlink: bool,
	pub is_dotfile: bool,
	pub toggle_on: bool,
	pub number_value: String,
	pub size: u64,
	pub modified: u64,
	pub created: u64,
	pub accessed: u64,
	pub group: u16,
	pub group_label: String,
	pub is_stashed: bool,
	pub custom_group: String,
	/// A live download, copy or move attached to this item.
	pub progress: Option<crate::progress::Progress>,
}

impl Item {
	/// An item with a live task cannot be opened or entered.
	pub fn is_busy(&self) -> bool {
		self.progress.is_some()
	}
}

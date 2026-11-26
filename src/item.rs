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
	Radio,
	/// A display-only row: it carries a label, an icon and a value, and has no
	/// action of its own.
	Info,
}

impl ItemKind {
	pub fn from_fs(kind: FsKind) -> Self {
		match kind {
			FsKind::Directory => Self::Directory,
			FsKind::File => Self::File,
		}
	}
}

/// How a control entry presents itself and what the frontend has to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlState {
	None,
	Radio {
		selected: bool,
	},
	/// A read-only value drawn beside the label, for a `Type=Info` row.
	Value {
		text: String,
	},
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
	/// `Generated=true`: a provider daemon writes this entry from live data.
	/// Only a provider-owned folder may trust the flag; elsewhere it is just
	/// text in a file the user can write.
	pub generated: bool,
	/// The entry has a removal of its own (`Destroy=`), or it is not
	/// generated. A generated entry without one has no file to remove.
	pub destroyable: bool,
	/// The target of a `Type=Link` entry.
	pub target: Option<PathBuf>,
	pub fs_kind: FsKind,
	pub kind: ItemKind,
	pub mime: Option<String>,
	pub icon: IconRef,
	/// Bare label art from a generated descriptor's `[Topscreen] IconArt=`.
	/// The 3D banner uses this instead of the file view's card drawing.
	pub model_icon: Option<PathBuf>,
	/// `UID`: this app's folder name in the save manager.
	pub uid: Option<String>,
	pub is_symlink: bool,
	pub is_dotfile: bool,
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

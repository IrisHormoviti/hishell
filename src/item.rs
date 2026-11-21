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
	Slider,
	Text,
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

/// The position and range of a `Type=Slider` control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Slider {
	pub value: i64,
	pub min: i64,
	pub max: i64,
	pub step: i64,
}

impl Default for Slider {
	fn default() -> Self {
		Self {
			value: 0,
			min: 0,
			max: 100,
			step: 1,
		}
	}
}

/// How a control entry presents itself and what the frontend has to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlState {
	None,
	Toggle {
		on: bool,
	},
	Number {
		value: String,
	},
	Slider(Slider),
	Text {
		value: String,
		placeholder: String,
	},
	Radio {
		selected: bool,
	},
	Button,
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
	pub toggle_on: bool,
	pub number_value: String,
	pub number_fields: Vec<crate::entry::NumberField>,
	pub slider: Option<Slider>,
	pub text_value: String,
	pub text_placeholder: String,
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

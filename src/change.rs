//! Change sets returned by engine operations.

use std::path::PathBuf;

use crate::config::ConfigDelta;
use crate::item::ItemId;
use crate::location::LocationKind;
use crate::view::PaneId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ChangeScope {
	Order,
	Items,
	View,
	All,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum FolderDelta {
	#[default]
	Reset,
	Items {
		added: Vec<ItemId>,
		removed: Vec<ItemId>,
		updated: Vec<ItemId>,
		order: Vec<ItemId>,
	},
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SelectionDelta {
	pub active: bool,
	pub selected: Vec<ItemId>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FocusDelta {
	pub pane: PaneId,
	pub item: Option<ItemId>,
	pub previous: Option<ItemId>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocationDelta {
	pub kind: Option<LocationKind>,
	pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChangeSet {
	pub folder: FolderDelta,
	pub config: Vec<ConfigDelta>,
	pub selection: Option<SelectionDelta>,
	pub focus: Option<FocusDelta>,
	pub location: Option<LocationDelta>,
}

impl ChangeSet {
	pub fn is_empty(&self) -> bool {
		self.config.is_empty()
			&& self.selection.is_none()
			&& self.focus.is_none()
			&& self.location.is_none()
			&& matches!(self.folder, FolderDelta::Reset)
	}
}

//! Action registry and availability.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::config::LocationScope;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActionId(pub &'static str);

impl Serialize for ActionId {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(self.0)
	}
}

impl<'de> Deserialize<'de> for ActionId {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let value = String::deserialize(deserializer)?;
		Self::from_str(&value).ok_or_else(|| serde::de::Error::custom("unknown action"))
	}
}

impl ActionId {
	pub const OPEN: Self = Self("open");
	pub const OPEN_WITH: Self = Self("open-with");
	pub const NAVIGATE: Self = Self("navigate");
	pub const COPY: Self = Self("copy");
	pub const CUT: Self = Self("cut");
	pub const PASTE: Self = Self("paste");
	pub const DUPLICATE: Self = Self("duplicate");
	pub const CREATE_LINK: Self = Self("create-link");
	pub const RENAME: Self = Self("rename");
	pub const TRASH: Self = Self("trash");
	pub const DELETE: Self = Self("delete");
	pub const DESTROY: Self = Self("destroy");
	pub const PROPERTIES: Self = Self("properties");
	pub const GO_UP: Self = Self("go-up");
	pub const REFRESH: Self = Self("refresh");
	pub const SELECT_ALL: Self = Self("select-all");
	pub const VIEW_SETTINGS: Self = Self("view-settings");
	pub const NEW_FOLDER: Self = Self("new-folder");
	pub const NEW_TEXT_FILE: Self = Self("new-text-file");
	pub const ROTATE_IMAGE: Self = Self("rotate-image");
	pub const TOGGLE: Self = Self("toggle");
	pub const NUMBER: Self = Self("number");
	pub const RADIO: Self = Self("radio");
	pub const SET_DEFAULT_APP: Self = Self("set-default-app");
	pub const QUIT_ACTIVE: Self = Self("quit-active");

	pub fn as_str(self) -> &'static str {
		self.0
	}

	pub fn from_str(value: &str) -> Option<Self> {
		DESCRIPTORS
			.iter()
			.find(|descriptor| descriptor.id.0 == value)
			.map(|descriptor| descriptor.id)
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MenuKind {
	Main,
	Context,
	Edit,
	New,
	FolderMenu,
	View,
	SelectionBar,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
	Location,
	Focus,
	Selection,
	Items(Vec<crate::item::ItemId>),
	Auto,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRequest {
	pub menu: MenuKind,
	pub target: Target,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActionSubjects(u32);

impl ActionSubjects {
	pub const HERE: Self = Self(1 << 0);
	pub const FOLDER: Self = Self(1 << 1);
	pub const FILE: Self = Self(1 << 2);
	pub const IMAGE: Self = Self(1 << 3);
	pub const AUDIO: Self = Self(1 << 4);
	pub const VIDEO: Self = Self(1 << 5);
	pub const APPLICATION: Self = Self(1 << 6);
	pub const LINK: Self = Self(1 << 7);
	pub const BUTTON: Self = Self(1 << 8);
	pub const TOGGLE: Self = Self(1 << 9);
	pub const NUMBER: Self = Self(1 << 10);
	pub const RADIO: Self = Self(1 << 11);
	pub const ITEMS: Self = Self(
		Self::FOLDER.0
			| Self::FILE.0
			| Self::IMAGE.0
			| Self::AUDIO.0
			| Self::VIDEO.0
			| Self::APPLICATION.0
			| Self::LINK.0
			| Self::BUTTON.0
			| Self::TOGGLE.0
			| Self::NUMBER.0
			| Self::RADIO.0,
	);

	pub fn contains(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}

	pub fn intersects(self, other: Self) -> bool {
		self.0 & other.0 != 0
	}

	pub const fn union(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupId(pub &'static str);

impl Serialize for GroupId {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(self.0)
	}
}

impl<'de> Deserialize<'de> for GroupId {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let value = String::deserialize(deserializer)?;
		Ok(Self(match value.as_str() {
			"open" => "open",
			"navigation" => "navigation",
			"edit" => "edit",
			"new" => "new",
			"media" => "media",
			"dangerous" => "dangerous",
			"view" => "view",
			"selection" => "selection",
			_ => return Err(serde::de::Error::custom("unknown group")),
		}))
	}
}

impl GroupId {
	pub const OPEN: Self = Self("open");
	pub const NAVIGATION: Self = Self("navigation");
	pub const EDIT: Self = Self("edit");
	pub const NEW: Self = Self("new");
	pub const MEDIA: Self = Self("media");
	pub const DANGEROUS: Self = Self("dangerous");
	pub const VIEW: Self = Self("view");
	pub const SELECTION: Self = Self("selection");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActionGroup {
	pub id: GroupId,
	pub order: i32,
}

pub struct ActionDescriptor {
	pub id: ActionId,
	pub group: ActionGroup,
	pub subjects: ActionSubjects,
	pub menus: &'static [MenuKind],
	pub location: LocationScope,
	pub icon_name: &'static str,
	pub default_shortcut: Option<&'static str>,
	pub i18n_key: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DisabledReason {
	NoItems,
	NoSelection,
	NotApplicable,
	Busy,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActionSpec {
	pub id: ActionId,
	pub group: ActionGroup,
	pub subjects: ActionSubjects,
	pub location: LocationScope,
	pub enabled: bool,
	pub visible: bool,
	pub checked: Option<bool>,
	pub disabled_reason: Option<DisabledReason>,
}

const GROUP_OPEN: ActionGroup = ActionGroup {
	id: GroupId::OPEN,
	order: 10,
};
const GROUP_NAVIGATION: ActionGroup = ActionGroup {
	id: GroupId::NAVIGATION,
	order: 20,
};
const GROUP_EDIT: ActionGroup = ActionGroup {
	id: GroupId::EDIT,
	order: 30,
};
const GROUP_NEW: ActionGroup = ActionGroup {
	id: GroupId::NEW,
	order: 40,
};
const GROUP_MEDIA: ActionGroup = ActionGroup {
	id: GroupId::MEDIA,
	order: 50,
};
const GROUP_DANGEROUS: ActionGroup = ActionGroup {
	id: GroupId::DANGEROUS,
	order: 60,
};
const GROUP_VIEW: ActionGroup = ActionGroup {
	id: GroupId::VIEW,
	order: 70,
};
const GROUP_SELECTION: ActionGroup = ActionGroup {
	id: GroupId::SELECTION,
	order: 80,
};

const MAIN_CONTEXT: &[MenuKind] = &[MenuKind::Main, MenuKind::Context, MenuKind::Edit];
const CONTEXT: &[MenuKind] = &[MenuKind::Context, MenuKind::Edit];
const NEW: &[MenuKind] = &[MenuKind::New, MenuKind::Context];
const VIEW: &[MenuKind] = &[MenuKind::View, MenuKind::FolderMenu];
const SELECTION: &[MenuKind] = &[MenuKind::SelectionBar, MenuKind::Edit];

pub static DESCRIPTORS: &[ActionDescriptor] = &[
	ActionDescriptor {
		id: ActionId::OPEN,
		group: GROUP_OPEN,
		subjects: ActionSubjects::ITEMS,
		menus: MAIN_CONTEXT,
		location: LocationScope::Both,
		icon_name: "document-open",
		default_shortcut: Some("Return"),
		i18n_key: "open",
	},
	ActionDescriptor {
		id: ActionId::OPEN_WITH,
		group: GROUP_OPEN,
		subjects: ActionSubjects::ITEMS,
		menus: CONTEXT,
		location: LocationScope::Both,
		icon_name: "document-open-recent",
		default_shortcut: None,
		i18n_key: "open-with",
	},
	ActionDescriptor {
		id: ActionId::NAVIGATE,
		group: GROUP_NAVIGATION,
		subjects: ActionSubjects::FOLDER.union(ActionSubjects::LINK),
		menus: MAIN_CONTEXT,
		location: LocationScope::Folder,
		icon_name: "go-next",
		default_shortcut: None,
		i18n_key: "navigate",
	},
	ActionDescriptor {
		id: ActionId::COPY,
		group: GROUP_EDIT,
		subjects: ActionSubjects::ITEMS,
		menus: MAIN_CONTEXT,
		location: LocationScope::Both,
		icon_name: "edit-copy",
		default_shortcut: Some("Ctrl+C"),
		i18n_key: "copy",
	},
	ActionDescriptor {
		id: ActionId::CUT,
		group: GROUP_EDIT,
		subjects: ActionSubjects::ITEMS,
		menus: MAIN_CONTEXT,
		location: LocationScope::Both,
		icon_name: "edit-cut",
		default_shortcut: Some("Ctrl+X"),
		i18n_key: "cut",
	},
	ActionDescriptor {
		id: ActionId::PASTE,
		group: GROUP_EDIT,
		subjects: ActionSubjects::HERE,
		menus: MAIN_CONTEXT,
		location: LocationScope::Folder,
		icon_name: "edit-paste",
		default_shortcut: Some("Ctrl+V"),
		i18n_key: "paste",
	},
	ActionDescriptor {
		id: ActionId::DUPLICATE,
		group: GROUP_EDIT,
		subjects: ActionSubjects::ITEMS,
		menus: CONTEXT,
		location: LocationScope::Both,
		icon_name: "edit-duplicate",
		default_shortcut: None,
		i18n_key: "duplicate",
	},
	ActionDescriptor {
		id: ActionId::CREATE_LINK,
		group: GROUP_EDIT,
		subjects: ActionSubjects::ITEMS,
		menus: CONTEXT,
		location: LocationScope::Both,
		icon_name: "edit-link",
		default_shortcut: None,
		i18n_key: "create-link",
	},
	ActionDescriptor {
		id: ActionId::RENAME,
		group: GROUP_EDIT,
		subjects: ActionSubjects::ITEMS,
		menus: MAIN_CONTEXT,
		location: LocationScope::Both,
		icon_name: "edit-rename",
		default_shortcut: Some("F2"),
		i18n_key: "rename",
	},
	ActionDescriptor {
		id: ActionId::TRASH,
		group: GROUP_DANGEROUS,
		subjects: ActionSubjects::ITEMS,
		menus: MAIN_CONTEXT,
		location: LocationScope::Both,
		icon_name: "edit-delete",
		default_shortcut: Some("Delete"),
		i18n_key: "trash",
	},
	ActionDescriptor {
		id: ActionId::DELETE,
		group: GROUP_DANGEROUS,
		subjects: ActionSubjects::ITEMS,
		menus: CONTEXT,
		location: LocationScope::Both,
		icon_name: "edit-delete",
		default_shortcut: None,
		i18n_key: "delete",
	},
	ActionDescriptor {
		id: ActionId::DESTROY,
		group: GROUP_DANGEROUS,
		subjects: ActionSubjects::ITEMS,
		menus: CONTEXT,
		location: LocationScope::Both,
		icon_name: "edit-delete-shred",
		default_shortcut: Some("Shift+Delete"),
		i18n_key: "destroy",
	},
	ActionDescriptor {
		id: ActionId::PROPERTIES,
		group: GROUP_EDIT,
		subjects: ActionSubjects::ITEMS,
		menus: CONTEXT,
		location: LocationScope::Both,
		icon_name: "document-properties",
		default_shortcut: None,
		i18n_key: "properties",
	},
	ActionDescriptor {
		id: ActionId::GO_UP,
		group: GROUP_NAVIGATION,
		subjects: ActionSubjects::HERE,
		menus: MAIN_CONTEXT,
		location: LocationScope::Folder,
		icon_name: "go-up",
		default_shortcut: Some("Alt+Up"),
		i18n_key: "go-up",
	},
	ActionDescriptor {
		id: ActionId::REFRESH,
		group: GROUP_VIEW,
		subjects: ActionSubjects::HERE,
		menus: MAIN_CONTEXT,
		location: LocationScope::Folder,
		icon_name: "view-refresh",
		default_shortcut: Some("F5"),
		i18n_key: "refresh",
	},
	ActionDescriptor {
		id: ActionId::SELECT_ALL,
		group: GROUP_SELECTION,
		subjects: ActionSubjects::HERE,
		menus: SELECTION,
		location: LocationScope::Folder,
		icon_name: "edit-select-all",
		default_shortcut: Some("Ctrl+A"),
		i18n_key: "select-all",
	},
	ActionDescriptor {
		id: ActionId::VIEW_SETTINGS,
		group: GROUP_VIEW,
		subjects: ActionSubjects::HERE,
		menus: VIEW,
		location: LocationScope::Folder,
		icon_name: "view-choose",
		default_shortcut: None,
		i18n_key: "view-settings",
	},
	ActionDescriptor {
		id: ActionId::NEW_FOLDER,
		group: GROUP_NEW,
		subjects: ActionSubjects::HERE,
		menus: NEW,
		location: LocationScope::Folder,
		icon_name: "folder-new",
		default_shortcut: Some("Ctrl+Shift+N"),
		i18n_key: "new-folder",
	},
	ActionDescriptor {
		id: ActionId::NEW_TEXT_FILE,
		group: GROUP_NEW,
		subjects: ActionSubjects::HERE,
		menus: NEW,
		location: LocationScope::Folder,
		icon_name: "document-new",
		default_shortcut: None,
		i18n_key: "new-text-file",
	},
	ActionDescriptor {
		id: ActionId::ROTATE_IMAGE,
		group: GROUP_MEDIA,
		subjects: ActionSubjects::IMAGE,
		menus: CONTEXT,
		location: LocationScope::Both,
		icon_name: "object-rotate-right",
		default_shortcut: None,
		i18n_key: "rotate-image",
	},
	ActionDescriptor {
		id: ActionId::TOGGLE,
		group: GROUP_MEDIA,
		subjects: ActionSubjects::TOGGLE,
		menus: MAIN_CONTEXT,
		location: LocationScope::Both,
		icon_name: "document-edit",
		default_shortcut: None,
		i18n_key: "toggle",
	},
	ActionDescriptor {
		id: ActionId::NUMBER,
		group: GROUP_MEDIA,
		subjects: ActionSubjects::NUMBER,
		menus: MAIN_CONTEXT,
		location: LocationScope::Both,
		icon_name: "document-edit",
		default_shortcut: None,
		i18n_key: "number",
	},
	ActionDescriptor {
		id: ActionId::RADIO,
		group: GROUP_MEDIA,
		subjects: ActionSubjects::RADIO,
		menus: MAIN_CONTEXT,
		location: LocationScope::Both,
		icon_name: "radio-checked",
		default_shortcut: None,
		i18n_key: "radio",
	},
	ActionDescriptor {
		id: ActionId::SET_DEFAULT_APP,
		group: GROUP_OPEN,
		subjects: ActionSubjects::ITEMS,
		menus: CONTEXT,
		location: LocationScope::Both,
		icon_name: "application-x-executable",
		default_shortcut: None,
		i18n_key: "set-default-app",
	},
	ActionDescriptor {
		id: ActionId::QUIT_ACTIVE,
		group: GROUP_DANGEROUS,
		subjects: ActionSubjects::APPLICATION,
		menus: CONTEXT,
		location: LocationScope::Both,
		icon_name: "process-stop",
		default_shortcut: None,
		i18n_key: "quit-active",
	},
];

pub fn descriptor(id: ActionId) -> Option<&'static ActionDescriptor> {
	DESCRIPTORS.iter().find(|descriptor| descriptor.id == id)
}

pub fn menu_contains(descriptor: &ActionDescriptor, menu: MenuKind) -> bool {
	descriptor.menus.contains(&menu) || descriptor.menus.is_empty()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn actions_round_trip_through_json() {
		let spec = ActionSpec {
			id: ActionId::RENAME,
			group: GROUP_EDIT,
			subjects: ActionSubjects::FILE,
			location: LocationScope::Both,
			enabled: true,
			visible: true,
			checked: None,
			disabled_reason: None,
		};
		let text = serde_json::to_string(&spec).unwrap();
		let restored: ActionSpec = serde_json::from_str(&text).unwrap();
		assert_eq!(restored, spec);
		assert_eq!(ActionId::from_str("rename"), Some(ActionId::RENAME));
		assert_eq!(ActionId::from_str("nope"), None);
	}
}

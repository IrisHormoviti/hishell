//! Schema-driven configuration.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::change::ChangeScope;
use crate::config_parser::ConfigParser;
use crate::error::Error;
use crate::location::{Location, LocationKind};
pub use crate::value::ConfigValue;
use crate::view::{
	CustomGroup, GridMode, GroupMode, HorizontalAlign, LabelText, ListColumn, RadioOnly,
	SortAlphaMode, SortDateMode, SortKey, VerticalAlign, ViewMode,
};

/// Where a key applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LocationScope {
	Both,
	Folder,
	File,
}

impl ConfigKey {
	pub fn location_scope(self) -> LocationScope {
		self.scope()
	}
}

macro_rules! config_keys {
	($(
		$variant:ident => $section:literal, $name:literal, $scope:ident, $change:ident, $default:expr;
	)*) => {
		#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
		pub enum ConfigKey {
			$($variant,)*
		}

		impl ConfigKey {
			pub const ALL: &'static [ConfigKey] = &[$(Self::$variant,)*];

			pub fn section(self) -> &'static str {
				match self {
					$(Self::$variant => $section,)*
				}
			}

			pub fn name(self) -> &'static str {
				match self {
					$(Self::$variant => $name,)*
				}
			}

			pub fn scope(self) -> LocationScope {
				match self {
					$(Self::$variant => LocationScope::$scope,)*
				}
			}

			pub fn change_scope(self) -> ChangeScope {
				match self {
					$(Self::$variant => ChangeScope::$change,)*
				}
			}

			pub fn default(self) -> ConfigValue {
				match self {
					$(Self::$variant => $default,)*
				}
			}

			pub fn from_section_name(section: &str, name: &str) -> Option<Self> {
				match (section, name) {
					$(($section, $name) => Some(Self::$variant),)*
					_ => None,
				}
			}
		}
	};
}

config_keys! {
	Title => "Desktop Entry", "Name", Both, All, ConfigValue::Str(String::new());
	Icon => "Desktop Entry", "Icon", Both, All, ConfigValue::Str(String::new());

	Wallpaper => "Folder View", "Wallpaper", Both, All, ConfigValue::Str(String::new());
	Maximized => "Folder View", "Maximized", Both, All, ConfigValue::Bool(false);

	GridMode => "Folder View", "GridMode", Folder, View, ConfigValue::Str(String::from("LINES"));
	GridSize => "Folder View", "GridSize", Folder, View, ConfigValue::Int(72);
	Lines => "Folder View", "Lines", Folder, View, ConfigValue::Int(4);
	GridHorizontalAlign => "Folder View", "GridHorizontalAlign", Folder, View, ConfigValue::Str(String::from("FILL"));
	GridVerticalAlign => "Folder View", "GridVerticalAlign", Folder, View, ConfigValue::Str(String::from("TOP"));
	ScrollDirection => "Folder View", "ScrollDirection", Folder, View, ConfigValue::Str(String::from("VERTICAL"));
	ViewMode => "Folder View", "ViewMode", Folder, View, ConfigValue::Str(String::from("GRID"));
	ListColumns => "Folder View", "ListColumns", Folder, View, ConfigValue::Json(String::from(r#"["TITLE"]"#));
	ListHeader => "Folder View", "ListHeader", Folder, View, ConfigValue::Bool(false);
	ShowLabels => "Folder View", "ShowLabels", Folder, View, ConfigValue::Bool(true);
	GridLabelsBesidesIcons => "Folder View", "GridLabelsBesidesIcons", Folder, View, ConfigValue::Bool(false);
	LabelText => "Folder View", "LabelText", Folder, View, ConfigValue::Str(String::from("TITLE"));
	Sort => "Folder View", "Sort", Folder, Order, ConfigValue::Str(String::from("NEWEST"));
	SortDateMode => "Folder View", "SortDateMode", Folder, Order, ConfigValue::Str(String::from("MODIFIED"));
	SortAlphaMode => "Folder View", "SortAlphaMode", Folder, Order, ConfigValue::Str(String::from("FILENAMES"));
	Group => "Folder View", "Group", Folder, Order, ConfigValue::Str(String::from("NONE"));
	Groups => "Folder View", "Groups", Folder, Order, ConfigValue::Json(String::from("[]"));
	ShowStash => "Folder View", "ShowStash", Folder, Items, ConfigValue::Bool(false);
	StashShown => "Folder View", "StashShown", Folder, Items, ConfigValue::Bool(false);
	StashDotFiles => "Folder View", "StashDotFiles", Folder, Items, ConfigValue::Bool(true);
	FreePlacementPositions => "Folder View", "FreePlacementPositions", Folder, View, ConfigValue::Json(String::from("{}"));

	Top => "Layout", "Top", Both, All, ConfigValue::Json(String::from("[]"));
	Middle => "Layout", "Middle", Both, All, ConfigValue::Json(String::from(r#"["./"]"#));
	Bottom => "Layout", "Bottom", Both, All, ConfigValue::Json(String::from("[]"));
	Header => "Layout", "Header", Both, All, ConfigValue::Json(String::from(
		r#"["toolkit/PathBar","toolkit/Spacer","toolkit/MenuBar","toolkit/WindowControl"]"#,
	));
	NativeMenuBar => "Layout", "NativeMenuBar", Both, All, ConfigValue::Bool(false);
	NativeMenus => "Layout", "NativeMenus", Both, All, ConfigValue::Bool(false);
	NativeTitleBar => "Layout", "NativeTitleBar", Both, All, ConfigValue::Bool(true);
	NativeDrag => "Layout", "NativeDrag", Both, All, ConfigValue::Bool(false);
	ShowShortcuts => "Layout", "ShowShortcuts", Both, All, ConfigValue::Bool(true);
	ExecutionWrapper => "Layout", "ExecutionWrapper", Both, All, ConfigValue::Str(String::new());
	Screen => "Layout", "Screen", Both, All, ConfigValue::Str(String::new());

	NavigationType => "Folder Navigation", "NavigationType", Folder, View, ConfigValue::Str(String::from("IN_PLACE"));
	CenterFocus => "Folder Navigation", "CenterFocus", Folder, View, ConfigValue::Bool(true);
	SmoothScrolling => "Folder Navigation", "SmoothScrolling", Folder, View, ConfigValue::Bool(true);
	OpenOnClick => "Folder Navigation", "OpenOnClick", Folder, View, ConfigValue::Bool(true);
	FocusEmptySlots => "Folder Navigation", "FocusEmptySlots", Folder, View, ConfigValue::Bool(false);
	DropItemPreference => "Folder Navigation", "DropItemPreference", Folder, View, ConfigValue::Str(String::from("MOVE"));
	DropItemPreferenceMandate => "Folder Navigation", "DropItemPreferenceMandate", Folder, View, ConfigValue::Bool(false);

	RadioEnabled => "Radio", "Enabled", Folder, View, ConfigValue::Bool(false);
	RadioOnly => "Radio", "Only", Folder, View, ConfigValue::Str(String::from("ALL"));
	RadioSelected => "Radio", "Selected", Folder, View, ConfigValue::Str(String::new());
	RadioExec => "Radio", "Exec", Folder, View, ConfigValue::Str(String::new());
}

/// How opening a folder changes the current view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NavigationType {
    /// Replace the view in place; Back walks the folder stack.
    #[default]
    InPlace,
    /// Open a floating window rooted at the folder; Back closes it at its root.
    Window,
}

impl NavigationType {
    pub fn as_key(self) -> &'static str {
        match self {
            Self::InPlace => "IN_PLACE",
            Self::Window => "WINDOW",
        }
    }

    pub fn from_key(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "IN_PLACE" | "INPLACE" => Some(Self::InPlace),
            "WINDOW" => Some(Self::Window),
            _ => None,
        }
    }
}

/// What a drop into a folder does when the destination does not mandate the
/// operation. The shell may still offer another one (the shake gesture).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DropPreference {
	#[default]
	Move,
	Copy,
	Link,
}

impl DropPreference {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::Move => "MOVE",
			Self::Copy => "COPY",
			Self::Link => "LINK",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.to_ascii_uppercase().as_str() {
			"MOVE" => Some(Self::Move),
			"COPY" => Some(Self::Copy),
			"LINK" => Some(Self::Link),
			_ => None,
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
	pub key: ConfigKey,
	pub message: String,
}

impl fmt::Display for ConfigError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(formatter, "{}: {}", self.key.name(), self.message)
	}
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConfigDiagnostic {
	pub file: PathBuf,
	pub line: Option<u32>,
	pub message: String,
}

#[derive(Clone, Debug)]
pub struct ConfigLoad {
	pub config: Config,
	pub diagnostics: Vec<ConfigDiagnostic>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfigDelta {
	pub key: ConfigKey,
	pub scope: ChangeScope,
	pub old: ConfigValue,
	pub new: ConfigValue,
}

/// Every configuration value, typed.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
	pub title: String,
	pub icon: String,
	pub wallpaper: String,
	pub maximized: bool,

	pub grid_mode: GridMode,
	pub grid_size: u16,
	pub lines: u8,
	pub horizontal_align: HorizontalAlign,
	pub vertical_align: VerticalAlign,
	pub scroll_horizontal: bool,
	pub view_mode: ViewMode,
	pub list_columns: Vec<ListColumn>,
	pub list_header: bool,
	pub show_labels: bool,
	pub labels_beside_icons: bool,
	pub label_text: LabelText,
	pub sort: SortKey,
	pub sort_date_mode: SortDateMode,
	pub sort_alpha_mode: SortAlphaMode,
	pub group_mode: GroupMode,
	pub groups: Vec<CustomGroup>,
	pub show_stash: bool,
	pub stash_shown: bool,
	pub stash_dotfiles: bool,
	pub free_placement_positions: HashMap<String, (i32, i32)>,

	pub top_layout: Vec<String>,
	pub middle_layout: Vec<String>,
	pub bottom_layout: Vec<String>,
	pub header_layout: Vec<String>,
	pub native_menubar: bool,
	pub native_menus: bool,
	pub native_titlebar: bool,
	pub native_drag: bool,
	pub show_shortcuts: bool,
	pub execution_wrapper: String,
	pub screen: String,

	pub navigation_type: NavigationType,
	pub center_focus: bool,
	pub smooth_scrolling: bool,
	pub open_on_click: bool,
	pub focus_empty_slots: bool,
	pub drop_preference: DropPreference,
	pub drop_preference_mandate: bool,

	pub radio_enabled: bool,
	pub radio_only: RadioOnly,
	pub radio_selected: String,
	pub radio_exec: String,
}

impl Config {
	fn blank() -> Self {
		Self {
			title: String::new(),
			icon: String::new(),
			wallpaper: String::new(),
			maximized: false,
			grid_mode: GridMode::Lines,
			grid_size: 0,
			lines: 0,
			horizontal_align: HorizontalAlign::Fill,
			vertical_align: VerticalAlign::Top,
			scroll_horizontal: false,
			view_mode: ViewMode::Grid,
			list_columns: Vec::new(),
			list_header: false,
			show_labels: true,
			labels_beside_icons: false,
			label_text: LabelText::Title,
			sort: SortKey::Newest,
			sort_date_mode: SortDateMode::Modified,
			sort_alpha_mode: SortAlphaMode::Filenames,
			group_mode: GroupMode::None,
			groups: Vec::new(),
			show_stash: false,
			stash_shown: false,
			stash_dotfiles: false,
			free_placement_positions: HashMap::new(),
			top_layout: Vec::new(),
			middle_layout: Vec::new(),
			bottom_layout: Vec::new(),
			header_layout: Vec::new(),
			native_menubar: false,
			native_menus: false,
			native_titlebar: true,
			native_drag: false,
			show_shortcuts: true,
			execution_wrapper: String::new(),
			screen: String::new(),
			navigation_type: NavigationType::InPlace,
			center_focus: false,
			smooth_scrolling: false,
			open_on_click: true,
			focus_empty_slots: false,
			drop_preference: DropPreference::Move,
			drop_preference_mandate: false,
			radio_enabled: false,
			radio_only: RadioOnly::All,
			radio_selected: String::new(),
			radio_exec: String::new(),
		}
	}

	pub fn get(&self, key: ConfigKey) -> ConfigValue {
		match key {
			ConfigKey::Title => ConfigValue::Str(self.title.clone()),
			ConfigKey::Icon => ConfigValue::Str(self.icon.clone()),
			ConfigKey::Wallpaper => ConfigValue::Str(self.wallpaper.clone()),
			ConfigKey::Maximized => ConfigValue::Bool(self.maximized),
			ConfigKey::GridMode => ConfigValue::Str(self.grid_mode.as_key().into()),
			ConfigKey::GridSize => ConfigValue::Int(self.grid_size as i64),
			ConfigKey::Lines => ConfigValue::Int(self.lines as i64),
			ConfigKey::GridHorizontalAlign => {
				ConfigValue::Str(self.horizontal_align.as_key().into())
			}
			ConfigKey::GridVerticalAlign => ConfigValue::Str(self.vertical_align.as_key().into()),
			ConfigKey::ScrollDirection => ConfigValue::Str(
				if self.scroll_horizontal {
					"HORIZONTAL"
				} else {
					"VERTICAL"
				}
				.into(),
			),
			ConfigKey::ViewMode => ConfigValue::Str(self.view_mode.as_key().into()),
			ConfigKey::ListColumns => ConfigValue::Json(
				serde_json::to_string(
					&self
						.list_columns
						.iter()
						.map(|column| column.as_key())
						.collect::<Vec<_>>(),
				)
				.unwrap_or_else(|_| "[]".into()),
			),
			ConfigKey::ListHeader => ConfigValue::Bool(self.list_header),
			ConfigKey::ShowLabels => ConfigValue::Bool(self.show_labels),
			ConfigKey::GridLabelsBesidesIcons => ConfigValue::Bool(self.labels_beside_icons),
			ConfigKey::LabelText => ConfigValue::Str(self.label_text.as_key().into()),
			ConfigKey::Sort => ConfigValue::Str(self.sort.as_key().into()),
			ConfigKey::SortDateMode => ConfigValue::Str(self.sort_date_mode.as_key().into()),
			ConfigKey::SortAlphaMode => ConfigValue::Str(self.sort_alpha_mode.as_key().into()),
			ConfigKey::Group => ConfigValue::Str(self.group_mode.as_key().into()),
			ConfigKey::Groups => ConfigValue::Json(
				serde_json::to_string(&self.groups).unwrap_or_else(|_| "[]".into()),
			),
			ConfigKey::ShowStash => ConfigValue::Bool(self.show_stash),
			ConfigKey::StashShown => ConfigValue::Bool(self.stash_shown),
			ConfigKey::StashDotFiles => ConfigValue::Bool(self.stash_dotfiles),
			ConfigKey::FreePlacementPositions => ConfigValue::Json(
				serde_json::to_string(&self.free_placement_positions)
					.unwrap_or_else(|_| "{}".into()),
			),
			ConfigKey::Top => ConfigValue::Json(
				serde_json::to_string(&self.top_layout).unwrap_or_else(|_| "[]".into()),
			),
			ConfigKey::Middle => ConfigValue::Json(
				serde_json::to_string(&self.middle_layout).unwrap_or_else(|_| "[]".into()),
			),
			ConfigKey::Bottom => ConfigValue::Json(
				serde_json::to_string(&self.bottom_layout).unwrap_or_else(|_| "[]".into()),
			),
			ConfigKey::Header => ConfigValue::Json(
				serde_json::to_string(&self.header_layout).unwrap_or_else(|_| "[]".into()),
			),
			ConfigKey::NativeMenuBar => ConfigValue::Bool(self.native_menubar),
			ConfigKey::NativeMenus => ConfigValue::Bool(self.native_menus),
			ConfigKey::NativeTitleBar => ConfigValue::Bool(self.native_titlebar),
			ConfigKey::NativeDrag => ConfigValue::Bool(self.native_drag),
			ConfigKey::ShowShortcuts => ConfigValue::Bool(self.show_shortcuts),
			ConfigKey::ExecutionWrapper => ConfigValue::Str(self.execution_wrapper.clone()),
			ConfigKey::Screen => ConfigValue::Str(self.screen.clone()),
			ConfigKey::NavigationType => ConfigValue::Str(self.navigation_type.as_key().into()),
			ConfigKey::CenterFocus => ConfigValue::Bool(self.center_focus),
			ConfigKey::SmoothScrolling => ConfigValue::Bool(self.smooth_scrolling),
			ConfigKey::OpenOnClick => ConfigValue::Bool(self.open_on_click),
			ConfigKey::FocusEmptySlots => ConfigValue::Bool(self.focus_empty_slots),
			ConfigKey::DropItemPreference => {
				ConfigValue::Str(self.drop_preference.as_key().into())
			}
			ConfigKey::DropItemPreferenceMandate => {
				ConfigValue::Bool(self.drop_preference_mandate)
			}
			ConfigKey::RadioEnabled => ConfigValue::Bool(self.radio_enabled),
			ConfigKey::RadioOnly => ConfigValue::Str(self.radio_only.as_key().into()),
			ConfigKey::RadioSelected => ConfigValue::Str(self.radio_selected.clone()),
			ConfigKey::RadioExec => ConfigValue::Str(self.radio_exec.clone()),
		}
	}

	pub fn set(&mut self, key: ConfigKey, value: ConfigValue) -> Result<ConfigDelta, ConfigError> {
		let old = self.get(key);

		match key {
			ConfigKey::Title => self.title = string(key, value)?,
			ConfigKey::Icon => self.icon = string(key, value)?,
			ConfigKey::Wallpaper => self.wallpaper = string(key, value)?,
			ConfigKey::Maximized => self.maximized = boolean(key, value)?,
			ConfigKey::GridMode => self.grid_mode = parse(key, value, GridMode::from_key)?,
			ConfigKey::GridSize => self.grid_size = integer(key, value, 0)?.max(0) as u16,
			ConfigKey::Lines => self.lines = integer(key, value, 0)?.max(0) as u8,
			ConfigKey::GridHorizontalAlign => {
				self.horizontal_align = parse(key, value, HorizontalAlign::from_key)?
			}
			ConfigKey::GridVerticalAlign => {
				self.vertical_align = parse(key, value, VerticalAlign::from_key)?
			}
			ConfigKey::ScrollDirection => {
				let value = string(key, value)?;
				self.scroll_horizontal = match value.to_ascii_uppercase().as_str() {
					"HORIZONTAL" => true,
					"VERTICAL" => false,
					_ => return Err(invalid(key, &value)),
				};
			}
			ConfigKey::ViewMode => self.view_mode = parse(key, value, ViewMode::from_key)?,
			ConfigKey::ListColumns => {
				self.list_columns = json::<Vec<String>>(key, value)?
					.into_iter()
					.filter_map(|column| ListColumn::from_key(&column))
					.collect();
			}
			ConfigKey::ListHeader => self.list_header = boolean(key, value)?,
			ConfigKey::ShowLabels => self.show_labels = boolean(key, value)?,
			ConfigKey::GridLabelsBesidesIcons => self.labels_beside_icons = boolean(key, value)?,
			ConfigKey::LabelText => self.label_text = parse(key, value, LabelText::from_key)?,
			ConfigKey::Sort => self.sort = parse(key, value, SortKey::from_key)?,
			ConfigKey::SortDateMode => {
				self.sort_date_mode = parse(key, value, SortDateMode::from_key)?
			}
			ConfigKey::SortAlphaMode => {
				self.sort_alpha_mode = parse(key, value, SortAlphaMode::from_key)?
			}
			ConfigKey::Group => self.group_mode = parse(key, value, GroupMode::from_key)?,
			ConfigKey::Groups => self.groups = json(key, value)?,
			ConfigKey::ShowStash => self.show_stash = boolean(key, value)?,
			ConfigKey::StashShown => self.stash_shown = boolean(key, value)?,
			ConfigKey::StashDotFiles => self.stash_dotfiles = boolean(key, value)?,
			ConfigKey::FreePlacementPositions => self.free_placement_positions = json(key, value)?,
			ConfigKey::Top => self.top_layout = json(key, value)?,
			ConfigKey::Middle => self.middle_layout = json(key, value)?,
			ConfigKey::Bottom => self.bottom_layout = json(key, value)?,
			ConfigKey::Header => self.header_layout = json(key, value)?,
			ConfigKey::NativeMenuBar => self.native_menubar = boolean(key, value)?,
			ConfigKey::NativeMenus => self.native_menus = boolean(key, value)?,
			ConfigKey::NativeTitleBar => self.native_titlebar = boolean(key, value)?,
			ConfigKey::NativeDrag => self.native_drag = boolean(key, value)?,
			ConfigKey::ShowShortcuts => self.show_shortcuts = boolean(key, value)?,
			ConfigKey::ExecutionWrapper => self.execution_wrapper = string(key, value)?,
			ConfigKey::Screen => self.screen = string(key, value)?,
			ConfigKey::NavigationType => {
				self.navigation_type = parse(key, value, NavigationType::from_key)?
			}
			ConfigKey::CenterFocus => self.center_focus = boolean(key, value)?,
			ConfigKey::SmoothScrolling => self.smooth_scrolling = boolean(key, value)?,
			ConfigKey::OpenOnClick => self.open_on_click = boolean(key, value)?,
			ConfigKey::FocusEmptySlots => self.focus_empty_slots = boolean(key, value)?,
			ConfigKey::DropItemPreference => {
				self.drop_preference = parse(key, value, DropPreference::from_key)?
			}
			ConfigKey::DropItemPreferenceMandate => {
				self.drop_preference_mandate = boolean(key, value)?
			}
			ConfigKey::RadioEnabled => self.radio_enabled = boolean(key, value)?,
			ConfigKey::RadioOnly => self.radio_only = parse(key, value, RadioOnly::from_key)?,
			ConfigKey::RadioSelected => self.radio_selected = string(key, value)?,
			ConfigKey::RadioExec => self.radio_exec = string(key, value)?,
		}

		Ok(ConfigDelta {
			key,
			scope: key.change_scope(),
			old,
			new: self.get(key),
		})
	}

	pub fn grid_sizing(&self) -> crate::view::GridSizing {
		match self.grid_mode {
			GridMode::Lines => crate::view::GridSizing::FixedLines {
				lines: if self.lines == 0 {
					5
				} else {
					self.lines.clamp(1, 8)
				},
			},
			GridMode::Size => crate::view::GridSizing::FixedIcon {
				icon_size: if self.grid_size == 0 {
					72
				} else {
					self.grid_size.clamp(16, 256)
				},
			},
		}
	}

	/// The folder entries in the header layout.
	///
	/// `Header` lists toolkit widgets (`toolkit/PathBar`) and folders. A folder
	/// entry docks a `FolderView` into the header; that is how the shell asks
	/// for an applet strip, and where it lives. The shell resolves the path,
	/// since only it knows the session root.
	pub fn header_folders(&self) -> impl Iterator<Item = &str> + '_ {
		self.header_layout
			.iter()
			.map(String::as_str)
			.filter(|entry| !entry.starts_with("toolkit/"))
			.filter(|entry| !entry.trim().is_empty())
	}

	pub fn load_for(location: &Location) -> ConfigLoad {
		let mut config = Self::default();
		let mut diagnostics = Vec::new();

		for path in config_paths(location) {
			let Ok(text) = std::fs::read_to_string(&path) else {
				continue;
			};
			let parsed = ConfigParser::parse_str(&text, &path.to_string_lossy());
			let (sections, errors) = parsed;
			for error in errors {
				diagnostics.push(ConfigDiagnostic {
					file: error
						.file
						.map(PathBuf::from)
						.unwrap_or_else(|| path.clone()),
					line: None,
					message: error.message,
				});
			}
			for (section, items) in sections {
				for (name, value) in items {
					let Some(key) = ConfigKey::from_section_name(&section, &name) else {
						continue;
					};
					if let Err(error) = config.set(key, from_parser(value)) {
						diagnostics.push(ConfigDiagnostic {
							file: path.clone(),
							line: None,
							message: error.message,
						});
					}
				}
			}
		}

		ConfigLoad {
			config,
			diagnostics,
		}
	}

	pub fn set_raw(
		&mut self,
		location: &Location,
		section: &str,
		name: &str,
		value: &str,
		local: bool,
	) -> Result<ConfigDelta, Error> {
		let target = if local {
			entry_file(location)
		} else {
			Some(user_config_path())
		};
		let target = target.ok_or(Error::Unsupported("no writable config file"))?;
		ConfigParser::set_value(&target, section, name, value);

		let key = ConfigKey::from_section_name(section, name)
			.ok_or(Error::Unsupported("unknown config key"))?;
		let (sections, _) =
			ConfigParser::parse_str(&format!("[{section}]\n{name} = {value}\n"), "<set_raw>");
		let stored = sections
			.get(section)
			.and_then(|items| items.get(name))
			.map(|value| from_parser(value.clone()))
			.unwrap_or_else(|| ConfigValue::Str(value.to_string()));

		self.set(key, stored)
			.map_err(|error| Error::Config(error.message))
	}
}

impl Default for Config {
	fn default() -> Self {
		let mut config = Self::blank();
		for key in ConfigKey::ALL {
			let _ = config.set(*key, key.default());
		}
		config
	}
}

/// The files that make up a location's config, lowest priority first.
pub fn config_paths(location: &Location) -> Vec<PathBuf> {
	let mut paths = system_config_paths();
	paths.push(user_config_path());
	if let Some(entry) = entry_file(location) {
		paths.push(entry);
	}
	paths
}

fn system_config_paths() -> Vec<PathBuf> {
	let dirs = std::env::var("XDG_CONFIG_DIRS")
		.ok()
		.filter(|value| !value.is_empty())
		.unwrap_or_else(|| String::from("/etc/xdg"));
	let mut paths: Vec<PathBuf> = dirs
		.split(':')
		.filter(|dir| !dir.is_empty())
		.map(|dir| Path::new(dir).join("hishell").join("folder.cfg"))
		.collect();
	paths.reverse();
	paths
}

pub fn user_config_path() -> PathBuf {
	dirs::config_dir()
		.unwrap_or_else(|| PathBuf::from("/etc/xdg"))
		.join("hishell")
		.join("folder.cfg")
}

/// The file that stores a location's own config.
pub fn entry_file(location: &Location) -> Option<PathBuf> {
	match location {
		Location::Folder(folder) => Some(folder.path().join(".directory")),
		Location::File(file) => {
			if file.path().is_file() {
				Some(file.path().to_path_buf())
			} else {
				None
			}
		}
	}
}

pub fn location_kind(location: &Location) -> LocationKind {
	location.kind()
}

fn from_parser(value: crate::config_parser::ConfigValue) -> ConfigValue {
	use crate::config_parser::ConfigValue as ParserValue;
	match value {
		ParserValue::String(value) => ConfigValue::Str(value),
		ParserValue::Number(value) => ConfigValue::Int(value as i64),
		ParserValue::Boolean(value) => ConfigValue::Bool(value),
		other => ConfigValue::Json(other.to_json_string()),
	}
}

fn string(key: ConfigKey, value: ConfigValue) -> Result<String, ConfigError> {
	match value {
		ConfigValue::Str(value) => Ok(value),
		_ => Err(invalid(key, "expected a string")),
	}
}

fn boolean(key: ConfigKey, value: ConfigValue) -> Result<bool, ConfigError> {
	match value {
		ConfigValue::Bool(value) => Ok(value),
		_ => Err(invalid(key, "expected true or false")),
	}
}

fn integer(key: ConfigKey, value: ConfigValue, default: i64) -> Result<i64, ConfigError> {
	match value {
		ConfigValue::Int(value) => Ok(value),
		ConfigValue::Str(value) if value.is_empty() => Ok(default),
		_ => Err(invalid(key, "expected a number")),
	}
}

fn parse<T>(
	key: ConfigKey,
	value: ConfigValue,
	parse: impl Fn(&str) -> Option<T>,
) -> Result<T, ConfigError> {
	match value {
		ConfigValue::Str(value) => parse(&value).ok_or_else(|| invalid(key, &value)),
		_ => Err(invalid(key, "expected a string")),
	}
}

fn json<T: serde::de::DeserializeOwned>(
	key: ConfigKey,
	value: ConfigValue,
) -> Result<T, ConfigError> {
	let text = match value {
		ConfigValue::Json(text) => text,
		ConfigValue::Str(text) => text,
		_ => return Err(invalid(key, "expected JSON")),
	};
	serde_json::from_str(&text).map_err(|error| ConfigError {
		key,
		message: error.to_string(),
	})
}

fn invalid(key: ConfigKey, value: &str) -> ConfigError {
	ConfigError {
		key,
		message: format!("invalid value {value:?}"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::location::Location;

	#[test]
	fn every_key_has_a_default_that_round_trips() {
		let config = Config::default();
		for key in ConfigKey::ALL {
			let value = config.get(*key);
			assert_eq!(value, key.default(), "{}", key.name());
		}
	}

	#[test]
	fn keys_report_their_section_and_scope() {
		assert_eq!(ConfigKey::GridMode.section(), "Folder View");
		assert_eq!(ConfigKey::GridMode.name(), "GridMode");
		assert_eq!(ConfigKey::GridMode.scope(), LocationScope::Folder);
		assert_eq!(ConfigKey::ExecutionWrapper.scope(), LocationScope::Both);
		assert_eq!(
			ConfigKey::from_section_name("Folder View", "Lines"),
			Some(ConfigKey::Lines)
		);
		assert_eq!(ConfigKey::from_section_name("Folder View", "Nope"), None);
	}

	#[test]
	fn the_drop_preference_parses_its_keys() {
		let mut config = Config::default();
		assert_eq!(config.drop_preference, DropPreference::Move);
		assert!(!config.drop_preference_mandate);

		for (text, preference) in [
			("COPY", DropPreference::Copy),
			("link", DropPreference::Link),
			("MOVE", DropPreference::Move),
		] {
			config
				.set(ConfigKey::DropItemPreference, ConfigValue::Str(text.into()))
				.unwrap();
			assert_eq!(config.drop_preference, preference);
			assert_eq!(
				config.get(ConfigKey::DropItemPreference),
				ConfigValue::Str(preference.as_key().into())
			);
		}

		config
			.set(ConfigKey::DropItemPreferenceMandate, ConfigValue::Bool(true))
			.unwrap();
		assert!(config.drop_preference_mandate);

		// `ASK` was in the old schema; the shell has no prompt for it yet, so
		// it is rejected rather than silently treated as MOVE.
		assert!(
			config
				.set(ConfigKey::DropItemPreference, ConfigValue::Str("ASK".into()))
				.is_err()
		);
	}

	#[test]
	fn setting_a_key_returns_a_delta() {
		let mut config = Config::default();
		let delta = config
			.set(ConfigKey::GridMode, ConfigValue::Str("SIZE".into()))
			.unwrap();
		assert_eq!(delta.key, ConfigKey::GridMode);
		assert_eq!(delta.old, ConfigValue::Str("LINES".into()));
		assert_eq!(delta.new, ConfigValue::Str("SIZE".into()));
		assert_eq!(config.grid_mode, GridMode::Size);
	}

	#[test]
	fn invalid_enum_values_are_rejected() {
		let mut config = Config::default();
		assert!(
			config
				.set(ConfigKey::GridMode, ConfigValue::Str("BANANAS".into()))
				.is_err()
		);
	}

	#[test]
	fn folder_config_overrides_user_config() {
		let root = std::env::temp_dir().join(format!("hishell-config-{}", std::process::id()));
		let folder = root.join("folder");
		let config_home = root.join("config");
		std::fs::create_dir_all(&folder).unwrap();
		std::fs::create_dir_all(config_home.join("hishell")).unwrap();
		std::fs::write(
			config_home.join("hishell/folder.cfg"),
			"[Folder View]\nLines = 2\nShowLabels = false\n",
		)
		.unwrap();
		std::fs::write(
			folder.join(".directory"),
			"[Folder View]\nLines = 6\nViewMode = LIST\n",
		)
		.unwrap();

		let previous = std::env::var("XDG_CONFIG_HOME").ok();
		unsafe { std::env::set_var("XDG_CONFIG_HOME", &config_home) };
		let load = Config::load_for(&Location::folder(&folder));
		match previous {
			Some(value) => unsafe { std::env::set_var("XDG_CONFIG_HOME", value) },
			None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
		}

		assert_eq!(load.config.lines, 6);
		assert!(!load.config.show_labels);
		assert_eq!(load.config.view_mode, ViewMode::List);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn header_folders_skip_the_toolkit_widgets() {
		let mut config = Config::default();
		config
			.set(
				ConfigKey::Header,
				ConfigValue::Json(
					r#"["toolkit/PathBar","~/.local/dock","toolkit/ViewButton"]"#.into(),
				),
			)
			.unwrap();
		assert_eq!(
			config.header_folders().collect::<Vec<_>>(),
			vec!["~/.local/dock"]
		);
	}
}

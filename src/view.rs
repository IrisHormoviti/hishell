//! View layout: how a folder or a single file is displayed.

use std::path::PathBuf;

use crate::config::Config;
use crate::item::ItemId;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
	pub width: f64,
	pub height: f64,
}

impl Size {
	pub fn new(width: f64, height: f64) -> Self {
		Self { width, height }
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
	pub x: f64,
	pub y: f64,
	pub width: f64,
	pub height: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewport {
	pub width: f64,
	pub height: f64,
	pub horizontal_scrollbar: f64,
	pub vertical_scrollbar: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
	pub grid_unit: f64,
	pub small_spacing: f64,
	pub medium_spacing: f64,
	pub large_spacing: f64,
	pub label_height: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct ViewRequest {
	pub viewport: Viewport,
	pub metrics: Metrics,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GridMode {
	#[default]
	Lines,
	Size,
}

impl GridMode {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::Lines => "LINES",
			Self::Size => "SIZE",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.to_ascii_uppercase().as_str() {
			"LINES" => Some(Self::Lines),
			"SIZE" => Some(Self::Size),
			_ => None,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HorizontalAlign {
	#[default]
	Fill,
	Left,
	Center,
	Right,
}

impl HorizontalAlign {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::Fill => "FILL",
			Self::Left => "LEFT",
			Self::Center => "CENTER",
			Self::Right => "RIGHT",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.to_ascii_uppercase().as_str() {
			"FILL" => Some(Self::Fill),
			"LEFT" => Some(Self::Left),
			"CENTER" => Some(Self::Center),
			"RIGHT" => Some(Self::Right),
			_ => None,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VerticalAlign {
	Fill,
	#[default]
	Top,
	Center,
	Bottom,
}

impl VerticalAlign {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::Fill => "FILL",
			Self::Top => "TOP",
			Self::Center => "CENTER",
			Self::Bottom => "BOTTOM",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.to_ascii_uppercase().as_str() {
			"FILL" => Some(Self::Fill),
			"TOP" => Some(Self::Top),
			"CENTER" => Some(Self::Center),
			"BOTTOM" => Some(Self::Bottom),
			_ => None,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ViewMode {
	#[default]
	Grid,
	List,
}

impl ViewMode {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::Grid => "GRID",
			Self::List => "LIST",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.to_ascii_uppercase().as_str() {
			"GRID" => Some(Self::Grid),
			"LIST" => Some(Self::List),
			_ => None,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LabelText {
	#[default]
	Title,
	Filename,
}

impl LabelText {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::Title => "TITLE",
			Self::Filename => "FILENAME",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.to_ascii_uppercase().as_str() {
			"TITLE" => Some(Self::Title),
			"FILENAME" => Some(Self::Filename),
			_ => None,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SortKey {
	#[default]
	Newest,
	Oldest,
	Alphabetical,
	Free,
}

impl SortKey {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::Newest => "NEWEST",
			Self::Oldest => "OLDEST",
			Self::Alphabetical => "ALPHABETICAL",
			Self::Free => "FREE",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.to_ascii_uppercase().as_str() {
			"NEWEST" => Some(Self::Newest),
			"OLDEST" => Some(Self::Oldest),
			"ALPHABETICAL" => Some(Self::Alphabetical),
			"FREE" => Some(Self::Free),
			_ => None,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SortDateMode {
	#[default]
	Modified,
	Created,
	Accessed,
}

impl SortDateMode {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::Modified => "MODIFIED",
			Self::Created => "CREATED",
			Self::Accessed => "ACCESSED",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.to_ascii_uppercase().as_str() {
			"MODIFIED" => Some(Self::Modified),
			"CREATED" => Some(Self::Created),
			"ACCESSED" => Some(Self::Accessed),
			_ => None,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SortAlphaMode {
	#[default]
	Titles,
	Filenames,
}

impl SortAlphaMode {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::Titles => "TITLES",
			Self::Filenames => "FILENAMES",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.to_ascii_uppercase().as_str() {
			"TITLES" => Some(Self::Titles),
			"FILENAMES" => Some(Self::Filenames),
			_ => None,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GroupMode {
	#[default]
	None,
	Date,
	Letters,
	Type,
	Custom,
}

impl GroupMode {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::None => "NONE",
			Self::Date => "DATE",
			Self::Letters => "LETTERS",
			Self::Type => "TYPE",
			Self::Custom => "CUSTOM",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.to_ascii_uppercase().as_str() {
			"NONE" => Some(Self::None),
			"DATE" => Some(Self::Date),
			"LETTERS" => Some(Self::Letters),
			"TYPE" => Some(Self::Type),
			"CUSTOM" => Some(Self::Custom),
			_ => None,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RadioOnly {
	#[default]
	All,
	Directories,
	Files,
}

impl RadioOnly {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::All => "ALL",
			Self::Directories => "DIRECTORIES",
			Self::Files => "FILES",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.to_ascii_uppercase().as_str() {
			"ALL" => Some(Self::All),
			"DIRECTORIES" => Some(Self::Directories),
			"FILES" => Some(Self::Files),
			_ => None,
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ListColumn {
	Title,
	Filename,
	Size,
	Modified,
	Created,
	Accessed,
	Type,
}

impl ListColumn {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::Title => "TITLE",
			Self::Filename => "FILENAME",
			Self::Size => "SIZE",
			Self::Modified => "MODIFIED",
			Self::Created => "CREATED",
			Self::Accessed => "ACCESSED",
			Self::Type => "TYPE",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.to_ascii_uppercase().as_str() {
			"TITLE" => Some(Self::Title),
			"FILENAME" => Some(Self::Filename),
			"SIZE" => Some(Self::Size),
			"MODIFIED" => Some(Self::Modified),
			"CREATED" => Some(Self::Created),
			"ACCESSED" => Some(Self::Accessed),
			"TYPE" => Some(Self::Type),
			_ => None,
		}
	}
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CustomGroup {
	pub name: String,
	pub files: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SortSpec {
	pub key: SortKey,
	pub date_mode: SortDateMode,
	pub alpha_mode: SortAlphaMode,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupingSpec {
	pub mode: GroupMode,
	pub groups: Vec<CustomGroup>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ListSpec {
	pub columns: Vec<ListColumn>,
	pub header: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InteractionSpec {
	pub open_on_click: bool,
	pub focus_empty_slots: bool,
	pub center_focus: bool,
	pub smooth_scrolling: bool,
}

impl Default for InteractionSpec {
	fn default() -> Self {
		Self {
			open_on_click: true,
			focus_empty_slots: false,
			center_focus: true,
			smooth_scrolling: true,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LabelSpec {
	pub show: bool,
	pub beside: bool,
	pub show_filename: bool,
	pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GridSizing {
	FixedLines { lines: u8 },
	FixedIcon { icon_size: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSpec {
	pub sizing: GridSizing,
	pub lines: u8,
	pub icon_size: f64,
	pub cell: Size,
	pub gap: f64,
	pub labels: LabelSpec,
	pub horizontal: bool,
}

#[derive(
	Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct PaneId(pub u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FocusSpec {
	pub pane: PaneId,
	pub item: Option<ItemId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotSpec {
	pub id: SlotId,
	pub item: ItemId,
	pub cell: (u32, u32),
	pub rect: Rect,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderSpec {
	pub title: String,
	pub item: Option<ItemId>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Preview {
	#[default]
	None,
	Image,
	Text,
	Thumbnail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataRow {
	pub label: String,
	pub value: String,
}

#[derive(Clone, Debug)]
pub enum FolderMode {
	Grid(GridSpec),
	List(ListSpec),
}

#[derive(Clone, Debug)]
pub struct FolderView {
	pub revision: u64,
	pub mode: FolderMode,
	pub sort: SortSpec,
	pub grouping: GroupingSpec,
	pub slots: Vec<SlotSpec>,
	pub headers: Vec<HeaderSpec>,
	pub focus: FocusSpec,
	pub interaction: InteractionSpec,
}

#[derive(Clone, Debug)]
pub struct FileView {
	pub revision: u64,
	pub item: ItemId,
	pub mime: String,
	pub thumbnail: Option<PathBuf>,
	pub metadata: Vec<MetadataRow>,
	pub preview: Preview,
	pub interaction: InteractionSpec,
}

#[derive(Clone, Debug)]
pub enum ViewSpec {
	Folder(FolderView),
	File(FileView),
}

/// Resolves the grid from the folder config and the measurements the frontend
/// supplies.
pub fn grid_spec(config: &Config, request: &ViewRequest) -> GridSpec {
	let gap = request.metrics.small_spacing;
	let small = request.metrics.small_spacing;
	let grid_unit = request.metrics.grid_unit;
	let horizontal = config.scroll_horizontal;
	let beside = config.show_labels && config.labels_beside_icons;
	let labels = LabelSpec {
		show: config.show_labels,
		beside,
		show_filename: config.label_text == LabelText::Filename,
		height: request.metrics.label_height.ceil() + small,
	};

	let bar = if horizontal {
		request.viewport.horizontal_scrollbar
	} else {
		request.viewport.vertical_scrollbar
	}
	.max(small);
	let main = ((if horizontal {
		request.viewport.height
	} else {
		request.viewport.width
	}) - bar
		- gap)
		.max(1.0);

	let padding = if beside || !config.show_labels {
		small * 2.0
	} else if horizontal {
		labels.height
	} else {
		small * 2.0
	};

	let (sizing, lines, icon_size) = match config.grid_mode {
		GridMode::Lines => {
			let lines = config.lines.clamp(1, 8) as f64;
			let cell = (main / lines - gap).min(main / 2.0 - gap);
			let icon = (cell - padding).clamp(16.0, 256.0);
			(
				GridSizing::FixedLines { lines: lines as u8 },
				lines as u8,
				icon,
			)
		}
		GridMode::Size => {
			let icon = if config.grid_size == 0 {
				72.0
			} else {
				(config.grid_size as f64).clamp(16.0, 256.0)
			};
			let pitch = icon + padding + gap;
			let lines = (((main + gap) / pitch).floor() as i64).clamp(1, 8) as u8;
			(
				GridSizing::FixedIcon {
					icon_size: icon as u16,
				},
				lines,
				icon,
			)
		}
	};

	let cell = if beside {
		Size::new(
			(grid_unit * 10.0).max(icon_size + grid_unit * 7.0 + grid_unit * 2.0),
			(grid_unit * 2.0).max(icon_size + small * 2.0),
		)
	} else if config.show_labels {
		Size::new(icon_size + small * 2.0, icon_size + labels.height)
	} else {
		Size::new(icon_size + small * 2.0, icon_size + small * 2.0)
	};

	GridSpec {
		sizing,
		lines,
		icon_size,
		cell,
		gap,
		labels,
		horizontal,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::config::{Config, ConfigKey, ConfigValue};

	fn request(width: f64, height: f64) -> ViewRequest {
		ViewRequest {
			viewport: Viewport {
				width,
				height,
				horizontal_scrollbar: 8.0,
				vertical_scrollbar: 8.0,
			},
			metrics: Metrics {
				grid_unit: 18.0,
				small_spacing: 4.0,
				medium_spacing: 6.0,
				large_spacing: 8.0,
				label_height: 16.0,
			},
		}
	}

	fn config(key: ConfigKey, value: ConfigValue) -> Config {
		let mut config = Config::default();
		config.set(key, value).unwrap();
		config
	}

	#[test]
	fn fixed_lines_keeps_the_line_count() {
		let config = config(ConfigKey::Lines, ConfigValue::Int(4));
		let spec = grid_spec(&config, &request(400.0, 300.0));
		assert_eq!(spec.lines, 4);
		assert_eq!(spec.sizing, GridSizing::FixedLines { lines: 4 });
		assert!(spec.icon_size >= 16.0 && spec.icon_size <= 256.0);
	}

	#[test]
	fn fixed_icon_derives_the_line_count() {
		let mut config = Config::default();
		config
			.set(ConfigKey::GridMode, ConfigValue::Str("SIZE".into()))
			.unwrap();
		config
			.set(ConfigKey::GridSize, ConfigValue::Int(64))
			.unwrap();
		let spec = grid_spec(&config, &request(400.0, 300.0));
		assert_eq!(spec.sizing, GridSizing::FixedIcon { icon_size: 64 });
		assert_eq!(spec.icon_size, 64.0);
		assert!(spec.lines >= 1);
	}

	#[test]
	fn grid_size_is_clamped() {
		let mut config = Config::default();
		config
			.set(ConfigKey::GridMode, ConfigValue::Str("SIZE".into()))
			.unwrap();
		config
			.set(ConfigKey::GridSize, ConfigValue::Int(4000))
			.unwrap();
		let spec = grid_spec(&config, &request(400.0, 300.0));
		assert_eq!(spec.icon_size, 256.0);

		config
			.set(ConfigKey::GridSize, ConfigValue::Int(0))
			.unwrap();
		let spec = grid_spec(&config, &request(400.0, 300.0));
		assert_eq!(spec.icon_size, 72.0);
	}

	#[test]
	fn a_tiny_viewport_keeps_the_line_count_and_a_minimum_cell() {
		let spec = grid_spec(&Config::default(), &request(10.0, 10.0));
		assert_eq!(spec.lines, 4);
		assert_eq!(spec.icon_size, 16.0);
	}
}

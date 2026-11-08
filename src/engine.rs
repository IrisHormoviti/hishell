//! The engine facade: state, configuration, and view layout.

use std::path::Path;

use crate::change::{ChangeSet, LocationDelta};
use crate::config::{Config, ConfigKey, ConfigLoad, ConfigValue};
use crate::error::Error;
use crate::folder::Folder;
use crate::item::{Item, ItemId};
use crate::location::Location;
use crate::view::{
	FocusSpec, FolderMode, FolderView, InteractionSpec, ListSpec, PaneId, Preview, Rect, SlotId,
	SlotSpec, ViewRequest, ViewSpec, grid_spec,
};

/// How much on-disk configuration the engine reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Mode {
	#[default]
	Config,
	Raw,
}

/// Host policy that survives raw mode.
#[derive(Clone, Debug, Default)]
pub struct RuntimeSettings {
	pub execution_wrapper: Option<String>,
	pub screen: Option<String>,
	pub default_layout: Vec<String>,
}

/// Services the engine depends on.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Deps {}

impl Deps {
	pub fn free_desktop() -> Self {
		Self {}
	}
}

#[derive(Clone, Debug, Default)]
pub struct OpenOptions {
	pub mode: Mode,
	pub deps: Deps,
	pub runtime: RuntimeSettings,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Command {
	SetConfig {
		key: ConfigKey,
		value: ConfigValue,
	},
	SetConfigRaw {
		section: String,
		key: String,
		value: String,
		local: bool,
	},
	Reload,
	SetMode(Mode),
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Effect {
	Reload(Vec<std::path::PathBuf>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JobHandle(pub u64);

#[derive(Clone, Debug, Default)]
pub struct Outcome {
	pub changes: ChangeSet,
	pub effects: Vec<Effect>,
	pub job: Option<JobHandle>,
}

#[derive(Clone, Debug, Default)]
pub struct TickResult {
	pub changes: ChangeSet,
	pub effects: Vec<Effect>,
}

pub struct Engine {
	mode: Mode,
	location: Location,
	config: Config,
	runtime: RuntimeSettings,
	folder: Option<Folder>,
	diagnostics: Vec<crate::config::ConfigDiagnostic>,
	revision: u64,
}

impl Engine {
	pub fn open(path: &Path) -> Result<Self, Error> {
		if !path.exists() {
			return Err(Error::NotFound(path.to_path_buf()));
		}
		if !path.is_dir() {
			return Err(Error::NotAFolder(path.to_path_buf()));
		}
		Self::open_with(path, OpenOptions::default())
	}

	pub fn open_with(path: &Path, options: OpenOptions) -> Result<Self, Error> {
		Self::open_location_with(Location::folder(path), options)
	}

	pub fn open_location(location: Location) -> Result<Self, Error> {
		Self::open_location_with(location, OpenOptions::default())
	}

	pub fn open_location_with(location: Location, options: OpenOptions) -> Result<Self, Error> {
		if !location.path().exists() {
			return Err(Error::NotFound(location.path().to_path_buf()));
		}
		let mut engine = Self {
			mode: options.mode,
			location,
			config: Config::default(),
			runtime: options.runtime,
			folder: None,
			diagnostics: Vec::new(),
			revision: 0,
		};
		engine.reload();
		Ok(engine)
	}

	pub fn mode(&self) -> Mode {
		self.mode
	}

	pub fn location(&self) -> &Location {
		&self.location
	}

	pub fn config(&self) -> &Config {
		&self.config
	}

	pub fn items(&self) -> &[Item] {
		self.folder
			.as_ref()
			.map(|folder| folder.items())
			.unwrap_or_default()
	}

	pub fn item(&self, id: crate::item::ItemId) -> Option<&Item> {
		self.folder.as_ref().and_then(|folder| folder.item(id))
	}

	pub fn item_at(&self, path: &Path) -> Option<&Item> {
		self.folder.as_ref().and_then(|folder| folder.item_at(path))
	}

	pub fn diagnostics(&self) -> &[crate::config::ConfigDiagnostic] {
		&self.diagnostics
	}

	pub fn view(&self, request: ViewRequest) -> ViewSpec {
		let interaction = InteractionSpec {
			open_on_click: self.config.open_on_click,
			focus_empty_slots: self.config.focus_empty_slots,
			center_focus: self.config.center_focus,
			smooth_scrolling: self.config.smooth_scrolling,
		};
		match &self.location {
			Location::Folder(_) => {
				let mode = self.folder_mode(&request);
				let slots = self.slots(&request, &mode);
				ViewSpec::Folder(FolderView {
					revision: self.revision,
					mode,
					sort: crate::view::SortSpec {
						key: self.config.sort,
						date_mode: self.config.sort_date_mode,
						alpha_mode: self.config.sort_alpha_mode,
					},
					grouping: crate::view::GroupingSpec {
						mode: self.config.group_mode,
						groups: self.config.groups.clone(),
					},
					slots,
					headers: Vec::new(),
					focus: FocusSpec {
						pane: PaneId(0),
						item: None,
					},
					interaction,
				})
			}
			Location::File(file) => ViewSpec::File(crate::view::FileView {
				revision: self.revision,
				item: ItemId::new(0),
				mime: mime_for(file.path()),
				thumbnail: None,
				metadata: Vec::new(),
				preview: Preview::None,
				interaction,
			}),
		}
	}

	pub fn set_mode(&mut self, mode: Mode) -> Result<ChangeSet, Error> {
		if self.mode == mode {
			return Ok(ChangeSet::default());
		}
		self.mode = mode;
		Ok(self.reload())
	}

	pub fn dispatch(&mut self, command: Command) -> Result<Outcome, Error> {
		let changes = match command {
			Command::SetConfig { key, value } => {
				let delta = self
					.config
					.set(key, value)
					.map_err(|error| Error::Config(error.message))?;
				self.revision += 1;
				ChangeSet {
					config: vec![delta],
					..ChangeSet::default()
				}
			}
			Command::SetConfigRaw {
				section,
				key,
				value,
				local,
			} => {
				let delta = self
					.config
					.set_raw(&self.location, &section, &key, &value, local)?;
				self.revision += 1;
				ChangeSet {
					config: vec![delta],
					..ChangeSet::default()
				}
			}
			Command::Reload => self.reload(),
			Command::SetMode(mode) => {
				self.mode = mode;
				self.reload()
			}
		};
		Ok(Outcome {
			changes,
			effects: Vec::new(),
			job: None,
		})
	}

	pub fn tick(&mut self) -> TickResult {
		TickResult::default()
	}

	fn folder_mode(&self, request: &ViewRequest) -> FolderMode {
		match self.config.view_mode {
			crate::view::ViewMode::Grid => FolderMode::Grid(grid_spec(&self.config, request)),
			crate::view::ViewMode::List => FolderMode::List(ListSpec {
				columns: self.config.list_columns.clone(),
				header: self.config.list_header,
			}),
		}
	}

	fn slots(&self, request: &ViewRequest, mode: &FolderMode) -> Vec<SlotSpec> {
		let Some(folder) = &self.folder else {
			return Vec::new();
		};
		let items = folder.items();
		match mode {
			FolderMode::Grid(grid) => {
				if items.is_empty() {
					return Vec::new();
				}
				let lines = grid.lines.max(1) as usize;
				let columns = if grid.horizontal {
					items.len().div_ceil(lines)
				} else {
					lines
				};
				items
					.iter()
					.enumerate()
					.map(|(index, item)| {
						let (column, row) = if grid.horizontal {
							(index / lines, index % lines)
						} else {
							(index % columns, index / columns)
						};
						SlotSpec {
							id: SlotId(item.id.get()),
							item: item.id,
							cell: (column as u32, row as u32),
							rect: Rect {
								x: column as f64 * (grid.cell.width + grid.gap),
								y: row as f64 * (grid.cell.height + grid.gap),
								width: grid.cell.width,
								height: grid.cell.height,
							},
						}
					})
					.collect()
			}
			FolderMode::List(_) => {
				let height = request.metrics.label_height.max(16.0) + 12.0;
				items
					.iter()
					.enumerate()
					.map(|(index, item)| SlotSpec {
						id: SlotId(item.id.get()),
						item: item.id,
						cell: (0, index as u32),
						rect: Rect {
							x: 0.0,
							y: index as f64 * height,
							width: request.viewport.width,
							height,
						},
					})
					.collect()
			}
		}
	}

	fn reload(&mut self) -> ChangeSet {
		let diagnostics;
		if self.mode == Mode::Raw {
			self.config = raw_config(&self.runtime);
			diagnostics = Vec::new();
		} else {
			let ConfigLoad {
				config,
				diagnostics: load_diagnostics,
			} = Config::load_for(&self.location);
			self.config = config;
			diagnostics = load_diagnostics;
			self.apply_runtime();
		}
		self.diagnostics = diagnostics;
		if let Location::Folder(folder) = &self.location {
			let mut scanned = Folder::scan(folder.path());
			scanned.sort(self.config.sort);
			self.folder = Some(scanned);
		} else {
			self.folder = None;
		}
		self.revision += 1;
		ChangeSet {
			location: Some(LocationDelta {
				kind: Some(self.location.kind()),
				path: Some(self.location.path().to_path_buf()),
			}),
			..ChangeSet::default()
		}
	}

	fn apply_runtime(&mut self) {
		if self.config.execution_wrapper.is_empty()
			&& let Some(wrapper) = &self.runtime.execution_wrapper
		{
			self.config.execution_wrapper = wrapper.clone();
		}
		if self.config.screen.is_empty()
			&& let Some(screen) = &self.runtime.screen
		{
			self.config.screen = screen.clone();
		}
	}
}

fn raw_config(runtime: &RuntimeSettings) -> Config {
	let middle_layout = if runtime.default_layout.is_empty() {
		vec![String::from("./")]
	} else {
		runtime.default_layout.clone()
	};
	Config {
		middle_layout,
		execution_wrapper: runtime.execution_wrapper.clone().unwrap_or_default(),
		screen: runtime.screen.clone().unwrap_or_default(),
		..Config::default()
	}
}

fn mime_for(path: &Path) -> String {
	match path.extension().and_then(|value| value.to_str()) {
		Some(extension) => mime_guess::from_ext(extension)
			.first_or_octet_stream()
			.to_string(),
		None => String::from("application/octet-stream"),
	}
}

impl std::fmt::Debug for Engine {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Engine")
			.field("mode", &self.mode)
			.field("location", &self.location)
			.field("kind", &self.location.kind())
			.field("revision", &self.revision)
			.finish()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::config::ConfigKey;

	fn temp_folder(name: &str) -> std::path::PathBuf {
		let path =
			std::env::temp_dir().join(format!("hishell-engine-{name}-{}", std::process::id()));
		std::fs::create_dir_all(&path).unwrap();
		path
	}

	fn request() -> ViewRequest {
		ViewRequest {
			viewport: crate::view::Viewport {
				width: 400.0,
				height: 300.0,
				horizontal_scrollbar: 8.0,
				vertical_scrollbar: 8.0,
			},
			metrics: crate::view::Metrics {
				grid_unit: 18.0,
				small_spacing: 4.0,
				medium_spacing: 6.0,
				large_spacing: 8.0,
				label_height: 16.0,
			},
		}
	}

	#[test]
	fn two_engines_run_in_one_process() {
		let first = temp_folder("first");
		let second = temp_folder("second");
		let mut engine_a = Engine::open(&first).unwrap();
		let mut engine_b = Engine::open(&second).unwrap();

		engine_a
			.dispatch(Command::SetConfig {
				key: ConfigKey::Lines,
				value: ConfigValue::Int(2),
			})
			.unwrap();
		engine_b
			.dispatch(Command::SetConfig {
				key: ConfigKey::Lines,
				value: ConfigValue::Int(7),
			})
			.unwrap();

		assert_eq!(engine_a.config().lines, 2);
		assert_eq!(engine_b.config().lines, 7);
		std::fs::remove_dir_all(first).unwrap();
		std::fs::remove_dir_all(second).unwrap();
	}

	#[test]
	fn raw_mode_ignores_folder_files() {
		let path = temp_folder("raw");
		std::fs::write(
			path.join(".directory"),
			"[Folder View]\nLines = 7\nViewMode = LIST\n",
		)
		.unwrap();
		let engine = Engine::open_location_with(
			Location::folder(&path),
			OpenOptions {
				mode: Mode::Raw,
				..OpenOptions::default()
			},
		)
		.unwrap();
		assert_eq!(engine.config().lines, 4);
		assert_eq!(engine.config().view_mode, crate::view::ViewMode::Grid);
		assert_eq!(engine.config().middle_layout, vec![String::from("./")]);
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn file_location_has_a_file_view() {
		let path = temp_folder("file").join("photo.png");
		std::fs::write(&path, b"not really a png").unwrap();
		let engine = Engine::open_location(Location::file(&path)).unwrap();
		match engine.view(request()) {
			ViewSpec::File(view) => assert_eq!(view.mime, "image/png"),
			ViewSpec::Folder(_) => panic!("expected a file view"),
		}
		std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
	}

	#[test]
	fn folder_scan_lists_directories_first() {
		let path = temp_folder("scan");
		std::fs::write(path.join("b.txt"), b"x").unwrap();
		std::fs::create_dir_all(path.join("a-dir")).unwrap();
		let engine = Engine::open(&path).unwrap();
		let items = engine.items();
		assert_eq!(items.len(), 2);
		assert_eq!(items[0].name, "a-dir");
		assert!(engine.item_at(&path.join("b.txt")).is_some());
		assert!(engine.item(items[0].id).is_some());
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn view_places_every_item_in_a_slot() {
		let path = temp_folder("slots");
		for index in 0..5 {
			std::fs::write(path.join(format!("f{index}")), b"x").unwrap();
		}
		let engine = Engine::open(&path).unwrap();
		match engine.view(request()) {
			ViewSpec::Folder(view) => {
				assert_eq!(view.slots.len(), 5);
				match view.mode {
					FolderMode::Grid(grid) => {
						assert_eq!(grid.lines, 4);
						let rows = view.slots.iter().map(|slot| slot.cell.1).max().unwrap();
						assert_eq!(rows, 1);
					}
					FolderMode::List(_) => panic!("expected a grid"),
				}
			}
			ViewSpec::File(_) => panic!("expected a folder view"),
		}
		std::fs::remove_dir_all(path).unwrap();
	}
}

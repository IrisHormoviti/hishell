//! The engine facade: state, configuration, and view layout.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::change::{ChangeScope, ChangeSet, FolderDelta, LocationDelta, SelectionDelta};
use crate::config::{Config, ConfigKey, ConfigLoad, ConfigValue};
use crate::error::Error;
use crate::folder::{Folder, ScanOptions};
use crate::item::{Item, ItemId};
use crate::jobs::JobHandle;
use crate::location::Location;
use crate::navigation::{NavigationPlan, NavigationRequest, PathRelation, SlotAnchor, relation};
use crate::selection::Selection;
use crate::view::{
	FocusSpec, FolderMode, FolderView, InteractionSpec, ListSpec, PaneId, Preview, ViewRequest,
	ViewSpec, grid_spec,
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
	SelectOnly(ItemId),
	ToggleSelection(ItemId),
	SelectRange(ItemId),
	SelectAll,
	ClearSelection,
	SetSelectionActive(bool),
	SetFreePosition {
		name: String,
		column: i32,
		row: i32,
	},
	SetFreePositions {
		positions: Vec<(String, i32, i32)>,
	},
	ResetFreePositions,
	RenameFreePosition {
		from: String,
		to: String,
	},
	Navigate(NavigationRequest),
	GoUp,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Effect {
	Reload(Vec<PathBuf>),
	Navigate(NavigationPlan),
}

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
	config_diagnostics: Vec<crate::config::ConfigDiagnostic>,
	revision: u64,
	last_view: RefCell<Option<FolderView>>,
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
			config_diagnostics: Vec::new(),
			revision: 0,
			last_view: RefCell::new(None),
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

	pub fn selection(&self) -> Option<&Selection> {
		self.folder.as_ref().map(|folder| folder.selection())
	}

	pub fn relation(&self, target: &Path) -> PathRelation {
		relation(&self.folder_path(), target)
	}

	pub fn navigation_plan(&self, request: &NavigationRequest) -> NavigationPlan {
		crate::navigation::plan(request, &self.folder_path(), self.config.smooth_scrolling)
	}

	/// The slot of `path` in the last view the frontend asked for, when it is
	/// visible there.
	pub fn anchor_for(&self, pane: PaneId, path: &Path) -> Option<SlotAnchor> {
		let view = self.last_view.borrow();
		let view = view.as_ref()?;
		if view.focus.pane != pane {
			return None;
		}
		let item = self.item_at(path)?;
		let slot = view.slots.iter().find(|slot| slot.item == item.id)?;
		Some(SlotAnchor {
			item: item.id,
			slot: slot.id,
			visible: true,
		})
	}

	/// The nearest ancestor of `target` that the current view shows.
	pub fn closest_visible_parent(&self, pane: PaneId, target: &Path) -> Option<SlotAnchor> {
		let mut current = target.parent();
		while let Some(path) = current {
			if let Some(anchor) = self.anchor_for(pane, path) {
				return Some(anchor);
			}
			current = path.parent();
		}
		None
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
				let items = self.folder.as_ref().map(Folder::items).unwrap_or_default();
				let layout = crate::layout::folder(&self.config, &request, &mode, items);
				let view = FolderView {
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
					slots: layout.slots,
					headers: layout.headers,
					focus: FocusSpec {
						pane: PaneId(0),
						item: None,
					},
					interaction,
				};
				*self.last_view.borrow_mut() = Some(view.clone());
				ViewSpec::Folder(view)
			}
			Location::File(file) => {
				*self.last_view.borrow_mut() = None;
				ViewSpec::File(crate::view::FileView {
					revision: self.revision,
					item: ItemId::new(0),
					mime: mime_for(file.path()),
					thumbnail: None,
					metadata: Vec::new(),
					preview: Preview::None,
					interaction,
				})
			}
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
				let folder = self.apply_config_change(delta.scope);
				self.revision += 1;
				ChangeSet {
					config: vec![delta],
					folder,
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
				let folder = self.apply_config_change(delta.scope);
				self.revision += 1;
				ChangeSet {
					config: vec![delta],
					folder,
					..ChangeSet::default()
				}
			}
			Command::Reload => self.reload(),
			Command::SetMode(mode) => {
				self.mode = mode;
				self.reload()
			}
			Command::SelectOnly(id) => {
				self.selection_changes(|selection, _| selection.select_only(id))
			}
			Command::ToggleSelection(id) => {
				self.selection_changes(|selection, _| selection.toggle(id))
			}
			Command::SelectRange(id) => {
				self.selection_changes(|selection, order| selection.range_to(order, id))
			}
			Command::SelectAll => {
				self.selection_changes(|selection, order| selection.select_all(order))
			}
			Command::ClearSelection => self.selection_changes(|selection, _| selection.clear()),
			Command::SetSelectionActive(active) => {
				self.selection_changes(|selection, _| selection.set_active(active))
			}
			Command::SetFreePosition { name, column, row } => {
				let mut positions = self.config.free_placement_positions.clone();
				positions.insert(name, (column, row));
				self.free_positions_changes(positions)?
			}
			Command::SetFreePositions { positions } => {
				let mut merged = self.config.free_placement_positions.clone();
				for (name, column, row) in &positions {
					merged.retain(|existing, value| {
						existing == name || value.0 != *column || value.1 != *row
					});
				}
				for (name, column, row) in positions {
					merged.insert(name, (column, row));
				}
				self.free_positions_changes(merged)?
			}
			Command::ResetFreePositions => self.free_positions_changes(HashMap::new())?,
			Command::RenameFreePosition { from, to } => {
				let mut positions = self.config.free_placement_positions.clone();
				if let Some(position) = positions.remove(&from) {
					positions.insert(to, position);
					self.free_positions_changes(positions)?
				} else {
					ChangeSet::default()
				}
			}
			Command::Navigate(request) => {
				let plan = self.navigation_plan(&request);
				return Ok(self.navigate(plan));
			}
			Command::GoUp => {
				let request = NavigationRequest {
					pane: PaneId(0),
					target: self
						.folder_path()
						.parent()
						.map(Path::to_path_buf)
						.unwrap_or_else(|| PathBuf::from("/")),
					record_history: true,
				};
				let plan = self.navigation_plan(&request);
				return Ok(self.navigate(plan));
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

	/// Whether the current folder's `[Radio]` group accepts `path` as a choice.
	pub fn is_radio_member(&self, path: &Path) -> bool {
		self.folder
			.as_ref()
			.is_some_and(|folder| folder.is_radio_member(path, &self.config))
	}

	fn folder_path(&self) -> PathBuf {
		match &self.location {
			Location::Folder(folder) => folder.path().to_path_buf(),
			Location::File(file) => file.parent().to_path_buf(),
		}
	}

	fn navigate(&mut self, plan: NavigationPlan) -> Outcome {
		self.location = Location::folder(plan.to.clone());
		self.folder = None;
		let changes = self.reload();
		Outcome {
			changes,
			effects: vec![Effect::Navigate(plan)],
			job: None,
		}
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

	fn selection_changes(&mut self, change: impl FnOnce(&mut Selection, &[ItemId])) -> ChangeSet {
		let Some(folder) = self.folder.as_mut() else {
			return ChangeSet::default();
		};
		let order = folder.order().to_vec();
		change(folder.selection_mut(), &order);
		let selection = folder.selection();
		ChangeSet {
			selection: Some(SelectionDelta {
				active: selection.active(),
				selected: selection.items().to_vec(),
			}),
			..ChangeSet::default()
		}
	}

	fn reload(&mut self) -> ChangeSet {
		if self.mode == Mode::Raw {
			self.config = raw_config(&self.runtime);
			self.config_diagnostics.clear();
		} else {
			let ConfigLoad {
				config,
				diagnostics,
			} = Config::load_for(&self.location);
			self.config = config;
			self.config_diagnostics = diagnostics;
			self.apply_runtime();
		}
		self.rescan();
		self.rebuild_diagnostics();
		self.revision += 1;
		ChangeSet {
			folder: FolderDelta::Reset,
			location: Some(LocationDelta {
				kind: Some(self.location.kind()),
				path: Some(self.location.path().to_path_buf()),
			}),
			..ChangeSet::default()
		}
	}

	fn apply_config_change(&mut self, scope: ChangeScope) -> FolderDelta {
		match scope {
			ChangeScope::Order => {
				if let Some(folder) = self.folder.as_mut() {
					let config = &self.config;
					folder.apply_config(config);
				}
				self.order_delta()
			}
			ChangeScope::Items => {
				self.rescan();
				self.rebuild_diagnostics();
				FolderDelta::Reset
			}
			ChangeScope::View | ChangeScope::All => self.order_delta(),
		}
	}

	fn rescan(&mut self) {
		let previous_selection = self
			.folder
			.as_ref()
			.map(|folder| folder.selection().clone());
		if let Location::Folder(folder) = &self.location {
			let include_hidden = !self.config.stash_dotfiles || self.config.show_stash;
			let mut scanned = Folder::scan_with(
				folder.path(),
				ScanOptions {
					read_entries: self.mode == Mode::Config,
					include_hidden,
				},
			);
			scanned.apply_config(&self.config);
			if let Some(selection) = previous_selection {
				*scanned.selection_mut() = selection;
			}
			scanned.retain_selection();
			self.folder = Some(scanned);
		} else {
			self.folder = None;
		}
	}

	fn rebuild_diagnostics(&mut self) {
		self.diagnostics = self.config_diagnostics.clone();
		if let Some(folder) = &self.folder {
			self.diagnostics
				.extend(folder.diagnostics().iter().cloned());
		}
	}

	fn order_delta(&self) -> FolderDelta {
		match &self.folder {
			Some(folder) => FolderDelta::Items {
				added: Vec::new(),
				removed: Vec::new(),
				updated: Vec::new(),
				order: folder.order().to_vec(),
			},
			None => FolderDelta::Reset,
		}
	}

	fn free_positions_changes(
		&mut self,
		positions: HashMap<String, (i32, i32)>,
	) -> Result<ChangeSet, Error> {
		let json = serde_json::to_string(&positions).unwrap_or_else(|_| String::from("{}"));
		let delta = if self.mode == Mode::Config {
			self.config.set_raw(
				&self.location,
				"Folder View",
				"FreePlacementPositions",
				&json,
				true,
			)?
		} else {
			self.config
				.set(ConfigKey::FreePlacementPositions, ConfigValue::Json(json))
				.map_err(|error| Error::Config(error.message))?
		};
		self.apply_config_change(delta.scope);
		self.revision += 1;
		Ok(ChangeSet {
			config: vec![delta],
			folder: self.order_delta(),
			..ChangeSet::default()
		})
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

	trait FolderSlots {
		fn folder_slots(self) -> Vec<crate::view::SlotSpec>;
	}

	impl FolderSlots for ViewSpec {
		fn folder_slots(self) -> Vec<crate::view::SlotSpec> {
			match self {
				ViewSpec::Folder(view) => view.slots,
				ViewSpec::File(_) => Vec::new(),
			}
		}
	}

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
	fn selection_survives_a_reload() {
		let path = temp_folder("selection");
		std::fs::write(path.join("a.txt"), b"x").unwrap();
		std::fs::write(path.join("b.txt"), b"x").unwrap();
		let mut engine = Engine::open(&path).unwrap();
		let id = engine.items()[0].id;
		engine.dispatch(Command::SelectOnly(id)).unwrap();
		engine.dispatch(Command::Reload).unwrap();
		assert!(engine.selection().unwrap().contains(id));
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn selection_commands_return_deltas() {
		let path = temp_folder("selection-delta");
		for index in 0..3 {
			std::fs::write(path.join(format!("f{index}")), b"x").unwrap();
		}
		let mut engine = Engine::open(&path).unwrap();
		let id = engine.items()[1].id;
		let outcome = engine.dispatch(Command::ToggleSelection(id)).unwrap();
		let delta = outcome.changes.selection.unwrap();
		assert!(delta.active);
		assert_eq!(delta.selected, vec![id]);
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn desktop_entries_apply_their_metadata() {
		let path = temp_folder("entry");
		std::fs::write(
			path.join("ride.desktop"),
			"[Desktop Entry]\nType=Application\nName=Ride\nIcon=ride\n",
		)
		.unwrap();
		let engine = Engine::open(&path).unwrap();
		let item = engine
			.items()
			.iter()
			.find(|item| item.name == "ride.desktop")
			.unwrap();
		assert_eq!(item.title, "Ride");
		assert_eq!(item.kind, crate::item::ItemKind::Application);
		assert_eq!(item.icon, crate::item::IconRef::Theme(String::from("ride")));
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn config_change_updates_the_view() {
		let path = temp_folder("config-view");
		for index in 0..5 {
			std::fs::write(path.join(format!("f{index}")), b"x").unwrap();
		}
		let mut engine = Engine::open(&path).unwrap();
		engine
			.dispatch(Command::SetConfig {
				key: ConfigKey::Lines,
				value: ConfigValue::Int(2),
			})
			.unwrap();
		match engine.view(request()) {
			ViewSpec::Folder(view) => match view.mode {
				FolderMode::Grid(grid) => assert_eq!(grid.lines, 2),
				FolderMode::List(_) => panic!("expected a grid"),
			},
			ViewSpec::File(_) => panic!("expected a folder view"),
		}
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
	fn a_folder_entry_can_make_the_folder_an_application() {
		let path = temp_folder("folder-entry");
		let folder = path.join("ride");
		std::fs::create_dir_all(&folder).unwrap();
		std::fs::write(
			folder.join(".directory"),
			"[Desktop Entry]\nType=Application\nName=Ride\n",
		)
		.unwrap();
		let engine = Engine::open(&path).unwrap();
		let item = engine.item_at(&folder).unwrap();
		assert_eq!(item.title, "Ride");
		assert_eq!(item.kind, crate::item::ItemKind::Application);
		assert_eq!(item.fs_kind, crate::item::FsKind::Directory);
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn file_descriptors_attach_their_metadata_to_the_target() {
		let path = temp_folder("descriptor");
		let file = path.join("game.rom");
		std::fs::write(&file, b"rom").unwrap();
		std::fs::write(
			path.join("game.desktop"),
			format!(
				"[Desktop Entry]\nType=File\nPath={}\nName=Test Game\nComment=Fun\n",
				file.display()
			),
		)
		.unwrap();
		let engine = Engine::open(&path).unwrap();
		assert_eq!(engine.items().len(), 1);
		let item = engine.item_at(&file).unwrap();
		assert_eq!(item.title, "Test Game");
		assert_eq!(item.comment, "Fun");
		assert_eq!(
			item.entry_path.as_deref(),
			Some(path.join("game.desktop").as_path())
		);
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn sidecars_attach_to_the_file_beside_them() {
		let path = temp_folder("sidecar");
		let file = path.join("photo.png");
		std::fs::write(&file, b"png").unwrap();
		std::fs::write(
			path.join("photo.png.desktop"),
			"[Desktop Entry]\nType=File\nName=Holiday\n",
		)
		.unwrap();
		let engine = Engine::open(&path).unwrap();
		assert_eq!(engine.items().len(), 1);
		assert_eq!(engine.item_at(&file).unwrap().title, "Holiday");
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn raw_mode_ignores_entries() {
		let path = temp_folder("raw-entry");
		let folder = path.join("ride");
		std::fs::create_dir_all(&folder).unwrap();
		std::fs::write(
			folder.join(".directory"),
			"[Desktop Entry]\nType=Application\nName=Ride\n",
		)
		.unwrap();
		std::fs::write(
			path.join("app.desktop"),
			"[Desktop Entry]\nType=Application\nName=App\n",
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
		assert_eq!(
			engine.item_at(&folder).unwrap().kind,
			crate::item::ItemKind::Directory
		);
		let desktop = engine.item_at(&path.join("app.desktop")).unwrap();
		assert_eq!(desktop.kind, crate::item::ItemKind::File);
		assert_eq!(desktop.title, "app.desktop");
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn grouping_reorders_items_and_labels_them() {
		let path = temp_folder("group");
		std::fs::write(path.join("a.png"), b"x").unwrap();
		std::fs::write(path.join("b.zip"), b"x").unwrap();
		std::fs::write(path.join("c.txt"), b"x").unwrap();
		let mut engine = Engine::open(&path).unwrap();
		engine
			.dispatch(Command::SetConfig {
				key: ConfigKey::Group,
				value: ConfigValue::Str("TYPE".into()),
			})
			.unwrap();
		let labels: Vec<&str> = engine
			.items()
			.iter()
			.map(|item| item.group_label.as_str())
			.collect();
		assert_eq!(labels, ["Images", "Documents", "Archives"]);
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn stashed_dotfiles_appear_only_when_requested() {
		let path = temp_folder("stash");
		std::fs::write(path.join("visible.txt"), b"x").unwrap();
		std::fs::write(path.join(".hidden"), b"x").unwrap();
		let mut engine = Engine::open(&path).unwrap();
		assert_eq!(engine.items().len(), 1);
		engine
			.dispatch(Command::SetConfig {
				key: ConfigKey::ShowStash,
				value: ConfigValue::Bool(true),
			})
			.unwrap();
		assert_eq!(engine.items().len(), 2);
		let stashed = engine.items().iter().find(|item| item.is_stashed).unwrap();
		assert_eq!(stashed.group_label, "Stash");
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn free_positions_place_items_in_their_cell() {
		let path = temp_folder("free");
		std::fs::write(path.join("a"), b"x").unwrap();
		std::fs::write(path.join("b"), b"x").unwrap();
		let mut engine = Engine::open(&path).unwrap();
		engine
			.dispatch(Command::SetConfig {
				key: ConfigKey::Sort,
				value: ConfigValue::Str("FREE".into()),
			})
			.unwrap();
		engine
			.dispatch(Command::SetFreePosition {
				name: String::from("b"),
				column: 1,
				row: 0,
			})
			.unwrap();
		let placed = engine
			.view(request())
			.folder_slots()
			.into_iter()
			.find(|slot| engine.item(slot.item).is_some_and(|item| item.name == "b"))
			.unwrap();
		assert_eq!(placed.cell, (1, 0));
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn radio_membership_follows_the_folder_config() {
		let path = temp_folder("radio");
		std::fs::write(path.join("one"), b"x").unwrap();
		let other = path.join("folder");
		std::fs::create_dir_all(&other).unwrap();
		let mut engine = Engine::open(&path).unwrap();
		engine
			.dispatch(Command::SetConfig {
				key: ConfigKey::RadioEnabled,
				value: ConfigValue::Bool(true),
			})
			.unwrap();
		engine
			.dispatch(Command::SetConfig {
				key: ConfigKey::RadioOnly,
				value: ConfigValue::Str("FILES".into()),
			})
			.unwrap();
		assert!(engine.is_radio_member(&path.join("one")));
		assert!(!engine.is_radio_member(&other));
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn navigation_changes_location_and_returns_a_plan() {
		let path = temp_folder("navigate");
		let child = path.join("child");
		std::fs::create_dir_all(&child).unwrap();
		let mut engine = Engine::open(&path).unwrap();
		let outcome = engine
			.dispatch(Command::Navigate(crate::navigation::NavigationRequest {
				pane: PaneId(0),
				target: child.clone(),
				record_history: true,
			}))
			.unwrap();
		assert_eq!(engine.location().path(), child);
		match &outcome.effects[0] {
			Effect::Navigate(plan) => {
				assert_eq!(plan.to, child);
				assert_eq!(plan.from, path);
			}
			Effect::Reload(_) => panic!("expected a navigation plan"),
		}
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn anchors_resolve_against_the_last_view() {
		let path = temp_folder("anchor");
		let child = path.join("child");
		std::fs::create_dir_all(&child).unwrap();
		let nested = child.join("nested");
		std::fs::create_dir_all(&nested).unwrap();
		let engine = Engine::open(&path).unwrap();
		let _ = engine.view(request());
		assert!(engine.anchor_for(PaneId(0), &child).is_some());
		assert!(engine.anchor_for(PaneId(0), &nested).is_none());
		assert!(engine.closest_visible_parent(PaneId(0), &nested).is_some());
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

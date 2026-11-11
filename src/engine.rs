//! The engine facade: state, configuration, and view layout.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::action::{
	ActionDescriptor, ActionId, ActionRequest, ActionSpec, ActionSubjects, Target, descriptor,
	menu_contains,
};
use crate::change::{
	ChangeScope, ChangeSet, FocusDelta, FolderDelta, LocationDelta, SelectionDelta,
};
use crate::config::{Config, ConfigKey, ConfigLoad, ConfigValue};
use crate::error::Error;
use crate::execution::{
	Execution, ExecutionWrapper, LaunchEvent, LaunchId, LaunchOutcome, LaunchRequest, Spawner,
	SystemClock,
};
use crate::focus::{Focus, FocusCommand};
use crate::folder::{Folder, ScanOptions};
use crate::history::{History, HistoryCommand, HistoryEntry, HistoryStatus};
use crate::item::{Item, ItemId};
use crate::jobs::{JobEvent, JobHandle, JobId, Jobs};
use crate::progress::{ProgressAction, ProgressControl};
use crate::location::Location;
use crate::navigation::{NavigationPlan, NavigationRequest, PathRelation, SlotAnchor, relation};
use crate::prompt::{FrontendRequest, Prompt};
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
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct Deps {
	pub spawner: Option<Arc<dyn Spawner>>,
	pub platform: crate::platform::Platform,
}

impl std::fmt::Debug for Deps {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Deps")
			.field("spawner", &self.spawner.is_some())
			.field("platform", &self.platform)
			.finish()
	}
}

impl Deps {
	pub fn free_desktop() -> Self {
		Self {
			platform: crate::platform::Platform::free_desktop(),
			..Self::default()
		}
	}
}

#[derive(Clone, Debug)]
pub struct OpenOptions {
	pub mode: Mode,
	pub deps: Deps,
	pub runtime: RuntimeSettings,
}

impl Default for OpenOptions {
	fn default() -> Self {
		Self {
			mode: Mode::default(),
			deps: Deps::free_desktop(),
			runtime: RuntimeSettings::default(),
		}
	}
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
	Run(crate::ops::Op),
	CancelJob(JobHandle),
	Launch(LaunchRequest),
	ConfirmLaunch(LaunchId),
	CancelLaunch(LaunchId),
	QuitActive(PathBuf),
	History(HistoryCommand),
	Focus(FocusCommand),
	Invoke(ActionId),
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Effect {
	Reload(Vec<PathBuf>),
	Navigate(NavigationPlan),
	Job(JobEvent),
	Launch(LaunchOutcome),
	LaunchEvent(LaunchEvent),
	FocusActivate(ItemId),
	Prompt(Prompt),
	Frontend(FrontendRequest),
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
	jobs: Jobs,
	/// Jobs that have not reported Finished or Cancelled yet.
	running_jobs: HashMap<JobId, JobHandle>,
	/// Destination path -> the job that is writing its `[Progress]` metadata.
	progress_jobs: HashMap<PathBuf, JobHandle>,
	execution: Execution,
	platform: crate::platform::Platform,
	history: History,
	focus: Focus,
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
		let spawner = options
			.deps
			.spawner
			.clone()
			.unwrap_or_else(|| Arc::new(crate::execution::FreeDesktopSpawner));
		let wrapper = options
			.runtime
			.execution_wrapper
			.clone()
			.filter(|command| !command.trim().is_empty())
			.map(|command| ExecutionWrapper { command });
		let screen = options
			.runtime
			.screen
			.clone()
			.filter(|screen| !screen.trim().is_empty());
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
			jobs: Jobs::default(),
			running_jobs: HashMap::new(),
			progress_jobs: HashMap::new(),
			execution: Execution::new(wrapper, screen, spawner, Arc::new(SystemClock)),
			platform: options.deps.platform,
			history: History::default(),
			focus: Focus::default(),
		};
		engine.reload();
		engine.cleanup_stale_progress();
		engine.history.record(HistoryEntry {
			pane: PaneId(0),
			path: engine.folder_path(),
			focus: None,
		});
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

	/// The platform's thumbnailer, when the host provides one. A frontend can
	/// hold the returned handle and render thumbnails off the UI thread.
	pub fn thumbnailer(&self) -> Option<Arc<dyn crate::platform::Thumbnailer>> {
		self.platform.thumbnails.clone()
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

	/// The controls a frontend should expose for an item with a live task.
	///
	/// Hishell's own transfers are controlled through their job; an external
	/// downloader is controlled through the `Pause=`, `Resume=` and `Stop=`
	/// commands in the sidecar.
	pub fn progress_control(&self, item: &Item) -> Option<ProgressControl> {
		if let Some(handle) = self.progress_jobs.get(&item.path) {
			let paused = handle.is_paused();
			return Some(ProgressControl {
				can_pause: !paused,
				can_resume: paused,
				can_stop: true,
				paused,
			});
		}
		item.progress.as_ref().map(crate::progress::Progress::control)
	}

	pub fn progress_paused(&self, path: &Path) -> bool {
		self.progress_jobs
			.get(path)
			.is_some_and(JobHandle::is_paused)
	}

	/// A Hishell transfer that died with the previous shell leaves its
	/// `[Progress]` group behind. No job in this process owns it, so remove it
	/// instead of showing an item that can never be opened or stopped.
	fn cleanup_stale_progress(&mut self) {
		let stale: Vec<(PathBuf, PathBuf, bool)> = self
			.items()
			.iter()
			.filter(|item| {
				item.progress.as_ref().is_some_and(|progress| {
					progress
						.owner
						.as_deref()
						.is_some_and(|owner| owner.eq_ignore_ascii_case("hishell"))
				})
			})
			.filter(|item| {
				!self.progress_jobs.contains_key(&item.path)
					&& !crate::progress::is_active(&item.path)
			})
			.filter_map(|item| {
				item.entry_path.clone().map(|entry| {
					(
						item.path.clone(),
						entry,
						item.fs_kind == crate::item::FsKind::File,
					)
				})
			})
			.collect();
		if stale.is_empty() {
			return;
		}
		for (path, entry, is_file) in stale {
			crate::progress::remove_section(&entry, crate::progress::SECTION);
			if is_file
				&& !path.exists()
				&& !crate::progress::has_meaningful_keys(&entry)
			{
				let _ = std::fs::remove_file(&entry);
			}
		}
		self.reload();
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
						pane: self.focus.pane(),
						item: self.focus.item(),
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
					mime: self.platform.mime_type(file.path()).0,
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
				let record = request.record_history;
				return Ok(self.navigate(plan, record));
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
				return Ok(self.navigate(plan, true));
			}
			Command::Run(op) => {
				let job = self.jobs.submit(op);
				self.running_jobs.insert(job.id(), job.clone());
				return Ok(Outcome {
					changes: ChangeSet::default(),
					effects: Vec::new(),
					job: Some(job),
				});
			}
			Command::CancelJob(handle) => {
				handle.cancel();
				return Ok(Outcome {
					changes: ChangeSet::default(),
					effects: Vec::new(),
					job: Some(handle),
				});
			}
			Command::Launch(request) => {
				let outcome = self.execution.request(request).map_err(Error::Launch)?;
				return Ok(Outcome {
					changes: ChangeSet::default(),
					effects: vec![Effect::Launch(outcome)],
					job: None,
				});
			}
			Command::ConfirmLaunch(id) => {
				self.execution.confirm(id).map_err(Error::Launch)?;
				return Ok(Outcome {
					changes: ChangeSet::default(),
					effects: vec![Effect::Launch(LaunchOutcome::Started)],
					job: None,
				});
			}
			Command::CancelLaunch(id) => {
				self.execution.cancel(id);
				return Ok(Outcome::default());
			}
			Command::QuitActive(path) => {
				self.execution.quit(&path).map_err(Error::Launch)?;
				return Ok(Outcome::default());
			}
			Command::Focus(command) => {
				return Ok(self.focus_command(command));
			}
			Command::Invoke(action) => {
				return self.invoke(action);
			}
			Command::History(command) => {
				let entry = match command {
					HistoryCommand::Back => self.history.back(),
					HistoryCommand::Forward => self.history.forward(),
					HistoryCommand::Clear => {
						self.history.clear();
						None
					}
				};
				let Some(entry) = entry else {
					return Ok(Outcome::default());
				};
				let target = entry.focus.clone().unwrap_or_else(|| entry.path.clone());
				let request = NavigationRequest {
					pane: entry.pane,
					target,
					record_history: false,
				};
				let mut plan = self.navigation_plan(&request);
				plan.focus = entry.focus;
				return Ok(self.navigate(plan, false));
			}
		};
		Ok(Outcome {
			changes,
			effects: Vec::new(),
			job: None,
		})
	}

	pub fn tick(&mut self) -> TickResult {
		let events = self.jobs.drain();
		for event in &events {
			match event {
				JobEvent::Progress { id, current, .. } => {
					if let Some(handle) = self.running_jobs.get(id) {
						self.progress_jobs.insert(current.clone(), handle.clone());
					}
				}
				JobEvent::Finished { id, .. } | JobEvent::Cancelled { id } => {
					self.running_jobs.remove(id);
					self.progress_jobs
						.retain(|_, handle| handle.id() != *id);
				}
				JobEvent::Started { .. } => {}
			}
		}
		let mut effects: Vec<Effect> = events.into_iter().map(Effect::Job).collect();
		effects.extend(self.execution.poll().into_iter().map(Effect::LaunchEvent));
		TickResult {
			changes: ChangeSet::default(),
			effects,
		}
	}

	pub fn execution(&self) -> &Execution {
		&self.execution
	}

	pub fn history(&self) -> HistoryStatus {
		self.history.status()
	}

	pub fn history_entries(&self) -> (Vec<HistoryEntry>, usize) {
		self.history.entries()
	}

	pub fn focus_state(&self) -> &Focus {
		&self.focus
	}

	pub fn focus(&self) -> Option<ItemId> {
		self.focus.item()
	}

	pub fn descriptor(&self, id: ActionId) -> &'static ActionDescriptor {
		descriptor(id).expect("known action")
	}

	pub fn actions(&self, request: &ActionRequest) -> Vec<ActionSpec> {
		let items: Vec<&Item> = match &request.target {
			Target::Location => Vec::new(),
			Target::Focus => self
				.focus
				.item()
				.and_then(|id| self.item(id))
				.into_iter()
				.collect(),
			Target::Selection => {
				let selected = self.selection().map(Selection::items).unwrap_or_default();
				selected
					.into_iter()
					.filter_map(|id| self.item(*id))
					.collect()
			}
			Target::Items(ids) => ids.iter().filter_map(|id| self.item(*id)).collect(),
			Target::Auto => {
				let selection = self.selection();
				if selection.is_some_and(Selection::active) {
					selection
						.map(Selection::items)
						.unwrap_or_default()
						.into_iter()
						.filter_map(|id| self.item(*id))
						.collect()
				} else {
					self.focus
						.item()
						.and_then(|id| self.item(id))
						.into_iter()
						.collect()
				}
			}
		};
		let has_items = !items.is_empty();
		let busy = items.iter().any(|item| item.is_busy());
		let controls: Vec<ProgressControl> = items
			.iter()
			.filter_map(|item| self.progress_control(item))
			.collect();
		let can_pause = controls.iter().any(|control| control.can_pause);
		let can_resume = controls.iter().any(|control| control.can_resume);
		let can_stop = controls.iter().any(|control| control.can_stop);
		let mut subjects = ActionSubjects::HERE;
		for item in &items {
			subjects = subjects.union(item_subjects(item));
			if self.is_radio_member(&item.path) {
				subjects = subjects.union(ActionSubjects::RADIO);
			}
		}
		let scope = self.location.kind();
		let location = match scope {
			crate::location::LocationKind::Folder => crate::config::LocationScope::Folder,
			crate::location::LocationKind::File => crate::config::LocationScope::File,
		};

		crate::action::DESCRIPTORS
			.iter()
			.filter(|descriptor| {
				(descriptor.location == crate::config::LocationScope::Both
					|| descriptor.location == location)
					&& menu_contains(descriptor, request.menu)
					&& (descriptor.subjects.contains(ActionSubjects::HERE)
						|| subjects.intersects(descriptor.subjects))
			})
			.map(|descriptor| {
				let needs_items = !descriptor.subjects.contains(ActionSubjects::HERE);
				let activation = matches!(
					descriptor.id,
					ActionId::OPEN | ActionId::OPEN_WITH | ActionId::NAVIGATE
				);
				let enabled = (!needs_items || has_items)
					&& !(busy && activation)
					&& match descriptor.id {
						ActionId::PAUSE_PROGRESS => can_pause,
						ActionId::RESUME_PROGRESS => can_resume,
						ActionId::STOP_PROGRESS => can_stop,
						_ => true,
					};
				ActionSpec {
					id: descriptor.id,
					group: descriptor.group,
					subjects: descriptor.subjects,
					location: descriptor.location,
					enabled,
					visible: true,
					checked: None,
					disabled_reason: (!enabled).then_some(if busy && activation {
						crate::action::DisabledReason::Busy
					} else {
						crate::action::DisabledReason::NotApplicable
					}),
				}
			})
			.collect()
	}

	fn invoke(&mut self, action: ActionId) -> Result<Outcome, Error> {
		let ids = self.invoke_targets();
		let items: Vec<Item> = ids
			.iter()
			.filter_map(|id| self.item(*id).cloned())
			.collect();
		let paths: Vec<PathBuf> = items.iter().map(|item| item.path.clone()).collect();
		let first = items.first().cloned();
		match action {
			ActionId::OPEN | ActionId::NAVIGATE => match first {
				Some(item) => self.open_item(&item),
				None => Ok(outcome(vec![Effect::Frontend(FrontendRequest::Action(
					action,
				))])),
			},
			ActionId::PAUSE_PROGRESS => match first {
				Some(item) => self.progress_action(&item, ProgressAction::Pause),
				None => Ok(Outcome::default()),
			},
			ActionId::RESUME_PROGRESS => match first {
				Some(item) => self.progress_action(&item, ProgressAction::Resume),
				None => Ok(Outcome::default()),
			},
			ActionId::STOP_PROGRESS => match first {
				Some(item) => self.progress_action(&item, ProgressAction::Stop),
				None => Ok(Outcome::default()),
			},
			ActionId::REFRESH => self.dispatch(Command::Reload),
			ActionId::GO_UP => self.dispatch(Command::GoUp),
			ActionId::SELECT_ALL => self.dispatch(Command::SelectAll),
			ActionId::NEW_FOLDER => self.dispatch(Command::Run(crate::ops::Op::NewFolder {
				parent: self.folder_path(),
			})),
			ActionId::NEW_TEXT_FILE => self.dispatch(Command::Run(crate::ops::Op::NewTextFile {
				parent: self.folder_path(),
			})),
			ActionId::TRASH => {
				if paths.is_empty() {
					return Ok(Outcome::default());
				}
				self.dispatch(Command::Run(crate::ops::Op::Trash { paths }))
			}
			ActionId::DELETE => {
				if paths.is_empty() {
					return Ok(Outcome::default());
				}
				self.dispatch(Command::Run(crate::ops::Op::Delete { paths }))
			}
			ActionId::DUPLICATE => {
				if paths.is_empty() {
					return Ok(Outcome::default());
				}
				self.dispatch(Command::Run(crate::ops::Op::Duplicate { paths }))
			}
			ActionId::CREATE_LINK => {
				if paths.is_empty() {
					return Ok(Outcome::default());
				}
				self.dispatch(Command::Run(crate::ops::Op::Link {
					sources: paths,
					dest: self.folder_path(),
				}))
			}
			ActionId::ROTATE_IMAGE => match first {
				Some(item)
					if item
						.mime
						.as_deref()
						.is_some_and(|mime| mime.starts_with("image/")) =>
				{
					self.dispatch(Command::Run(crate::ops::Op::RotateImage {
						path: item.path,
						degrees: 90,
					}))
				}
				_ => Ok(outcome(vec![Effect::Frontend(FrontendRequest::Action(
					action,
				))])),
			},
			ActionId::RENAME => match first {
				Some(item) => Ok(outcome(vec![Effect::Prompt(Prompt::Rename {
					// Rename edits the on-disk name, which can differ from the
					// display title supplied by a desktop entry or descriptor.
					suggested: item.name,
					path: item.path,
				})])),
				None => Ok(Outcome::default()),
			},
			ActionId::OPEN_WITH => match first {
				Some(item) => Ok(outcome(vec![Effect::Prompt(Prompt::OpenWith {
					path: item.path,
				})])),
				None => Ok(Outcome::default()),
			},
			ActionId::DESTROY => Ok(outcome(vec![Effect::Prompt(Prompt::Destroy { paths })])),
			ActionId::NUMBER => match first {
				Some(item) => Ok(outcome(vec![Effect::Prompt(Prompt::Number {
					path: item.path,
				})])),
				None => Ok(Outcome::default()),
			},
			ActionId::RADIO => match first {
				Some(item) => {
					let value = crate::folder::radio_value(&item.path);
					if value.is_empty() {
						return Ok(Outcome::default());
					}
					let delta = self.config.set_raw(
						&self.location,
						"Radio",
						"Selected",
						&value,
						true,
					)?;
					let folder = self.apply_config_change(delta.scope);
					self.revision += 1;
					let exec = self.config.radio_exec.clone();
					let mut outcome = Outcome {
						changes: ChangeSet {
							config: vec![delta],
							folder,
							..ChangeSet::default()
						},
						..Outcome::default()
					};
					if !exec.trim().is_empty() {
						let launch = self.dispatch(Command::Launch(
							LaunchRequest::Radio {
								exec,
								value,
								name: item.title.clone(),
								path: item.path.clone(),
							},
						))?;
						outcome.effects.extend(launch.effects);
					}
					Ok(outcome)
				}
				None => Ok(Outcome::default()),
			},
			ActionId::TOGGLE => match first {
				Some(item) => {
					// A provider-owned toggle runs its Exec with the target
					// state and writes `Toggle=` itself. Without one, flip the
					// entry directly.
					let directory = item.path.is_dir();
					let entry_path = if directory {
						item.path.join(".directory")
					} else {
						item.path.clone()
					};
					let kind = if directory {
						crate::item::FsKind::Directory
					} else {
						crate::item::FsKind::File
					};
					let has_exec = crate::entry::Entry::read(&entry_path, kind)
						.is_some_and(|entry| {
							entry
								.exec
								.as_deref()
								.is_some_and(|exec| !exec.trim().is_empty())
						});
					if has_exec {
						self.dispatch(Command::Launch(LaunchRequest::Control {
							path: item.path.clone(),
							values: Vec::new(),
						}))
					} else {
						self.dispatch(Command::Run(crate::ops::Op::Toggle {
							path: item.path.clone(),
						}))
					}
				}
				None => Ok(Outcome::default()),
			},
			ActionId::QUIT_ACTIVE => match first {
				Some(item) => self.dispatch(Command::QuitActive(item.path)),
				None => Ok(Outcome::default()),
			},
			ActionId::VIEW_SETTINGS => Ok(outcome(vec![Effect::Frontend(
				FrontendRequest::ViewSettings,
			)])),
			_ => Ok(outcome(vec![Effect::Frontend(FrontendRequest::Action(
				action,
			))])),
		}
	}

	fn invoke_targets(&self) -> Vec<ItemId> {
		if let Some(selection) = self.selection()
			&& selection.active()
			&& !selection.is_empty()
		{
			return selection.items().to_vec();
		}
		self.focus.item().into_iter().collect()
	}

	/// Whether OPEN has something to do for `item`: start an app or a control,
	/// navigate, or hand it to the frontend because no handler is registered.
	///
	/// A frontend checks this before playing a launch animation, so an item
	/// that cannot be opened reports that instead of animating a launch that
	/// never happens. This mirrors `open_item`.
	pub fn can_open(&self, item: &Item) -> bool {
		if matches!(
			item.kind,
			crate::item::ItemKind::Application
				| crate::item::ItemKind::Button
				| crate::item::ItemKind::Toggle
				| crate::item::ItemKind::Number
				| crate::item::ItemKind::Radio
		) {
			return true;
		}
		if matches!(item.kind, crate::item::ItemKind::Link) && item.target.is_some() {
			return true;
		}
		if item.fs_kind == crate::item::FsKind::Directory {
			return true;
		}
		self.platform
			.apps
			.as_ref()
			.is_some_and(|apps| apps.default_app(&item.path).is_some())
	}

	fn open_item(&mut self, item: &Item) -> Result<Outcome, Error> {
		if item.is_busy() {
			return Ok(Outcome::default());
		}
		match item.kind {
			crate::item::ItemKind::Application
			| crate::item::ItemKind::Button
			| crate::item::ItemKind::Toggle
			| crate::item::ItemKind::Number
			| crate::item::ItemKind::Radio => {
				return self.dispatch(Command::Launch(LaunchRequest::Item {
					path: item.path.clone(),
				}));
			}
			crate::item::ItemKind::Link => {
				if let Some(target) = &item.target {
					let request = NavigationRequest {
						pane: self.focus.pane(),
						target: target.clone(),
						record_history: true,
					};
					return self.dispatch(Command::Navigate(request));
				}
			}
			crate::item::ItemKind::Directory | crate::item::ItemKind::File => {}
		}
		if item.fs_kind == crate::item::FsKind::Directory {
			let request = NavigationRequest {
				pane: self.focus.pane(),
				target: item.path.clone(),
				record_history: true,
			};
			return self.dispatch(Command::Navigate(request));
		}
		let apps = self.platform.apps.clone();
		if let Some(apps) = apps
			&& let Some(app) = apps.default_app(&item.path)
		{
			return self.dispatch(Command::Launch(LaunchRequest::App {
				path: item.path.clone(),
				app: crate::execution::AppId(app.path.to_string_lossy().into_owned()),
			}));
		}
		Ok(outcome(vec![Effect::Frontend(FrontendRequest::OpenPath(
			item.path.clone(),
		))]))
	}

	fn progress_action(
		&mut self,
		item: &Item,
		action: ProgressAction,
	) -> Result<Outcome, Error> {
		let Some(progress) = item.progress.as_ref() else {
			return Ok(Outcome::default());
		};
		let handle = self.progress_jobs.get(&item.path).cloned();
		match action {
			ProgressAction::Pause => {
				if let Some(handle) = &handle {
					handle.pause();
				} else if let Some(command) = progress.command(action) {
					Self::spawn_progress_command(command, item)?;
				} else {
					return Ok(Outcome::default());
				}
				self.set_progress_paused(item, true);
			}
			ProgressAction::Resume => {
				if let Some(handle) = &handle {
					handle.resume();
				} else if let Some(command) = progress.command(action) {
					Self::spawn_progress_command(command, item)?;
				} else {
					return Ok(Outcome::default());
				}
				self.set_progress_paused(item, false);
			}
			ProgressAction::Stop => {
				if let Some(handle) = &handle {
					handle.cancel();
				} else if let Some(command) = progress.command(action) {
					Self::spawn_progress_command(command, item)?;
				} else {
					return Ok(Outcome::default());
				}
			}
		}
		Ok(outcome(vec![Effect::Reload(vec![self.folder_path()])]))
	}

	/// An external downloader owns the process; Hishell only records that the
	/// user asked it to pause or resume, so the view updates immediately.
	fn set_progress_paused(&self, item: &Item, paused: bool) {
		if let Some(entry) = &item.entry_path {
			crate::progress::set_paused(entry, paused);
		}
	}

	fn spawn_progress_command(command: &str, item: &Item) -> Result<(), Error> {
		let entry = item.entry_path.as_deref().unwrap_or(&item.path);
		let args = crate::progress::expand_command(command, &item.path, entry, &item.title)
			.map_err(Error::Config)?;
		let Some((program, rest)) = args.split_first() else {
			return Ok(());
		};
		std::process::Command::new(program)
			.args(rest)
			.current_dir(item.path.parent().unwrap_or(Path::new(".")))
			.spawn()
			.map_err(|error| {
				Error::Config(format!("cannot run progress command: {error}"))
			})?;
		Ok(())
	}

	fn focus_command(&mut self, command: FocusCommand) -> Outcome {
		let previous = self.focus.item();
		let order = self
			.folder
			.as_ref()
			.map(|folder| folder.order().to_vec())
			.unwrap_or_default();
		self.focus.set_order(self.focus.pane(), order.clone());
		// Grouped and stashed views do not fill a plain row-major grid: their
		// slot cells can jump columns and rows. Register the cells the view
		// actually laid out so Left/Right/Up/Down follow the picture instead
		// of a naive `lines`-wide grid.
		if let Some(positions) = self.view_positions(&order) {
			self.focus.apply(FocusCommand::SetPositions {
				pane: self.focus.pane(),
				positions,
			});
		}
		match command {
			FocusCommand::First => {
				self.focus.first(&order);
			}
			FocusCommand::Last => {
				self.focus.last(&order);
			}
			FocusCommand::Move(direction) => {
				self.focus.set_columns(self.focus_columns());
				self.focus.move_focus(direction, &order);
			}
			FocusCommand::Set { pane, path } => {
				let item = self.item_at(&path).map(|item| item.id);
				self.focus.set(pane, item);
			}
			FocusCommand::Clear => {
				self.focus.set(self.focus.pane(), None);
			}
			FocusCommand::Activate => {
				return Outcome {
					changes: ChangeSet::default(),
					effects: self
						.focus
						.item()
						.map(Effect::FocusActivate)
						.into_iter()
						.collect(),
					job: None,
				};
			}
			other => {
				self.focus.apply(other);
			}
		}
		Outcome {
			changes: ChangeSet {
				focus: Some(FocusDelta {
					pane: self.focus.pane(),
					item: self.focus.item(),
					previous,
				}),
				..ChangeSet::default()
			},
			effects: Vec::new(),
			job: None,
		}
	}

	/// Slot cells in the order the focus engine follows, or None when the
	/// last view does not describe every item (file views, stale order).
	fn view_positions(&self, order: &[ItemId]) -> Option<Vec<(i32, i32)>> {
		let last_view = self.last_view.borrow();
		let view = last_view.as_ref()?;
		let mut positions = Vec::with_capacity(order.len());
		for item in order {
			let slot = view.slots.iter().find(|slot| slot.item == *item)?;
			positions.push((slot.cell.0 as i32, slot.cell.1 as i32));
		}
		(!positions.is_empty()).then_some(positions)
	}

	fn focus_columns(&self) -> usize {
		let last_view = self.last_view.borrow();
		let Some(view) = last_view.as_ref() else {
			return 1;
		};
		match &view.mode {
			FolderMode::Grid(grid) => {
				if grid.horizontal {
					let items = self.folder.as_ref().map(Folder::len).unwrap_or(0);
					items.div_ceil(grid.lines.max(1) as usize).max(1)
				} else {
					grid.lines.max(1) as usize
				}
			}
			FolderMode::List(_) => 1,
		}
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

	fn navigate(&mut self, plan: NavigationPlan, record: bool) -> Outcome {
		self.location = Location::folder(plan.to.clone());
		self.folder = None;
		let changes = self.reload();
		if record {
			self.history.record(HistoryEntry {
				pane: plan.pane,
				path: plan.to.clone(),
				focus: plan.focus.clone(),
			});
		}
		let order = self
			.folder
			.as_ref()
			.map(|folder| folder.order().to_vec())
			.unwrap_or_default();
		self.focus.set_order(plan.pane, order);
		let focus = plan
			.focus
			.as_deref()
			.and_then(|path| self.item_at(path).map(|item| item.id));
		self.focus.set(plan.pane, focus);
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
		self.reconcile_focus();
		self.rebuild_diagnostics();
		self.sync_execution();
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

	fn sync_execution(&mut self) {
		let wrapper =
			(!self.config.execution_wrapper.trim().is_empty()).then(|| ExecutionWrapper {
				command: self.config.execution_wrapper.clone(),
			});
		self.execution.set_wrapper(wrapper);
		self.execution.set_screen(
			(!self.config.screen.trim().is_empty()).then(|| self.config.screen.clone()),
		);
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
				self.reconcile_focus();
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

	/// Keep the cursor on the item it was on after a rescan.
	///
	/// `ItemId` is derived from the path, so an item that is still in the
	/// folder keeps its focus. When it is gone (trashed, moved, renamed away)
	/// the stored index would keep pointing at that position while the id it
	/// names no longer exists: nothing draws the cursor, and the next move
	/// starts from a stale index. Hand the rebuilt order to the focus instead,
	/// which finds the same item by id or clamps the index into the new list.
	fn reconcile_focus(&mut self) {
		let Some(folder) = self.folder.as_ref() else {
			return;
		};
		let order = folder.order().to_vec();
		let pane = self.focus.pane();
		self.focus.set_order(pane, order);
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

fn outcome(effects: Vec<Effect>) -> Outcome {
	Outcome {
		changes: ChangeSet::default(),
		effects,
		job: None,
	}
}

fn item_subjects(item: &Item) -> ActionSubjects {
	use crate::item::{FsKind, ItemKind};
	let mut subjects = match item.fs_kind {
		FsKind::Directory => ActionSubjects::FOLDER,
		FsKind::File => ActionSubjects::FILE,
	};
	subjects = subjects.union(match item.kind {
		ItemKind::Directory => ActionSubjects::FOLDER,
		ItemKind::File => ActionSubjects::FILE,
		ItemKind::Application => ActionSubjects::APPLICATION,
		ItemKind::Link => ActionSubjects::LINK,
		ItemKind::Button => ActionSubjects::BUTTON,
		ItemKind::Toggle => ActionSubjects::TOGGLE,
		ItemKind::Number => ActionSubjects::NUMBER,
		ItemKind::Radio => ActionSubjects::RADIO,
	});
	if let Some(mime) = item.mime.as_deref() {
		if mime.starts_with("image/") {
			subjects = subjects.union(ActionSubjects::IMAGE);
		} else if mime.starts_with("audio/") {
			subjects = subjects.union(ActionSubjects::AUDIO);
		} else if mime.starts_with("video/") {
			subjects = subjects.union(ActionSubjects::VIDEO);
		}
	}
	if item.is_busy() {
		subjects = subjects.union(ActionSubjects::PROGRESS);
	}
	subjects
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
	use crate::action::MenuKind;
	use crate::focus::Direction;
	use crate::view::PaneId;
	use crate::config::ConfigKey;

	struct AcceptAll;

	impl crate::execution::Spawner for AcceptAll {
		fn command_exists(&self, _command: &str) -> bool {
			false
		}

		fn preflight(
			&self,
			_wrapper: &str,
			_request: &crate::execution::SpawnRequest,
		) -> Option<String> {
			None
		}

		fn spawn(
			&self,
			_wrapper: Option<&str>,
			_request: &crate::execution::SpawnRequest,
		) -> Result<(), crate::execution::LaunchError> {
			Ok(())
		}

		fn quit(&self, _wrapper: &str, _item: &Path) -> Result<(), crate::execution::LaunchError> {
			Ok(())
		}

		fn active_items(&self, _wrapper: &str) -> Vec<PathBuf> {
			Vec::new()
		}
	}

	struct NoApps;

	impl crate::platform::AppCatalog for NoApps {
		fn apps_for(&self, _path: &Path) -> Vec<crate::platform::AppEntry> {
			Vec::new()
		}

		fn default_app(&self, _path: &Path) -> Option<crate::platform::AppEntry> {
			None
		}

		fn set_default(
			&self,
			_mime: &str,
			_app: &crate::execution::AppId,
		) -> Result<(), crate::platform::PlatformError> {
			Ok(())
		}
	}

	struct AnyHandler;

	impl crate::platform::AppCatalog for AnyHandler {
		fn apps_for(&self, _path: &Path) -> Vec<crate::platform::AppEntry> {
			Vec::new()
		}

		fn default_app(&self, path: &Path) -> Option<crate::platform::AppEntry> {
			Some(crate::platform::AppEntry {
				id: crate::execution::AppId(String::from("test.desktop")),
				name: String::from("Test"),
				icon: None,
				path: path.to_path_buf(),
			})
		}

		fn set_default(
			&self,
			_mime: &str,
			_app: &crate::execution::AppId,
		) -> Result<(), crate::platform::PlatformError> {
			Ok(())
		}
	}

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
				key: ConfigKey::GridMode,
				value: ConfigValue::Str(String::from("LINES")),
			})
			.unwrap();
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
	fn reload_keeps_the_focused_item_and_clamps_when_it_goes() {
		let path = temp_folder("refocus");
		for name in ["a", "b", "c"] {
			std::fs::write(path.join(name), b"x").unwrap();
		}
		let mut engine = Engine::open(&path).unwrap();
		engine
			.dispatch(Command::Focus(FocusCommand::Set {
				pane: PaneId(0),
				path: path.join("b"),
			}))
			.unwrap();
		let focused = engine.focus().expect("b is focused");
		assert_eq!(engine.item(focused).unwrap().name, "b");

		// Another item appears: the focus stays on the same path.
		std::fs::write(path.join("d"), b"x").unwrap();
		engine.dispatch(Command::Reload).unwrap();
		assert_eq!(engine.item(engine.focus().unwrap()).unwrap().name, "b");

		// The focused item is gone: the cursor must still name a live item
		// rather than a stale id that draws nothing.
		std::fs::remove_file(path.join("b")).unwrap();
		engine.dispatch(Command::Reload).unwrap();
		let focused = engine.focus().expect("the cursor survives a removal");
		assert!(engine.item(focused).is_some());
		assert_ne!(engine.item(focused).unwrap().name, "b");

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
	fn setting_two_free_positions_swaps_the_items() {
		let path = temp_folder("swap");
		for name in ["a", "b", "c"] {
			std::fs::write(path.join(name), b"x").unwrap();
		}
		let mut engine = Engine::open(&path).unwrap();
		engine
			.dispatch(Command::SetConfig {
				key: ConfigKey::Sort,
				value: ConfigValue::Str("FREE".into()),
			})
			.unwrap();
		for (name, column) in [("a", 0), ("b", 1), ("c", 2)] {
			engine
				.dispatch(Command::SetFreePosition {
					name: String::from(name),
					column,
					row: 0,
				})
				.unwrap();
		}

		// `a` is dropped on `b`: one command writes both cells, and `c` keeps
		// the cell it was in.
		engine
			.dispatch(Command::SetFreePositions {
				positions: vec![
					(String::from("b"), 0, 0),
					(String::from("a"), 1, 0),
				],
			})
			.unwrap();

		let cell = |engine: &Engine, name: &str| {
			engine
				.view(request())
				.folder_slots()
				.into_iter()
				.find(|slot| engine.item(slot.item).is_some_and(|item| item.name == name))
				.unwrap()
				.cell
		};
		assert_eq!(cell(&engine, "a"), (1, 0));
		assert_eq!(cell(&engine, "b"), (0, 0));
		assert_eq!(cell(&engine, "c"), (2, 0));

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
			Effect::Reload(_)
			| Effect::Job(_)
			| Effect::Launch(_)
			| Effect::LaunchEvent(_)
			| Effect::FocusActivate(_)
			| Effect::Prompt(_)
			| Effect::Frontend(_) => panic!("expected a navigation plan"),
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
	fn jobs_run_through_the_engine_and_report_on_tick() {
		let path = temp_folder("jobs");
		let file = path.join("a.txt");
		std::fs::write(&file, b"x").unwrap();
		let mut engine = Engine::open(&path).unwrap();
		let outcome = engine
			.dispatch(Command::Run(crate::ops::Op::Delete {
				paths: vec![file.clone()],
			}))
			.unwrap();
		assert!(outcome.job.is_some());
		let mut finished = false;
		for _ in 0..200 {
			for effect in engine.tick().effects {
				if let Effect::Job(crate::jobs::JobEvent::Finished { outcome, .. }) = effect {
					assert!(outcome.failed.is_empty());
					finished = true;
				}
			}
			if finished {
				break;
			}
			std::thread::sleep(std::time::Duration::from_millis(10));
		}
		assert!(finished);
		assert!(!file.exists());
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn a_busy_item_cannot_be_opened_and_offers_progress_actions() {
		let path = temp_folder("progress-action");
		std::fs::write(
			path.join("movie.mkv.desktop"),
			"[Desktop Entry]\nType=File\nName=Movie\nIcon=video-x-generic\n\
			 [Progress]\nValue=2\nTarget=10\nTaskName=Downloading...\n\
			 Pause=download pause %p\nResume=download resume %p\n\
			 Stop=download stop %p\nPaused=false\n",
		)
		.unwrap();
		let mut engine = Engine::open(&path).unwrap();
		engine
			.dispatch(Command::Focus(FocusCommand::First))
			.unwrap();
		let item = engine
			.focus()
			.and_then(|id| engine.item(id))
			.cloned()
			.unwrap();
		assert!(item.is_busy());

		let actions = engine.actions(&ActionRequest {
			menu: MenuKind::Context,
			target: Target::Focus,
		});
		let open = actions
			.iter()
			.find(|action| action.id == ActionId::OPEN)
			.unwrap();
		assert!(!open.enabled);
		assert_eq!(
			open.disabled_reason,
			Some(crate::action::DisabledReason::Busy)
		);
		assert!(
			actions
				.iter()
				.find(|action| action.id == ActionId::PAUSE_PROGRESS)
				.is_some_and(|action| action.enabled)
		);
		assert!(
			actions
				.iter()
				.find(|action| action.id == ActionId::STOP_PROGRESS)
				.is_some_and(|action| action.enabled)
		);

		let outcome = engine.dispatch(Command::Invoke(ActionId::OPEN)).unwrap();
		assert!(outcome.effects.is_empty());
		assert_eq!(engine.item_at(&item.path).map(|item| item.title.as_str()), Some("Movie"));
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn a_stale_hishell_progress_group_is_cleaned_on_open() {
		let path = temp_folder("progress-stale");
		let sidecar = path.join("movie.mkv.desktop");
		std::fs::write(
			&sidecar,
			"[Desktop Entry]\nType=File\n\
			 [Progress]\nValue=2\nTarget=10\nOwner=Hishell\n",
		)
		.unwrap();
		let engine = Engine::open(&path).unwrap();
		assert!(engine.items().is_empty());
		assert!(!sidecar.exists());
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn launching_an_entry_returns_a_launch_effect() {
		let path = temp_folder("launch");
		let entry = path.join("ride.desktop");
		std::fs::write(
			&entry,
			"[Desktop Entry]\nType=Application\nName=Ride\nExec=ride\n",
		)
		.unwrap();
		let mut engine = Engine::open_location_with(
			Location::folder(&path),
			OpenOptions {
				deps: Deps {
					spawner: Some(Arc::new(AcceptAll)),
					..Deps::default()
				},
				..OpenOptions::default()
			},
		)
		.unwrap();
		let outcome = engine
			.dispatch(Command::Launch(LaunchRequest::Item { path: entry }))
			.unwrap();
		assert!(matches!(
			outcome.effects[0],
			Effect::Launch(LaunchOutcome::Started)
		));
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn a_file_without_a_handler_cannot_be_opened() {
		let path = temp_folder("can-open");
		let file = path.join("notes.txt");
		std::fs::write(&file, b"x").unwrap();

		let without = Engine::open_location_with(
			Location::folder(&path),
			OpenOptions {
				deps: Deps {
					platform: crate::platform::Platform {
						apps: Some(Arc::new(NoApps)),
						..crate::platform::Platform::default()
					},
					..Deps::default()
				},
				..OpenOptions::default()
			},
		)
		.unwrap();
		let item = without
			.items()
			.iter()
			.find(|item| item.path == file)
			.cloned()
			.unwrap();
		assert!(!without.can_open(&item));

		let with_handler = Engine::open_location_with(
			Location::folder(&path),
			OpenOptions {
				deps: Deps {
					platform: crate::platform::Platform {
						apps: Some(Arc::new(AnyHandler)),
						..crate::platform::Platform::default()
					},
					..Deps::default()
				},
				..OpenOptions::default()
			},
		)
		.unwrap();
		let item = with_handler
			.items()
			.iter()
			.find(|item| item.path == file)
			.cloned()
			.unwrap();
		assert!(with_handler.can_open(&item));

		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn navigation_records_history_and_back_replays_it() {
		let path = temp_folder("history");
		let child = path.join("child");
		std::fs::create_dir_all(&child).unwrap();
		let mut engine = Engine::open(&path).unwrap();
		engine
			.dispatch(Command::Navigate(NavigationRequest {
				pane: PaneId(0),
				target: child.clone(),
				record_history: true,
			}))
			.unwrap();
		assert!(engine.history().can_back);
		engine
			.dispatch(Command::History(HistoryCommand::Back))
			.unwrap();
		assert_eq!(engine.location().path(), path);
		assert!(engine.history().can_forward);
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn actions_filter_by_target_and_item_kind() {
		let path = temp_folder("actions");
		let image = path.join("a.png");
		let text = path.join("b.txt");
		std::fs::write(&image, b"x").unwrap();
		std::fs::write(&text, b"x").unwrap();
		let mut engine = Engine::open(&path).unwrap();
		engine
			.dispatch(Command::Focus(FocusCommand::Set {
				pane: PaneId(0),
				path: image.clone(),
			}))
			.unwrap();
		let request = ActionRequest {
			menu: MenuKind::Context,
			target: Target::Focus,
		};
		let ids: Vec<&str> = engine
			.actions(&request)
			.iter()
			.map(|action| action.id.as_str())
			.collect();
		assert!(ids.contains(&"rotate-image"));
		assert!(ids.contains(&"rename"));
		assert!(!ids.contains(&"toggle"));

		engine
			.dispatch(Command::Focus(FocusCommand::Set {
				pane: PaneId(0),
				path: text,
			}))
			.unwrap();
		let ids: Vec<&str> = engine
			.actions(&request)
			.iter()
			.map(|action| action.id.as_str())
			.collect();
		assert!(!ids.contains(&"rotate-image"));

		let ids: Vec<&str> = engine
			.actions(&ActionRequest {
				menu: MenuKind::Context,
				target: Target::Location,
			})
			.iter()
			.map(|action| action.id.as_str())
			.collect();
		assert!(ids.contains(&"new-folder"));
		assert!(ids.contains(&"refresh"));
		assert!(!ids.contains(&"rename"));
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn invoke_routes_prompts_and_operations() {
		let path = temp_folder("invoke");
		let file = path.join("a.txt");
		std::fs::write(&file, b"x").unwrap();
		let folder = path.join("sub");
		std::fs::create_dir_all(&folder).unwrap();
		let mut engine = Engine::open(&path).unwrap();

		engine
			.dispatch(Command::Focus(FocusCommand::Set {
				pane: PaneId(0),
				path: file.clone(),
			}))
			.unwrap();
		let outcome = engine.dispatch(Command::Invoke(ActionId::RENAME)).unwrap();
		match &outcome.effects[0] {
			Effect::Prompt(Prompt::Rename { path, suggested }) => {
				assert_eq!(path, &file);
				assert_eq!(suggested, "a.txt");
			}
			_ => panic!("expected a rename prompt"),
		}

		let outcome = engine
			.dispatch(Command::Invoke(ActionId::NEW_FOLDER))
			.unwrap();
		assert!(outcome.job.is_some());
		let mut created = false;
		for _ in 0..200 {
			for effect in engine.tick().effects {
				if let Effect::Job(crate::jobs::JobEvent::Finished { outcome, .. }) = effect
					&& outcome.failed.is_empty()
				{
					created = true;
				}
			}
			if created {
				break;
			}
			std::thread::sleep(std::time::Duration::from_millis(10));
		}
		assert!(created);

		engine
			.dispatch(Command::Focus(FocusCommand::Set {
				pane: PaneId(0),
				path: folder.clone(),
			}))
			.unwrap();
		let outcome = engine.dispatch(Command::Invoke(ActionId::OPEN)).unwrap();
		assert!(matches!(outcome.effects[0], Effect::Navigate(_)));
		assert_eq!(engine.location().path(), folder);

		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn selecting_a_radio_choice_writes_and_launches() {
		let path = temp_folder("radio-select");
		std::fs::write(
			path.join(".directory"),
			"[Radio]\nEnabled = true\nSelected =\nExec = /bin/true %v %n %p\n",
		)
		.unwrap();
		let choice = path.join("one");
		std::fs::create_dir_all(&choice).unwrap();
		let mut engine = Engine::open(&path).unwrap();
		engine
			.dispatch(Command::Focus(FocusCommand::Set {
				pane: PaneId(0),
				path: choice,
			}))
			.unwrap();
		let actions = engine.actions(&ActionRequest {
			menu: MenuKind::Context,
			target: Target::Focus,
		});
		assert!(actions.iter().any(|action| action.id == ActionId::RADIO));
		let outcome = engine.dispatch(Command::Invoke(ActionId::RADIO)).unwrap();
		assert_eq!(engine.config().radio_selected, "one");
		assert!(matches!(outcome.effects.first(), Some(Effect::Launch(_))));
		let text = std::fs::read_to_string(path.join(".directory")).unwrap();
		assert!(text.contains("Selected = one"));
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn toggle_with_a_provider_launches_its_exec() {
		let path = temp_folder("toggle-provider");
		let toggle = path.join("mute.desktop");
		std::fs::write(
			&toggle,
			"[Desktop Entry]\nType=Toggle\nName=Mute\nExec=/bin/true %v\nToggle=false\n",
		)
		.unwrap();
		let mut engine = Engine::open(&path).unwrap();
		engine
			.dispatch(Command::Focus(FocusCommand::Set {
				pane: PaneId(0),
				path: toggle.clone(),
			}))
			.unwrap();
		let outcome = engine
			.dispatch(Command::Invoke(ActionId::TOGGLE))
			.unwrap();
		assert!(matches!(outcome.effects.first(), Some(Effect::Launch(_))));
		// The provider owns the stored state.
		let text = std::fs::read_to_string(&toggle).unwrap();
		assert!(text.contains("Toggle=false"));
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn invoke_trash_and_rotate_run_jobs() {
		let path = temp_folder("invoke-ops");
		let image = path.join("a.png");
		image::RgbaImage::new(2, 2).save(&image).unwrap();
		let text = path.join("b.txt");
		std::fs::write(&text, b"x").unwrap();
		let mut engine = Engine::open(&path).unwrap();

		engine
			.dispatch(Command::SelectOnly(engine.item_at(&image).unwrap().id))
			.unwrap();
		let outcome = engine
			.dispatch(Command::Invoke(ActionId::ROTATE_IMAGE))
			.unwrap();
		assert!(outcome.job.is_some());
		engine.dispatch(Command::ClearSelection).unwrap();

		engine
			.dispatch(Command::Focus(FocusCommand::Set {
				pane: PaneId(0),
				path: text.clone(),
			}))
			.unwrap();
		let outcome = engine.dispatch(Command::Invoke(ActionId::DELETE)).unwrap();
		assert!(outcome.job.is_some());
		for _ in 0..200 {
			let _ = engine.tick();
			if !text.exists() {
				break;
			}
			std::thread::sleep(std::time::Duration::from_millis(10));
		}
		assert!(!text.exists());
		assert!(image.exists());
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn focus_moves_along_the_grouped_cells() {
		let path = temp_folder("group-focus");
		for (name, bytes) in [
			("a.png", b"x".as_slice()),
			("b.zip", b"x"),
			("c.txt", b"x"),
			("d.png", b"x"),
			("e.zip", b"x"),
		] {
			std::fs::write(path.join(name), bytes).unwrap();
		}
		let mut engine = Engine::open(&path).unwrap();
		engine
			.dispatch(Command::SetConfig {
				key: ConfigKey::Group,
				value: ConfigValue::Str("TYPE".into()),
			})
			.unwrap();
		let slots = engine.view(request()).folder_slots();
		assert!(
			slots.iter().any(|slot| slot.cell.0 > 0 && slot.cell.1 > 0),
			"grouped layout should place items in lanes"
		);
		let pane = PaneId(0);
		for index in 0..slots.len() {
			let (column, row) = (slots[index].cell.0 as i32, slots[index].cell.1 as i32);
			for direction in [
				Direction::Right,
				Direction::Left,
				Direction::Down,
				Direction::Up,
			] {
				engine
					.dispatch(Command::Focus(FocusCommand::SetIndex {
						pane,
						index: index as i32,
					}))
					.unwrap();
				engine
					.dispatch(Command::Focus(FocusCommand::Move(direction)))
					.unwrap();
				let focus = engine.focus().unwrap();
				let target = slots.iter().find(|slot| slot.item == focus).unwrap();
				let (target_column, target_row) = (target.cell.0 as i32, target.cell.1 as i32);
				let stayed = target_column == column && target_row == row;
				let valid = stayed
					|| match direction {
						Direction::Right => target_column > column,
						Direction::Left => target_column < column,
						Direction::Down => target_row > row,
						Direction::Up => target_row < row,
					};
				assert!(
					valid,
					"{direction:?} from ({column},{row}) landed on ({target_column},{target_row})"
				);
			}
		}
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

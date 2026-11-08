//! Launching items through an optional execution wrapper.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::entry::{Entry, EntryType};
use crate::item::FsKind;

pub const ITEM_ENV: &str = "HISHELL_APP_ITEM";
pub const NAME_ENV: &str = "HISHELL_APP_NAME";
pub const SCREEN_ENV: &str = "HISHELL_SCREEN";

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExecutionWrapper {
	pub command: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LaunchId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AppId(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchRequest {
	Item { path: PathBuf },
	App { path: PathBuf, app: AppId },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LaunchWarning {
	pub item: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LaunchBlockReason {
	MissingExec,
	NotExecutable,
	Unsupported(String),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LaunchOutcome {
	Started,
	NeedsConfirmation {
		id: LaunchId,
		warning: LaunchWarning,
	},
	Blocked {
		reason: LaunchBlockReason,
	},
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LaunchError {
	NotFound(PathBuf),
	NotExecutable(PathBuf),
	Exec(String),
	Spawn(String),
	Unsupported(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExitStatus {
	pub code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LaunchEvent {
	Started { id: LaunchId },
	Exited { id: LaunchId, status: ExitStatus },
	Failed { id: LaunchId, error: LaunchError },
}

/// Everything a spawner needs to start one program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnRequest {
	pub program: String,
	pub args: Vec<String>,
	pub item: PathBuf,
	pub name: String,
	pub cwd: Option<PathBuf>,
	pub animation: String,
	pub screen: Option<String>,
	pub through_wrapper: bool,
}

pub trait Spawner: Send + Sync {
	fn command_exists(&self, command: &str) -> bool;
	fn preflight(&self, wrapper: &str, request: &SpawnRequest) -> Option<String>;
	fn spawn(&self, wrapper: Option<&str>, request: &SpawnRequest) -> Result<(), LaunchError>;
	fn quit(&self, wrapper: &str, item: &Path) -> Result<(), LaunchError>;
	fn active_items(&self, wrapper: &str) -> Vec<PathBuf>;
	fn poll(&self) -> Vec<LaunchEvent> {
		Vec::new()
	}
}

pub trait Clock: Send + Sync {
	fn now_ms(&self) -> u64;
}

#[derive(Default)]
pub struct SystemClock;

impl Clock for SystemClock {
	fn now_ms(&self) -> u64 {
		SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.ok()
			.map(|duration| duration.as_millis() as u64)
			.unwrap_or(0)
	}
}

pub struct FreeDesktopSpawner;

impl Spawner for FreeDesktopSpawner {
	fn command_exists(&self, command: &str) -> bool {
		let program = command.split_whitespace().next().unwrap_or_default();
		if program.is_empty() {
			return false;
		}
		if program.contains('/') {
			return Path::new(program).exists();
		}
		std::env::var_os("PATH")
			.map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).exists()))
			.unwrap_or(false)
	}

	fn preflight(&self, wrapper: &str, request: &SpawnRequest) -> Option<String> {
		let (program, rest) = split_command(wrapper);
		let program = program?;
		let mut command = Command::new(program);
		command.args(rest);
		command.arg("--preflight");
		command.arg(&request.program);
		command.args(&request.args);
		apply_environment(&mut command, request);
		let output = command.output().ok()?;
		let message = String::from_utf8_lossy(&output.stdout).trim().to_string();
		(!message.is_empty()).then_some(message)
	}

	fn spawn(&self, wrapper: Option<&str>, request: &SpawnRequest) -> Result<(), LaunchError> {
		let mut command = match wrapper {
			Some(wrapper) if request.through_wrapper => {
				let (program, rest) = split_command(wrapper);
				let program = program
					.ok_or_else(|| LaunchError::Spawn(String::from("empty execution wrapper")))?;
				let mut command = Command::new(program);
				command.args(rest);
				command.arg(&request.program);
				command.args(&request.args);
				command
			}
			_ => {
				let mut command = Command::new(&request.program);
				command.args(&request.args);
				command
			}
		};
		apply_environment(&mut command, request);
		if let Some(cwd) = &request.cwd {
			command.current_dir(cwd);
		}
		announce_launch(&request.animation, &request.item);
		command
			.spawn()
			.map(|_| ())
			.map_err(|error| LaunchError::Spawn(error.to_string()))
	}

	fn quit(&self, wrapper: &str, item: &Path) -> Result<(), LaunchError> {
		let (program, rest) = split_command(wrapper);
		let program =
			program.ok_or_else(|| LaunchError::Spawn(String::from("empty execution wrapper")))?;
		let mut command = Command::new(program);
		command.args(rest);
		command.arg("--quit");
		command.env(ITEM_ENV, item);
		let status = command
			.status()
			.map_err(|error| LaunchError::Spawn(error.to_string()))?;
		if status.success() {
			Ok(())
		} else {
			Err(LaunchError::Spawn(format!("wrapper exited with {status}")))
		}
	}

	fn active_items(&self, wrapper: &str) -> Vec<PathBuf> {
		let (program, rest) = split_command(wrapper);
		let Some(program) = program else {
			return Vec::new();
		};
		let mut command = Command::new(program);
		command.args(rest);
		command.arg("--active-items");
		let Ok(output) = command.output() else {
			return Vec::new();
		};
		String::from_utf8_lossy(&output.stdout)
			.lines()
			.map(str::trim)
			.filter(|line| !line.is_empty())
			.map(PathBuf::from)
			.collect()
	}
}

struct ActiveLaunch {
	id: LaunchId,
	started_ms: u64,
}

pub struct Execution {
	wrapper: Option<ExecutionWrapper>,
	screen: Option<String>,
	pending: HashMap<LaunchId, SpawnRequest>,
	active: HashMap<PathBuf, ActiveLaunch>,
	spawner: Arc<dyn Spawner>,
	clock: Arc<dyn Clock>,
	next_id: u64,
	last_poll: u64,
}

impl Execution {
	pub fn new(
		wrapper: Option<ExecutionWrapper>,
		screen: Option<String>,
		spawner: Arc<dyn Spawner>,
		clock: Arc<dyn Clock>,
	) -> Self {
		Self {
			wrapper,
			screen,
			pending: HashMap::new(),
			active: HashMap::new(),
			spawner,
			clock,
			next_id: 1,
			last_poll: 0,
		}
	}

	pub fn free_desktop(wrapper: Option<ExecutionWrapper>, screen: Option<String>) -> Self {
		Self::new(
			wrapper,
			screen,
			Arc::new(FreeDesktopSpawner),
			Arc::new(SystemClock),
		)
	}

	pub fn set_wrapper(&mut self, wrapper: Option<ExecutionWrapper>) {
		self.wrapper = wrapper;
	}

	pub fn set_screen(&mut self, screen: Option<String>) {
		self.screen = screen;
	}

	pub fn screen(&self) -> Option<&str> {
		self.screen.as_deref()
	}

	pub fn request(&mut self, request: LaunchRequest) -> Result<LaunchOutcome, LaunchError> {
		let request = self.spawn_request(request)?;
		self.request_spawn(request)
	}

	/// Ask to start an already-resolved program. The wrapper, preflight,
	/// screen, and launch marker are the same as for an entry launch, so a
	/// frontend with its own command line does not reimplement them.
	pub fn request_spawn(
		&mut self,
		mut request: SpawnRequest,
	) -> Result<LaunchOutcome, LaunchError> {
		if request.screen.is_none() {
			request.screen = self.screen.clone();
		}
		let wrapper = self.usable_wrapper();
		if request.through_wrapper
			&& let Some(wrapper) = wrapper.as_deref()
			&& let Some(message) = self.spawner.preflight(wrapper, &request)
		{
			let id = self.next_launch_id();
			self.pending.insert(id, request);
			return Ok(LaunchOutcome::NeedsConfirmation {
				id,
				warning: LaunchWarning {
					item: PathBuf::from(message),
				},
			});
		}
		self.start(request, wrapper.as_deref())?;
		Ok(LaunchOutcome::Started)
	}

	pub fn confirm(&mut self, id: LaunchId) -> Result<(), LaunchError> {
		let Some(request) = self.pending.remove(&id) else {
			return Err(LaunchError::NotFound(PathBuf::from("pending launch")));
		};
		let wrapper = self.usable_wrapper();
		self.start(request, wrapper.as_deref())
	}

	pub fn cancel(&mut self, id: LaunchId) {
		self.pending.remove(&id);
	}

	pub fn quit(&mut self, item: &Path) -> Result<(), LaunchError> {
		self.active.remove(item);
		match self.usable_wrapper() {
			Some(wrapper) => self.spawner.quit(&wrapper, item),
			None => Ok(()),
		}
	}

	pub fn active_items(&self) -> Vec<PathBuf> {
		match self.usable_wrapper() {
			Some(wrapper) => self.spawner.active_items(&wrapper),
			None => self.active.keys().cloned().collect(),
		}
	}

	pub fn is_active(&self, item: &Path) -> bool {
		self.active.contains_key(item)
	}

	pub fn active_launch(&self, item: &Path) -> Option<(LaunchId, u64)> {
		self.active
			.get(item)
			.map(|active| (active.id, active.started_ms))
	}

	pub fn poll(&mut self) -> Vec<LaunchEvent> {
		self.last_poll = self.clock.now_ms();
		self.spawner.poll()
	}

	pub fn last_poll(&self) -> u64 {
		self.last_poll
	}

	fn spawn_request(&self, request: LaunchRequest) -> Result<SpawnRequest, LaunchError> {
		match request {
			LaunchRequest::Item { path } => {
				let directory = path.is_dir();
				let entry_path = if directory {
					path.join(".directory")
				} else {
					path.clone()
				};
				let kind = if directory {
					FsKind::Directory
				} else {
					FsKind::File
				};
				let Some(entry) = Entry::read(&entry_path, kind) else {
					return Err(LaunchError::NotExecutable(path));
				};
				self.entry_request(&entry, &path, None)
			}
			LaunchRequest::App { path, app } => {
				let entry_path = PathBuf::from(&app.0);
				let Some(entry) = Entry::read(&entry_path, FsKind::File) else {
					return Err(LaunchError::NotFound(entry_path));
				};
				self.entry_request(&entry, &entry_path, Some(&path))
			}
		}
	}

	fn entry_request(
		&self,
		entry: &Entry,
		item: &Path,
		file: Option<&Path>,
	) -> Result<SpawnRequest, LaunchError> {
		let file = match entry.entry_type {
			Some(EntryType::Application)
			| Some(EntryType::Button)
			| Some(EntryType::Toggle)
			| Some(EntryType::Number)
			| Some(EntryType::Radio) => file.map(Path::to_path_buf),
			Some(EntryType::File) => Some(
				file.or(entry.target.as_deref())
					.unwrap_or(item)
					.to_path_buf(),
			),
			Some(EntryType::Link) | Some(EntryType::Directory) | None => {
				return Err(LaunchError::NotExecutable(item.to_path_buf()));
			}
		};
		let args = entry
			.exec_args(file.as_deref())
			.map_err(LaunchError::Exec)?;
		let Some(program) = args.first().cloned() else {
			return Err(LaunchError::Exec(String::from("empty Exec")));
		};
		let through_wrapper = matches!(
			entry.entry_type,
			Some(EntryType::Application | EntryType::File | EntryType::Link)
		);
		Ok(SpawnRequest {
			program,
			args: args[1..].to_vec(),
			item: file.unwrap_or_else(|| item.to_path_buf()),
			name: entry
				.name
				.clone()
				.or_else(|| {
					item.file_name()
						.map(|name| name.to_string_lossy().into_owned())
				})
				.unwrap_or_default(),
			cwd: entry.path.parent().map(Path::to_path_buf),
			animation: entry.launch_animation.clone().unwrap_or_default(),
			screen: self.screen.clone(),
			through_wrapper,
		})
	}

	fn usable_wrapper(&self) -> Option<String> {
		let wrapper = self.wrapper.as_ref()?;
		let command = wrapper.command.trim();
		(!command.is_empty() && self.spawner.command_exists(command)).then(|| command.to_string())
	}

	fn start(&mut self, request: SpawnRequest, wrapper: Option<&str>) -> Result<(), LaunchError> {
		self.spawner.spawn(wrapper, &request)?;
		let id = self.next_launch_id();
		if !request.item.as_os_str().is_empty() {
			self.active.insert(
				request.item.clone(),
				ActiveLaunch {
					id,
					started_ms: self.clock.now_ms(),
				},
			);
		}
		Ok(())
	}

	fn next_launch_id(&mut self) -> LaunchId {
		let id = LaunchId(self.next_id);
		self.next_id += 1;
		id
	}
}

fn split_command(command: &str) -> (Option<String>, Vec<String>) {
	let mut parts = command.split_whitespace().map(String::from);
	let program = parts.next();
	(program, parts.collect())
}

fn apply_environment(command: &mut Command, request: &SpawnRequest) {
	if !request.item.as_os_str().is_empty() {
		command.env(ITEM_ENV, &request.item);
	}
	if !request.name.is_empty() {
		command.env(NAME_ENV, &request.name);
	}
	if let Some(screen) = request
		.screen
		.as_deref()
		.filter(|screen| !screen.is_empty())
	{
		command.env(SCREEN_ENV, screen);
	}
}

fn announce_launch(animation: &str, item: &Path) {
	let animation = animation.trim();
	let colour = match animation {
		"white" => "white",
		"black" => "black",
		_ => "none",
	};
	let path = match std::env::var("CLAM_LAUNCH_MARKER") {
		Ok(path) if !path.is_empty() => PathBuf::from(path),
		_ => {
			let dir = std::env::var("CLAM_APP_MANAGER_DIR")
				.unwrap_or_else(|_| String::from("/tmp/clam-app-manager"));
			PathBuf::from(dir).join("launch")
		}
	};
	if let Some(parent) = path.parent() {
		let _ = std::fs::create_dir_all(parent);
	}
	let nonce = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|elapsed| elapsed.as_nanos())
		.unwrap_or(0);
	let text = format!("{colour}\n{nonce}\n{}\n", item.to_string_lossy());
	let temporary = path.with_extension("tmp");
	if std::fs::write(&temporary, text).is_ok() {
		let _ = std::fs::rename(&temporary, path);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::Mutex;

	#[derive(Default)]
	struct FakeSpawner {
		exists: bool,
		preflight: Mutex<Option<String>>,
		spawned: Mutex<Vec<(Option<String>, SpawnRequest)>>,
	}

	impl Spawner for FakeSpawner {
		fn command_exists(&self, _command: &str) -> bool {
			self.exists
		}

		fn preflight(&self, _wrapper: &str, _request: &SpawnRequest) -> Option<String> {
			self.preflight.lock().unwrap().clone()
		}

		fn spawn(&self, wrapper: Option<&str>, request: &SpawnRequest) -> Result<(), LaunchError> {
			self.spawned
				.lock()
				.unwrap()
				.push((wrapper.map(String::from), request.clone()));
			Ok(())
		}

		fn quit(&self, _wrapper: &str, _item: &Path) -> Result<(), LaunchError> {
			Ok(())
		}

		fn active_items(&self, _wrapper: &str) -> Vec<PathBuf> {
			Vec::new()
		}
	}

	fn tree(name: &str) -> PathBuf {
		let root =
			std::env::temp_dir().join(format!("hishell-execution-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		root
	}

	fn app_entry(root: &Path) -> PathBuf {
		let path = root.join("ride.desktop");
		std::fs::write(
			&path,
			"[Desktop Entry]\nType=Application\nName=Ride\nExec=ride %f\n",
		)
		.unwrap();
		path
	}

	#[test]
	fn a_launch_starts_the_entry_without_a_wrapper() {
		let root = tree("plain");
		let entry = app_entry(&root);
		let spawner = Arc::new(FakeSpawner::default());
		let mut execution = Execution::new(None, None, spawner.clone(), Arc::new(SystemClock));
		let outcome = execution
			.request(LaunchRequest::Item { path: entry })
			.unwrap();
		assert_eq!(outcome, LaunchOutcome::Started);
		let spawned = spawner.spawned.lock().unwrap();
		assert_eq!(spawned.len(), 1);
		assert_eq!(spawned[0].0, None);
		assert_eq!(spawned[0].1.program, "ride");
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_wrapper_can_hold_a_launch_for_confirmation() {
		let root = tree("wrapper");
		let entry = app_entry(&root);
		let spawner = Arc::new(FakeSpawner {
			exists: true,
			..FakeSpawner::default()
		});
		*spawner.preflight.lock().unwrap() = Some(String::from("/other/app"));
		let mut execution = Execution::new(
			Some(ExecutionWrapper {
				command: String::from("wrapper"),
			}),
			None,
			spawner.clone(),
			Arc::new(SystemClock),
		);
		let outcome = execution
			.request(LaunchRequest::Item { path: entry })
			.unwrap();
		let LaunchOutcome::NeedsConfirmation { id, warning } = outcome else {
			panic!("expected a confirmation");
		};
		assert_eq!(warning.item, PathBuf::from("/other/app"));
		assert!(spawner.spawned.lock().unwrap().is_empty());
		execution.confirm(id).unwrap();
		let spawned = spawner.spawned.lock().unwrap();
		assert_eq!(spawned[0].0.as_deref(), Some("wrapper"));
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn cancelling_a_pending_launch_does_not_spawn() {
		let root = tree("cancel");
		let entry = app_entry(&root);
		let spawner = Arc::new(FakeSpawner {
			exists: true,
			..FakeSpawner::default()
		});
		*spawner.preflight.lock().unwrap() = Some(String::from("/other/app"));
		let mut execution = Execution::new(
			Some(ExecutionWrapper {
				command: String::from("wrapper"),
			}),
			None,
			spawner.clone(),
			Arc::new(SystemClock),
		);
		let LaunchOutcome::NeedsConfirmation { id, .. } = execution
			.request(LaunchRequest::Item { path: entry })
			.unwrap()
		else {
			panic!("expected a confirmation");
		};
		execution.cancel(id);
		assert!(spawner.spawned.lock().unwrap().is_empty());
		std::fs::remove_dir_all(root).unwrap();
	}
}

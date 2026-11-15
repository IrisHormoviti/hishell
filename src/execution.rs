//! Launching items through an optional execution wrapper.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config_parser::{ConfigParser, ConfigValue};
use crate::entry::{Entry, EntryType};
use crate::item::FsKind;

pub const ITEM_ENV: &str = "HISHELL_APP_ITEM";
pub const NAME_ENV: &str = "HISHELL_APP_NAME";

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
	Item {
		path: PathBuf,
	},
	App {
		path: PathBuf,
		app: AppId,
	},
	/// Run a `Type=Toggle` or `Type=Number` provider command. An empty
	/// `values` means the toggle target (on/off); otherwise `%1`, `%2`, …
	/// are replaced by the dialog values.
	Control {
		path: PathBuf,
		values: Vec<i64>,
	},
	/// Run a `Type=Text` provider command. `%v` is replaced by the text the
	/// user entered.
	Text {
		path: PathBuf,
		value: String,
	},
	/// Run a folder's `[Radio] Exec` after a choice is selected. `%v`, `%n`
	/// and `%p` expand to the value, display name and path.
	Radio {
		exec: String,
		value: String,
		name: String,
		path: PathBuf,
	},
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
	/// Whether this spawn tells the compositor a launch is starting. Control,
	/// text and radio commands change a setting; they are not launches, so they
	/// must not wash the screens or hold the shell down.
	pub announce: bool,
	/// `[App Display] MainDisplay`; becomes the compositor's panel hint.
	pub main_display: Option<i32>,
	pub through_wrapper: bool,
	/// The item's own `[Execution] ExecutionWrapper`, when it sets one. It
	/// replaces the engine's wrapper for this launch.
	pub wrapper_override: Option<String>,
	/// Whether this launch represents a running application, rather than a
	/// short-lived provider command.
	pub track_active: bool,
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

/// Host-specific side effects around a launch.
///
/// The core is host-agnostic: it knows a program's panel hint and launch
/// animation, but not how a particular session expresses them. A frontend
/// supplies this to add environment, announce a launch for a compositor, or
/// withdraw that announcement when the spawn fails.
pub trait LaunchHook: Send + Sync {
	/// Extra environment for the spawned program.
	fn environment(&self, request: &SpawnRequest) -> Vec<(String, String)> {
		let _ = request;
		Vec::new()
	}

	/// Called when an announced launch starts. Returns a marker path the hook
	/// wrote, so a failed spawn can withdraw it.
	fn announce(&self, request: &SpawnRequest) -> Option<PathBuf> {
		let _ = request;
		None
	}

	/// Withdraw an announcement whose spawn failed.
	fn withdraw(&self, marker: &Path) {
		let _ = std::fs::remove_file(marker);
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

/// The default spawner. It runs the program directly, or through the
/// configured wrapper, and applies the frontend's [`LaunchHook`].
#[derive(Default)]
pub struct FreeDesktopSpawner {
	hook: Option<Arc<dyn LaunchHook>>,
}

impl FreeDesktopSpawner {
	pub fn new(hook: Option<Arc<dyn LaunchHook>>) -> Self {
		Self { hook }
	}
}

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
		apply_environment(&mut command, request, self.hook.as_ref());
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
		apply_environment(&mut command, request, self.hook.as_ref());
		if let Some(cwd) = &request.cwd {
			command.current_dir(cwd);
		}
		let marker = if request.announce {
			self.hook.as_ref().and_then(|hook| hook.announce(request))
		} else {
			None
		};
		match command.spawn() {
			Ok(_) => Ok(()),
			Err(error) => {
				// A hook that announced the launch withdraws it, so a
				// synchronous spawn failure does not leave the host waiting.
				if let (Some(hook), Some(marker)) = (self.hook.as_ref(), marker.as_ref()) {
					hook.withdraw(marker);
				}
				Err(LaunchError::Spawn(error.to_string()))
			}
		}
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
		spawner: Arc<dyn Spawner>,
		clock: Arc<dyn Clock>,
	) -> Self {
		Self {
			wrapper,
			pending: HashMap::new(),
			active: HashMap::new(),
			spawner,
			clock,
			next_id: 1,
			last_poll: 0,
		}
	}

	pub fn free_desktop(wrapper: Option<ExecutionWrapper>) -> Self {
		Self::new(
			wrapper,
			Arc::new(FreeDesktopSpawner::default()),
			Arc::new(SystemClock),
		)
	}

	pub fn set_wrapper(&mut self, wrapper: Option<ExecutionWrapper>) {
		self.wrapper = wrapper;
	}

	pub fn request(&mut self, request: LaunchRequest) -> Result<LaunchOutcome, LaunchError> {
		let request = self.spawn_request(request)?;
		self.request_spawn(request)
	}

	/// Ask to start an already-resolved program. The wrapper, preflight, and
	/// launch marker are the same as for an entry launch, so a frontend with
	/// its own command line does not reimplement them.
	pub fn request_spawn(&mut self, request: SpawnRequest) -> Result<LaunchOutcome, LaunchError> {
		let wrapper = self.wrapper_for(&request);
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
		let wrapper = self.wrapper_for(&request);
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
			LaunchRequest::Control { path, values } => {
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
				self.control_request(&entry, &path, &values)
			}
			LaunchRequest::Text { path, value } => {
				let entry_path = if path.is_dir() {
					path.join(".directory")
				} else {
					path.clone()
				};
				let kind = if path.is_dir() {
					FsKind::Directory
				} else {
					FsKind::File
				};
				let Some(entry) = Entry::read(&entry_path, kind) else {
					return Err(LaunchError::NotExecutable(path));
				};
				self.text_request(&entry, &path, &value)
			}
			LaunchRequest::Radio {
				exec,
				value,
				name,
				path,
			} => {
				let tokens = crate::entry::tokenize_exec(&exec).map_err(LaunchError::Exec)?;
				let Some(program) = tokens.first().cloned() else {
					return Err(LaunchError::Exec(String::from("empty Exec")));
				};
				let item = path.to_string_lossy().into_owned();
				let args = tokens[1..]
					.iter()
					.map(|argument| {
						argument
							.replace("%v", &value)
							.replace("%n", &name)
							.replace("%p", &item)
					})
					.collect();
				Ok(SpawnRequest {
					program,
					args,
					item: path.clone(),
					name,
					cwd: path.parent().map(Path::to_path_buf),
					animation: String::new(),
					announce: false,
					main_display: None,
					through_wrapper: false,
					wrapper_override: None,
					track_active: false,
				})
			}
		}
	}

	fn terminal_command(&self) -> (String, Vec<String>) {
		if let Some(value) = std::env::var("TERMINAL")
			.ok()
			.filter(|value| !value.trim().is_empty())
			&& let Some((program, args)) = terminal_config_command(&value)
			&& self.spawner.command_exists(&program)
		{
			return (program, args);
		}
		if let Some(value) = kde_terminal_application()
			&& let Some((program, args)) = terminal_config_command(&value)
			&& self.spawner.command_exists(&program)
		{
			return (program, args);
		}
		for candidate in ["konsole", "x-terminal-emulator", "xterm"] {
			if self.spawner.command_exists(candidate) {
				return (candidate.to_string(), Vec::new());
			}
		}
		(String::from("konsole"), Vec::new())
	}

	fn terminal_request(
		&self,
		program: &str,
		args: &[String],
		options: Option<&str>,
	) -> (String, Vec<String>) {
		let (terminal, terminal_args) = self.terminal_command();
		let options: Vec<String> = options
			.map(|value| value.split_whitespace().map(String::from).collect())
			.unwrap_or_default();
		terminal_launch(&terminal, terminal_args, &options, program, args)
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
			| Some(EntryType::Text)
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
		let (program, args) = if entry.terminal {
			self.terminal_request(&program, &args[1..], entry.terminal_options.as_deref())
		} else {
			(program, args[1..].to_vec())
		};
		let through_wrapper = matches!(
			entry.entry_type,
			Some(EntryType::Application | EntryType::File | EntryType::Link)
		);
		// An app that does not ask for a wash still gets one. Upstream entries
		// rarely set LaunchAnimation, and without the key the compositor wrote
		// "none" and left the shell visible until the app mapped a window.
		// `LaunchAnimation=none` is still the explicit opt-out.
		let animation = entry
			.launch_animation
			.clone()
			.filter(|value| !value.trim().is_empty())
			.unwrap_or_else(|| {
				if through_wrapper {
					String::from("black")
				} else {
					String::new()
				}
			});
		Ok(SpawnRequest {
			program,
			args,
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
			animation,
			announce: true,
			main_display: entry.main_display,
			through_wrapper,
			wrapper_override: entry_execution_wrapper(&entry.path),
			track_active: true,
		})
	}

	fn control_request(
		&self,
		entry: &Entry,
		item: &Path,
		values: &[i64],
	) -> Result<SpawnRequest, LaunchError> {
		let args = entry.control_args(values).map_err(LaunchError::Exec)?;
		let Some(program) = args.first().cloned() else {
			return Err(LaunchError::Exec(String::from("empty Exec")));
		};
		Ok(SpawnRequest {
			program,
			args: args[1..].to_vec(),
			item: item.to_path_buf(),
			name: entry
				.name
				.clone()
				.or_else(|| {
					item.file_name()
						.map(|name| name.to_string_lossy().into_owned())
				})
				.unwrap_or_default(),
			cwd: entry.path.parent().map(Path::to_path_buf),
			animation: String::new(),
			announce: false,
			main_display: entry.main_display,
			// Controls talk to their provider directly; the execution wrapper
			// is for launching applications.
			through_wrapper: false,
			wrapper_override: None,
			track_active: false,
		})
	}

	fn text_request(
		&self,
		entry: &Entry,
		item: &Path,
		value: &str,
	) -> Result<SpawnRequest, LaunchError> {
		let args = entry.text_args(value).map_err(LaunchError::Exec)?;
		let Some(program) = args.first().cloned() else {
			return Err(LaunchError::Exec(String::from("empty Exec")));
		};
		Ok(SpawnRequest {
			program,
			args: args[1..].to_vec(),
			item: item.to_path_buf(),
			name: entry
				.name
				.clone()
				.or_else(|| {
					item.file_name()
						.map(|name| name.to_string_lossy().into_owned())
				})
				.unwrap_or_default(),
			cwd: entry.path.parent().map(Path::to_path_buf),
			animation: String::new(),
			announce: false,
			main_display: entry.main_display,
			through_wrapper: false,
			wrapper_override: None,
			track_active: false,
		})
	}

	fn usable_wrapper(&self) -> Option<String> {
		let wrapper = self.wrapper.as_ref()?;
		let command = wrapper.command.trim();
		(!command.is_empty() && self.spawner.command_exists(command)).then(|| command.to_string())
	}

	/// The wrapper for one launch: the item's own `[Execution] ExecutionWrapper`
	/// when it sets a usable one, otherwise the engine's.
	fn wrapper_for(&self, request: &SpawnRequest) -> Option<String> {
		if let Some(wrapper) = request
			.wrapper_override
			.as_deref()
			.map(str::trim)
			.filter(|wrapper| !wrapper.is_empty())
			&& self.spawner.command_exists(wrapper)
		{
			return Some(wrapper.to_string());
		}
		self.usable_wrapper()
	}

	fn start(&mut self, request: SpawnRequest, wrapper: Option<&str>) -> Result<(), LaunchError> {
		self.spawner.spawn(wrapper, &request)?;
		let id = self.next_launch_id();
		if request.track_active && !request.item.as_os_str().is_empty() {
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

/// Wrap a command in a terminal emulator. `TerminalOptions` are inserted
/// before the emulator's command separator, as KDE's terminal launcher does.
fn terminal_launch(
	terminal: &str,
	terminal_args: Vec<String>,
	options: &[String],
	program: &str,
	args: &[String],
) -> (String, Vec<String>) {
	let mut arguments = terminal_args;
	arguments.extend(options.iter().cloned());
	let name = Path::new(terminal)
		.file_name()
		.and_then(|name| name.to_str())
		.unwrap_or(terminal);
	match name {
		"gnome-terminal" | "kgx" | "ptyxis" => arguments.push(String::from("--")),
		"wezterm" => {
			arguments.push(String::from("start"));
			arguments.push(String::from("--"));
		}
		_ => arguments.push(String::from("-e")),
	}
	arguments.push(String::from(program));
	arguments.extend(args.iter().cloned());
	(terminal.to_string(), arguments)
}

/// KDE's configured terminal, from `$XDG_CONFIG_HOME/kdeglobals` over the
/// system defaults. `TerminalApplication` is the command; older installs used
/// `TerminalService`, which can be a desktop file id.
fn kde_terminal_application() -> Option<String> {
	let mut files = Vec::new();
	let system_dirs = std::env::var_os("XDG_CONFIG_DIRS")
		.map(|dirs| std::env::split_paths(&dirs).collect::<Vec<_>>())
		.filter(|dirs| !dirs.is_empty())
		.unwrap_or_else(|| vec![PathBuf::from("/etc/xdg")]);
	for dir in system_dirs {
		files.push(dir.join("kdeglobals"));
	}
	if let Some(config) = dirs::config_dir() {
		files.push(config.join("kdeglobals"));
	}
	let paths: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
	let parsed = ConfigParser::parse_files(&paths);
	let general = parsed.sections.get("General")?;
	for key in ["TerminalApplication", "TerminalService"] {
		if let Some(ConfigValue::String(value)) = general.get(key) {
			let value = value.trim();
			if !value.is_empty() {
				return Some(value.to_string());
			}
		}
	}
	None
}

fn terminal_config_command(value: &str) -> Option<(String, Vec<String>)> {
	let value = value.trim();
	if value.is_empty() {
		return None;
	}
	if value.ends_with(".desktop") {
		let stem = Path::new(value).file_stem()?.to_str()?;
		// `org.kde.konsole.desktop` names the `konsole` executable.
		let program = stem.rsplit('.').next().unwrap_or(stem);
		return Some((program.to_string(), Vec::new()));
	}
	let (program, args) = split_command(value);
	program.map(|program| (program, args))
}

fn split_command(command: &str) -> (Option<String>, Vec<String>) {
	let mut parts = command.split_whitespace().map(String::from);
	let program = parts.next();
	(program, parts.collect())
}

/// The `[Execution] ExecutionWrapper` an item sets for its own launch.
///
/// It is read from the item's own entry — a folder's `.directory`, a file, or
/// an application's `.desktop` — so an application can override the wrapper
/// without touching the session-wide configuration. System and user config are
/// not consulted here; the engine's own wrapper is the fallback.
fn entry_execution_wrapper(entry_path: &Path) -> Option<String> {
	if entry_path.as_os_str().is_empty() {
		return None;
	}
	let text = std::fs::read_to_string(entry_path).ok()?;
	let (sections, _) = ConfigParser::parse_str(&text, &entry_path.to_string_lossy());
	let value = sections.get("Execution")?.get("ExecutionWrapper")?;
	let wrapper = match value {
		ConfigValue::String(value) => value.clone(),
		other => other.to_json_string(),
	};
	(!wrapper.trim().is_empty()).then_some(wrapper)
}

fn apply_environment(
	command: &mut Command,
	request: &SpawnRequest,
	hook: Option<&Arc<dyn LaunchHook>>,
) {
	if !request.item.as_os_str().is_empty() {
		command.env(ITEM_ENV, &request.item);
	}
	if !request.name.is_empty() {
		command.env(NAME_ENV, &request.name);
	}
	if let Some(hook) = hook {
		for (key, value) in hook.environment(request) {
			command.env(key, value);
		}
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
		let mut execution = Execution::new(None, spawner.clone(), Arc::new(SystemClock));
		let outcome = execution
			.request(LaunchRequest::Item { path: entry })
			.unwrap();
		assert_eq!(outcome, LaunchOutcome::Started);
		let spawned = spawner.spawned.lock().unwrap();
		assert_eq!(spawned.len(), 1);
		assert_eq!(spawned[0].0, None);
		assert_eq!(spawned[0].1.program, "ride");
		assert!(
			spawned[0].1.announce,
			"an application launch reaches the compositor"
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_control_runs_its_provider_without_the_wrapper() {
		let root = tree("control");
		let entry = root.join("volume.desktop");
		std::fs::write(
			&entry,
			"[Desktop Entry]\nType=Number\nName=Volume\nExec=set %1 %2\n",
		)
		.unwrap();
		let spawner = Arc::new(FakeSpawner {
			exists: true,
			..FakeSpawner::default()
		});
		let mut execution = Execution::new(
			Some(ExecutionWrapper {
				command: String::from("wrapper"),
			}),
			spawner.clone(),
			Arc::new(SystemClock),
		);
		let outcome = execution
			.request(LaunchRequest::Control {
				path: entry,
				values: vec![3, 7],
			})
			.unwrap();
		assert_eq!(outcome, LaunchOutcome::Started);
		let spawned = spawner.spawned.lock().unwrap();
		assert_eq!(spawned.len(), 1);
		assert!(!spawned[0].1.through_wrapper);
		assert!(!spawned[0].1.track_active);
		assert!(
			!spawned[0].1.announce,
			"a control must not wash the screens"
		);
		assert_eq!(spawned[0].1.program, "set");
		assert_eq!(spawned[0].1.args, vec!["3", "7"]);
		assert!(execution.active_items().is_empty());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_radio_runs_the_folder_exec_with_placeholders() {
		let root = tree("radio");
		let choice = root.join("one");
		std::fs::create_dir_all(&choice).unwrap();
		let spawner = Arc::new(FakeSpawner::default());
		let mut execution = Execution::new(None, spawner.clone(), Arc::new(SystemClock));
		let outcome = execution
			.request(LaunchRequest::Radio {
				exec: String::from("provider %v %n %p"),
				value: String::from("one"),
				name: String::from("One"),
				path: choice.clone(),
			})
			.unwrap();
		assert_eq!(outcome, LaunchOutcome::Started);
		let spawned = spawner.spawned.lock().unwrap();
		assert_eq!(spawned.len(), 1);
		assert_eq!(spawned[0].1.program, "provider");
		assert_eq!(
			spawned[0].1.args,
			vec!["one", "One", choice.to_string_lossy().as_ref()]
		);
		assert!(!spawned[0].1.track_active);
		assert!(
			!spawned[0].1.announce,
			"a radio change must not wash the screens"
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_text_command_does_not_announce_a_launch() {
		let root = tree("text");
		let entry = root.join("name.desktop");
		std::fs::write(
			&entry,
			"[Desktop Entry]\nType=Text\nName=Name\nExec=set-name %v\n",
		)
		.unwrap();
		let spawner = Arc::new(FakeSpawner::default());
		let mut execution = Execution::new(None, spawner.clone(), Arc::new(SystemClock));
		let outcome = execution
			.request(LaunchRequest::Text {
				path: entry,
				value: String::from("Ada"),
			})
			.unwrap();
		assert_eq!(outcome, LaunchOutcome::Started);
		let spawned = spawner.spawned.lock().unwrap();
		assert_eq!(spawned[0].1.args, vec!["Ada"]);
		assert!(!spawned[0].1.announce);
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
	fn terminal_launch_uses_the_emulator_separator() {
		let (program, args) = terminal_launch(
			"konsole",
			Vec::new(),
			&[String::from("--hold")],
			"micro",
			&[String::from("notes.txt")],
		);
		assert_eq!(program, "konsole");
		assert_eq!(args, vec!["--hold", "-e", "micro", "notes.txt"]);

		let (_, args) = terminal_launch("gnome-terminal", Vec::new(), &[], "micro", &[]);
		assert_eq!(args, vec!["--", "micro"]);

		let (_, args) = terminal_launch("wezterm", Vec::new(), &[], "micro", &[]);
		assert_eq!(args, vec!["start", "--", "micro"]);
	}

	#[test]
	fn the_launch_hook_supplies_environment() {
		struct PanelHook;
		impl LaunchHook for PanelHook {
			fn environment(&self, request: &SpawnRequest) -> Vec<(String, String)> {
				request
					.main_display
					.filter(|display| *display >= 0)
					.map(|display| vec![(String::from("APP_PANEL"), display.to_string())])
					.unwrap_or_default()
			}
		}
		let hook: Arc<dyn LaunchHook> = Arc::new(PanelHook);
		let mut command = Command::new("true");
		apply_environment(
			&mut command,
			&SpawnRequest {
				program: String::from("true"),
				args: Vec::new(),
				item: PathBuf::new(),
				name: String::new(),
				cwd: None,
				animation: String::new(),
				announce: false,
				main_display: Some(0),
				through_wrapper: false,
				wrapper_override: None,
				track_active: false,
			},
			Some(&hook),
		);
		let value = command
			.get_envs()
			.find(|(key, _)| *key == "APP_PANEL")
			.and_then(|(_, value)| value)
			.map(|value| value.to_string_lossy().into_owned());
		assert_eq!(value.as_deref(), Some("0"));
	}

	#[test]
	fn an_entry_overrides_the_execution_wrapper() {
		let root = tree("entry-wrapper");
		let entry = root.join("viewer.desktop");
		std::fs::write(
			&entry,
			"[Desktop Entry]\nType=Application\nName=Viewer\nExec=viewer %f\n\n\
			 [Execution]\nExecutionWrapper=applet-wrapper\n",
		)
		.unwrap();
		let spawner = Arc::new(FakeSpawner {
			exists: true,
			..FakeSpawner::default()
		});
		let mut execution = Execution::new(
			Some(ExecutionWrapper {
				command: String::from("guest-wrapper"),
			}),
			spawner.clone(),
			Arc::new(SystemClock),
		);
		let outcome = execution
			.request(LaunchRequest::Item { path: entry })
			.unwrap();
		assert_eq!(outcome, LaunchOutcome::Started);
		let spawned = spawner.spawned.lock().unwrap();
		assert_eq!(spawned.len(), 1);
		assert_eq!(spawned[0].0.as_deref(), Some("applet-wrapper"));
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn an_entry_without_a_wrapper_uses_the_engine_one() {
		let root = tree("entry-no-wrapper");
		let entry = app_entry(&root);
		let spawner = Arc::new(FakeSpawner {
			exists: true,
			..FakeSpawner::default()
		});
		let mut execution = Execution::new(
			Some(ExecutionWrapper {
				command: String::from("guest-wrapper"),
			}),
			spawner.clone(),
			Arc::new(SystemClock),
		);
		execution
			.request(LaunchRequest::Item { path: entry })
			.unwrap();
		let spawned = spawner.spawned.lock().unwrap();
		assert_eq!(spawned[0].0.as_deref(), Some("guest-wrapper"));
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

//! Freedesktop desktop-entry metadata for folder items.

use std::path::{Path, PathBuf};

use crate::config::ConfigDiagnostic;
use crate::config_parser::{ConfigParser, ConfigValue};
use crate::item::{FsKind, IconRef, ItemKind};
use crate::path;

const MAX_ENTRY_LEN: u64 = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EntryType {
	Application,
	Link,
	Directory,
	Button,
	Toggle,
	Number,
	Radio,
	File,
}

impl EntryType {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::Application => "Application",
			Self::Link => "Link",
			Self::Directory => "Directory",
			Self::Button => "Button",
			Self::Toggle => "Toggle",
			Self::Number => "Number",
			Self::Radio => "Radio",
			Self::File => "File",
		}
	}

	pub fn from_key(value: &str) -> Option<Self> {
		match value.trim().to_ascii_lowercase().as_str() {
			"application" => Some(Self::Application),
			"link" => Some(Self::Link),
			"directory" => Some(Self::Directory),
			"button" => Some(Self::Button),
			"toggle" => Some(Self::Toggle),
			"number" => Some(Self::Number),
			"radio" => Some(Self::Radio),
			"file" => Some(Self::File),
			_ => None,
		}
	}

	pub fn item_kind(self) -> ItemKind {
		match self {
			Self::Application => ItemKind::Application,
			Self::Link => ItemKind::Link,
			Self::Directory => ItemKind::Directory,
			Self::Button => ItemKind::Button,
			Self::Toggle => ItemKind::Toggle,
			Self::Number => ItemKind::Number,
			Self::Radio => ItemKind::Radio,
			Self::File => ItemKind::File,
		}
	}
}

/// One labelled integer a `Type=Number` dialog edits.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NumberField {
	pub label: String,
	pub value: i64,
	pub min: i64,
	pub max: i64,
	pub default: i64,
}

/// The metadata a desktop entry supplies for one item.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
	pub path: PathBuf,
	pub entry_type: Option<EntryType>,
	/// `Path=` or a file `URL=`, resolved against the entry file.
	pub target: Option<PathBuf>,
	pub mime: Option<String>,
	pub name: Option<String>,
	pub comment: Option<String>,
	pub icon: Option<String>,
	pub exec: Option<String>,
	/// `Terminal=true`: run the command in a terminal emulator window.
	pub terminal: bool,
	/// Extra arguments passed to the terminal before the command.
	pub terminal_options: Option<String>,
	pub launch_animation: Option<String>,
	/// `[App Display] MainDisplay`: the panel this app's first window goes to.
	/// 0 is the top panel, 1 the bottom; absent or -1 uses the system default.
	pub main_display: Option<i32>,
	pub hidden: bool,
	pub no_display: bool,
	pub toggle: bool,
	pub number_value: String,
	pub number_fields: Vec<NumberField>,
	/// Written by a provider daemon rather than the user.
	pub generated: bool,
	/// Provider actions for a generated entry or section.
	pub destroy: Option<String>,
	pub reset: Option<String>,
	pub rename: Option<String>,
	pub refresh: Option<String>,
	pub custom_group: Option<String>,
	/// A live task described by the entry's `[Progress]` group.
	pub progress: Option<crate::progress::Progress>,
	pub diagnostics: Vec<ConfigDiagnostic>,
}

impl Entry {
	/// Read the `[Desktop Entry]` section of an entry file. `fs_kind` is the
	/// kind of the thing the entry is attached to, not the entry file itself.
	pub fn read(path: &Path, fs_kind: FsKind) -> Option<Self> {
		if !is_entry_path(path) {
			return None;
		}
		let metadata = std::fs::metadata(path).ok()?;
		if metadata.len() > MAX_ENTRY_LEN {
			return None;
		}
		let text = std::fs::read_to_string(path).ok()?;
		let (sections, _) = ConfigParser::parse_str(&text, &path.to_string_lossy());
		let values = sections.get("Desktop Entry")?;

		let mut diagnostics = Vec::new();
		let string = |key: &str| value_string(values.get(key));
		let entry_type = match string("Type") {
			Some(value) => match EntryType::from_key(&value) {
				Some(entry_type) => Some(entry_type),
				None => {
					diagnostics.push(diagnostic(path, format!("unknown entry type {value:?}")));
					None
				}
			},
			None => None,
		};

		let mut target = None;
		if let Some(value) = string("Path").filter(|value| !value.is_empty()) {
			target = Some(resolve_entry_path(path, &value));
		} else if let Some(value) = string("URL").filter(|value| !value.is_empty()) {
			if let Some(rest) = value.strip_prefix("file://") {
				target = Some(PathBuf::from(rest));
			} else if value.starts_with('/') {
				target = Some(PathBuf::from(value));
			} else {
				diagnostics.push(diagnostic(path, "only file URLs are supported"));
			}
		}

		let mut exec = string("Exec").filter(|value| !value.is_empty());
		if entry_type == Some(EntryType::File) && exec.is_some() {
			diagnostics.push(diagnostic(path, "Exec= is ignored for Type=File"));
			exec = None;
		}

		if entry_type == Some(EntryType::Directory) && fs_kind == FsKind::File {
			diagnostics.push(diagnostic(path, "Type=Directory describes a file"));
		}
		if entry_type == Some(EntryType::File) && fs_kind == FsKind::Directory {
			diagnostics.push(diagnostic(path, "Type=File describes a directory"));
		}
		let progress = sections
			.get(crate::progress::SECTION)
			.and_then(crate::progress::Progress::from_values);
		let app_display = sections.get("App Display");
		let main_display = app_display
			.and_then(|values| {
				value_int(values, "MainDisplay").or_else(|| value_int(values, "MainScreen"))
			})
			.and_then(|value| i32::try_from(value).ok())
			.filter(|value| *value >= 0);

		if entry_type == Some(EntryType::File)
			&& progress.is_none()
			&& let Some(target) = &target
			&& !target.is_file()
		{
			diagnostics.push(diagnostic(
				path,
				format!("Type=File target does not exist: {}", target.display()),
			));
		}

		Some(Self {
			path: path.to_path_buf(),
			entry_type,
			target,
			mime: string("MimeType").filter(|value| !value.is_empty()),
			name: string("Name").filter(|name| !name.is_empty()),
			comment: string("Comment").filter(|comment| !comment.is_empty()),
			icon: string("Icon").filter(|icon| !icon.is_empty()),
			exec,
			terminal: value_bool(values.get("Terminal")),
			terminal_options: string("TerminalOptions")
				.filter(|value| !value.trim().is_empty()),
			launch_animation: string("LaunchAnimation").filter(|value| !value.trim().is_empty()),
			main_display,
			hidden: value_bool(values.get("Hidden")),
			no_display: value_bool(values.get("NoDisplay")),
			toggle: value_bool(values.get("Toggle")),
			number_value: number_value(values),
			number_fields: number_fields(values),
			generated: value_bool(values.get("Generated")),
			destroy: string("Destroy").filter(|command| !command.trim().is_empty()),
			reset: string("Reset").filter(|command| !command.trim().is_empty()),
			rename: string("Rename").filter(|command| !command.trim().is_empty()),
			refresh: string("Refresh").filter(|command| !command.trim().is_empty()),
			custom_group: string("Group").filter(|group| !group.trim().is_empty()),
			progress,
			diagnostics,
		})
	}

	/// Expand the entry's `Exec=` field codes. `file` fills `%f`/`%u`; the
	/// entry path fills `%k` and the entry name fills `%c`.
	pub fn exec_args(&self, file: Option<&Path>) -> Result<Vec<String>, String> {
		let exec = self
			.exec
			.as_deref()
			.filter(|exec| !exec.trim().is_empty())
			.ok_or_else(|| String::from("entry has no Exec"))?;
		self.exec_args_from(exec, file)
	}

	/// The provider command for a `Type=Toggle` or `Type=Number` control.
	/// Toggle targets replace `%v` with `on`/`off`; number values replace
	/// `%1`, `%2`, …
	pub fn control_args(&self, values: &[i64]) -> Result<Vec<String>, String> {
		let exec = self
			.exec
			.as_deref()
			.filter(|exec| !exec.trim().is_empty())
			.ok_or_else(|| String::from("entry has no Exec"))?;
		let expanded = if values.is_empty() {
			exec.replace("%v", if self.toggle { "off" } else { "on" })
		} else {
			let mut expanded = exec.to_string();
			for (index, value) in values.iter().enumerate() {
				expanded = expanded.replace(&format!("%{}", index + 1), &value.to_string());
			}
			expanded
		};
		self.exec_args_from(&expanded, None)
	}

	fn exec_args_from(&self, exec: &str, file: Option<&Path>) -> Result<Vec<String>, String> {
		let tokens = tokenize_exec(exec)?;
		let mut args = Vec::new();
		for token in tokens {
			if token == "%i" {
				if let Some(icon) = &self.icon {
					args.push(String::from("--icon"));
					args.push(icon.clone());
				}
				continue;
			}
			let mut expanded = String::new();
			let mut chars = token.chars();
			while let Some(character) = chars.next() {
				if character != '%' {
					expanded.push(character);
					continue;
				}
				match chars.next() {
					Some('%') => expanded.push('%'),
					Some('c') => expanded.push_str(self.name.as_deref().unwrap_or_default()),
					Some('k') => expanded.push_str(&self.path.to_string_lossy()),
					Some('f' | 'F' | 'u' | 'U') => {
						if let Some(file) = file {
							expanded.push_str(&file.to_string_lossy());
						}
					}
					Some('d' | 'D' | 'n' | 'N' | 'v' | 'm' | 'i') => {}
					Some(other) => {
						return Err(format!("unsupported field code %{other} in Exec"));
					}
					None => {
						return Err(String::from("Exec ends with an incomplete field code"));
					}
				}
			}
			if !expanded.is_empty() {
				args.push(expanded);
			}
		}
		Ok(args)
	}

	/// A generated control whose only action is changing its own label.
	pub fn is_rename_only(&self) -> bool {
		self.generated
			&& self.entry_type == Some(EntryType::Button)
			&& self.rename.is_some()
			&& self
				.exec
				.as_deref()
				.is_none_or(|exec| exec.trim().is_empty())
	}

	/// Whether the user may remove this item through the folder.
	pub fn is_destroyable(&self) -> bool {
		self.destroy.is_some() || !self.generated
	}

	/// Expand a provider command: `%v` is the new value, `%n` the display
	/// name, `%p` the item path, `%k` the entry path and `%c` the name.
	fn hook_args(
		&self,
		command: &str,
		value: Option<&str>,
	) -> Result<Vec<String>, String> {
		expand_hook(
			command,
			value,
			self.name.as_deref().unwrap_or_default(),
			&self.path,
		)
	}

	pub fn rename_args(&self, new_name: &str) -> Result<Vec<String>, String> {
		let command = self
			.rename
			.as_deref()
			.ok_or_else(|| String::from("entry has no Rename command"))?;
		self.hook_args(command, Some(new_name))
	}

	pub fn destroy_args(&self) -> Result<Vec<String>, String> {
		let command = self
			.destroy
			.as_deref()
			.ok_or_else(|| String::from("entry has no Destroy command"))?;
		self.hook_args(command, None)
	}

	pub fn reset_args(&self) -> Result<Vec<String>, String> {
		let command = self
			.reset
			.as_deref()
			.ok_or_else(|| String::from("entry has no Reset command"))?;
		self.hook_args(command, None)
	}

	pub fn refresh_args(&self) -> Result<Vec<String>, String> {
		let command = self
			.refresh
			.as_deref()
			.ok_or_else(|| String::from("entry has no Refresh command"))?;
		self.hook_args(command, None)
	}

	/// The icon this entry asks for, resolved against the entry file.
	pub fn icon_ref(&self) -> Option<IconRef> {
		let icon = self.icon.as_deref()?;
		let base = self.path.parent().unwrap_or(Path::new("."));
		Some(if let Some(rest) = icon.strip_prefix("file://") {
			IconRef::Path(PathBuf::from(rest))
		} else if icon.contains('/') || icon.starts_with('.') {
			IconRef::Path(base.join(icon))
		} else {
			IconRef::Theme(icon.to_string())
		})
	}

	/// A descriptor names the item it describes through `Path=`.
	pub fn is_descriptor(&self) -> bool {
		self.entry_type == Some(EntryType::File) && self.target.is_some()
	}

	/// A `Type=File` entry without `Path=` describes a file beside it.
	pub fn is_sidecar(&self) -> bool {
		self.entry_type == Some(EntryType::File) && self.target.is_none()
	}

	pub fn item_kind(&self, fs_kind: FsKind) -> ItemKind {
		match self.entry_type {
			Some(EntryType::File) | Some(EntryType::Directory) => ItemKind::from_fs(fs_kind),
			Some(entry_type) => entry_type.item_kind(),
			None => ItemKind::from_fs(fs_kind),
		}
	}
}

fn is_entry_path(path: &Path) -> bool {
	path.file_name()
		.and_then(|name| name.to_str())
		.is_some_and(|name| name.eq_ignore_ascii_case(".directory"))
		|| path
			.extension()
			.and_then(|extension| extension.to_str())
			.is_some_and(|extension| {
				extension.eq_ignore_ascii_case("desktop")
					|| extension.eq_ignore_ascii_case("directory")
			})
}

fn value_string(value: Option<&ConfigValue>) -> Option<String> {
	match value? {
		ConfigValue::String(value) => Some(value.clone()),
		ConfigValue::Number(value) => Some(value.to_string()),
		ConfigValue::Boolean(value) => Some(value.to_string()),
		_ => None,
	}
}

fn value_bool(value: Option<&ConfigValue>) -> bool {
	match value {
		Some(ConfigValue::Boolean(value)) => *value,
		Some(ConfigValue::String(value)) => value.eq_ignore_ascii_case("true"),
		_ => false,
	}
}

fn value_int(values: &std::collections::HashMap<String, ConfigValue>, key: &str) -> Option<i64> {
	match values.get(key)? {
		ConfigValue::Number(value) => Some(*value as i64),
		ConfigValue::String(value) => value.trim().parse().ok(),
		ConfigValue::Boolean(value) => Some(i64::from(*value)),
		_ => None,
	}
}

/// The `NumberNLabel`/`NumberNValue`/`NumberNMin`/`NumberNMax` fields a
/// `Type=Number` provider writes.
fn number_fields(values: &std::collections::HashMap<String, ConfigValue>) -> Vec<NumberField> {
	let count = value_int(values, "NumberCount").unwrap_or(0).clamp(0, 16);
	(1..=count)
		.filter_map(|index| {
			let label = value_string(values.get(&format!("Number{index}Label")))?;
			let value = value_int(values, &format!("Number{index}Value"))?;
			Some(NumberField {
				label,
				value,
				min: value_int(values, &format!("Number{index}Min")).unwrap_or(value),
				max: value_int(values, &format!("Number{index}Max")).unwrap_or(value),
				default: value_int(values, &format!("Number{index}Default")).unwrap_or(value),
			})
		})
		.collect()
}

fn number_value(values: &std::collections::HashMap<String, ConfigValue>) -> String {
	let count = value_int(values, "NumberCount").unwrap_or(0).clamp(0, 16);
	(1..=count)
		.filter_map(|index| value_int(values, &format!("Number{index}Value")))
		.map(|value| value.to_string())
		.collect::<Vec<_>>()
		.join(" ")
}

/// A path value is relative to the entry file, and `~/` is the home folder.
fn resolve_entry_path(entry: &Path, value: &str) -> PathBuf {
	let base = entry.parent().unwrap_or(Path::new("."));
	let value = path::expand_home(value);
	if value.is_absolute() {
		value
	} else {
		base.join(value)
	}
}

fn diagnostic(path: &Path, message: impl Into<String>) -> ConfigDiagnostic {
	ConfigDiagnostic {
		file: path.to_path_buf(),
		line: None,
		message: message.into(),
	}
}

/// Expand a provider command with `%v`, `%n`, `%p`, `%k`, `%c` and `%%`.
pub fn expand_hook(
	command: &str,
	value: Option<&str>,
	name: &str,
	path: &Path,
) -> Result<Vec<String>, String> {
	let tokens = tokenize_exec(command)?;
	let path = path.to_string_lossy();
	let mut args = Vec::new();
	for token in tokens {
		let mut expanded = String::new();
		let mut chars = token.chars();
		while let Some(character) = chars.next() {
			if character != '%' {
				expanded.push(character);
				continue;
			}
			match chars.next() {
				Some('%') => expanded.push('%'),
				Some('v') => expanded.push_str(value.unwrap_or_default()),
				Some('n') => expanded.push_str(name),
				Some('p') => expanded.push_str(&path),
				Some('k') => expanded.push_str(&path),
				Some('c') => expanded.push_str(name),
				Some(other) => {
					return Err(format!("unsupported field code %{other} in command"));
				}
				None => {
					return Err(String::from("command ends with an incomplete field code"));
				}
			}
		}
		if !expanded.is_empty() {
			args.push(expanded);
		}
	}
	Ok(args)
}

pub(crate) fn tokenize_exec(exec: &str) -> Result<Vec<String>, String> {
	let mut args = Vec::new();
	let mut token = String::new();
	let mut chars = exec.chars().peekable();
	let mut quote = None;
	let mut token_started = false;

	while let Some(character) = chars.next() {
		match character {
			'\\' => {
				let escaped = chars
					.next()
					.ok_or_else(|| String::from("Exec ends with an incomplete escape"))?;
				token.push(escaped);
				token_started = true;
			}
			'"' | '\'' if quote == Some(character) => quote = None,
			'"' | '\'' if quote.is_none() => {
				quote = Some(character);
				token_started = true;
			}
			character if character.is_whitespace() && quote.is_none() => {
				if token_started {
					args.push(std::mem::take(&mut token));
					token_started = false;
				}
			}
			_ => {
				token.push(character);
				token_started = true;
			}
		}
	}

	if quote.is_some() {
		return Err(String::from("Exec contains an unterminated quote"));
	}
	if token_started {
		args.push(token);
	}
	Ok(args)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn window(title: &str) -> PathBuf {
		let root =
			std::env::temp_dir().join(format!("hishell-entry-{title}-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		root
	}

	#[test]
	fn reads_desktop_entry_fields() {
		let root = window("fields");
		let path = root.join("ride.desktop");
		std::fs::write(
			&path,
			"[Desktop Entry]\nType=Application\nName=Ride\nComment=Go fast\nIcon=ride\nExec=ride %f\n",
		)
		.unwrap();
		let entry = Entry::read(&path, FsKind::File).unwrap();
		assert_eq!(entry.name.as_deref(), Some("Ride"));
		assert_eq!(entry.comment.as_deref(), Some("Go fast"));
		assert_eq!(entry.icon.as_deref(), Some("ride"));
		assert_eq!(entry.entry_type, Some(EntryType::Application));
		assert_eq!(entry.exec.as_deref(), Some("ride %f"));
		assert!(!entry.hidden);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn reads_the_main_display_from_the_app_display_group() {
		let root = window("main-display");
		let path = root.join("game.desktop");
		std::fs::write(
			&path,
			"[Desktop Entry]\nType=Application\nName=Game\nExec=game\n\n\
			 [App Display]\nMainDisplay=0\n",
		)
		.unwrap();
		let entry = Entry::read(&path, FsKind::File).unwrap();
		assert_eq!(entry.main_display, Some(0));
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn reads_terminal_launch_fields() {
		let root = window("terminal");
		let path = root.join("micro.desktop");
		std::fs::write(
			&path,
			"[Desktop Entry]\nType=Application\nName=Micro\nExec=micro %F\n\
			 Terminal=true\nTerminalOptions=--hold\n",
		)
		.unwrap();
		let entry = Entry::read(&path, FsKind::File).unwrap();
		assert!(entry.terminal);
		assert_eq!(entry.terminal_options.as_deref(), Some("--hold"));
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn reads_provider_action_commands() {
		let root = window("provider-actions");
		let path = root.join("device.desktop");
		std::fs::write(
			&path,
			"[Desktop Entry]\nType=Button\nName=Device\nGenerated=true\n\
			 Rename=provider rename %v %n %p\n\
			 Destroy=provider destroy %n %p\n\
			 Reset=provider reset %p\n\
			 Refresh=provider refresh %p\n",
		)
		.unwrap();
		let entry = Entry::read(&path, FsKind::File).unwrap();
		assert!(entry.generated);
		assert_eq!(
			entry.rename_args("New name").unwrap(),
			vec![
				"provider",
				"rename",
				"New name",
				"Device",
				path.to_string_lossy().as_ref()
			]
		);
		assert_eq!(
			entry.destroy_args().unwrap(),
			vec!["provider", "destroy", "Device", path.to_string_lossy().as_ref()]
		);
		assert_eq!(
			entry.reset_args().unwrap(),
			vec!["provider", "reset", path.to_string_lossy().as_ref()]
		);
		assert_eq!(
			entry.refresh_args().unwrap(),
			vec!["provider", "refresh", path.to_string_lossy().as_ref()]
		);
		assert!(entry.is_destroyable());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_generated_button_without_exec_is_rename_only() {
		let root = window("rename-only");
		let path = root.join("name.desktop");
		std::fs::write(
			&path,
			"[Desktop Entry]\nType=Button\nName=User\nGenerated=true\n\
			 Rename=provider rename %v\n",
		)
		.unwrap();
		assert!(Entry::read(&path, FsKind::File).unwrap().is_rename_only());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn control_commands_expand_toggle_and_number_values() {
		let root = window("controls");
		let toggle = root.join("mute.desktop");
		std::fs::write(
			&toggle,
			"[Desktop Entry]\nType=Toggle\nName=Mute\nExec=provider %v\nToggle=true\n",
		)
		.unwrap();
		let entry = Entry::read(&toggle, FsKind::File).unwrap();
		assert_eq!(entry.control_args(&[]).unwrap(), vec!["provider", "off"]);
		let number = root.join("volume.desktop");
		std::fs::write(
			&number,
			"[Desktop Entry]\nType=Number\nName=Volume\nExec=set %1 %2\n",
		)
		.unwrap();
		let entry = Entry::read(&number, FsKind::File).unwrap();
		assert_eq!(
			entry.control_args(&[3, 7]).unwrap(),
			vec!["set", "3", "7"]
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn reads_number_control_fields() {
		let root = window("numbers");
		let path = root.join("volume.desktop");
		std::fs::write(
			&path,
			"[Desktop Entry]\nType=Number\nName=Volume\nNumberCount=2\n\
			 Number1Label=Left\nNumber1Value=3\nNumber1Min=0\nNumber1Max=10\n\
			 Number2Label=Right\nNumber2Value=7\nNumber2Min=0\nNumber2Max=10\n",
		)
		.unwrap();
		let entry = Entry::read(&path, FsKind::File).unwrap();
		assert_eq!(entry.number_fields.len(), 2);
		assert_eq!(entry.number_fields[0].label, "Left");
		assert_eq!(entry.number_fields[0].value, 3);
		assert_eq!(entry.number_fields[0].max, 10);
		assert_eq!(entry.number_fields[1].label, "Right");
		assert_eq!(entry.number_fields[1].value, 7);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn hidden_and_stashed_entries_are_distinct() {
		let root = window("hidden");
		let hidden = root.join("gone.desktop");
		std::fs::write(&hidden, "[Desktop Entry]\nType=File\nHidden=true\n").unwrap();
		let stashed = root.join("quiet.desktop");
		std::fs::write(&stashed, "[Desktop Entry]\nType=File\nNoDisplay=true\n").unwrap();
		assert!(Entry::read(&hidden, FsKind::File).unwrap().hidden);
		assert!(!Entry::read(&stashed, FsKind::File).unwrap().hidden);
		assert!(Entry::read(&stashed, FsKind::File).unwrap().no_display);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn file_descriptors_resolve_their_target() {
		let root = window("target");
		let file = root.join("game.rom");
		std::fs::write(&file, b"rom").unwrap();
		let descriptor = root.join("game.desktop");
		std::fs::write(
			&descriptor,
			format!(
				"[Desktop Entry]\nType=File\nPath={}\nName=Test Game\nExec=nope\n",
				file.display()
			),
		)
		.unwrap();
		let entry = Entry::read(&descriptor, FsKind::File).unwrap();
		assert!(entry.is_descriptor());
		assert_eq!(entry.target.as_deref(), Some(file.as_path()));
		assert!(entry.exec.is_none());
		assert!(!entry.diagnostics.is_empty());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_folder_entry_can_make_the_folder_an_application() {
		let root = window("folder");
		let folder = root.join("ride");
		std::fs::create_dir_all(&folder).unwrap();
		let entry_path = folder.join(".directory");
		std::fs::write(
			&entry_path,
			"[Desktop Entry]\nType=Application\nName=Ride\nExec=ride\n",
		)
		.unwrap();
		let entry = Entry::read(&entry_path, FsKind::Directory).unwrap();
		assert_eq!(entry.item_kind(FsKind::Directory), ItemKind::Application);
		assert!(entry.diagnostics.is_empty());
		std::fs::remove_dir_all(root).unwrap();
	}
}

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
	pub hidden: bool,
	pub no_display: bool,
	pub toggle: bool,
	pub number_value: String,
	pub custom_group: Option<String>,
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
		if entry_type == Some(EntryType::File)
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
			hidden: value_bool(values.get("Hidden")),
			no_display: value_bool(values.get("NoDisplay")),
			toggle: value_bool(values.get("Toggle")),
			number_value: number_value(values),
			custom_group: string("Group").filter(|group| !group.trim().is_empty()),
			diagnostics,
		})
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

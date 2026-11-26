//! Item descriptors: desktop entries that name a path and give it metadata
//! without writing a `.directory` beside it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::config_parser::{ConfigParser, ConfigValue};

const REFRESH: Duration = Duration::from_secs(10);

/// Even when no descriptor root grew or shrank, re-read them this often: a file
/// rewritten in place does not move its directory's stamp.
const HARD_REFRESH: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Descriptor {
	pub name: Option<String>,
	pub comment: Option<String>,
	pub icon: Option<String>,
	/// The content type this descriptor assigns its target, overriding the
	/// extension guess.
	pub mime: Option<String>,
	pub unmount: Option<String>,
	pub source: PathBuf,
}

impl Descriptor {
	pub fn icon_ref(&self) -> Option<crate::item::IconRef> {
		let icon = self.icon.as_deref()?.trim();
		if icon.is_empty() {
			return None;
		}
		Some(if let Some(rest) = icon.strip_prefix("file://") {
			crate::item::IconRef::Path(PathBuf::from(rest))
		} else if icon.starts_with('/') {
			crate::item::IconRef::Path(PathBuf::from(icon))
		} else if icon.contains('/') || icon.starts_with('.') {
			crate::item::IconRef::Path(self.source.parent().unwrap_or(Path::new(".")).join(icon))
		} else {
			crate::item::IconRef::Theme(icon.to_string())
		})
	}

	/// Expand this descriptor's `Unmount=` command for `path`.
	pub fn unmount_args(&self, path: &Path) -> Result<Vec<String>, String> {
		let command = self
			.unmount
			.as_deref()
			.filter(|command| !command.trim().is_empty())
			.ok_or_else(|| String::from("descriptor has no Unmount command"))?;
		crate::entry::expand_hook(
			command,
			None,
			self.name.as_deref().unwrap_or_default(),
			path,
		)
	}
}

struct Cache {
	built: Instant,
	/// When `entries` was last read in full.
	built_entries: Instant,
	/// When each descriptor root last changed. A timed refresh compares these
	/// instead of re-reading every entry.
	stamps: Vec<(PathBuf, Option<SystemTime>)>,
	entries: HashMap<PathBuf, Descriptor>,
}

static CACHE: LazyLock<Mutex<Option<Cache>>> = LazyLock::new(|| Mutex::new(None));

/// The descriptor that describes `path`, if any.
pub fn lookup(path: &Path) -> Option<Descriptor> {
	let mut cache = CACHE.lock().unwrap();
	let stale = cache
		.as_ref()
		.is_none_or(|cache| cache.built.elapsed() >= REFRESH);
	if stale {
		let stamps = root_stamps();
		let now = Instant::now();
		match cache.as_mut() {
			Some(current)
				if current.stamps == stamps && current.built_entries.elapsed() < HARD_REFRESH =>
			{
				current.built = now;
			}
			_ => {
				*cache = Some(Cache {
					built: now,
					built_entries: now,
					stamps,
					entries: build(),
				})
			}
		}
	}
	let cache = cache.as_ref()?;
	// A scan looks up the same paths a directory listing just produced, which
	// are already canonical. Only pay for `realpath` when that misses.
	if let Some(found) = cache.entries.get(path) {
		return Some(found.clone());
	}
	let key = path.canonicalize().ok()?;
	cache.entries.get(&key).cloned()
}

fn root_stamps() -> Vec<(PathBuf, Option<SystemTime>)> {
	roots()
		.into_iter()
		.map(|root| {
			let stamp = std::fs::metadata(&root)
				.and_then(|metadata| metadata.modified())
				.ok();
			(root, stamp)
		})
		.collect()
}

fn build() -> HashMap<PathBuf, Descriptor> {
	let mut map = HashMap::new();
	for root in roots() {
		read_root(&root, &mut map);
	}
	map
}

fn roots() -> Vec<PathBuf> {
	let mut roots = Vec::new();
	if let Ok(value) = std::env::var("XDG_DATA_DIRS") {
		for dir in value.split(':').filter(|dir| !dir.is_empty()) {
			roots.push(PathBuf::from(dir).join("hishell/directories"));
		}
	} else {
		roots.push(PathBuf::from("/usr/local/share/hishell/directories"));
		roots.push(PathBuf::from("/usr/share/hishell/directories"));
	}
	if let Some(cache) = dirs::cache_dir() {
		roots.push(cache.join("hishell/descriptors"));
	}
	if let Some(dir) = dirs::data_dir() {
		roots.push(dir.join("hishell/directories"));
	}
	roots.extend(runtime_roots());
	roots
}

/// Runtime descriptor roots the host exports, colon-separated in
/// `HISHELL_RUNTIME_DIRS`. A session service writes live descriptors for
/// mounted devices here; the data directories and the cache are always read.
fn runtime_roots() -> Vec<PathBuf> {
	std::env::var("HISHELL_RUNTIME_DIRS")
		.ok()
		.map(|value| std::env::split_paths(&value).collect())
		.unwrap_or_default()
}

fn read_root(root: &Path, map: &mut HashMap<PathBuf, Descriptor>) {
	let Ok(entries) = std::fs::read_dir(root) else {
		return;
	};
	let mut files: Vec<PathBuf> = entries
		.flatten()
		.map(|entry| entry.path())
		.filter(|path| {
			path.extension()
				.is_some_and(|extension| extension.eq_ignore_ascii_case("desktop"))
		})
		.collect();
	files.sort();
	for file in files {
		let Ok(text) = std::fs::read_to_string(&file) else {
			continue;
		};
		let Some((target, descriptor)) = parse(&text, &file) else {
			continue;
		};
		let key = target.canonicalize().unwrap_or(target);
		map.insert(key, descriptor);
	}
}

fn parse(text: &str, source: &Path) -> Option<(PathBuf, Descriptor)> {
	let (sections, _) = ConfigParser::parse_str(text, &source.to_string_lossy());
	let values = sections.get("Desktop Entry")?;
	let string = |key: &str| match values.get(key) {
		Some(ConfigValue::String(value)) => Some(unescape(value)),
		_ => None,
	};
	let target = PathBuf::from(string("Path")?);
	if target.as_os_str().is_empty() {
		return None;
	}
	let mime = string("MimeType").and_then(|value| {
		value
			.split(';')
			.map(str::trim)
			.find(|item| !item.is_empty())
			.map(str::to_string)
	});
	Some((
		target,
		Descriptor {
			name: string("Name"),
			comment: string("Comment"),
			icon: string("Icon"),
			mime,
			unmount: string("Unmount"),
			source: source.to_path_buf(),
		},
	))
}

fn unescape(value: &str) -> String {
	value
		.replace("\\n", "\n")
		.replace("\\t", "\t")
		.replace("\\r", "\r")
		.replace("\\\\", "\\")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_descriptor_names_its_target() {
		let (target, descriptor) = parse(
			"[Desktop Entry]\nType=Directory\nPath=/mnt/sdcard\nName=microSD card\n\
			 Icon=media-flash-microsd\nComment=Removable\n",
			Path::new("/usr/share/hishell/directories/sd.desktop"),
		)
		.unwrap();
		assert_eq!(target, PathBuf::from("/mnt/sdcard"));
		assert_eq!(descriptor.name.as_deref(), Some("microSD card"));
		assert_eq!(descriptor.comment.as_deref(), Some("Removable"));
		assert_eq!(
			descriptor.icon_ref(),
			Some(crate::item::IconRef::Theme(String::from(
				"media-flash-microsd"
			)))
		);
	}

	#[test]
	fn a_descriptor_can_name_the_content_type() {
		let (_, descriptor) = parse(
			"[Desktop Entry]\nType=File\nPath=/home/config.cfg\n\
			 MimeType=application/x-clam-config;\n",
			Path::new("/home/.local/share/hishell/directories/config.desktop"),
		)
		.unwrap();
		assert_eq!(
			descriptor.mime.as_deref(),
			Some("application/x-clam-config")
		);
	}

	#[test]
	fn a_quoted_numeric_name_is_still_a_name() {
		let (_, descriptor) = parse(
			"[Desktop Entry]\nType=Directory\nPath=/mnt/sdcard\nName=\"2024\"\n\
			 Comment=microSD card\n",
			Path::new("/usr/share/hishell/directories/sd.desktop"),
		)
		.unwrap();
		assert_eq!(descriptor.name.as_deref(), Some("2024"));
		assert_eq!(descriptor.comment.as_deref(), Some("microSD card"));
	}

	#[test]
	fn a_descriptor_expands_its_unmount_command() {
		let (target, descriptor) = parse(
			"[Desktop Entry]\nType=Directory\nPath=/mnt/sdcard\nName=microSD card\n\
			 Unmount=provider unmount %n %p\n",
			Path::new("/usr/share/hishell/directories/sd.desktop"),
		)
		.unwrap();
		assert_eq!(
			descriptor.unmount_args(&target).unwrap(),
			vec!["provider", "unmount", "microSD card", "/mnt/sdcard"]
		);
	}

	#[test]
	fn a_root_indexes_descriptors_by_target() {
		let root = std::env::temp_dir().join(format!("hishell-descriptor-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let descriptors = root.join("directories");
		let target = root.join("microSD card");
		std::fs::create_dir_all(&descriptors).unwrap();
		std::fs::create_dir_all(&target).unwrap();
		std::fs::write(
			descriptors.join("sd.desktop"),
			format!(
				"[Desktop Entry]\nType=Directory\nPath={}\nName=microSD card\n",
				target.display()
			),
		)
		.unwrap();
		let mut map = HashMap::new();
		read_root(&descriptors, &mut map);
		let key = target.canonicalize().unwrap();
		assert_eq!(
			map.get(&key).and_then(|entry| entry.name.as_deref()),
			Some("microSD card")
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_relative_icon_resolves_against_the_descriptor() {
		let (_, descriptor) = parse(
			"[Desktop Entry]\nPath=/mnt/sdcard\nIcon=icons/sd.png\n",
			Path::new("/usr/share/hishell/directories/sd.desktop"),
		)
		.unwrap();
		assert_eq!(
			descriptor.icon_ref(),
			Some(crate::item::IconRef::Path(PathBuf::from(
				"/usr/share/hishell/directories/icons/sd.png"
			)))
		);
	}
}

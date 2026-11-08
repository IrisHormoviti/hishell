//! Item descriptors: desktop entries that name a path and give it metadata
//! without writing a `.directory` beside it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::config_parser::{ConfigParser, ConfigValue};

pub const RUNTIME_ROOT: &str = "/run/clam-removable/directories";
const REFRESH: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Descriptor {
	pub name: Option<String>,
	pub comment: Option<String>,
	pub icon: Option<String>,
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
}

static CACHE: LazyLock<Mutex<Option<(Instant, HashMap<PathBuf, Descriptor>)>>> =
	LazyLock::new(|| Mutex::new(None));

/// The descriptor that describes `path`, if any.
pub fn lookup(path: &Path) -> Option<Descriptor> {
	let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
	let mut cache = CACHE.lock().unwrap();
	let stale = cache
		.as_ref()
		.is_none_or(|(built, _)| built.elapsed() >= REFRESH);
	if stale {
		*cache = Some((Instant::now(), build()));
	}
	cache.as_ref()?.1.get(&key).cloned()
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
	if let Some(dir) = dirs::data_dir() {
		roots.push(dir.join("hishell/directories"));
	}
	if let Some(cache) = dirs::cache_dir() {
		roots.push(cache.join("hishell/descriptors"));
	}
	roots.push(PathBuf::from(RUNTIME_ROOT));
	roots
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
	Some((
		target,
		Descriptor {
			name: string("Name"),
			comment: string("Comment"),
			icon: string("Icon"),
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
			"[Desktop Entry]\nType=Directory\nPath=/mnt/sdcard\nName=SD Card\n\
			 Icon=media-flash-sd-mmc\nComment=Removable\n",
			Path::new("/usr/share/hishell/directories/sd.desktop"),
		)
		.unwrap();
		assert_eq!(target, PathBuf::from("/mnt/sdcard"));
		assert_eq!(descriptor.name.as_deref(), Some("SD Card"));
		assert_eq!(descriptor.comment.as_deref(), Some("Removable"));
		assert_eq!(
			descriptor.icon_ref(),
			Some(crate::item::IconRef::Theme(String::from(
				"media-flash-sd-mmc"
			)))
		);
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

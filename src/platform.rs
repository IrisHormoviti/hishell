//! Host capabilities the core can use.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::entry::{Entry, EntryType};
use crate::execution::AppId;
use crate::item::IconRef;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Mime(pub String);

impl Mime {
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AppEntry {
	pub id: AppId,
	pub name: String,
	pub icon: Option<IconRef>,
	pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Thumbnail {
	pub path: PathBuf,
	pub size: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThumbnailRequest {
	pub path: PathBuf,
	pub size: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PlatformError {
	Unsupported(String),
	Io(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PortalState {
	Finished,
	Pending,
	Failed,
}

pub trait Launcher: Send + Sync {
	fn activate(&self, app: &AppId, uris: &[&Path]) -> Result<(), PlatformError>;
}

pub trait AppCatalog: Send + Sync {
	fn apps_for(&self, path: &Path) -> Vec<AppEntry>;
	fn default_app(&self, path: &Path) -> Option<AppEntry>;
	fn set_default(&self, mime: &str, app: &AppId) -> Result<(), PlatformError>;
}

pub trait MimeResolver: Send + Sync {
	fn mime_type(&self, path: &Path) -> Mime;
}

pub trait Thumbnailer: Send + Sync {
	fn thumbnail(&self, request: &ThumbnailRequest) -> Result<Option<Thumbnail>, PlatformError>;
}

pub trait SoundPlayer: Send + Sync {
	fn play(&self, event: &str);
	fn set_volume(&self, volume: f32);
}

pub trait Portal: Send + Sync {
	fn open_with(&self, path: &Path) -> Result<(), PlatformError>;
	fn poll(&self) -> PortalState;
}

#[derive(Clone, Default)]
pub struct Platform {
	pub launcher: Option<Arc<dyn Launcher>>,
	pub apps: Option<Arc<dyn AppCatalog>>,
	pub mime: Option<Arc<dyn MimeResolver>>,
	pub thumbnails: Option<Arc<dyn Thumbnailer>>,
	pub sound: Option<Arc<dyn SoundPlayer>>,
	pub portal: Option<Arc<dyn Portal>>,
}

impl std::fmt::Debug for Platform {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Platform")
			.field("launcher", &self.launcher.is_some())
			.field("apps", &self.apps.is_some())
			.field("mime", &self.mime.is_some())
			.field("thumbnails", &self.thumbnails.is_some())
			.field("sound", &self.sound.is_some())
			.field("portal", &self.portal.is_some())
			.finish()
	}
}

impl Platform {
	pub fn free_desktop() -> Self {
		Self {
			mime: Some(Arc::new(FreeDesktopMime)),
			apps: Some(Arc::new(FreeDesktopApps)),
			thumbnails: Some(Arc::new(crate::thumbnail::FreeDesktopThumbnailer)),
			..Self::default()
		}
	}

	pub fn mime_type(&self, path: &Path) -> Mime {
		match &self.mime {
			Some(resolver) => resolver.mime_type(path),
			None => Mime(String::from("application/octet-stream")),
		}
	}

	pub fn open_with(&self, _path: &Path) -> Result<(), PlatformError> {
		Err(PlatformError::Unsupported(String::from("portal")))
	}
}

#[derive(Default)]
pub struct FreeDesktopMime;

impl MimeResolver for FreeDesktopMime {
	fn mime_type(&self, path: &Path) -> Mime {
		Mime(
			crate::icon::mime_for_path(path)
				.unwrap_or_else(|| String::from("application/octet-stream")),
		)
	}
}

#[derive(Default)]
pub struct FreeDesktopApps;

impl AppCatalog for FreeDesktopApps {
	fn apps_for(&self, path: &Path) -> Vec<AppEntry> {
		let mime = FreeDesktopMime.mime_type(path).0;
		let mut apps = Vec::new();
		for (id, entry_path) in application_entries() {
			let Some(entry) = Entry::read(&entry_path, crate::item::FsKind::File) else {
				continue;
			};
			if entry.hidden || entry.no_display {
				continue;
			}
			if entry.entry_type != Some(EntryType::Application) {
				continue;
			}
			if !mime_matches(&entry, &mime) {
				continue;
			}
			apps.push(app_entry(&id, &entry));
		}
		apps
	}

	fn default_app(&self, path: &Path) -> Option<AppEntry> {
		let mime = FreeDesktopMime.mime_type(path).0;
		let id = default_application(&mime)?;
		let entry_path = application_entries()
			.into_iter()
			.find(|(candidate, _)| candidate == &id)
			.map(|(_, path)| path)?;
		let entry = Entry::read(&entry_path, crate::item::FsKind::File)?;
		Some(app_entry(&id, &entry))
	}

	fn set_default(&self, mime: &str, app: &AppId) -> Result<(), PlatformError> {
		if mime.is_empty() || app.0.is_empty() {
			return Err(PlatformError::Unsupported(String::from(
				"empty mime or application",
			)));
		}
		let path = config_home().join("mimeapps.list");
		crate::config_parser::ConfigParser::set_value(&path, "Default Applications", mime, &app.0);
		Ok(())
	}
}

fn app_entry(id: &str, entry: &Entry) -> AppEntry {
	AppEntry {
		id: AppId(id.to_string()),
		name: entry.name.clone().unwrap_or_else(|| id.to_string()),
		icon: entry.icon_ref(),
		path: entry.path.clone(),
	}
}

fn mime_matches(entry: &Entry, mime: &str) -> bool {
	let Some(types) = entry_mime_types(entry) else {
		return true;
	};
	types.into_iter().any(|candidate| {
		candidate == mime
			|| candidate
				.strip_suffix("/*")
				.is_some_and(|prefix| mime.starts_with(&format!("{prefix}/")))
	})
}

fn entry_mime_types(entry: &Entry) -> Option<Vec<String>> {
	let text = std::fs::read_to_string(&entry.path).ok()?;
	let (sections, _) = crate::config_parser::ConfigParser::parse_str(&text, "<entry>");
	let values = sections.get("Desktop Entry")?;
	let value = match values.get("MimeType")? {
		crate::config_parser::ConfigValue::String(value) => value.clone(),
		_ => return None,
	};
	Some(
		value
			.split(';')
			.filter(|item| !item.is_empty())
			.map(str::to_string)
			.collect(),
	)
}

fn application_entries() -> Vec<(String, PathBuf)> {
	let mut entries = Vec::new();
	for dir in data_home_dirs() {
		let root = dir.join("applications");
		let Ok(files) = std::fs::read_dir(root) else {
			continue;
		};
		for file in files.flatten() {
			let path = file.path();
			if path
				.extension()
				.and_then(|extension| extension.to_str())
				.is_some_and(|extension| extension.eq_ignore_ascii_case("desktop"))
				&& let Some(name) = path.file_name().and_then(|name| name.to_str())
				&& !entries.iter().any(|(id, _)| id == name)
			{
				entries.push((name.to_string(), path));
			}
		}
	}
	entries
}

fn default_application(mime: &str) -> Option<String> {
	for dir in config_dirs() {
		let path = dir.join("mimeapps.list");
		let Ok(text) = std::fs::read_to_string(path) else {
			continue;
		};
		let (sections, _) = crate::config_parser::ConfigParser::parse_str(&text, "<mimeapps>");
		for section in ["Default Applications", "Added Associations"] {
			if let Some(value) = sections.get(section).and_then(|values| values.get(mime))
				&& let crate::config_parser::ConfigValue::String(value) = value
			{
				let first = value.split(';').find(|item| !item.is_empty())?;
				return Some(first.to_string());
			}
		}
	}
	None
}

fn data_home_dirs() -> Vec<PathBuf> {
	let mut dirs = Vec::new();
	if let Some(dir) = dirs::data_dir() {
		dirs.push(dir);
	}
	if let Ok(value) = std::env::var("XDG_DATA_DIRS") {
		for dir in value.split(':').filter(|dir| !dir.is_empty()) {
			dirs.push(PathBuf::from(dir));
		}
	} else {
		dirs.push(PathBuf::from("/usr/local/share"));
		dirs.push(PathBuf::from("/usr/share"));
	}
	dirs
}

fn config_dirs() -> Vec<PathBuf> {
	let mut dirs = Vec::new();
	if let Some(dir) = dirs::config_dir() {
		dirs.push(dir);
	}
	if let Ok(value) = std::env::var("XDG_CONFIG_DIRS") {
		for dir in value.split(':').filter(|dir| !dir.is_empty()) {
			dirs.push(PathBuf::from(dir));
		}
	} else {
		dirs.push(PathBuf::from("/etc/xdg"));
	}
	dirs
}

fn config_home() -> PathBuf {
	dirs::config_dir().unwrap_or_else(|| PathBuf::from("/etc/xdg"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn mime_uses_the_extension_table() {
		let root = std::env::temp_dir().join(format!("hishell-platform-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		let image = root.join("photo.png");
		std::fs::write(&image, b"png").unwrap();
		assert_eq!(FreeDesktopMime.mime_type(&image).0, "image/png");
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn missing_capabilities_fall_back() {
		let platform = Platform::default();
		assert_eq!(
			platform.mime_type(Path::new("/tmp/a.png")).0,
			"application/octet-stream"
		);
		assert!(platform.open_with(Path::new("/tmp/a.png")).is_err());
	}
}

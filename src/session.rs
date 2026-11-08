//! Persisted session state.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::history::HistoryEntry;
use crate::location::{Location, LocationKind};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SessionData {
	pub schema_version: u32,
	pub location: LocationKind,
	pub path: PathBuf,
	pub focus: Option<PathBuf>,
	pub history: Vec<HistoryEntry>,
	pub history_index: usize,
}

impl Default for SessionData {
	fn default() -> Self {
		Self {
			schema_version: SCHEMA_VERSION,
			location: LocationKind::Folder,
			path: PathBuf::new(),
			focus: None,
			history: Vec::new(),
			history_index: 0,
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStore {
	path: PathBuf,
}

impl Default for SessionStore {
	fn default() -> Self {
		let path = dirs::config_dir()
			.unwrap_or_else(|| PathBuf::from("/etc/xdg"))
			.join("hishell")
			.join("session.json");
		Self { path }
	}
}

impl SessionStore {
	pub fn new(path: impl Into<PathBuf>) -> Self {
		Self { path: path.into() }
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	pub fn load(&self) -> SessionData {
		let Ok(text) = std::fs::read_to_string(&self.path) else {
			return SessionData::default();
		};
		let data: SessionData = serde_json::from_str(&text).unwrap_or_default();
		if data.schema_version > SCHEMA_VERSION {
			return SessionData::default();
		}
		data
	}

	pub fn save(&self, data: &SessionData) -> Result<(), Error> {
		if let Some(parent) = self.path.parent() {
			std::fs::create_dir_all(parent)?;
		}
		let text =
			serde_json::to_string_pretty(data).map_err(|error| Error::Config(error.to_string()))?;
		let temporary = self.path.with_extension("json.tmp");
		std::fs::write(&temporary, text)?;
		std::fs::rename(&temporary, &self.path)?;
		Ok(())
	}

	/// The location to open at startup: the saved one when it still exists,
	/// otherwise `fallback`.
	pub fn startup_location(&self, fallback: &Path) -> Location {
		let data = self.load();
		let path = if !data.path.as_os_str().is_empty() && data.path.exists() {
			data.path
		} else {
			fallback.to_path_buf()
		};
		if path.is_dir() {
			Location::folder(path)
		} else {
			Location::file(path)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::view::PaneId;

	#[test]
	fn session_data_round_trips() {
		let root = std::env::temp_dir().join(format!("hishell-session-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		let store = SessionStore::new(root.join("session.json"));
		let data = SessionData {
			schema_version: SCHEMA_VERSION,
			location: LocationKind::Folder,
			path: root.clone(),
			focus: Some(root.join("a.txt")),
			history: vec![HistoryEntry {
				pane: PaneId(0),
				path: root.clone(),
				focus: None,
			}],
			history_index: 0,
		};
		store.save(&data).unwrap();
		assert_eq!(store.load(), data);
		assert_eq!(
			store.startup_location(Path::new("/tmp")),
			Location::folder(root.clone())
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_missing_session_falls_back() {
		let root =
			std::env::temp_dir().join(format!("hishell-session-missing-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		let store = SessionStore::new(root.join("session.json"));
		assert_eq!(
			store.startup_location(Path::new("/tmp")),
			Location::folder("/tmp")
		);
		std::fs::remove_dir_all(root).unwrap();
	}
}

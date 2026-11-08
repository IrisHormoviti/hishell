//! Folder and single-file locations.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LocationKind {
	Folder,
	File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
	path: PathBuf,
}

impl Folder {
	pub fn new(path: impl Into<PathBuf>) -> Self {
		Self { path: path.into() }
	}

	pub fn path(&self) -> &Path {
		&self.path
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
	path: PathBuf,
	parent: PathBuf,
}

impl File {
	pub fn new(path: impl Into<PathBuf>) -> Self {
		let path = path.into();
		let parent = path
			.parent()
			.map(Path::to_path_buf)
			.unwrap_or_else(|| PathBuf::from("."));
		Self { path, parent }
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	pub fn parent(&self) -> &Path {
		&self.parent
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
	Folder(Folder),
	File(File),
}

impl Location {
	pub fn folder(path: impl Into<PathBuf>) -> Self {
		Self::Folder(Folder::new(path))
	}

	pub fn file(path: impl Into<PathBuf>) -> Self {
		Self::File(File::new(path))
	}

	pub fn path(&self) -> &Path {
		match self {
			Self::Folder(folder) => folder.path(),
			Self::File(file) => file.path(),
		}
	}

	pub fn kind(&self) -> LocationKind {
		match self {
			Self::Folder(_) => LocationKind::Folder,
			Self::File(_) => LocationKind::File,
		}
	}

	pub fn is_folder(&self) -> bool {
		matches!(self, Self::Folder(_))
	}
}

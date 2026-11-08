//! Synchronous filesystem operations.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Overwrite {
	Never,
	Replace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CopyOptions {
	pub overwrite: Overwrite,
	pub preserve_times: bool,
	pub follow_symlinks: bool,
}

impl Default for CopyOptions {
	fn default() -> Self {
		Self {
			overwrite: Overwrite::Never,
			preserve_times: true,
			follow_symlinks: false,
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PasteOptions {
	pub cut: bool,
	pub overwrite: Overwrite,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClipboardSnapshot {
	pub paths: Vec<PathBuf>,
	pub cut: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Op {
	Copy {
		sources: Vec<PathBuf>,
		dest: PathBuf,
		options: CopyOptions,
	},
	Move {
		sources: Vec<PathBuf>,
		dest: PathBuf,
		options: CopyOptions,
	},
	Link {
		sources: Vec<PathBuf>,
		dest: PathBuf,
	},
	Paste {
		snapshot: ClipboardSnapshot,
		dest: PathBuf,
		options: PasteOptions,
	},
	Rename {
		path: PathBuf,
		new_name: String,
	},
	Trash {
		paths: Vec<PathBuf>,
	},
	Destroy {
		paths: Vec<PathBuf>,
	},
	Delete {
		paths: Vec<PathBuf>,
	},
	NewFolder {
		parent: PathBuf,
	},
	NewTextFile {
		parent: PathBuf,
	},
	Duplicate {
		paths: Vec<PathBuf>,
	},
	RotateImage {
		path: PathBuf,
		degrees: i32,
	},
	Toggle {
		path: PathBuf,
	},
	Number {
		path: PathBuf,
		values: Vec<i64>,
	},
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OpKind {
	Copy,
	Move,
	Link,
	Paste,
	Rename,
	Trash,
	Destroy,
	Delete,
	NewFolder,
	NewTextFile,
	Duplicate,
	RotateImage,
	Toggle,
	Number,
	Launch,
}

impl Op {
	pub fn kind(&self) -> OpKind {
		match self {
			Self::Copy { .. } => OpKind::Copy,
			Self::Move { .. } => OpKind::Move,
			Self::Link { .. } => OpKind::Link,
			Self::Paste { .. } => OpKind::Paste,
			Self::Rename { .. } => OpKind::Rename,
			Self::Trash { .. } => OpKind::Trash,
			Self::Destroy { .. } => OpKind::Destroy,
			Self::Delete { .. } => OpKind::Delete,
			Self::NewFolder { .. } => OpKind::NewFolder,
			Self::NewTextFile { .. } => OpKind::NewTextFile,
			Self::Duplicate { .. } => OpKind::Duplicate,
			Self::RotateImage { .. } => OpKind::RotateImage,
			Self::Toggle { .. } => OpKind::Toggle,
			Self::Number { .. } => OpKind::Number,
		}
	}
}

impl OpKind {
	pub fn as_key(self) -> &'static str {
		match self {
			Self::Copy => "copy",
			Self::Move => "move",
			Self::Link => "link",
			Self::Paste => "paste",
			Self::Rename => "rename",
			Self::Trash => "trash",
			Self::Destroy => "destroy",
			Self::Delete => "delete",
			Self::NewFolder => "new-folder",
			Self::NewTextFile => "new-text-file",
			Self::Duplicate => "duplicate",
			Self::RotateImage => "rotate-image",
			Self::Toggle => "toggle",
			Self::Number => "number",
			Self::Launch => "launch",
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OpError {
	Io(String),
	NotFound(PathBuf),
	Exists(PathBuf),
	InvalidName(String),
	Unsupported(String),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpFailure {
	pub path: PathBuf,
	pub error: OpError,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeclinedDrop {
	pub path: PathBuf,
	pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpOutcome {
	pub succeeded: Vec<PathBuf>,
	pub failed: Vec<OpFailure>,
	pub declined: Vec<DeclinedDrop>,
	pub cancelled: bool,
}

pub type JobOutcome = OpOutcome;

/// Run one operation on the calling thread. Workers call this; it never touches
/// a frontend.
pub fn run(op: &Op, cancel: &AtomicBool) -> OpOutcome {
	let mut outcome = OpOutcome::default();
	match op {
		Op::Copy {
			sources,
			dest,
			options,
		} => transfer(sources, dest, *options, false, cancel, &mut outcome),
		Op::Move {
			sources,
			dest,
			options,
		} => transfer(sources, dest, *options, true, cancel, &mut outcome),
		Op::Link { sources, dest } => {
			for source in sources {
				if cancelled(cancel, &mut outcome) {
					break;
				}
				let target = destination_for(source, dest, sources.len());
				match link(source, &target) {
					Ok(()) => outcome.succeeded.push(target),
					Err(error) => fail(&mut outcome, source, error),
				}
			}
		}
		Op::Paste {
			snapshot,
			dest,
			options,
		} => {
			let copy = CopyOptions {
				overwrite: options.overwrite,
				..CopyOptions::default()
			};
			transfer(
				&snapshot.paths,
				dest,
				copy,
				options.cut,
				cancel,
				&mut outcome,
			);
		}
		Op::Rename { path, new_name } => rename(path, new_name, &mut outcome),
		Op::Trash { paths } => {
			if paths.is_empty() {
				return outcome;
			}
			if cancelled(cancel, &mut outcome) {
				return outcome;
			}
			match trash::delete_all(paths) {
				Ok(()) => outcome.succeeded.extend(paths.iter().cloned()),
				Err(error) => {
					for path in paths {
						fail(&mut outcome, path, OpError::Io(error.to_string()));
					}
				}
			}
		}
		Op::Delete { paths } => {
			for path in paths {
				if cancelled(cancel, &mut outcome) {
					break;
				}
				match remove(path) {
					Ok(()) => outcome.succeeded.push(path.clone()),
					Err(error) => fail(&mut outcome, path, error),
				}
			}
		}
		Op::Destroy { paths } => {
			for path in paths {
				fail(
					&mut outcome,
					path,
					OpError::Unsupported(String::from("Destroy= command")),
				);
			}
		}
		Op::NewFolder { parent } => match new_folder(parent) {
			Ok(path) => outcome.succeeded.push(path),
			Err(error) => fail(&mut outcome, parent, error),
		},
		Op::NewTextFile { parent } => match new_text_file(parent) {
			Ok(path) => outcome.succeeded.push(path),
			Err(error) => fail(&mut outcome, parent, error),
		},
		Op::Duplicate { paths } => {
			for path in paths {
				if cancelled(cancel, &mut outcome) {
					break;
				}
				match duplicate(path, cancel) {
					Ok(target) => outcome.succeeded.push(target),
					Err(error) => fail(&mut outcome, path, error),
				}
			}
		}
		Op::RotateImage { path, degrees } => match rotate_image(path, *degrees) {
			Ok(()) => outcome.succeeded.push(path.clone()),
			Err(error) => fail(&mut outcome, path, error),
		},
		Op::Toggle { path } => match toggle_entry(path) {
			Ok(()) => outcome.succeeded.push(path.clone()),
			Err(error) => fail(&mut outcome, path, error),
		},
		Op::Number { path, values } => match set_numbers(path, values) {
			Ok(()) => outcome.succeeded.push(path.clone()),
			Err(error) => fail(&mut outcome, path, error),
		},
	}
	outcome
}

fn cancelled(cancel: &AtomicBool, outcome: &mut OpOutcome) -> bool {
	if cancel.load(Ordering::Relaxed) {
		outcome.cancelled = true;
		true
	} else {
		false
	}
}

fn fail(outcome: &mut OpOutcome, path: &Path, error: OpError) {
	outcome.failed.push(OpFailure {
		path: path.to_path_buf(),
		error,
	});
}

fn transfer(
	sources: &[PathBuf],
	dest: &Path,
	options: CopyOptions,
	move_mode: bool,
	cancel: &AtomicBool,
	outcome: &mut OpOutcome,
) {
	if sources.len() > 1 && !dest.exists() {
		if let Err(error) = fs::create_dir_all(dest) {
			for source in sources {
				fail(outcome, source, OpError::Io(error.to_string()));
			}
			return;
		}
	}
	for source in sources {
		if cancelled(cancel, outcome) {
			break;
		}
		let target = destination_for(source, dest, sources.len());
		let result = if move_mode {
			move_item(source, &target, options, cancel)
		} else {
			copy_item(source, &target, options, cancel)
		};
		match result {
			Ok(()) => outcome.succeeded.push(target),
			Err(error) => fail(outcome, source, error),
		}
	}
}

fn destination_for(source: &Path, dest: &Path, source_count: usize) -> PathBuf {
	if dest.is_dir() {
		return source
			.file_name()
			.map(|name| dest.join(name))
			.unwrap_or_else(|| dest.to_path_buf());
	}
	if source_count == 1 {
		dest.to_path_buf()
	} else {
		source
			.file_name()
			.map(|name| dest.join(name))
			.unwrap_or_else(|| dest.to_path_buf())
	}
}

fn copy_item(
	source: &Path,
	target: &Path,
	options: CopyOptions,
	cancel: &AtomicBool,
) -> Result<(), OpError> {
	if cancel.load(Ordering::Relaxed) {
		return Ok(());
	}
	if let Some(parent) = target.parent() {
		fs::create_dir_all(parent).map_err(io_error)?;
	}
	let metadata = fs::symlink_metadata(source).map_err(io_error)?;
	if metadata.file_type().is_symlink() && !options.follow_symlinks {
		let link_target = fs::read_link(source).map_err(io_error)?;
		remove_for_overwrite(target, options.overwrite)?;
		return make_symlink(&link_target, target);
	}
	if metadata.is_dir() {
		remove_for_overwrite(target, options.overwrite)?;
		fs::create_dir_all(target).map_err(io_error)?;
		for entry in fs::read_dir(source).map_err(io_error)? {
			let entry = entry.map_err(io_error)?;
			copy_item(
				&entry.path(),
				&target.join(entry.file_name()),
				options,
				cancel,
			)?;
			if cancel.load(Ordering::Relaxed) {
				return Ok(());
			}
		}
		if options.preserve_times {
			copy_times(source, target)?;
		}
		return Ok(());
	}

	remove_for_overwrite(target, options.overwrite)?;
	fs::copy(source, target).map_err(io_error)?;
	if options.preserve_times {
		copy_times(source, target)?;
	}
	Ok(())
}

fn move_item(
	source: &Path,
	target: &Path,
	options: CopyOptions,
	cancel: &AtomicBool,
) -> Result<(), OpError> {
	if let Some(parent) = target.parent() {
		fs::create_dir_all(parent).map_err(io_error)?;
	}
	remove_for_overwrite(target, options.overwrite)?;
	match fs::rename(source, target) {
		Ok(()) => Ok(()),
		Err(error) if error.raw_os_error() == Some(18) => {
			copy_item(source, target, options, cancel)?;
			if cancel.load(Ordering::Relaxed) {
				return Ok(());
			}
			remove(source)
		}
		Err(error) => Err(io_error(error)),
	}
}

fn link(source: &Path, target: &Path) -> Result<(), OpError> {
	if target.exists() {
		return Err(OpError::Exists(target.to_path_buf()));
	}
	if let Some(parent) = target.parent() {
		fs::create_dir_all(parent).map_err(io_error)?;
	}
	make_symlink(source, target)
}

#[cfg(unix)]
fn make_symlink(target: &Path, link: &Path) -> Result<(), OpError> {
	std::os::unix::fs::symlink(target, link).map_err(io_error)
}

#[cfg(not(unix))]
fn make_symlink(_target: &Path, _link: &Path) -> Result<(), OpError> {
	Err(OpError::Unsupported(String::from("symlinks")))
}

fn rename(path: &Path, new_name: &str, outcome: &mut OpOutcome) {
	let new_name = new_name.trim();
	if new_name.is_empty() || new_name == "." || new_name == ".." || new_name.contains('/') {
		fail(outcome, path, OpError::InvalidName(new_name.to_string()));
		return;
	}
	let Some(parent) = path.parent() else {
		fail(outcome, path, OpError::InvalidName(new_name.to_string()));
		return;
	};
	let target = parent.join(new_name);
	if target == path {
		outcome.succeeded.push(path.to_path_buf());
		return;
	}
	if target.exists() {
		fail(outcome, path, OpError::Exists(target));
		return;
	}
	match fs::rename(path, &target) {
		Ok(()) => outcome.succeeded.push(target),
		Err(error) => fail(outcome, path, io_error(error)),
	}
}

fn remove(path: &Path) -> Result<(), OpError> {
	let metadata = fs::symlink_metadata(path).map_err(io_error)?;
	if metadata.is_dir() && !metadata.file_type().is_symlink() {
		fs::remove_dir_all(path).map_err(io_error)
	} else {
		fs::remove_file(path).map_err(io_error)
	}
}

fn remove_for_overwrite(target: &Path, overwrite: Overwrite) -> Result<(), OpError> {
	let Ok(metadata) = fs::symlink_metadata(target) else {
		return Ok(());
	};
	match overwrite {
		Overwrite::Never => Err(OpError::Exists(target.to_path_buf())),
		Overwrite::Replace => {
			if metadata.is_dir() && !metadata.file_type().is_symlink() {
				fs::remove_dir_all(target).map_err(io_error)
			} else {
				fs::remove_file(target).map_err(io_error)
			}
		}
	}
}

fn copy_times(source: &Path, target: &Path) -> Result<(), OpError> {
	let metadata = fs::metadata(source).map_err(io_error)?;
	let accessed = filetime::FileTime::from_last_access_time(&metadata);
	let modified = filetime::FileTime::from_last_modification_time(&metadata);
	filetime::set_file_times(target, accessed, modified).map_err(io_error)
}

fn new_folder(parent: &Path) -> Result<PathBuf, OpError> {
	let target = unique_path(parent, "New Folder", "");
	fs::create_dir(&target).map_err(io_error)?;
	Ok(target)
}

fn new_text_file(parent: &Path) -> Result<PathBuf, OpError> {
	let target = unique_path(parent, "New Text File", ".txt");
	fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(&target)
		.map_err(io_error)?;
	Ok(target)
}

fn duplicate(path: &Path, cancel: &AtomicBool) -> Result<PathBuf, OpError> {
	let parent = path
		.parent()
		.ok_or_else(|| OpError::InvalidName(path.to_string_lossy().into_owned()))?;
	let stem = path
		.file_stem()
		.and_then(|stem| stem.to_str())
		.unwrap_or("copy");
	let extension = path
		.extension()
		.and_then(|extension| extension.to_str())
		.map(|extension| format!(".{extension}"))
		.unwrap_or_default();
	let target = unique_path(parent, &format!("{stem} copy"), &extension);
	copy_item(path, &target, CopyOptions::default(), cancel)?;
	Ok(target)
}

fn unique_path(parent: &Path, stem: &str, extension: &str) -> PathBuf {
	let first = parent.join(format!("{stem}{extension}"));
	if !first.exists() {
		return first;
	}
	for index in 2..=9_999 {
		let candidate = parent.join(format!("{stem} {index}{extension}"));
		if !candidate.exists() {
			return candidate;
		}
	}
	parent.join(format!("{stem} 10000{extension}"))
}

fn rotate_image(path: &Path, degrees: i32) -> Result<(), OpError> {
	let image = image::open(path).map_err(|error| OpError::Io(error.to_string()))?;
	let rotated = match (degrees % 360 + 360) % 360 {
		0 => return Ok(()),
		90 => image.rotate90(),
		180 => image.rotate180(),
		270 => image.rotate270(),
		_ => {
			return Err(OpError::Unsupported(format!(
				"rotation of {degrees} degrees"
			)));
		}
	};
	rotated
		.save(path)
		.map_err(|error| OpError::Io(error.to_string()))
}

fn toggle_entry(path: &Path) -> Result<(), OpError> {
	let entry = crate::entry::Entry::read(path, crate::item::FsKind::File)
		.ok_or_else(|| OpError::Unsupported(String::from("not an entry")))?;
	let value = if entry.toggle { "false" } else { "true" };
	crate::config_parser::ConfigParser::set_value(path, "Desktop Entry", "Toggle", value);
	Ok(())
}

fn set_numbers(path: &Path, values: &[i64]) -> Result<(), OpError> {
	if crate::entry::Entry::read(path, crate::item::FsKind::File).is_none() {
		return Err(OpError::Unsupported(String::from("not an entry")));
	}
	for (index, value) in values.iter().enumerate() {
		crate::config_parser::ConfigParser::set_value(
			path,
			"Desktop Entry",
			&format!("Number{}Value", index + 1),
			&value.to_string(),
		);
	}
	Ok(())
}

fn io_error(error: std::io::Error) -> OpError {
	OpError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tree(name: &str) -> PathBuf {
		let root = std::env::temp_dir().join(format!("hishell-ops-{name}-{}", std::process::id()));
		let _ = fs::remove_dir_all(&root);
		fs::create_dir_all(&root).unwrap();
		root
	}

	fn run_ok(op: Op) -> OpOutcome {
		let outcome = run(&op, &AtomicBool::new(false));
		assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
		outcome
	}

	#[test]
	fn copies_files_and_trees() {
		let root = tree("copy");
		let source = root.join("a.txt");
		fs::write(&source, b"hello").unwrap();
		let folder = root.join("folder");
		fs::create_dir_all(folder.join("inner")).unwrap();
		fs::write(folder.join("inner/b.txt"), b"world").unwrap();
		let dest = root.join("dest");
		fs::create_dir_all(&dest).unwrap();

		run_ok(Op::Copy {
			sources: vec![source, folder],
			dest: dest.clone(),
			options: CopyOptions::default(),
		});
		assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"hello");
		assert_eq!(fs::read(dest.join("folder/inner/b.txt")).unwrap(), b"world");
		fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn copy_never_overwrites_by_default() {
		let root = tree("overwrite");
		let source = root.join("a.txt");
		fs::write(&source, b"one").unwrap();
		let dest = root.join("dest");
		fs::create_dir_all(&dest).unwrap();
		fs::write(dest.join("a.txt"), b"two").unwrap();
		let outcome = run(
			&Op::Copy {
				sources: vec![source],
				dest,
				options: CopyOptions::default(),
			},
			&AtomicBool::new(false),
		);
		assert_eq!(outcome.failed.len(), 1);
		fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn moves_duplicates_renames_and_creates() {
		let root = tree("move");
		let source = root.join("a.txt");
		fs::write(&source, b"hello").unwrap();
		let other = root.join("other");
		fs::create_dir_all(&other).unwrap();
		run_ok(Op::Move {
			sources: vec![source.clone()],
			dest: other.clone(),
			options: CopyOptions::default(),
		});
		assert!(!source.exists());
		assert!(other.join("a.txt").exists());

		let duplicate = run_ok(Op::Duplicate {
			paths: vec![other.join("a.txt")],
		});
		assert_eq!(duplicate.succeeded[0].file_name().unwrap(), "a copy.txt");

		run_ok(Op::Rename {
			path: other.join("a.txt"),
			new_name: String::from("b.txt"),
		});
		assert!(other.join("b.txt").exists());

		let folder = run_ok(Op::NewFolder {
			parent: root.clone(),
		});
		assert!(folder.succeeded[0].is_dir());
		let text = run_ok(Op::NewTextFile {
			parent: root.clone(),
		});
		assert!(text.succeeded[0].is_file());
		fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn delete_removes_files_and_folders() {
		let root = tree("delete");
		let file = root.join("a.txt");
		let folder = root.join("folder");
		fs::write(&file, b"x").unwrap();
		fs::create_dir_all(&folder).unwrap();
		fs::write(folder.join("b.txt"), b"x").unwrap();
		run_ok(Op::Delete {
			paths: vec![file.clone(), folder.clone()],
		});
		assert!(!file.exists());
		assert!(!folder.exists());
		fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn cancellation_stops_a_transfer() {
		let root = tree("cancel");
		let source = root.join("a.txt");
		fs::write(&source, b"x").unwrap();
		let dest = root.join("dest");
		fs::create_dir_all(&dest).unwrap();
		let cancel = AtomicBool::new(true);
		let outcome = run(
			&Op::Copy {
				sources: vec![source],
				dest,
				options: CopyOptions::default(),
			},
			&cancel,
		);
		assert!(outcome.cancelled);
		fs::remove_dir_all(root).unwrap();
	}
}

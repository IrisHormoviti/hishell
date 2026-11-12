//! Synchronous filesystem operations.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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

/// Receives transfer progress and owns the destination's `[Progress]` metadata.
pub trait ProgressSink {
	fn begin(&mut self, path: &Path, is_dir: bool, task: &str, target: u64);
	fn update(&mut self, path: &Path, value: u64, total: u64);
	fn finish(&mut self, path: &Path);
}

struct NoProgress;

impl ProgressSink for NoProgress {
	fn begin(&mut self, _path: &Path, _is_dir: bool, _task: &str, _target: u64) {}
	fn update(&mut self, _path: &Path, _value: u64, _total: u64) {}
	fn finish(&mut self, _path: &Path) {}
}

#[derive(Clone, Copy)]
struct Control<'a> {
	cancel: &'a AtomicBool,
	paused: Option<&'a AtomicBool>,
}

impl Control<'_> {
	/// True when the job was cancelled. While paused this blocks until the
	/// user resumes or stops it.
	fn stopped(&self) -> bool {
		loop {
			if self.cancel.load(Ordering::Relaxed) {
				return true;
			}
			if self
				.paused
				.is_some_and(|paused| paused.load(Ordering::Relaxed))
			{
				std::thread::sleep(Duration::from_millis(40));
				continue;
			}
			return false;
		}
	}
}

/// Run one operation on the calling thread. Workers call this; it never touches
/// a frontend.
pub fn run(op: &Op, cancel: &AtomicBool) -> OpOutcome {
	run_with(op, cancel, None, &mut NoProgress)
}

pub fn run_with(
	op: &Op,
	cancel: &AtomicBool,
	paused: Option<&AtomicBool>,
	progress: &mut dyn ProgressSink,
) -> OpOutcome {
	let control = Control { cancel, paused };
	let mut outcome = OpOutcome::default();
	match op {
		Op::Copy {
			sources,
			dest,
			options,
		} => transfer(
			sources,
			dest,
			TransferSpec {
				options: *options,
				move_mode: false,
				control,
				task: "Copying...",
			},
			progress,
			&mut outcome,
		),
		Op::Move {
			sources,
			dest,
			options,
		} => transfer(
			sources,
			dest,
			TransferSpec {
				options: *options,
				move_mode: true,
				control,
				task: "Moving...",
			},
			progress,
			&mut outcome,
		),
		Op::Link { sources, dest } => {
			for source in sources {
				if control.stopped() {
					outcome.cancelled = true;
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
				TransferSpec {
					options: copy,
					move_mode: options.cut,
					control,
					task: if options.cut {
						"Moving..."
					} else {
						"Copying..."
					},
				},
				progress,
				&mut outcome,
			);
		}
		Op::Rename { path, new_name } => rename(path, new_name, &mut outcome),
		Op::Trash { paths } => {
			if paths.is_empty() {
				return outcome;
			}
			if control.stopped() {
				outcome.cancelled = true;
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
				if control.stopped() {
					outcome.cancelled = true;
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
				if control.stopped() {
					outcome.cancelled = true;
					break;
				}
				match duplicate(path, control, progress) {
					Ok(Some(target)) => outcome.succeeded.push(target),
					Ok(None) => {
						outcome.cancelled = true;
						break;
					}
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

fn fail(outcome: &mut OpOutcome, path: &Path, error: OpError) {
	outcome.failed.push(OpFailure {
		path: path.to_path_buf(),
		error,
	});
}

struct TransferSpec<'a> {
	options: CopyOptions,
	move_mode: bool,
	control: Control<'a>,
	task: &'a str,
}

fn transfer(
	sources: &[PathBuf],
	dest: &Path,
	spec: TransferSpec,
	progress: &mut dyn ProgressSink,
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
		if spec.control.stopped() {
			outcome.cancelled = true;
			break;
		}
		let target = destination_for(source, dest, sources.len());
		let result = if spec.move_mode {
			move_item(
				source,
				&target,
				spec.options,
				spec.control,
				spec.task,
				progress,
			)
		} else {
			copy_root(
				source,
				&target,
				spec.options,
				spec.control,
				spec.task,
				progress,
			)
		};
		match result {
			Ok(true) => outcome.succeeded.push(target),
			Ok(false) => {
				outcome.cancelled = true;
				break;
			}
			Err(error) => fail(outcome, source, error),
		}
	}
}

pub(crate) fn destination_for(source: &Path, dest: &Path, source_count: usize) -> PathBuf {
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

struct CopyContext<'a> {
	progress: &'a mut dyn ProgressSink,
	task: &'a str,
	total: u64,
	root: PathBuf,
	done: u64,
	started: bool,
	/// The destination is a folder with a temporary progress `.directory`.
	dir_progress: bool,
}

impl CopyContext<'_> {
	fn start(&mut self, is_dir: bool) {
		if self.started {
			return;
		}
		self.started = true;
		self.dir_progress = is_dir;
		self.progress
			.begin(&self.root, is_dir, self.task, self.total);
	}

	fn add(&mut self, bytes: u64) {
		self.done = self.done.saturating_add(bytes).min(self.total);
		self.progress.update(&self.root, self.done, self.total);
	}

	fn finish(&mut self) {
		if self.started {
			self.started = false;
			self.progress.finish(&self.root);
		}
	}
}

fn copy_root(
	source: &Path,
	target: &Path,
	options: CopyOptions,
	control: Control,
	task: &str,
	progress: &mut dyn ProgressSink,
) -> Result<bool, OpError> {
	let total = total_size(source).max(1);
	let mut context = CopyContext {
		progress,
		task,
		total,
		root: target.to_path_buf(),
		done: 0,
		started: false,
		dir_progress: false,
	};
	let result = copy_item(source, target, options, control, &mut context, true);
	context.finish();
	result
}

fn copy_item(
	source: &Path,
	target: &Path,
	options: CopyOptions,
	control: Control,
	context: &mut CopyContext,
	top: bool,
) -> Result<bool, OpError> {
	if control.stopped() {
		return Ok(false);
	}
	if source == target {
		return Ok(true);
	}
	if source.is_dir() && target.starts_with(source) {
		return Err(OpError::InvalidName(format!(
			"cannot copy {} into itself",
			source.display()
		)));
	}
	if let Some(parent) = target.parent() {
		fs::create_dir_all(parent).map_err(io_error)?;
	}
	let metadata = fs::symlink_metadata(source).map_err(io_error)?;
	if metadata.file_type().is_symlink() && !options.follow_symlinks {
		let link_target = fs::read_link(source).map_err(io_error)?;
		remove_for_overwrite(target, options.overwrite)?;
		if top {
			context.start(false);
		}
		make_symlink(&link_target, target)?;
		return Ok(true);
	}
	if metadata.is_dir() {
		remove_for_overwrite(target, options.overwrite)?;
		fs::create_dir_all(target).map_err(io_error)?;
		if top {
			context.start(true);
		}
		for entry in fs::read_dir(source).map_err(io_error)? {
			let entry = entry.map_err(io_error)?;
			if !copy_item(
				&entry.path(),
				&target.join(entry.file_name()),
				options,
				control,
				context,
				false,
			)? {
				return Ok(false);
			}
		}
		if options.preserve_times {
			copy_times(source, target)?;
		}
		return Ok(true);
	}

	// A folder copy writes its progress into `<target>/.directory`. When the
	// source folder has a `.directory` of its own, merge that metadata into
	// the temporary file instead of replacing the progress group.
	if context.dir_progress
		&& source
			.file_name()
			.is_some_and(|name| name == std::ffi::OsStr::new(".directory"))
		&& target.exists()
	{
		copy_directory_entry(source, target, context)?;
		if options.preserve_times {
			copy_times(source, target)?;
		}
		return Ok(true);
	}

	remove_for_overwrite(target, options.overwrite)?;
	if top {
		context.start(false);
	}
	if !copy_file(source, target, control, context)? {
		return Ok(false);
	}
	if options.preserve_times {
		copy_times(source, target)?;
	}
	Ok(true)
}

fn copy_file(
	source: &Path,
	target: &Path,
	control: Control,
	context: &mut CopyContext,
) -> Result<bool, OpError> {
	let mut input = fs::File::open(source).map_err(io_error)?;
	let mut output = fs::OpenOptions::new()
		.write(true)
		.create(true)
		.truncate(true)
		.open(target)
		.map_err(io_error)?;
	let mut buffer = vec![0u8; 1024 * 1024];
	loop {
		if control.stopped() {
			return Ok(false);
		}
		let read = input.read(&mut buffer).map_err(io_error)?;
		if read == 0 {
			break;
		}
		output.write_all(&buffer[..read]).map_err(io_error)?;
		context.add(read as u64);
	}
	output.flush().map_err(io_error)?;
	let permissions = fs::metadata(source).map_err(io_error)?.permissions();
	fs::set_permissions(target, permissions).map_err(io_error)?;
	Ok(true)
}

/// Copy a source `.directory` over the progress file while keeping the live
/// `[Progress]` group the transfer is writing.
fn copy_directory_entry(
	source: &Path,
	target: &Path,
	context: &mut CopyContext,
) -> Result<(), OpError> {
	let written = crate::progress::merge_section_file(source, target, crate::progress::SECTION)
		.map_err(io_error)?;
	context.add(written as u64);
	Ok(())
}

fn move_item(
	source: &Path,
	target: &Path,
	options: CopyOptions,
	control: Control,
	task: &str,
	progress: &mut dyn ProgressSink,
) -> Result<bool, OpError> {
	if control.stopped() {
		return Ok(false);
	}
	if source == target {
		return Ok(true);
	}
	if source.is_dir() && target.starts_with(source) {
		return Err(OpError::InvalidName(format!(
			"cannot move {} into itself",
			source.display()
		)));
	}
	if let Some(parent) = target.parent() {
		fs::create_dir_all(parent).map_err(io_error)?;
	}
	remove_for_overwrite(target, options.overwrite)?;
	match fs::rename(source, target) {
		Ok(()) => Ok(true),
		Err(error) if error.raw_os_error() == Some(18) => {
			if !copy_root(source, target, options, control, task, progress)? {
				return Ok(false);
			}
			remove(source)?;
			Ok(true)
		}
		Err(error) => Err(io_error(error)),
	}
}

fn total_size(path: &Path) -> u64 {
	let Ok(metadata) = fs::symlink_metadata(path) else {
		return 0;
	};
	if metadata.file_type().is_symlink() {
		return 0;
	}
	if metadata.is_file() {
		return metadata.len();
	}
	if !metadata.is_dir() {
		return 0;
	}
	let mut total: u64 = 0;
	if let Ok(entries) = fs::read_dir(path) {
		for entry in entries.flatten() {
			total = total.saturating_add(total_size(&entry.path()));
		}
	}
	total
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

fn duplicate(
	path: &Path,
	control: Control,
	progress: &mut dyn ProgressSink,
) -> Result<Option<PathBuf>, OpError> {
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
	if !copy_root(
		path,
		&target,
		CopyOptions::default(),
		control,
		"Duplicating...",
		progress,
	)? {
		return Ok(None);
	}
	Ok(Some(target))
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
	fn moving_a_file_back_to_its_own_folder_keeps_it() {
		let root = tree("move-self");
		let source = root.join("a.txt");
		fs::write(&source, b"keep me").unwrap();
		run(
			&Op::Move {
				sources: vec![source.clone()],
				dest: root.clone(),
				options: CopyOptions {
					overwrite: Overwrite::Replace,
					..CopyOptions::default()
				},
			},
			&AtomicBool::new(false),
		);
		assert_eq!(fs::read(&source).unwrap(), b"keep me");
		fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn moving_a_folder_into_itself_keeps_it() {
		let root = tree("move-into-self");
		let folder = root.join("folder");
		fs::create_dir_all(folder.join("inner")).unwrap();
		fs::write(folder.join("inner/a.txt"), b"keep me").unwrap();
		let outcome = run(
			&Op::Move {
				sources: vec![folder.clone()],
				dest: folder.clone(),
				options: CopyOptions {
					overwrite: Overwrite::Replace,
					..CopyOptions::default()
				},
			},
			&AtomicBool::new(false),
		);
		assert_eq!(outcome.failed.len(), 1);
		assert_eq!(fs::read(folder.join("inner/a.txt")).unwrap(), b"keep me");
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

	struct TrackingSink {
		trackers: std::collections::HashMap<PathBuf, crate::progress::Tracker>,
		begun: Vec<PathBuf>,
		updates: Vec<u64>,
		finished: Vec<PathBuf>,
		sidecar_at_begin: Vec<bool>,
	}

	impl ProgressSink for TrackingSink {
		fn begin(&mut self, path: &Path, is_dir: bool, task: &str, target: u64) {
			let tracker = crate::progress::Tracker::begin(path, is_dir, task, target);
			self.sidecar_at_begin
				.push(crate::progress::entry_path(path, is_dir).is_file());
			self.begun.push(path.to_path_buf());
			self.trackers.insert(path.to_path_buf(), tracker);
		}

		fn update(&mut self, path: &Path, value: u64, _total: u64) {
			self.updates.push(value);
			if let Some(tracker) = self.trackers.get_mut(path) {
				tracker.update(value);
			}
		}

		fn finish(&mut self, path: &Path) {
			self.finished.push(path.to_path_buf());
			if let Some(tracker) = self.trackers.remove(path) {
				tracker.finish();
			}
		}
	}

	#[test]
	fn a_copy_reports_progress_and_removes_its_sidecar() {
		let root = tree("progress");
		let source = root.join("a.bin");
		fs::write(&source, vec![0u8; 64 * 1024]).unwrap();
		let dest = root.join("dest");
		fs::create_dir_all(&dest).unwrap();
		let mut sink = TrackingSink {
			trackers: std::collections::HashMap::new(),
			begun: Vec::new(),
			updates: Vec::new(),
			finished: Vec::new(),
			sidecar_at_begin: Vec::new(),
		};
		let outcome = run_with(
			&Op::Copy {
				sources: vec![source],
				dest: dest.clone(),
				options: CopyOptions::default(),
			},
			&AtomicBool::new(false),
			None,
			&mut sink,
		);
		assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
		let target = dest.join("a.bin");
		assert_eq!(sink.begun, vec![target.clone()]);
		assert_eq!(sink.finished, vec![target.clone()]);
		assert_eq!(sink.sidecar_at_begin, vec![true]);
		assert!(sink.updates.iter().any(|value| *value > 0));
		assert!(!crate::progress::entry_path(&target, false).exists());
		assert!(target.is_file());
		fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_folder_copy_keeps_the_source_directory_metadata() {
		let root = tree("progress-folder");
		let source = root.join("app");
		fs::create_dir_all(&source).unwrap();
		fs::write(
			source.join(".directory"),
			"[Desktop Entry]\nType=Application\nName=App\nExec=./app\n",
		)
		.unwrap();
		fs::write(source.join("app"), b"x").unwrap();
		let dest = root.join("dest");
		fs::create_dir_all(&dest).unwrap();
		let mut sink = TrackingSink {
			trackers: std::collections::HashMap::new(),
			begun: Vec::new(),
			updates: Vec::new(),
			finished: Vec::new(),
			sidecar_at_begin: Vec::new(),
		};
		let outcome = run_with(
			&Op::Copy {
				sources: vec![source.clone()],
				dest: dest.clone(),
				options: CopyOptions::default(),
			},
			&AtomicBool::new(false),
			None,
			&mut sink,
		);
		assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
		let entry = dest.join("app/.directory");
		let text = fs::read_to_string(&entry).unwrap();
		assert!(text.contains("Name=App"));
		assert!(!text.contains("[Progress]"));
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

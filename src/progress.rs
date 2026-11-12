//! Live progress metadata for a file or folder.
//!
//! A downloader, a browser or Hishell itself writes a `[Progress]` group into
//! the item's sidecar (`<file>.desktop`) or into a folder's `.directory`. The
//! item is then shown while the target does not exist yet, it cannot be
//! opened, and the progress can be paused or stopped.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::config_parser::{ConfigParser, ConfigValue};

pub const SECTION: &str = "Progress";

/// Paths a tracker in this process is currently writing. Two engines in one
/// process share this, so one opening a folder cannot clear the other's live
/// transfer. It dies with the process, which makes abandoned groups stale.
static ACTIVE: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// A transfer worker updates `Value` while the engine writes `Paused` for the
/// same sidecar. Serialize those read-modify-write passes so neither loses the
/// other's key.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn set_section(path: &Path, section: &str, values: &[(&str, &str)]) {
	let _guard = WRITE_LOCK.lock();
	ConfigParser::set_section(path, section, values);
}

fn set_value(path: &Path, section: &str, key: &str, value: &str) {
	let _guard = WRITE_LOCK.lock();
	ConfigParser::set_value(path, section, key, value);
}

pub fn remove_section(path: &Path, section: &str) {
	let _guard = WRITE_LOCK.lock();
	ConfigParser::remove_section(path, section);
}

pub fn set_paused(path: &Path, paused: bool) {
	set_value(
		path,
		SECTION,
		"Paused",
		if paused { "true" } else { "false" },
	);
}

pub fn is_active(path: &Path) -> bool {
	ACTIVE.lock().is_ok_and(|active| active.contains(path))
}

fn mark_active(path: &Path) {
	if let Ok(mut active) = ACTIVE.lock() {
		active.insert(path.to_path_buf());
	}
}

fn unmark_active(path: &Path) {
	if let Ok(mut active) = ACTIVE.lock() {
		active.remove(path);
	}
}

/// One live task attached to a file or folder.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Progress {
	pub value: f64,
	pub target: f64,
	pub task_name: Option<String>,
	/// Commands an external authority runs when the user presses Pause.
	pub pause: Option<String>,
	/// Commands an external authority runs when the user presses Resume.
	pub resume: Option<String>,
	/// Commands an external authority runs when the user presses Stop.
	pub stop: Option<String>,
	/// Whether an external authority has paused the task.
	pub paused: bool,
	/// The program that owns the task. Hishell writes `Hishell` for its own
	/// transfers; an external authority may set its own name.
	pub owner: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ProgressAction {
	Pause,
	Resume,
	Stop,
}

/// The controls a frontend may expose for one item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProgressControl {
	pub can_pause: bool,
	pub can_resume: bool,
	pub can_stop: bool,
	pub paused: bool,
}

impl Progress {
	/// Read a `[Progress]` group. A missing or non-positive `Target` means the
	/// group carries no usable progress.
	pub fn from_values(values: &HashMap<String, ConfigValue>) -> Option<Self> {
		let target = number(values.get("Target"))?;
		if !target.is_finite() || target <= 0.0 {
			return None;
		}
		let value = number(values.get("Value"))
			.unwrap_or(0.0)
			.clamp(0.0, target);
		Some(Self {
			value,
			target,
			task_name: string(values.get("TaskName")),
			pause: string(values.get("Pause")),
			resume: string(values.get("Resume")),
			stop: string(values.get("Stop")),
			paused: boolean(values.get("Paused")),
			owner: string(values.get("Owner")),
		})
	}

	/// Completion in the range `0.0..=1.0`.
	pub fn fraction(&self) -> f64 {
		if self.target <= 0.0 {
			return 0.0;
		}
		(self.value / self.target).clamp(0.0, 1.0)
	}

	/// Completion as a whole percentage, for display only.
	pub fn percent(&self) -> u32 {
		(self.fraction() * 100.0).round() as u32
	}

	pub fn is_complete(&self) -> bool {
		self.target > 0.0 && self.value >= self.target
	}

	pub fn control(&self) -> ProgressControl {
		ProgressControl {
			can_pause: !self.paused && self.pause.is_some(),
			can_resume: self.paused && self.resume.is_some(),
			can_stop: self.stop.is_some(),
			paused: self.paused,
		}
	}

	pub fn command(&self, action: ProgressAction) -> Option<&str> {
		match action {
			ProgressAction::Pause => self.pause.as_deref(),
			ProgressAction::Resume => self.resume.as_deref(),
			ProgressAction::Stop => self.stop.as_deref(),
		}
	}
}

/// Expand a progress command. `%p` is the item path, `%k` the sidecar or
/// `.directory` path, `%n`/`%c` the display name and `%%` a literal percent.
pub fn expand_command(
	command: &str,
	item: &Path,
	entry: &Path,
	name: &str,
) -> Result<Vec<String>, String> {
	let tokens = crate::entry::tokenize_exec(command)?;
	let item = item.to_string_lossy();
	let entry = entry.to_string_lossy();
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
				Some('p') => expanded.push_str(&item),
				Some('k') => expanded.push_str(&entry),
				Some('n' | 'c') => expanded.push_str(name),
				Some(other) => {
					return Err(format!(
						"unsupported field code %{other} in progress command"
					));
				}
				None => {
					return Err(String::from(
						"progress command ends with an incomplete field code",
					));
				}
			}
		}
		if !expanded.is_empty() {
			args.push(expanded);
		}
	}
	Ok(args)
}

/// Whether an entry file still carries metadata beyond a bare `Type=`.
pub fn has_meaningful_keys(path: &Path) -> bool {
	let _guard = WRITE_LOCK.lock();
	let Ok(text) = std::fs::read_to_string(path) else {
		return false;
	};
	text.lines().any(|line| {
		let line = line.trim();
		!line.is_empty()
			&& !line.starts_with('[')
			&& !line.starts_with('#')
			&& !line.starts_with(';')
			&& !line.starts_with("Type")
	})
}

/// Remove a group from entry text, keeping every other line.
pub fn strip_section(text: &str, section: &str) -> String {
	let mut out = String::new();
	let mut inside = false;
	for line in text.lines() {
		let trimmed = line.trim();
		if trimmed.starts_with('[') && trimmed.ends_with(']') {
			inside = &trimmed[1..trimmed.len() - 1] == section;
		}
		if !inside {
			out.push_str(line);
			out.push('\n');
		}
	}
	out
}

fn section_text(text: &str, section: &str) -> Option<String> {
	let mut out = String::new();
	let mut inside = false;
	for line in text.lines() {
		let trimmed = line.trim();
		if trimmed.starts_with('[') && trimmed.ends_with(']') {
			inside = &trimmed[1..trimmed.len() - 1] == section;
		}
		if inside {
			out.push_str(line);
			out.push('\n');
		}
	}
	(!out.is_empty()).then_some(out)
}

/// Copy `source` over `target` while keeping `target`'s live section.
///
/// A folder transfer uses this to install the source's own `.directory` over
/// the progress file it created, without dropping the `[Progress]` group.
pub fn merge_section_file(source: &Path, target: &Path, section: &str) -> std::io::Result<usize> {
	let _guard = WRITE_LOCK.lock();
	let mut text = strip_section(&std::fs::read_to_string(source)?, section);
	if let Ok(existing) = std::fs::read_to_string(target)
		&& let Some(progress) = section_text(&existing, section)
	{
		if !text.is_empty() && !text.ends_with('\n') {
			text.push('\n');
		}
		text.push_str(&progress);
	}
	std::fs::write(target, text.as_bytes())?;
	Ok(text.len())
}

/// The sidecar path that carries an item's `[Progress]` group.
pub fn entry_path(item: &Path, is_dir: bool) -> PathBuf {
	if is_dir {
		return item.join(".directory");
	}
	let mut path = item.as_os_str().to_os_string();
	path.push(".desktop");
	PathBuf::from(path)
}

/// Writes and updates the `[Progress]` group for one item, then removes it.
///
/// The tracker never removes metadata it did not create. A sidecar or
/// `.directory` that already existed is left in place with its other groups
/// intact; a file Hishell created only for the transfer is removed.
pub struct Tracker {
	item: PathBuf,
	entry: PathBuf,
	created: bool,
	target: u64,
	last_percent: i64,
	last_write: Instant,
	done: bool,
}

impl Tracker {
	pub fn begin(item: &Path, is_dir: bool, task: &str, target: u64) -> Self {
		let entry = entry_path(item, is_dir);
		let created = !entry.exists();
		mark_active(item);
		if created {
			let kind = if is_dir { "Directory" } else { "File" };
			set_section(&entry, "Desktop Entry", &[("Type", kind)]);
		}
		let target = target.max(1);
		set_section(
			&entry,
			SECTION,
			&[
				("Value", "0"),
				("Target", &target.to_string()),
				("TaskName", task),
				("Paused", "false"),
				("Owner", "Hishell"),
			],
		);
		Self {
			item: item.to_path_buf(),
			entry,
			created,
			target,
			last_percent: 0,
			last_write: Instant::now() - Duration::from_secs(1),
			done: false,
		}
	}

	pub fn update(&mut self, value: u64) {
		if self.done {
			return;
		}
		let value = value.min(self.target);
		let percent = percent(value, self.target);
		let now = Instant::now();
		if percent == self.last_percent
			&& value < self.target
			&& now.duration_since(self.last_write) < Duration::from_millis(250)
		{
			return;
		}
		set_value(&self.entry, SECTION, "Value", &value.to_string());
		self.last_percent = percent;
		self.last_write = now;
	}

	pub fn finish(mut self) {
		self.cleanup();
	}

	fn cleanup(&mut self) {
		if self.done {
			return;
		}
		self.done = true;
		unmark_active(&self.item);
		remove_section(&self.entry, SECTION);
		if self.created {
			// A folder copy may merge the source's own `.directory` into the
			// file we created. Keep it when it now carries real metadata; only
			// remove the skeleton that held the progress group.
			if !has_meaningful_keys(&self.entry) {
				let _ = std::fs::remove_file(&self.entry);
			}
		}
	}
}

impl Drop for Tracker {
	fn drop(&mut self) {
		self.cleanup();
	}
}

fn percent(value: u64, target: u64) -> i64 {
	if target == 0 {
		return 0;
	}
	((value.min(target) as f64 / target as f64) * 100.0).round() as i64
}

fn number(value: Option<&ConfigValue>) -> Option<f64> {
	match value? {
		ConfigValue::Number(value) => Some(*value),
		ConfigValue::String(value) => value.trim().parse().ok(),
		ConfigValue::Boolean(value) => Some(if *value { 1.0 } else { 0.0 }),
		_ => None,
	}
}

fn string(value: Option<&ConfigValue>) -> Option<String> {
	match value? {
		ConfigValue::String(value) => {
			let value = value.trim();
			(!value.is_empty()).then(|| value.to_string())
		}
		ConfigValue::Number(value) => Some(value.to_string()),
		ConfigValue::Boolean(value) => Some(value.to_string()),
		_ => None,
	}
}

fn boolean(value: Option<&ConfigValue>) -> bool {
	match value {
		Some(ConfigValue::Boolean(value)) => *value,
		Some(ConfigValue::String(value)) => value.eq_ignore_ascii_case("true"),
		_ => false,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_progress_fields_and_reports_completion() {
		let values = HashMap::from([
			(String::from("Value"), ConfigValue::Number(42.0)),
			(String::from("Target"), ConfigValue::Number(100.0)),
			(
				String::from("TaskName"),
				ConfigValue::String(String::from("Downloading...")),
			),
			(
				String::from("Pause"),
				ConfigValue::String(String::from("download pause %p")),
			),
			(String::from("Paused"), ConfigValue::Boolean(true)),
		]);
		let progress = Progress::from_values(&values).unwrap();
		assert_eq!(progress.percent(), 42);
		assert_eq!(progress.task_name.as_deref(), Some("Downloading..."));
		assert!(progress.control().paused);
		assert!(!progress.control().can_pause);
		assert!(!progress.control().can_resume);
	}

	#[test]
	fn a_missing_or_zero_target_is_not_progress() {
		assert!(Progress::from_values(&HashMap::new()).is_none());
		assert!(
			Progress::from_values(&HashMap::from([(
				String::from("Target"),
				ConfigValue::Number(0.0)
			)]))
			.is_none()
		);
	}

	#[test]
	fn commands_expand_the_item_and_entry_paths() {
		let args = expand_command(
			"download pause %p %k %n %%",
			Path::new("/home/movie.mkv"),
			Path::new("/home/movie.mkv.desktop"),
			"movie.mkv",
		)
		.unwrap();
		assert_eq!(
			args,
			vec![
				"download",
				"pause",
				"/home/movie.mkv",
				"/home/movie.mkv.desktop",
				"movie.mkv",
				"%",
			]
		);
	}

	#[test]
	fn a_tracker_writes_updates_and_removes_its_sidecar() {
		let root = std::env::temp_dir().join(format!("hishell-progress-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		let item = root.join("movie.mkv");
		let mut tracker = Tracker::begin(&item, false, "Downloading...", 100);
		let sidecar = root.join("movie.mkv.desktop");
		assert!(sidecar.is_file());
		tracker.update(40);
		let text = std::fs::read_to_string(&sidecar).unwrap();
		assert!(text.contains("Value = 40"));
		assert!(text.contains("Target = 100"));
		tracker.finish();
		assert!(!sidecar.exists());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_tracker_keeps_preexisting_metadata() {
		let root =
			std::env::temp_dir().join(format!("hishell-progress-keep-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		let item = root.join("movie.mkv");
		let sidecar = root.join("movie.mkv.desktop");
		std::fs::write(
			&sidecar,
			"[Desktop Entry]\nType=File\nName=Movie\nIcon=video-x-generic\n",
		)
		.unwrap();
		let tracker = Tracker::begin(&item, false, "Downloading...", 100);
		tracker.finish();
		let text = std::fs::read_to_string(&sidecar).unwrap();
		assert!(text.contains("Name=Movie"));
		assert!(!text.contains("[Progress]"));
		std::fs::remove_dir_all(root).unwrap();
	}
}

//! Descriptor generators: external programs that describe a file the way a
//! directory descriptor does, discovered by content type like a thumbnailer.
//!
//! A directory descriptor is a small desktop entry that names a directory and
//! gives it a `Name` and `Icon` (see [`crate::directory_descriptor`]). Some
//! files can describe themselves just as well — a DS ROM carries its own title
//! and icon — but the shell must not know how each format stores them. A
//! *descriptor generator* is the missing piece: a definition installed under
//! `hishell/descriptor-generators` in the XDG data directories, matched
//! against the file's content type exactly like a freedesktop thumbnailer:
//!
//! ```ini
//! [Descriptor Generator Entry]
//! TryExec=clam-nds-descriptor
//! Exec=clam-nds-descriptor %i %o
//! MimeType=application/vnd.nintendo.nitro.rom;
//! Icon=application-x-nintendo-ds-rom
//! ```
//!
//! `Icon` is optional: it names the themed placeholder the file view shows
//! before a descriptor exists and whenever generation fails, so a type owned by
//! a generator never falls back to a generic or another theme's icon.
//!
//! `%i` is the file, `%o` the desktop entry the generator must write. The
//! generated entry uses the same `Type=File`/`Path=` form as a directory
//! descriptor, so [`crate::directory_descriptor`] reads it with no format of
//! its own:
//!
//! ```ini
//! [Desktop Entry]
//! Type=File
//! Path=/roms/Game.nds
//! Name=Game
//! Comment=Nintendo DS ROM · USA · ABCE
//! Icon=/home/root/.cache/hishell/descriptors/<hash>.svg
//! ```
//!
//! Generated entries are cached under `$XDG_CACHE_HOME/hishell/descriptors`;
//! a `.stamp` beside each one records the source's modification time and size,
//! so a changed file is regenerated and an unchanged folder costs only a stat.
//! The work runs on the same kind of background workers as thumbnails: a folder
//! scan queues the files whose descriptor is missing, and the views pick the
//! finished titles and icons up on their next poll. A generator that hangs is
//! killed, and a generator that fails leaves the file's ordinary name and MIME
//! icon in place.

use crate::entry::Entry;
use crate::item::{FsKind, IconRef};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

/// A hung generator would otherwise hold a worker forever.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);

/// Generators are mostly I/O: a small number of workers is plenty and keeps a
/// folder scan from spawning one process per ROM at once.
const MAX_WORKERS: usize = 1;

/// Pending jobs per folder. Every item's title and icon depend on this work, so
/// the bound is generous; a folder bigger than this still gets the first items
/// immediately and the rest on the next visit.
const MAX_PENDING_PER_FOLDER: usize = 1024;

/// Finished results are kept briefly so every view showing the folder can pick
/// them up, then dropped.
const RESULT_TTL: Duration = Duration::from_secs(120);

/// Bump when a generator's output changes in a way the source fingerprint
/// cannot see (a new descriptor group, a different icon). Old stamps have no
/// version and are regenerated once.
const CACHE_VERSION: u32 = 3;

#[derive(Clone, Debug)]
struct Generator {
	mime_types: Vec<String>,
	exec: String,
	/// The themed placeholder for this type. A generated entry normally
	/// overrides it, but the file must still look deliberate while generation
	/// runs and after a generator fails.
	icon: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Generated {
	pub name: Option<String>,
	pub icon: Option<String>,
}

#[derive(Clone)]
struct Job {
	src: PathBuf,
	/// The cache path, not the source path: two names for the same file (a
	/// symlink and its target) must not generate the same entry twice.
	key: PathBuf,
	folder: PathBuf,
	mime: String,
	exec: String,
	epoch: u64,
}

#[derive(Default)]
struct Pending {
	queue: VecDeque<Job>,
	queued: HashSet<PathBuf>,
	folder_counts: HashMap<PathBuf, usize>,
	/// Jobs a worker has taken but not finished. Together with `folder_counts`
	/// this says whether a folder is still producing descriptors, so the view
	/// can apply one batch instead of repainting after every item.
	active_counts: HashMap<PathBuf, usize>,
	epochs: HashMap<PathBuf, u64>,
}

struct State {
	pending: Mutex<Pending>,
	cond: Condvar,
	results: Mutex<HashMap<String, (Generated, Instant)>>,
}

static STATE: OnceLock<State> = OnceLock::new();
static GENERATORS: OnceLock<Vec<Generator>> = OnceLock::new();

/// Bumped whenever a worker publishes a result. A directory compares this with
/// the value it last applied, so the model is only inspected when there is
/// actually something new; polling the whole item list on a timer forever would
/// keep the view repainting long after the descriptors are in place.
static RESULTS_EPOCH: AtomicU64 = AtomicU64::new(0);

/// The current result epoch. Callers remember it and skip work when unchanged.
pub fn results_epoch() -> u64 {
	RESULTS_EPOCH.load(Ordering::Relaxed)
}

fn state() -> &'static State {
	STATE.get_or_init(|| {
		let st = State {
			pending: Mutex::new(Pending::default()),
			cond: Condvar::new(),
			results: Mutex::new(HashMap::new()),
		};
		let workers = thread::available_parallelism()
			.map(|n| n.get())
			.unwrap_or(2)
			.min(MAX_WORKERS);
		for _ in 0..workers {
			thread::spawn(worker_loop);
		}
		st
	})
}

/// `$XDG_DATA_DIRS`, which the launcher sets to the bundle's share directory
/// ahead of `/usr/share`. The `dirs` crate only exposes the singular data
/// directory, so the user's own data directory is added separately.
fn data_dirs() -> Vec<PathBuf> {
	let mut dirs = Vec::new();
	if let Some(dir) = dirs::data_dir() {
		dirs.push(dir);
	}
	let value = std::env::var("XDG_DATA_DIRS")
		.unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
	for part in value.split(':').filter(|part| !part.is_empty()) {
		dirs.push(PathBuf::from(part));
	}
	dirs
}

/// Where generator definitions live, lowest precedence first.
fn generator_dirs() -> Vec<PathBuf> {
	data_dirs()
		.into_iter()
		.map(|dir| dir.join("hishell/descriptor-generators"))
		.collect()
}

/// The generated-descriptor cache. It is also a descriptor root in
/// [`crate::directory_descriptor`], so a cached entry needs no special lookup.
pub fn cache_root() -> PathBuf {
	dirs::cache_dir()
		.unwrap_or_else(|| PathBuf::from("/tmp"))
		.join("hishell")
		.join("descriptors")
}

fn command_available(name: &str) -> bool {
	if name.contains('/') {
		return Path::new(name).is_file();
	}
	std::env::var_os("PATH")
		.map(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
		.unwrap_or(false)
}

fn load_generators() -> &'static Vec<Generator> {
	GENERATORS.get_or_init(|| {
		let mut out = Vec::new();
		for dir in generator_dirs() {
			let Ok(entries) = fs::read_dir(&dir) else {
				continue;
			};
			let mut files: Vec<PathBuf> = entries
				.flatten()
				.map(|entry| entry.path())
				.filter(|path| {
					path.extension()
						.is_some_and(|ext| ext.eq_ignore_ascii_case("descriptor-generator"))
				})
				.collect();
			// A stable order means a user generator can be placed first by
			// naming, and the same definition always wins within a directory.
			files.sort();
			for file in files {
				if let Some(generator) = read_generator(&file) {
					out.push(generator);
				}
			}
		}
		out
	})
}

fn read_generator(file: &Path) -> Option<Generator> {
	let contents = fs::read_to_string(file).ok()?;
	parse_generator(&contents)
}

fn parse_generator(contents: &str) -> Option<Generator> {
	let mut exec_line = None;
	let mut try_exec = None;
	let mut icon = None;
	let mut mimes: Vec<String> = Vec::new();
	for line in contents.lines() {
		let line = line.trim();
		if let Some(rest) = line.strip_prefix("Exec=") {
			exec_line = Some(rest.trim().to_string());
		} else if let Some(rest) = line.strip_prefix("TryExec=") {
			try_exec = Some(rest.trim().to_string());
		} else if let Some(rest) = line.strip_prefix("Icon=") {
			icon = Some(rest.trim().to_string()).filter(|icon| !icon.is_empty());
		} else if let Some(rest) = line.strip_prefix("MimeType=") {
			for mime in rest.split(';') {
				let mime = mime.trim();
				if !mime.is_empty() {
					mimes.push(mime.to_string());
				}
			}
		}
	}
	let exec = exec_line.filter(|exec| !exec.is_empty())?;
	if mimes.is_empty() {
		return None;
	}
	if let Some(try_exec) = try_exec
		&& !try_exec.is_empty()
		&& !command_available(&try_exec)
	{
		return None;
	}
	Some(Generator {
		mime_types: mimes,
		exec,
		icon,
	})
}

fn mime_matches(pattern: &str, mime: &str) -> bool {
	if pattern == mime {
		return true;
	}
	if let Some(prefix) = pattern.strip_suffix("/*") {
		return mime.starts_with(&format!("{prefix}/"));
	}
	false
}

fn generator_for(mime: &str) -> Option<Generator> {
	load_generators()
		.iter()
		.find(|generator| {
			generator
				.mime_types
				.iter()
				.any(|pattern| mime_matches(pattern, mime))
		})
		.cloned()
}

/// The content type of a file through the same QMimeDatabase KDE uses, with
/// `mime_guess` as the fallback when Qt cannot answer.
fn mime_for(src: &Path) -> String {
	crate::icon::mime_for_path(src).unwrap_or_else(|| String::from("application/octet-stream"))
}

fn source_fingerprint(src: &Path) -> Option<(u64, u64)> {
	let meta = fs::metadata(src).ok()?;
	let mtime = meta
		.modified()
		.ok()
		.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
		.map(|duration| duration.as_secs())
		.unwrap_or(0);
	Some((mtime, meta.len()))
}

fn cache_path(src: &Path) -> PathBuf {
	let canonical = fs::canonicalize(src).unwrap_or_else(|_| src.to_path_buf());
	cache_root().join(format!(
		"{:x}.desktop",
		digest(&canonical.to_string_lossy())
	))
}

fn digest(input: &str) -> u64 {
	let mut hash = 0xcbf2_9ce4_8422_2325u64;
	for byte in input.as_bytes() {
		hash ^= *byte as u64;
		hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
	}
	hash
}

fn stamp_path(src: &Path) -> PathBuf {
	cache_path(src).with_extension("stamp")
}

fn stamp(src: &Path) -> Option<(u64, u64, u32)> {
	let value = fs::read_to_string(stamp_path(src)).ok()?;
	let mut parts = value.split_whitespace();
	let mtime = parts.next()?.parse().ok()?;
	let size = parts.next()?.parse().ok()?;
	let version = parts.next()?.parse().ok()?;
	Some((mtime, size, version))
}

/// Whether the entry's own image file still exists. A themed icon name needs
/// no file; an absolute or relative path does. Without this check a deleted
/// SVG made `cached()` accept the entry forever, so the ROM kept the
/// placeholder and the generator was never asked to write the image again.
fn icon_file_present(descriptor: &Path, entry: &Entry) -> bool {
	let Some(icon) = entry.icon.as_deref() else {
		return true;
	};
	if icon.trim().is_empty() {
		return true;
	}
	if icon.starts_with('/') {
		return Path::new(icon).is_file();
	}
	if icon.contains('/') || icon.starts_with('.') {
		return descriptor
			.parent()
			.map(|parent| parent.join(icon).is_file())
			.unwrap_or(false);
	}
	true
}

/// Whether the cached entry still describes this exact version of the file.
fn cached(src: &Path) -> Option<PathBuf> {
	let out = cache_path(src);
	if !out.is_file() {
		return None;
	}
	let (mtime, size) = source_fingerprint(src)?;
	if stamp(src)? != (mtime, size, CACHE_VERSION) {
		return None;
	}
	let entry = Entry::read(&out, FsKind::File)?;
	if !icon_file_present(&out, &entry) {
		return None;
	}
	Some(out)
}

fn write_stamp(src: &Path) -> Result<(), String> {
	let path = stamp_path(src);
	let (mtime, size) = source_fingerprint(src).ok_or("cannot stat source")?;
	let tmp = path.with_extension(format!("stamp.tmp{}", std::process::id()));
	fs::write(&tmp, format!("{mtime} {size} {CACHE_VERSION}\n"))
		.map_err(|error| error.to_string())?;
	fs::rename(&tmp, &path).map_err(|error| {
		let _ = fs::remove_file(&tmp);
		error.to_string()
	})
}

fn shell_quote(value: &str) -> String {
	let mut out = String::from("'");
	for character in value.chars() {
		if character == '\'' {
			out.push_str("'\\''");
		} else {
			out.push(character);
		}
	}
	out.push('\'');
	out
}

/// Expand a generator's `Exec` line. Substitution happens in one pass so a
/// path containing `%i` can never be re-expanded as another field code.
fn expand_exec(exec: &str, src: &Path, out: &Path, mime: &str) -> String {
	let uri = format!("file://{}", src.to_string_lossy());
	let replacements = [
		('i', shell_quote(&src.to_string_lossy())),
		('o', shell_quote(&out.to_string_lossy())),
		('u', shell_quote(&uri)),
		('m', shell_quote(mime)),
		(
			's',
			source_fingerprint(src)
				.map(|(_, size)| size.to_string())
				.unwrap_or_default(),
		),
	];

	let mut expanded = String::with_capacity(exec.len() + 64);
	let mut chars = exec.chars();
	while let Some(character) = chars.next() {
		if character != '%' {
			expanded.push(character);
			continue;
		}
		match chars.next() {
			Some('%') => expanded.push('%'),
			Some(code) => match replacements.iter().find(|(key, _)| *key == code) {
				Some((_, value)) => expanded.push_str(value),
				None => {
					expanded.push('%');
					expanded.push(code);
				}
			},
			None => expanded.push('%'),
		}
	}
	expanded
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
	let start = Instant::now();
	loop {
		match child.try_wait() {
			Ok(Some(status)) => return Some(status),
			Ok(None) => {}
			Err(_) => {
				let _ = child.kill();
				let _ = child.wait();
				return None;
			}
		}
		if start.elapsed() >= timeout {
			let _ = child.kill();
			let _ = child.wait();
			return None;
		}
		thread::sleep(Duration::from_millis(25));
	}
}

fn run_generator(exec: &str, src: &Path, out: &Path, mime: &str) -> bool {
	let command_line = expand_exec(exec, src, out, mime);
	// Descriptors are a background nicety; a generator must never compete with
	// the shell for the CPU while the user is scrolling.
	let Ok(mut child) = Command::new("nice")
		.arg("-n")
		.arg("10")
		.arg("sh")
		.arg("-c")
		.arg(command_line)
		.spawn()
	else {
		return false;
	};
	matches!(
		wait_with_timeout(&mut child, COMMAND_TIMEOUT),
		Some(status) if status.success()
	) && out.is_file()
}

/// Resolve an entry's `Icon` against the generated entry's own directory;
/// a bare name stays a themed icon.
fn resolve_icon(entry: &Entry, descriptor: &Path) -> Option<String> {
	let icon = entry
		.icon
		.as_deref()
		.filter(|icon| !icon.trim().is_empty())?;
	if icon.starts_with('/') {
		let path = std::fs::canonicalize(icon).unwrap_or_else(|_| Path::new(icon).to_path_buf());
		return Some(format!("file://{}", path.to_string_lossy()));
	}
	if icon.contains('/') || icon.starts_with('.') {
		let path = descriptor.parent()?.join(icon);
		if path.exists() {
			let path = std::fs::canonicalize(&path).unwrap_or(path);
			return Some(format!("file://{}", path.to_string_lossy()));
		}
		return None;
	}
	Some(icon.to_string())
}

fn generate(job: &Job) -> Result<Generated, String> {
	let out = cache_path(&job.src);
	if let Some(parent) = out.parent() {
		fs::create_dir_all(parent).map_err(|error| error.to_string())?;
	}

	if !run_generator(&job.exec, &job.src, &out, &job.mime) {
		return Err(format!("generator failed for {}", job.src.display()));
	}

	let entry = Entry::read(&out, FsKind::File)
		.ok_or_else(|| format!("unreadable generated entry for {}", job.src.display()))?;
	let icon = resolve_icon(&entry, &out);
	write_stamp(&job.src)?;

	Ok(Generated {
		name: entry.name.filter(|name| !name.trim().is_empty()),
		icon,
	})
}

fn next_job(st: &'static State) -> Job {
	let mut pending = st.pending.lock().unwrap();
	loop {
		while let Some(job) = pending.queue.pop_front() {
			pending.queued.remove(&job.key);
			if let Some(count) = pending.folder_counts.get_mut(&job.folder) {
				*count = count.saturating_sub(1);
			}
			// A folder the user has left no longer needs its results.
			if pending.epochs.get(&job.folder).copied().unwrap_or(0) == job.epoch {
				*pending.active_counts.entry(job.folder.clone()).or_insert(0) += 1;
				return job;
			}
		}
		pending = st.cond.wait(pending).unwrap();
	}
}

fn worker_loop() {
	let st = state();
	loop {
		let job = next_job(st);
		match generate(&job) {
			Ok(generated) => {
				let mut results = st.results.lock().unwrap();
				results.retain(|_, (_, at)| at.elapsed() < RESULT_TTL);
				results.insert(
					job.src.to_string_lossy().to_string(),
					(generated, Instant::now()),
				);
				RESULTS_EPOCH.fetch_add(1, Ordering::Relaxed);
			}
			Err(error) => eprintln!("descriptor generator: {error}"),
		}

		let mut pending = st.pending.lock().unwrap();
		let finished = match pending.active_counts.get_mut(&job.folder) {
			Some(count) => {
				*count = count.saturating_sub(1);
				*count == 0
			}
			None => false,
		};
		if finished {
			pending.active_counts.remove(&job.folder);
		}
	}
}

fn drop_oldest_for_folder(pending: &mut Pending, folder: &Path) {
	if let Some(position) = pending.queue.iter().position(|job| job.folder == folder)
		&& let Some(job) = pending.queue.remove(position)
	{
		pending.queued.remove(&job.key);
		if let Some(count) = pending.folder_counts.get_mut(folder) {
			*count = count.saturating_sub(1);
		}
	}
}

/// Whether an installed descriptor generator owns this file's presentation.
///
/// The file views use this to suppress thumbnails for types a generator
/// handles: a descriptor's deliberate artwork and a thumbnailer's generic
/// picture of the same file would otherwise fight, and whichever finished last
/// would decide what the user sees.
pub fn handles(src: &Path) -> bool {
	if !src.is_file() {
		return false;
	}
	generator_for(&mime_for(src)).is_some()
}

/// The themed icon a generator declares for this file's content type.
///
/// The file view asks for this after a generated descriptor and before the
/// generic MIME icon. A generator that cannot describe a particular file (an
/// unusual or damaged ROM) then still shows its own placeholder instead of
/// falling through to whatever icon `gio` reports — which is the content type
/// name, not necessarily one the active theme ships.
pub fn placeholder_icon(src: &Path) -> Option<String> {
	if !src.is_file() {
		return None;
	}
	generator_for(&mime_for(src)).and_then(|generator| generator.icon)
}

/// Ask for a descriptor for `src`. A valid cached entry is used directly; when
/// one is missing or stale the job is queued. Called for every file in a folder
/// scan, so it must stay cheap when everything is already cached.
pub fn request(src: &Path, folder: &Path) {
	if !src.is_file() || cached(src).is_some() {
		return;
	}
	let mime = mime_for(src);
	let Some(generator) = generator_for(&mime) else {
		return;
	};

	let key = cache_path(src);
	let st = state();
	let mut pending = st.pending.lock().unwrap();
	if pending.queued.contains(&key) {
		return;
	}
	if pending.folder_counts.get(folder).copied().unwrap_or(0) >= MAX_PENDING_PER_FOLDER {
		drop_oldest_for_folder(&mut pending, folder);
	}

	let epoch = pending.epochs.get(folder).copied().unwrap_or(0);
	pending.queued.insert(key.clone());
	*pending
		.folder_counts
		.entry(folder.to_path_buf())
		.or_insert(0) += 1;
	pending.queue.push_back(Job {
		src: src.to_path_buf(),
		key,
		folder: folder.to_path_buf(),
		mime,
		exec: generator.exec,
		epoch,
	});
	st.cond.notify_one();
}

/// Drop queued work for a folder the view has left.
pub fn cancel_folder(folder: &Path) {
	let st = state();
	let mut pending = st.pending.lock().unwrap();
	*pending.epochs.entry(folder.to_path_buf()).or_insert(0) += 1;
	let mut index = 0;
	while index < pending.queue.len() {
		if pending.queue[index].folder == folder {
			if let Some(job) = pending.queue.remove(index) {
				pending.queued.remove(&job.key);
				if let Some(count) = pending.folder_counts.get_mut(folder) {
					*count = count.saturating_sub(1);
				}
			}
		} else {
			index += 1;
		}
	}
}

/// Whether a folder still has descriptor jobs queued or in flight. The view
/// waits for this to clear before applying results, so a folder full of ROMs
/// changes its titles and icons in one batch instead of once per ROM.
pub fn folder_busy(folder: &Path) -> bool {
	let pending = state().pending.lock().unwrap();
	pending.folder_counts.get(folder).copied().unwrap_or(0) > 0
		|| pending.active_counts.get(folder).copied().unwrap_or(0) > 0
}

/// Whether any generated descriptor is waiting to be picked up.
pub fn has_results() -> bool {
	!state().results.lock().unwrap().is_empty()
}

/// The generated name and icon for a source path, once generation has finished.
pub fn result_for(src: &str) -> Option<Generated> {
	state()
		.results
		.lock()
		.unwrap()
		.get(src)
		.map(|(generated, _)| generated.clone())
}

/// Drop the cache entries for a path, so the next scan regenerates them.
#[allow(dead_code)]
pub fn invalidate(src: &Path) {
	let _ = fs::remove_file(cache_path(src));
	let _ = fs::remove_file(stamp_path(src));
}

#[cfg(test)]
mod tests {
	use super::{expand_exec, icon_file_present, mime_matches, parse_generator};
	use crate::entry::Entry;
	use std::fs;
	use std::path::Path;

	#[test]
	fn reads_a_generators_placeholder_icon() {
		let generator = parse_generator(
			"[Descriptor Generator Entry]\n\
			 Exec=gen %i %o\n\
			 Icon=application-x-test-rom\n\
			 MimeType=application/x-test-rom;\n",
		)
		.expect("a generator with Exec and MimeType");
		assert_eq!(generator.icon.as_deref(), Some("application-x-test-rom"));
	}

	#[test]
	fn a_generator_without_a_placeholder_has_no_icon() {
		let generator = parse_generator(
			"[Descriptor Generator Entry]\n\
			 Exec=gen %i %o\n\
			 MimeType=application/x-test-rom;\n",
		)
		.expect("a generator with Exec and MimeType");
		assert_eq!(generator.icon, None);
	}

	#[test]
	fn matches_exact_and_wildcard_mime_types() {
		assert!(mime_matches(
			"application/vnd.nintendo.nitro.rom",
			"application/vnd.nintendo.nitro.rom"
		));
		assert!(mime_matches("image/*", "image/png"));
		assert!(!mime_matches("image/*", "video/mp4"));
		assert!(!mime_matches("image/png", "image/jpeg"));
	}

	fn entry_with_icon(icon: &str) -> Entry {
		Entry {
			path: std::path::PathBuf::new(),
			entry_type: None,
			target: None,
			mime: None,
			name: None,
			comment: None,
			icon: Some(icon.to_string()),
			exec: None,
			launch_animation: None,
			hidden: false,
			no_display: false,
			toggle: false,
			number_value: String::new(),
			custom_group: None,
			diagnostics: Vec::new(),
		}
	}

	#[test]
	fn a_missing_generated_icon_invalidates_the_entry() {
		let dir =
			std::env::temp_dir().join(format!("hishell-descriptor-icon-{}", std::process::id()));
		fs::create_dir_all(&dir).unwrap();
		let descriptor = dir.join("game.desktop");
		let icon = dir.join("game.svg");
		let entry = entry_with_icon(&icon.to_string_lossy());

		assert!(!icon_file_present(&descriptor, &entry));
		fs::write(&icon, b"<svg/>").unwrap();
		assert!(icon_file_present(&descriptor, &entry));

		let themed = entry_with_icon("application-x-test-rom");
		assert!(icon_file_present(&descriptor, &themed));

		fs::remove_dir_all(dir).unwrap();
	}

	#[test]
	fn expands_field_codes_once() {
		let expanded = expand_exec(
			"gen %i %o %m %%",
			Path::new("/roms/100%i.nds"),
			Path::new("/cache/out.desktop"),
			"application/x-test",
		);
		assert!(expanded.contains("'/roms/100%i.nds'"));
		assert!(expanded.contains("'/cache/out.desktop'"));
		assert!(expanded.contains("'application/x-test'"));
		assert!(expanded.ends_with(" %"));
	}
}

/// Turn a generated `Icon=` value into an `IconRef`.
pub fn icon_ref(icon: &str) -> IconRef {
	let icon = icon.trim();
	if let Some(rest) = icon.strip_prefix("file://") {
		IconRef::Path(PathBuf::from(rest))
	} else if icon.starts_with('/') || icon.contains('/') || icon.starts_with('.') {
		IconRef::Path(PathBuf::from(icon))
	} else {
		IconRef::Theme(icon.to_string())
	}
}

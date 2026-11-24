//! Folder scanning and item lookup.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::config::Config;
use crate::entry::{Entry, EntryType};
use crate::group;
use crate::item::{FsKind, IconRef, Item, ItemId, ItemKind};
use crate::selection::Selection;
use crate::view::{SortAlphaMode, SortDateMode, SortKey};

/// How a folder scan reads the filesystem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanOptions {
	/// Read `.desktop` and `.directory` entries, sidecars, and folder
	/// `.directory` files.
	pub read_entries: bool,
	/// List dotfiles and `NoDisplay` entries.
	pub include_hidden: bool,
}

impl Default for ScanOptions {
	fn default() -> Self {
		Self {
			read_entries: true,
			include_hidden: true,
		}
	}
}

pub struct Folder {
	path: PathBuf,
	items: Vec<Item>,
	by_path: HashMap<PathBuf, ItemId>,
	order: Vec<ItemId>,
	selection: Selection,
	diagnostics: Vec<crate::config::ConfigDiagnostic>,
}

/// What an in-place `[Progress]` refresh changed.
///
/// A sidecar rewrite changes neither the item's name nor its position, so the
/// view can patch the affected rows instead of scanning and rebuilding the
/// whole folder. `structural` is set when a cleared group leaves an item that
/// only existed because of it, which needs a rescan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProgressRefresh {
	pub changed: Vec<ItemId>,
	pub structural: bool,
}

struct RawEntry {
	name: String,
	path: PathBuf,
	fs_kind: FsKind,
	is_symlink: bool,
	metadata: Option<std::fs::Metadata>,
}

impl Folder {
	pub fn scan(path: &Path) -> Self {
		Self::scan_with(path, ScanOptions::default())
	}

	pub fn scan_with(path: &Path, options: ScanOptions) -> Self {
		Self::scan_roots(path, options, &[])
	}

	/// Scan `path`, with the items of `roots` merged beside its own.
	///
	/// The primary folder comes first, then each root in order, so a name that
	/// already appeared is skipped and the primary shadows the root. That is the
	/// freedesktop precedence for application entries. Every item keeps the real
	/// path inside the root it came from, so each one's own permissions apply to
	/// it and writes still go to the primary folder.
	pub fn scan_roots(path: &Path, options: ScanOptions, roots: &[PathBuf]) -> Self {
		let mut items = Vec::new();
		let mut diagnostics = Vec::new();
		let mut names = HashSet::new();
		for root in std::iter::once(path).chain(roots.iter().map(PathBuf::as_path)) {
			let (found, notes) = Self::scan_root(root, options);
			diagnostics.extend(notes);
			for item in found {
				if names.insert(item.name.clone()) {
					items.push(item);
				}
			}
		}

		let by_path = items
			.iter()
			.map(|item| (item.path.clone(), item.id))
			.collect();
		let order = items.iter().map(|item| item.id).collect();

		Self {
			path: path.to_path_buf(),
			items,
			by_path,
			order,
			selection: Selection::default(),
			diagnostics,
		}
	}

	/// One root of a folder scan: its own entries, with the folder's `.directory`
	/// and the descriptors beside them consumed rather than listed.
	fn scan_root(
		root: &Path,
		options: ScanOptions,
	) -> (Vec<Item>, Vec<crate::config::ConfigDiagnostic>) {
		let mut raw_entries = read_entries(root);

		let mut entries: HashMap<PathBuf, Entry> = HashMap::new();
		let mut consumed: HashSet<PathBuf> = HashSet::new();
		let mut virtual_entries = Vec::new();
		let mut diagnostics = Vec::new();

		if options.read_entries {
			for raw in &raw_entries {
				match raw.fs_kind {
					FsKind::Directory => {
						let candidate = raw.path.join(".directory");
						if let Some(entry) = Entry::read(&candidate, FsKind::Directory) {
							diagnostics.extend(entry.diagnostics.iter().cloned());
							if entry.entry_type == Some(EntryType::File) {
								consumed.insert(raw.path.clone());
								continue;
							}
							entries.insert(raw.path.clone(), entry);
						}
					}
					FsKind::File => {
						// The folder's own `.directory` is its metadata, never an item,
						// even when the scan lists dotfiles.
						if raw.name == ".directory" {
							consumed.insert(raw.path.clone());
							continue;
						}
						if is_desktop_entry(&raw.path) {
							let Some(entry) = Entry::read(&raw.path, FsKind::File) else {
								continue;
							};
							diagnostics.extend(entry.diagnostics.iter().cloned());
							if entry.is_descriptor() {
								if let Some(target) = &entry.target
									&& target.parent() == Some(root)
									&& (target.is_file() || entry.progress.is_some())
								{
									if missing(target) {
										virtual_entries.push(virtual_item(target));
									}
									entries.insert(target.clone(), entry);
								}
								consumed.insert(raw.path.clone());
							} else if entry.is_sidecar() {
								if let Some(target) = sidecar_target(&raw.path)
									&& target.parent() == Some(root)
									&& (target.is_file() || entry.progress.is_some())
								{
									if missing(&target) {
										virtual_entries.push(virtual_item(&target));
									}
									entries.insert(target, entry);
								}
								consumed.insert(raw.path.clone());
							} else if entry.entry_type == Some(EntryType::Directory) {
								consumed.insert(raw.path.clone());
							} else {
								entries.insert(raw.path.clone(), entry);
							}
						}
					}
				}
			}
		}
		raw_entries.extend(virtual_entries);

		let mut items = Vec::new();
		for raw in raw_entries {
			if !options.include_hidden && raw.name.starts_with('.') {
				continue;
			}
			if consumed.contains(&raw.path) {
				continue;
			}
			let entry = entries.get(&raw.path);
			if entry.is_some_and(|entry| entry.hidden) {
				continue;
			}
			if !options.include_hidden && entry.is_some_and(|entry| entry.no_display) {
				continue;
			}

			let name = raw.name;
			let mime = match raw.fs_kind {
				FsKind::File => entry
					.and_then(|entry| entry.mime.clone())
					.or_else(|| crate::icon::mime_for_path(&raw.path)),
				FsKind::Directory => None,
			};
			let descriptor = entry
				.is_none()
				.then(|| crate::descriptor::lookup(&raw.path))
				.flatten();
			let appimage = (raw.fs_kind == FsKind::File
				&& entry.is_none()
				&& crate::appimage::is_appimage(&raw.path))
			.then(|| crate::appimage::entry(&raw.path))
			.flatten();
			let generated_entry = (options.read_entries && raw.fs_kind == FsKind::File)
				.then(|| {
					crate::descriptor_generator::request_with_mime(
						&raw.path,
						root,
						mime.as_deref().unwrap_or_default(),
					)
				})
				.flatten();
			let generated = (raw.fs_kind == FsKind::File)
				.then(|| crate::descriptor_generator::result_for(&raw.path.to_string_lossy()))
				.flatten();
			// The default icon is `./.icon` for every entry, so a folder uses
			// its own `.icon.<extension>` with or without a `.directory`.
			let icon = entry
				.and_then(Entry::icon_ref)
				.or_else(|| {
					(raw.fs_kind == FsKind::Directory)
						.then(|| crate::entry::folder_icon_ref(&raw.path))
						.flatten()
				})
				.or_else(|| {
					descriptor
						.as_ref()
						.and_then(crate::descriptor::Descriptor::icon_ref)
				})
				.or_else(|| {
					appimage
						.as_ref()
						.and_then(|entry| entry.icon.clone())
						.map(IconRef::Path)
				})
				.or_else(|| {
					generated
						.as_ref()
						.and_then(|entry| entry.icon.as_deref())
						.map(crate::descriptor_generator::icon_ref)
				})
				.or_else(|| {
					(raw.fs_kind == FsKind::File)
						.then(|| {
							crate::descriptor_generator::placeholder_icon_mime(
								mime.as_deref().unwrap_or_default(),
							)
						})
						.flatten()
						.map(IconRef::Theme)
				})
				.unwrap_or_else(|| match raw.fs_kind {
					FsKind::Directory => IconRef::Theme(String::from("folder")),
					FsKind::File => crate::icon::for_mime(mime.as_deref().unwrap_or_default()),
				});
			let is_stashed = name.starts_with('.') || entry.is_some_and(|entry| entry.no_display);
			// A cached generated descriptor is read as a descriptor root, not
			// as a fresh generator result, so both paths can carry the bare
			// `[Topscreen] IconArt=` the 3D banner needs.
			let model_icon = generated
				.as_ref()
				.and_then(|entry| entry.icon_art.clone())
				.or_else(|| {
					descriptor.as_ref().and_then(|descriptor| {
						crate::descriptor_generator::topscreen_icon_art(&descriptor.source)
					})
				})
				.or_else(|| {
					generated_entry
						.as_deref()
						.and_then(crate::descriptor_generator::topscreen_icon_art)
				});

			items.push(Item {
				id: ItemId::for_path(&raw.path),
				name: name.clone(),
				title: entry
					.and_then(|entry| entry.name.clone())
					.or_else(|| descriptor.as_ref().and_then(|entry| entry.name.clone()))
					.or_else(|| appimage.as_ref().and_then(|entry| entry.name.clone()))
					.or_else(|| generated.as_ref().and_then(|entry| entry.name.clone()))
					.unwrap_or_else(|| name.clone()),
				comment: entry
					.and_then(|entry| entry.comment.clone())
					.or_else(|| descriptor.as_ref().and_then(|entry| entry.comment.clone()))
					.or_else(|| appimage.as_ref().and_then(|entry| entry.comment.clone()))
					.unwrap_or_default(),
				path: raw.path.clone(),
				entry_path: entry.map(|entry| entry.path.clone()),
				generated: entry.is_some_and(|entry| entry.generated),
				destroyable: entry.is_none_or(|entry| entry.is_destroyable()),
				target: entry.and_then(|entry| entry.target.clone()),
				fs_kind: raw.fs_kind,
				kind: entry
					.map(|entry| entry.item_kind(raw.fs_kind))
					.unwrap_or_else(|| ItemKind::from_fs(raw.fs_kind)),
				mime,
				icon,
				model_icon,
				uid: entry.and_then(|entry| entry.uid.clone()),
				is_symlink: raw.is_symlink,
				is_dotfile: name.starts_with('.'),
				toggle_on: entry.is_some_and(|entry| entry.toggle),
				number_value: entry
					.map(|entry| entry.number_value.clone())
					.unwrap_or_default(),
				number_fields: entry
					.map(|entry| entry.number_fields.clone())
					.unwrap_or_default(),
				slider: entry.and_then(|entry| entry.slider),
				text_value: entry
					.map(|entry| entry.text_value.clone())
					.unwrap_or_default(),
				text_placeholder: entry
					.and_then(|entry| entry.text_placeholder.clone())
					.unwrap_or_default(),
				select_options: entry
					.map(|entry| entry.select_options.clone())
					.unwrap_or_default(),
				select_value: entry
					.map(|entry| entry.select_value.clone())
					.unwrap_or_default(),
				size: raw
					.metadata
					.as_ref()
					.map(|metadata| metadata.len())
					.unwrap_or(0),
				modified: time(raw.metadata.as_ref().and_then(|meta| meta.modified().ok())),
				created: time(raw.metadata.as_ref().and_then(|meta| meta.created().ok())),
				accessed: time(raw.metadata.as_ref().and_then(|meta| meta.accessed().ok())),
				group: 0,
				group_label: String::new(),
				is_stashed,
				custom_group: entry
					.and_then(|entry| entry.custom_group.clone())
					.unwrap_or_default(),
				progress: entry.and_then(|entry| entry.progress.clone()),
			});
		}

		(items, diagnostics)
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	pub fn items(&self) -> &[Item] {
		&self.items
	}

	/// Update one control's in-memory state without rescanning the folder.
	///
	/// A provider control writes its entry on disk, and the folder's revision
	/// brings the new value back. A toggle is the one control the user sees
	/// change under the finger, so it must not wait for that round trip; the
	/// rescan that follows confirms it (or puts it back if the command failed).
	/// Returns whether the item was found and its value changed.
	pub fn set_toggle(&mut self, path: &Path, on: bool) -> bool {
		let Some(item) = self.items.iter_mut().find(|item| item.path == path) else {
			return false;
		};
		if item.toggle_on == on {
			return false;
		}
		item.toggle_on = on;
		true
	}

	/// Update a slider's in-memory position without rescanning the folder.
	///
	/// The provider writes the value back; the slider still has to follow the
	/// finger while the drag is in progress. The rescan that follows confirms
	/// it, or puts the knob back when the command failed.
	pub fn set_slider(&mut self, path: &Path, value: i64) -> bool {
		let Some(item) = self.items.iter_mut().find(|item| item.path == path) else {
			return false;
		};
		let Some(slider) = item.slider.as_mut() else {
			return false;
		};
		if slider.value == value {
			return false;
		}
		slider.value = value;
		true
	}

	pub fn order(&self) -> &[ItemId] {
		&self.order
	}

	pub fn item(&self, id: ItemId) -> Option<&Item> {
		self.items.iter().find(|item| item.id == id)
	}

	pub fn item_at(&self, path: &Path) -> Option<&Item> {
		self.by_path.get(path).and_then(|id| self.item(*id))
	}

	pub fn len(&self) -> usize {
		self.items.len()
	}

	pub fn is_empty(&self) -> bool {
		self.items.is_empty()
	}

	pub fn diagnostics(&self) -> &[crate::config::ConfigDiagnostic] {
		&self.diagnostics
	}

	/// Re-read the `[Progress]` group of every busy item, without touching the
	/// directory.
	///
	/// This is the cheap path a frontend polls while a download writes its
	/// sidecar: the item set and its order are unchanged, only the group is.
	/// Non-busy items are left alone; a group that appears on one of them
	/// changes the folder's entries, which a scan already covers.
	pub fn refresh_progress(&mut self) -> ProgressRefresh {
		let mut refresh = ProgressRefresh::default();
		for item in self.items.iter_mut().filter(|item| item.is_busy()) {
			let Some(entry_path) = item.entry_path.clone() else {
				continue;
			};
			let progress = match crate::entry::Entry::read(&entry_path, item.fs_kind) {
				Some(entry) => entry.progress,
				// A writer may replace its sidecar atomically; a path that is
				// still there but unreadable is a transient, not a finish.
				None if entry_path.exists() => continue,
				None => None,
			};
			if progress == item.progress {
				continue;
			}
			let cleared = progress.is_none();
			item.progress = progress;
			refresh.changed.push(item.id);
			if cleared && !item.path.exists() {
				refresh.structural = true;
			}
		}
		refresh
	}

	/// Apply the folder's sort and grouping. Called after a scan and after any
	/// config change that affects item order.
	pub fn apply_config(&mut self, config: &Config) {
		self.sort_items(config);
		group::assign(&mut self.items, config);
		self.order = self.items.iter().map(|item| item.id).collect();
		self.selection.reorder(&self.order);
	}

	fn sort_items(&mut self, config: &Config) {
		let date_key = |item: &Item| match config.sort_date_mode {
			SortDateMode::Created => item.created,
			SortDateMode::Accessed => item.accessed,
			SortDateMode::Modified => item.modified,
		};
		let name_key = |item: &Item| match config.sort_alpha_mode {
			SortAlphaMode::Filenames => item.name.to_lowercase(),
			SortAlphaMode::Titles => item.title.to_lowercase(),
		};
		let dotfile = |item: &Item| item.is_dotfile;
		let directory = |item: &Item| item.fs_kind == FsKind::Directory;

		match config.sort {
			SortKey::Newest => self.items.sort_by(|left, right| {
				dotfile(left)
					.cmp(&dotfile(right))
					.then_with(|| date_key(right).cmp(&date_key(left)))
					.then_with(|| name_key(left).cmp(&name_key(right)))
			}),
			SortKey::Oldest => self.items.sort_by(|left, right| {
				dotfile(left)
					.cmp(&dotfile(right))
					.then_with(|| date_key(left).cmp(&date_key(right)))
					.then_with(|| name_key(left).cmp(&name_key(right)))
			}),
			SortKey::Alphabetical => self.items.sort_by(|left, right| {
				dotfile(left)
					.cmp(&dotfile(right))
					.then_with(|| name_key(left).cmp(&name_key(right)))
			}),
			SortKey::Free => self.items.sort_by(|left, right| {
				dotfile(left)
					.cmp(&dotfile(right))
					.then_with(|| directory(right).cmp(&directory(left)))
					.then_with(|| left.name.cmp(&right.name))
			}),
		}
	}

	pub fn selection(&self) -> &Selection {
		&self.selection
	}

	pub fn selection_mut(&mut self) -> &mut Selection {
		&mut self.selection
	}

	pub fn retain_selection(&mut self) {
		self.selection.retain(&self.order);
	}

	/// Whether this folder's `[Radio]` group accepts `path` as a choice.
	pub fn is_radio_member(&self, path: &Path, config: &Config) -> bool {
		radio_member(&self.path, path, config)
	}
}

fn read_entries(path: &Path) -> Vec<RawEntry> {
	let Ok(entries) = std::fs::read_dir(path) else {
		return Vec::new();
	};
	entries
		.flatten()
		.filter_map(|entry| {
			let path = entry.path();
			let file_type = entry.file_type().ok();
			let is_symlink = file_type.is_some_and(|kind| kind.is_symlink());
			let metadata = entry.metadata().ok();
			let is_dir = metadata
				.as_ref()
				.map(|metadata| metadata.is_dir())
				.unwrap_or(false)
				|| (is_symlink && path.is_dir());
			Some(RawEntry {
				name: entry.file_name().to_string_lossy().into_owned(),
				path,
				fs_kind: if is_dir {
					FsKind::Directory
				} else {
					FsKind::File
				},
				is_symlink,
				metadata,
			})
		})
		.collect()
}

/// A file that carries a desktop entry. An entry may use the `.directory`
/// extension, which names an application entry in an `applications/` folder as
/// well as a folder's own metadata, so both extensions are read wherever a
/// folder is scanned.
fn is_desktop_entry(path: &Path) -> bool {
	path
		.extension()
		.and_then(|extension| extension.to_str())
		.is_some_and(|extension| {
			extension.eq_ignore_ascii_case("desktop")
				|| extension.eq_ignore_ascii_case("directory")
		})
}

/// Whether nothing at all exists at `path`, including a broken symlink.
fn missing(path: &Path) -> bool {
	std::fs::symlink_metadata(path).is_err()
}

/// A target a sidecar names before the file exists, so its progress is
/// visible while it is being downloaded or copied.
fn virtual_item(path: &Path) -> RawEntry {
	RawEntry {
		name: path
			.file_name()
			.map(|name| name.to_string_lossy().into_owned())
			.unwrap_or_else(|| path.display().to_string()),
		path: path.to_path_buf(),
		fs_kind: FsKind::File,
		is_symlink: false,
		metadata: None,
	}
}

/// `photo.png.desktop` describes `photo.png` beside it.
fn sidecar_target(entry: &Path) -> Option<PathBuf> {
	let name = entry.file_name()?.to_str()?;
	let stem = name.strip_suffix(".desktop")?;
	Some(entry.with_file_name(stem))
}

fn time(value: Option<std::time::SystemTime>) -> u64 {
	value
		.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
		.map(|duration| duration.as_millis() as u64)
		.unwrap_or(0)
}

/// The value a parent `[Radio]` group stores for a selected child.
pub fn radio_value(path: &Path) -> String {
	let name = path
		.file_name()
		.and_then(|name| name.to_str())
		.unwrap_or_default()
		.to_string();
	if path.is_dir() {
		return name;
	}
	if is_desktop_entry(path) {
		return path
			.file_stem()
			.and_then(|stem| stem.to_str())
			.unwrap_or(&name)
			.to_string();
	}
	name
}

/// Whether `path` is a direct, selectable child of `folder`'s radio group.
pub fn radio_member(folder: &Path, path: &Path, config: &Config) -> bool {
	if !config.radio_enabled {
		return false;
	}
	if path.parent() != Some(folder) {
		return false;
	}
	if path
		.file_name()
		.and_then(|name| name.to_str())
		.is_some_and(|name| name.starts_with('.'))
	{
		return false;
	}
	match config.radio_only {
		crate::view::RadioOnly::All => true,
		crate::view::RadioOnly::Directories => path.is_dir(),
		crate::view::RadioOnly::Files => path.is_file(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tree(name: &str) -> PathBuf {
		let root = std::env::temp_dir().join(format!(
			"hishell-folder-progress-{name}-{}",
			std::process::id()
		));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		root
	}

	#[test]
	fn a_folder_icon_needs_no_directory_file() {
		let root = tree("folder-icon");
		let plain = root.join("plain");
		let with_entry = root.join("with-entry");
		std::fs::create_dir_all(&plain).unwrap();
		std::fs::create_dir_all(&with_entry).unwrap();
		std::fs::write(plain.join(".icon.svg"), "<svg/>").unwrap();
		std::fs::write(with_entry.join(".icon.png"), "").unwrap();
		std::fs::write(
			with_entry.join(".directory"),
			"[Desktop Entry]\nType=Directory\nName=With entry\n",
		)
		.unwrap();

		let folder = Folder::scan(&root);
		let icon = |name: &str| {
			folder
				.items()
				.iter()
				.find(|item| item.name == name)
				.map(|item| item.icon.clone())
				.unwrap()
		};
		assert_eq!(
			icon("plain"),
			IconRef::Path(plain.join(".icon.svg"))
		);
		assert_eq!(
			icon("with-entry"),
			IconRef::Path(with_entry.join(".icon.png"))
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_folder_without_an_icon_keeps_the_folder_icon() {
		let root = tree("folder-no-icon");
		std::fs::create_dir_all(root.join("plain")).unwrap();
		let folder = Folder::scan(&root);
		assert_eq!(folder.items()[0].icon, IconRef::Theme(String::from("folder")));
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_sidecar_with_progress_shows_a_missing_target() {
		let root = tree("file");
		std::fs::write(
			root.join("movie.mkv.desktop"),
			"[Desktop Entry]\nType=File\nName=Movie\nIcon=video-x-generic\n\
			 [Progress]\nValue=5\nTarget=10\nTaskName=Downloading...\n",
		)
		.unwrap();
		let folder = Folder::scan(&root);
		assert_eq!(folder.len(), 1);
		let item = &folder.items()[0];
		assert_eq!(item.name, "movie.mkv");
		assert_eq!(item.title, "Movie");
		assert_eq!(item.fs_kind, FsKind::File);
		assert_eq!(item.icon, IconRef::Theme(String::from("video-x-generic")));
		assert!(item.is_busy());
		assert_eq!(item.progress.as_ref().map(|p| p.percent()), Some(50));
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_sidecar_without_progress_does_not_show_a_missing_target() {
		let root = tree("hidden");
		std::fs::write(
			root.join("movie.mkv.desktop"),
			"[Desktop Entry]\nType=File\nName=Movie\n",
		)
		.unwrap();
		let folder = Folder::scan(&root);
		assert!(folder.is_empty());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_sidecar_for_an_existing_directory_does_not_duplicate_the_item() {
		let root = tree("folder-sidecar");
		let folder_path = root.join("movie");
		std::fs::create_dir_all(&folder_path).unwrap();
		std::fs::write(
			root.join("movie.desktop"),
			"[Desktop Entry]\nType=File\nName=Movie\n\
			 [Progress]\nValue=1\nTarget=10\n",
		)
		.unwrap();
		let folder = Folder::scan(&root);
		assert_eq!(folder.len(), 1);
		assert_eq!(folder.items()[0].fs_kind, FsKind::Directory);
		assert!(folder.items()[0].is_busy());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_folder_directory_can_carry_progress() {
		let root = tree("folder");
		let folder_path = root.join("movie");
		std::fs::create_dir_all(&folder_path).unwrap();
		std::fs::write(
			folder_path.join(".directory"),
			"[Desktop Entry]\nType=Directory\nName=Movie\n\
			 [Progress]\nValue=0\nTarget=100\nTaskName=Downloading...\n",
		)
		.unwrap();
		let folder = Folder::scan(&root);
		assert_eq!(folder.len(), 1);
		let item = &folder.items()[0];
		assert_eq!(item.name, "movie");
		assert_eq!(item.title, "Movie");
		assert!(item.is_busy());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_directory_extension_entry_is_an_application_item() {
		let root = tree("applet-entry");
		std::fs::write(
			root.join("clam-text-editor.directory"),
			"[Desktop Entry]\nType=Application\nName=Text editor\n\
			 Icon=text-x-generic\nExec=/usr/lib/clam/applets/clam-text-editor %f\n",
		)
		.unwrap();
		std::fs::write(
			root.join(".directory"),
			"[Desktop Entry]\nType=Directory\nName=Applications\n",
		)
		.unwrap();
		let folder = Folder::scan(&root);
		assert_eq!(folder.len(), 1);
		let item = &folder.items()[0];
		assert_eq!(item.name, "clam-text-editor.directory");
		assert_eq!(item.title, "Text editor");
		assert_eq!(item.kind, ItemKind::Application);
		assert_eq!(item.fs_kind, FsKind::File);
		assert!(item.entry_path.is_some());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn progress_refresh_updates_the_item_without_a_rescan() {
		let root = tree("refresh");
		std::fs::write(
			root.join("movie.mkv.desktop"),
			"[Desktop Entry]\nType=File\nName=Movie\n\
			 [Progress]\nValue=1\nTarget=10\nTaskName=Downloading...\n",
		)
		.unwrap();
		let mut folder = Folder::scan(&root);
		let id = folder.items()[0].id;
		std::fs::write(
			root.join("movie.mkv.desktop"),
			"[Desktop Entry]\nType=File\nName=Movie\n\
			 [Progress]\nValue=7\nTarget=10\nTaskName=Downloading...\n",
		)
		.unwrap();

		let refresh = folder.refresh_progress();
		assert_eq!(refresh.changed, vec![id]);
		assert!(!refresh.structural);
		assert_eq!(
			folder
				.item(id)
				.unwrap()
				.progress
				.as_ref()
				.unwrap()
				.percent(),
			70
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn clearing_progress_keeps_an_existing_target_in_place() {
		let root = tree("refresh-done");
		let target = root.join("movie.mkv");
		std::fs::write(&target, b"done").unwrap();
		std::fs::write(
			root.join("movie.mkv.desktop"),
			"[Desktop Entry]\nType=File\nName=Movie\n\
			 [Progress]\nValue=10\nTarget=10\n",
		)
		.unwrap();
		let mut folder = Folder::scan(&root);
		let id = folder.items()[0].id;
		std::fs::remove_file(root.join("movie.mkv.desktop")).unwrap();

		let refresh = folder.refresh_progress();
		assert_eq!(refresh.changed, vec![id]);
		assert!(!refresh.structural);
		assert!(!folder.item(id).unwrap().is_busy());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn clearing_progress_for_a_missing_target_needs_a_rescan() {
		let root = tree("refresh-gone");
		std::fs::write(
			root.join("movie.mkv.desktop"),
			"[Desktop Entry]\nType=File\nName=Movie\n\
			 [Progress]\nValue=1\nTarget=10\n",
		)
		.unwrap();
		let mut folder = Folder::scan(&root);
		assert_eq!(folder.len(), 1);
		std::fs::remove_file(root.join("movie.mkv.desktop")).unwrap();

		let refresh = folder.refresh_progress();
		assert!(refresh.structural);
		std::fs::remove_dir_all(root).unwrap();
	}
}

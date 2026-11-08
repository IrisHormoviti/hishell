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
	/// Read `.desktop` entries, sidecars, and folder `.directory` files.
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
		let raw_entries = read_entries(path);

		let mut entries: HashMap<PathBuf, Entry> = HashMap::new();
		let mut consumed: HashSet<PathBuf> = HashSet::new();
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
						if is_desktop_entry(&raw.path) {
							let Some(entry) = Entry::read(&raw.path, FsKind::File) else {
								continue;
							};
							diagnostics.extend(entry.diagnostics.iter().cloned());
							if entry.is_descriptor() {
								if let Some(target) = &entry.target
									&& target.is_file()
								{
									entries.insert(target.clone(), entry);
								}
								consumed.insert(raw.path.clone());
							} else if entry.is_sidecar() {
								if let Some(target) = sidecar_target(&raw.path)
									&& target.is_file()
								{
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
			let icon = entry
				.and_then(Entry::icon_ref)
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
				.unwrap_or_else(|| match raw.fs_kind {
					FsKind::Directory => IconRef::Theme(String::from("folder")),
					FsKind::File => crate::icon::for_mime(mime.as_deref().unwrap_or_default()),
				});
			let is_stashed = name.starts_with('.') || entry.is_some_and(|entry| entry.no_display);

			items.push(Item {
				id: ItemId::for_path(&raw.path),
				name: name.clone(),
				title: entry
					.and_then(|entry| entry.name.clone())
					.or_else(|| descriptor.as_ref().and_then(|entry| entry.name.clone()))
					.or_else(|| appimage.as_ref().and_then(|entry| entry.name.clone()))
					.unwrap_or_else(|| name.clone()),
				comment: entry
					.and_then(|entry| entry.comment.clone())
					.or_else(|| descriptor.as_ref().and_then(|entry| entry.comment.clone()))
					.or_else(|| appimage.as_ref().and_then(|entry| entry.comment.clone()))
					.unwrap_or_default(),
				path: raw.path.clone(),
				entry_path: entry.map(|entry| entry.path.clone()),
				target: entry.and_then(|entry| entry.target.clone()),
				fs_kind: raw.fs_kind,
				kind: entry
					.map(|entry| entry.item_kind(raw.fs_kind))
					.unwrap_or_else(|| ItemKind::from_fs(raw.fs_kind)),
				mime,
				icon,
				is_symlink: raw.is_symlink,
				is_dotfile: name.starts_with('.'),
				toggle_on: entry.is_some_and(|entry| entry.toggle),
				number_value: entry
					.map(|entry| entry.number_value.clone())
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
			});
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

	pub fn path(&self) -> &Path {
		&self.path
	}

	pub fn items(&self) -> &[Item] {
		&self.items
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

fn is_desktop_entry(path: &Path) -> bool {
	path.extension()
		.and_then(|extension| extension.to_str())
		.is_some_and(|extension| extension.eq_ignore_ascii_case("desktop"))
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

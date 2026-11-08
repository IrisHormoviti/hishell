//! Folder scanning and item lookup.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::item::{FsKind, IconRef, Item, ItemId, ItemKind};

pub struct Folder {
	path: PathBuf,
	items: Vec<Item>,
	by_path: HashMap<PathBuf, ItemId>,
	order: Vec<ItemId>,
}

impl Folder {
	pub fn scan(path: &Path) -> Self {
		let mut items = Vec::new();
		let mut next_id = 0u64;

		if let Ok(entries) = std::fs::read_dir(path) {
			items = entries
				.flatten()
				.map(|entry| {
					let item_path = entry.path();
					let file_type = entry.file_type().ok();
					let metadata = entry.metadata().ok();
					let is_symlink = file_type.map(|kind| kind.is_symlink()).unwrap_or(false);
					let fs_kind = if metadata
						.as_ref()
						.map(|metadata| metadata.is_dir())
						.unwrap_or(false)
					{
						FsKind::Directory
					} else {
						FsKind::File
					};
					let name = entry.file_name().to_string_lossy().into_owned();
					let id = ItemId::new(next_id);
					next_id += 1;
					Item {
						id,
						name: name.clone(),
						title: name.clone(),
						comment: String::new(),
						path: item_path,
						fs_kind,
						kind: ItemKind::from_fs(fs_kind),
						mime: if fs_kind == FsKind::File {
							mime_for(&entry.path())
						} else {
							None
						},
						icon: match fs_kind {
							FsKind::Directory => IconRef::Theme(String::from("folder")),
							FsKind::File => IconRef::Theme(String::from("text-x-generic")),
						},
						is_symlink,
						is_dotfile: name.starts_with('.'),
						size: metadata
							.as_ref()
							.map(|metadata| metadata.len())
							.unwrap_or(0),
						modified: metadata
							.as_ref()
							.and_then(|metadata| metadata.modified().ok())
							.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
							.map(|duration| duration.as_millis() as u64)
							.unwrap_or(0),
						created: metadata
							.as_ref()
							.and_then(|metadata| metadata.created().ok())
							.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
							.map(|duration| duration.as_millis() as u64)
							.unwrap_or(0),
						accessed: metadata
							.as_ref()
							.and_then(|metadata| metadata.accessed().ok())
							.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
							.map(|duration| duration.as_millis() as u64)
							.unwrap_or(0),
					}
				})
				.collect();
		}

		items.sort_by(|left, right| {
			directory_first(left, right)
				.then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
		});

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

	/// Reorder the items for the configured sort.
	pub fn sort(&mut self, sort: crate::view::SortKey) {
		match sort {
			crate::view::SortKey::Newest => self.items.sort_by(|left, right| {
				directory_first(left, right)
					.then_with(|| right.modified.cmp(&left.modified))
					.then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
			}),
			crate::view::SortKey::Oldest => self.items.sort_by(|left, right| {
				directory_first(left, right)
					.then_with(|| left.modified.cmp(&right.modified))
					.then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
			}),
			crate::view::SortKey::Alphabetical | crate::view::SortKey::Free => {
				self.items.sort_by(|left, right| {
					directory_first(left, right)
						.then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
				})
			}
		}
		self.order = self.items.iter().map(|item| item.id).collect();
	}
}

fn directory_first(left: &Item, right: &Item) -> std::cmp::Ordering {
	right.fs_kind.cmp(&left.fs_kind)
}

fn mime_for(path: &Path) -> Option<String> {
	path.extension()
		.and_then(|extension| extension.to_str())
		.map(|extension| {
			mime_guess::from_ext(extension)
				.first_or_octet_stream()
				.to_string()
		})
}

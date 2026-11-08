//! Path relations and navigation plans.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::item::ItemId;
use crate::view::{PaneId, SlotId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NavDirection {
	Close,
	Open,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PathRelation {
	Same,
	Child { distance: usize },
	Parent { distance: usize },
	Sibling { common_ancestor: PathBuf },
	Unrelated { common_ancestor: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NavStep {
	pub path: PathBuf,
	pub direction: NavDirection,
	pub anchor: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NavigationRequest {
	pub pane: PaneId,
	pub target: PathBuf,
	pub record_history: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NavigationPlan {
	pub pane: PaneId,
	pub requested: PathBuf,
	pub from: PathBuf,
	pub to: PathBuf,
	pub focus: Option<PathBuf>,
	pub relation: PathRelation,
	pub common_ancestor: PathBuf,
	pub close: Vec<NavStep>,
	pub open: Vec<NavStep>,
	pub animated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SlotAnchor {
	pub item: ItemId,
	pub slot: SlotId,
	pub visible: bool,
}

/// The lexical relation between two paths. Symlinks are not resolved.
pub fn relation(from: &Path, to: &Path) -> PathRelation {
	let from_parts = parts(from);
	let to_parts = parts(to);
	match from_parts.cmp(&to_parts) {
		std::cmp::Ordering::Equal => PathRelation::Same,
		std::cmp::Ordering::Less if to_parts.starts_with(&from_parts) => PathRelation::Child {
			distance: to_parts.len() - from_parts.len(),
		},
		std::cmp::Ordering::Greater if from_parts.starts_with(&to_parts) => PathRelation::Parent {
			distance: from_parts.len() - to_parts.len(),
		},
		_ => {
			let common = common_prefix(&from_parts, &to_parts);
			let ancestor = path_at(&from_parts, common);
			if common == 0 {
				PathRelation::Unrelated {
					common_ancestor: ancestor,
				}
			} else {
				PathRelation::Sibling {
					common_ancestor: ancestor,
				}
			}
		}
	}
}

/// Build the close/open steps from `from` to `request.target`. A file target
/// lands in its parent folder with the file as `focus`.
pub fn plan(request: &NavigationRequest, from: &Path, animated: bool) -> NavigationPlan {
	let requested = normalize(&request.target);
	let target_is_file = requested.is_file();
	let to = if target_is_file {
		requested
			.parent()
			.map(Path::to_path_buf)
			.unwrap_or_else(|| PathBuf::from("/"))
	} else {
		requested.clone()
	};
	let from = normalize(from);
	let relation = relation(&from, &to);
	let from_parts = parts(&from);
	let to_parts = parts(&to);
	let common = common_prefix(&from_parts, &to_parts);
	let common_ancestor = path_at(&from_parts, common);

	let mut close = Vec::new();
	if !matches!(relation, PathRelation::Same | PathRelation::Child { .. }) {
		for depth in (common + 1..=from_parts.len()).rev() {
			let path = path_at(&from_parts, depth);
			close.push(NavStep {
				anchor: Some(path.clone()),
				path,
				direction: NavDirection::Close,
			});
		}
	}

	let mut open = Vec::new();
	if !matches!(relation, PathRelation::Same | PathRelation::Parent { .. }) {
		for depth in common + 1..=to_parts.len() {
			let path = path_at(&to_parts, depth);
			open.push(NavStep {
				anchor: Some(path.clone()),
				path,
				direction: NavDirection::Open,
			});
		}
	}

	NavigationPlan {
		pane: request.pane,
		requested,
		from,
		to,
		focus: target_is_file.then_some(request.target.clone()),
		relation,
		common_ancestor,
		close,
		open,
		animated,
	}
}

/// Collapse `.`/`..` and trailing separators without touching the filesystem.
pub fn normalize(path: &Path) -> PathBuf {
	let parts = parts(path);
	path_at(&parts, parts.len())
}

fn parts(path: &Path) -> Vec<OsString> {
	let mut parts = Vec::new();
	for component in path.components() {
		match component {
			Component::RootDir | Component::CurDir => {}
			Component::ParentDir => {
				parts.pop();
			}
			Component::Normal(part) => parts.push(part.to_os_string()),
			Component::Prefix(prefix) => parts.push(prefix.as_os_str().to_os_string()),
		}
	}
	parts
}

fn path_at(parts: &[OsString], count: usize) -> PathBuf {
	let mut path = PathBuf::from("/");
	for part in &parts[..count.min(parts.len())] {
		path.push(part);
	}
	path
}

fn common_prefix(left: &[OsString], right: &[OsString]) -> usize {
	let mut common = 0;
	while common < left.len() && common < right.len() && left[common] == right[common] {
		common += 1;
	}
	common
}

#[cfg(test)]
mod tests {
	use super::*;

	fn request(target: &str) -> NavigationRequest {
		NavigationRequest {
			pane: PaneId(0),
			target: PathBuf::from(target),
			record_history: true,
		}
	}

	#[test]
	fn relations_are_lexical() {
		assert_eq!(
			relation(Path::new("/a/b"), Path::new("/a/b/")),
			PathRelation::Same
		);
		assert_eq!(
			relation(Path::new("/a"), Path::new("/a/b/c")),
			PathRelation::Child { distance: 2 }
		);
		assert_eq!(
			relation(Path::new("/a/b/c"), Path::new("/a")),
			PathRelation::Parent { distance: 2 }
		);
		assert!(matches!(
			relation(Path::new("/a/b"), Path::new("/a/c")),
			PathRelation::Sibling { .. }
		));
		assert!(matches!(
			relation(Path::new("/a/b"), Path::new("/x/y")),
			PathRelation::Unrelated { .. }
		));
	}

	#[test]
	fn child_and_parent_walk_one_level() {
		let child = plan(&request("/a/b/c"), Path::new("/a"), false);
		assert!(child.close.is_empty());
		assert_eq!(
			child
				.open
				.iter()
				.map(|step| step.path.as_path())
				.collect::<Vec<_>>(),
			[Path::new("/a/b"), Path::new("/a/b/c")]
		);

		let parent = plan(&request("/a"), Path::new("/a/b/c"), false);
		assert!(parent.open.is_empty());
		assert_eq!(
			parent
				.close
				.iter()
				.map(|step| step.path.as_path())
				.collect::<Vec<_>>(),
			[Path::new("/a/b/c"), Path::new("/a/b")]
		);
	}

	#[test]
	fn cousins_close_to_the_common_ancestor_and_open_again() {
		let plan = plan(&request("/a/c/d"), Path::new("/a/b"), false);
		assert_eq!(
			plan.close
				.iter()
				.map(|step| step.path.as_path())
				.collect::<Vec<_>>(),
			[Path::new("/a/b")]
		);
		assert_eq!(
			plan.open
				.iter()
				.map(|step| step.path.as_path())
				.collect::<Vec<_>>(),
			[Path::new("/a/c"), Path::new("/a/c/d")]
		);
		assert_eq!(plan.common_ancestor, PathBuf::from("/a"));
	}

	#[test]
	fn a_file_target_lands_in_its_parent() {
		let root = std::env::temp_dir().join(format!("hishell-nav-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		let file = root.join("photo.png");
		std::fs::write(&file, b"png").unwrap();
		let plan = plan(&request(&file.to_string_lossy()), &root, true);
		assert_eq!(plan.to, root);
		assert_eq!(plan.focus, Some(file));
		assert!(plan.close.is_empty());
		std::fs::remove_dir_all(root).unwrap();
	}
}

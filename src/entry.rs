//! Freedesktop desktop-entry metadata for folder items.

use std::path::{Path, PathBuf};

use crate::config_parser::ConfigParser;
use crate::item::ItemKind;

const MAX_ENTRY_LEN: u64 = 256 * 1024;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryFields {
	pub name: Option<String>,
	pub comment: Option<String>,
	pub icon: Option<String>,
	pub kind: Option<ItemKind>,
	pub hidden: bool,
	pub target: Option<PathBuf>,
	pub exec: Option<String>,
}

/// Read the `[Desktop Entry]` section of a `.desktop` file.
pub fn read_entry(path: &Path) -> Option<EntryFields> {
	if !path
		.extension()
		.and_then(|extension| extension.to_str())
		.is_some_and(|extension| extension.eq_ignore_ascii_case("desktop"))
	{
		return None;
	}
	let metadata = std::fs::metadata(path).ok()?;
	if metadata.len() > MAX_ENTRY_LEN {
		return None;
	}
	let text = std::fs::read_to_string(path).ok()?;
	let (sections, _) = ConfigParser::parse_str(&text, &path.to_string_lossy());
	let values = sections.get("Desktop Entry")?;

	let string = |key: &str| match values.get(key) {
		Some(crate::config_parser::ConfigValue::String(value)) => Some(value.clone()),
		Some(crate::config_parser::ConfigValue::Number(value)) => Some(value.to_string()),
		_ => None,
	};
	let boolean = |key: &str| match values.get(key) {
		Some(crate::config_parser::ConfigValue::Boolean(value)) => *value,
		_ => false,
	};

	let kind = string("Type").and_then(|kind| match kind.as_str() {
		"Application" => Some(ItemKind::Application),
		"Link" => Some(ItemKind::Link),
		"Directory" => Some(ItemKind::Directory),
		"Button" => Some(ItemKind::Button),
		"Toggle" => Some(ItemKind::Toggle),
		"Number" => Some(ItemKind::Number),
		"Radio" => Some(ItemKind::Radio),
		"File" => Some(ItemKind::File),
		_ => None,
	});

	Some(EntryFields {
		name: string("Name").filter(|name| !name.is_empty()),
		comment: string("Comment").filter(|comment| !comment.is_empty()),
		icon: string("Icon").filter(|icon| !icon.is_empty()),
		kind,
		hidden: boolean("NoDisplay") || boolean("Hidden"),
		target: string("Path")
			.or_else(|| string("URL"))
			.map(PathBuf::from)
			.filter(|target| !target.as_os_str().is_empty()),
		exec: string("Exec").filter(|exec| !exec.is_empty()),
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_desktop_entry_fields() {
		let root = std::env::temp_dir().join(format!("hishell-entry-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("ride.desktop");
		std::fs::write(
			&path,
			"[Desktop Entry]\nType=Application\nName=Ride\nComment=Go fast\nIcon=ride\nExec=ride %f\n",
		)
		.unwrap();
		let entry = read_entry(&path).unwrap();
		assert_eq!(entry.name.as_deref(), Some("Ride"));
		assert_eq!(entry.comment.as_deref(), Some("Go fast"));
		assert_eq!(entry.icon.as_deref(), Some("ride"));
		assert_eq!(entry.kind, Some(ItemKind::Application));
		assert_eq!(entry.exec.as_deref(), Some("ride %f"));
		assert!(!entry.hidden);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn hidden_entries_are_flagged() {
		let root =
			std::env::temp_dir().join(format!("hishell-entry-hidden-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("secret.desktop");
		std::fs::write(&path, "[Desktop Entry]\nType=File\nNoDisplay=true\n").unwrap();
		assert!(read_entry(&path).unwrap().hidden);
		std::fs::remove_dir_all(root).unwrap();
	}
}

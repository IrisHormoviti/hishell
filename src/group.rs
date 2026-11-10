//! Display grouping for folder items.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::item::{FsKind, Item};
use crate::view::GroupMode;

const UNTITLED_RANK: i64 = 1_000_000;
const STASH_RANK: i64 = 1_000_001;

/// Put sorted items into their display groups. Grouping only moves whole blocks
/// of items around; the sort inside a group is the order the folder already
/// chose. Free placement owns its cells, so it is never grouped.
pub fn assign(items: &mut [Item], config: &Config) {
	for item in items.iter_mut() {
		item.group = 0;
		item.group_label.clear();
	}

	if config.sort == crate::view::SortKey::Free {
		return;
	}
	let stash_active = config.show_stash;
	if config.group_mode == GroupMode::None && !stash_active {
		return;
	}

	let now = date_context();
	let mut custom_names: Vec<String> = config
		.groups
		.iter()
		.map(|group| group.name.clone())
		.collect();
	let mut file_groups: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
	for (index, group) in config.groups.iter().enumerate() {
		for file in &group.files {
			file_groups.insert(file.as_str(), index);
		}
	}

	let mut keyed: Vec<((i64, String), usize)> = Vec::with_capacity(items.len());
	for (index, item) in items.iter().enumerate() {
		let key = if stash_active && config.stash_dotfiles && item.is_stashed {
			(STASH_RANK, String::from("Stash"))
		} else {
			match config.group_mode {
				GroupMode::Date => date_group_key(item.modified, now.as_ref()),
				GroupMode::Letters => letter_group_key(item),
				GroupMode::Type => type_group_key(item),
				GroupMode::Custom => custom_group_key(item, &mut custom_names, &file_groups),
				GroupMode::None => (0, String::new()),
			}
		};
		keyed.push((key, index));
	}

	let mut order: Vec<(i64, String)> = Vec::new();
	{
		let mut seen = BTreeMap::new();
		for (key, _) in &keyed {
			seen.insert(key.clone(), ());
		}
		order.extend(seen.into_keys());
	}
	let mut positions = std::collections::HashMap::new();
	for (index, key) in order.iter().enumerate() {
		positions.insert(key.clone(), index as u16);
	}
	for (key, item_index) in keyed {
		if let Some(&group) = positions.get(&key) {
			items[item_index].group = group;
			items[item_index].group_label = key.1;
		}
	}
	items.sort_by_key(|item| item.group);
}

fn custom_group_key(
	item: &Item,
	names: &mut Vec<String>,
	file_groups: &std::collections::HashMap<&str, usize>,
) -> (i64, String) {
	let explicit = item.custom_group.trim();
	let name = if !explicit.is_empty() {
		explicit.to_string()
	} else {
		file_groups
			.get(item.name.as_str())
			.or_else(|| file_groups.get(item.title.as_str()))
			.and_then(|index| names.get(*index))
			.cloned()
			.unwrap_or_default()
	};
	if name.is_empty() {
		return (UNTITLED_RANK, String::new());
	}
	let rank = match names.iter().position(|known| known == &name) {
		Some(position) => position as i64,
		None => {
			names.push(name.clone());
			(names.len() - 1) as i64
		}
	};
	(rank, name)
}

fn letter_group_key(item: &Item) -> (i64, String) {
	let name = if item.title.is_empty() {
		&item.name
	} else {
		&item.title
	};
	let letter = name
		.chars()
		.find(|character| character.is_alphanumeric())
		.unwrap_or('#');
	(0, letter.to_uppercase().collect())
}

fn type_group_key(item: &Item) -> (i64, String) {
	if item.fs_kind == FsKind::Directory {
		return (0, String::from("Folders"));
	}
	if !matches!(
		item.kind,
		crate::item::ItemKind::File | crate::item::ItemKind::Directory
	) || Path::new(&item.path)
		.extension()
		.and_then(|extension| extension.to_str())
		.is_some_and(|extension| {
			extension.eq_ignore_ascii_case("desktop") || extension.eq_ignore_ascii_case("appimage")
		}) {
		return (6, String::from("Apps"));
	}
	let mime = item.mime.as_deref().unwrap_or("application/octet-stream");
	let (rank, label) = if mime.starts_with("image/") {
		(1, "Images")
	} else if mime.starts_with("video/") {
		(2, "Videos")
	} else if mime.starts_with("audio/") {
		(3, "Audio")
	} else if mime.starts_with("text/")
		|| mime == "application/pdf"
		|| mime == "application/json"
		|| mime == "application/xml"
	{
		(4, "Documents")
	} else if mime.contains("zip")
		|| mime.contains("tar")
		|| mime.contains("gzip")
		|| mime.contains("compressed")
		|| mime.contains("archive")
	{
		(5, "Archives")
	} else {
		(7, "Other")
	};
	(rank, String::from(label))
}

struct DateContext {
	tm: libc::tm,
	days: i64,
}

const WEEKDAYS: [&str; 7] = [
	"Sunday",
	"Monday",
	"Tuesday",
	"Wednesday",
	"Thursday",
	"Friday",
	"Saturday",
];

const MONTHS: [&str; 12] = [
	"January",
	"February",
	"March",
	"April",
	"May",
	"June",
	"July",
	"August",
	"September",
	"October",
	"November",
	"December",
];

fn date_group_key(ms: u64, now: Option<&DateContext>) -> (i64, String) {
	let (Some(now), Some(tm)) = (now, local_tm(ms)) else {
		return (999_999, String::from("Unknown"));
	};
	let days = days_from_civil(
		i64::from(tm.tm_year) + 1900,
		i64::from(tm.tm_mon) + 1,
		i64::from(tm.tm_mday),
	);
	let ago = now.days - days;
	if ago <= 0 {
		return (0, String::from("Today"));
	}
	match ago {
		1 => (1, String::from("Yesterday")),
		2..=6 => (
			ago,
			String::from(WEEKDAYS[(tm.tm_wday.clamp(0, 6)) as usize]),
		),
		7..=13 => (7, String::from("Last week")),
		_ if tm.tm_year == now.tm.tm_year && tm.tm_mon == now.tm.tm_mon => {
			(8, String::from("This month"))
		}
		_ if tm.tm_year == now.tm.tm_year => {
			let months_ago = now.tm.tm_mon - tm.tm_mon;
			(
				20 + i64::from(months_ago),
				String::from(MONTHS[(tm.tm_mon.clamp(0, 11)) as usize]),
			)
		}
		_ => {
			let years_ago = now.tm.tm_year - tm.tm_year;
			(
				100 + i64::from(years_ago),
				format!("{}", i64::from(tm.tm_year) + 1900),
			)
		}
	}
}

fn local_tm(ms: u64) -> Option<libc::tm> {
	if ms == 0 {
		return None;
	}
	let seconds = (ms / 1000) as libc::time_t;
	let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
	// SAFETY: `localtime_r` fills the caller-owned struct.
	let filled = unsafe { libc::localtime_r(&seconds, tm.as_mut_ptr()) };
	if filled.is_null() {
		None
	} else {
		Some(unsafe { tm.assume_init() })
	}
}

fn date_context() -> Option<DateContext> {
	let now_ms = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.ok()
		.map(|duration| duration.as_millis() as u64)?;
	let tm = local_tm(now_ms)?;
	let days = days_from_civil(
		i64::from(tm.tm_year) + 1900,
		i64::from(tm.tm_mon) + 1,
		i64::from(tm.tm_mday),
	);
	Some(DateContext { tm, days })
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
	let y = year - i64::from(month <= 2);
	let era = if y >= 0 { y } else { y - 399 } / 400;
	let yoe = y - era * 400;
	let mp = (month + 9) % 12;
	let doy = (153 * mp + 2) / 5 + day - 1;
	let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
	era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::item::{FsKind, IconRef, ItemId, ItemKind};
	use std::path::PathBuf;

	fn item(name: &str) -> Item {
		Item {
			id: ItemId::for_path(Path::new(name)),
			name: name.to_string(),
			title: name.to_string(),
			comment: String::new(),
			path: PathBuf::from(name),
			entry_path: None,
			target: None,
			fs_kind: FsKind::File,
			kind: ItemKind::File,
			mime: Some(String::from("image/png")),
			icon: IconRef::Theme(String::from("image-x-generic")),
			model_icon: None,
			is_symlink: false,
			is_dotfile: false,
			toggle_on: false,
			number_value: String::new(),
			size: 0,
			modified: 0,
			created: 0,
			accessed: 0,
			group: 0,
			group_label: String::new(),
			is_stashed: false,
			custom_group: String::new(),
		}
	}

	#[test]
	fn letters_group_by_initial() {
		let mut items = vec![item("beta"), item("Alpha"), item("2 things")];
		let mut config = Config::default();
		config.group_mode = GroupMode::Letters;
		assign(&mut items, &config);
		let labels: Vec<&str> = items.iter().map(|item| item.group_label.as_str()).collect();
		assert_eq!(labels, ["2", "A", "B"]);
	}

	#[test]
	fn type_grouping_puts_folders_first_and_images_before_other() {
		let mut folder = item("a-folder");
		folder.fs_kind = FsKind::Directory;
		folder.kind = ItemKind::Directory;
		folder.mime = None;
		let mut archive = item("z.zip");
		archive.mime = Some(String::from("application/zip"));
		let mut items = vec![archive, item("b.png"), folder];
		let mut config = Config::default();
		config.group_mode = GroupMode::Type;
		assign(&mut items, &config);
		let labels: Vec<&str> = items.iter().map(|item| item.group_label.as_str()).collect();
		assert_eq!(labels, ["Folders", "Images", "Archives"]);
	}
}

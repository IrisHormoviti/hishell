//! Metadata and icons embedded in an AppImage.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::UNIX_EPOCH;

use crate::config_parser::{ConfigParser, ConfigValue};
use crate::squashfs::Squashfs;

const MAX_ENTRY_BYTES: u64 = 512 * 1024;
const MAX_ICON_BYTES: usize = 4 * 1024 * 1024;
const ICON_EXTENSIONS: &[&str] = &["png", "svg", "xpm", "jpg", "jpeg", "webp"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppImageEntry {
	pub name: Option<String>,
	pub comment: Option<String>,
	pub icon: Option<PathBuf>,
}

#[derive(Clone)]
struct Embedded {
	name: Option<String>,
	comment: Option<String>,
	icon: Option<String>,
}

struct CachedEntry {
	mtime: i64,
	size: u64,
	entry: Option<Embedded>,
}

static ENTRIES: LazyLock<Mutex<HashMap<PathBuf, CachedEntry>>> =
	LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn is_appimage(path: &Path) -> bool {
	path.extension()
		.and_then(|extension| extension.to_str())
		.is_some_and(|extension| extension.eq_ignore_ascii_case("appimage"))
}

/// The name, comment, and extracted icon of an AppImage, cached per file.
pub fn entry(path: &Path) -> Option<AppImageEntry> {
	let embedded = embedded_entry(path)?;
	Some(AppImageEntry {
		name: embedded.name,
		comment: embedded.comment,
		icon: icon(path),
	})
}

fn embedded_entry(path: &Path) -> Option<Embedded> {
	if !is_appimage(path) {
		return None;
	}
	let (mtime, size) = stat_key(path)?;
	if let Some(cached) = ENTRIES.lock().unwrap().get(path)
		&& cached.mtime == mtime
		&& cached.size == size
	{
		return cached.entry.clone();
	}
	let entry = extract_entry(path);
	ENTRIES.lock().unwrap().insert(
		path.to_path_buf(),
		CachedEntry {
			mtime,
			size,
			entry: entry.clone(),
		},
	);
	entry
}

fn extract_entry(path: &Path) -> Option<Embedded> {
	let mut image = Squashfs::open(path)?;
	let root = image.root_inode();
	let name = image
		.read_dir(root)?
		.into_iter()
		.find(|entry| entry.name.to_lowercase().ends_with(".desktop"))?
		.name;
	let inode = image.lookup_path(&name)?;
	let bytes = image.read_file(&inode)?;
	if bytes.len() as u64 > MAX_ENTRY_BYTES {
		return None;
	}
	parse_entry(&bytes)
}

fn parse_entry(bytes: &[u8]) -> Option<Embedded> {
	let text = String::from_utf8_lossy(bytes);
	let (sections, _) = ConfigParser::parse_str(&text, "<appimage>");
	let values = sections.get("Desktop Entry")?;
	let string = |key: &str| match values.get(key) {
		Some(ConfigValue::String(value)) => Some(value.clone()),
		_ => None,
	};
	Some(Embedded {
		name: string("Name").map(|value| unescape(&value)),
		comment: string("Comment").map(|value| unescape(&value)),
		icon: string("Icon"),
	})
}

fn unescape(value: &str) -> String {
	value
		.replace("\\n", "\n")
		.replace("\\t", "\t")
		.replace("\\r", "\r")
		.replace("\\\\", "\\")
}

/// Extract the AppImage's icon into the cache and return its path.
pub fn icon(path: &Path) -> Option<PathBuf> {
	if !is_appimage(path) {
		return None;
	}
	let (mtime, size) = stat_key(path)?;
	let embedded = embedded_entry(path);
	if let Some(icon) = embedded.as_ref().and_then(|entry| entry.icon.as_deref()) {
		let icon = icon.trim();
		if icon.starts_with('/') && Path::new(icon).is_file() {
			return Some(PathBuf::from(icon));
		}
	}
	let cache = dirs::cache_dir()?.join("hishell").join("appimage");
	let key = format!("{:x}", hash(&format!("{}|{mtime}|{size}", path.display())));
	if let Some(cached) = cached_icon(&cache, &key) {
		return Some(cached);
	}
	let mut image = Squashfs::open(path)?;
	let (extension, bytes) = extract_icon(&mut image, embedded.as_ref())?;
	std::fs::create_dir_all(&cache).ok()?;
	let target = cache.join(format!("{key}.{extension}"));
	let temporary = cache.join(format!(".{key}.tmp"));
	std::fs::write(&temporary, &bytes).ok()?;
	std::fs::rename(&temporary, &target).ok()?;
	remove_stale_icons(&cache, &key, &target);
	Some(target)
}

fn extract_icon(image: &mut Squashfs, entry: Option<&Embedded>) -> Option<(String, Vec<u8>)> {
	let mut candidates = Vec::new();
	if let Some(icon) = entry.and_then(|entry| entry.icon.as_deref()) {
		let icon = icon.trim();
		if icon.starts_with('/') {
			candidates.push(icon.trim_start_matches('/').to_string());
		} else if icon.contains('/') {
			candidates.push(icon.trim_start_matches("./").to_string());
		} else {
			for extension in ICON_EXTENSIONS {
				candidates.push(format!("{icon}.{extension}"));
			}
			candidates.push(icon.to_string());
		}
	}
	candidates.push(".DirIcon".to_string());
	for name in candidates {
		let Some(inode) = image.lookup_path(&name) else {
			continue;
		};
		let Some(bytes) = image.read_file(&inode) else {
			continue;
		};
		if bytes.len() > MAX_ICON_BYTES {
			continue;
		}
		if let Some(extension) = sniff_image(&bytes) {
			return Some((extension.to_string(), bytes));
		}
	}
	None
}

fn cached_icon(cache: &Path, key: &str) -> Option<PathBuf> {
	let prefix = format!("{key}.");
	std::fs::read_dir(cache)
		.ok()?
		.flatten()
		.map(|entry| entry.path())
		.find(|path| {
			path.file_name()
				.and_then(|name| name.to_str())
				.is_some_and(|name| name.starts_with(&prefix))
		})
}

fn remove_stale_icons(cache: &Path, key: &str, keep: &Path) {
	let prefix = format!("{key}.");
	let Ok(entries) = std::fs::read_dir(cache) else {
		return;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if path == keep {
			continue;
		}
		let stale = path
			.file_name()
			.and_then(|name| name.to_str())
			.is_some_and(|name| name.starts_with(&prefix));
		if stale {
			let _ = std::fs::remove_file(path);
		}
	}
}

fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
	if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
		return Some("png");
	}
	if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
		return Some("jpg");
	}
	if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
		return Some("gif");
	}
	if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
		return Some("webp");
	}
	if bytes.starts_with(b"BM") {
		return Some("bmp");
	}
	if bytes.starts_with(&[0x00, 0x00, 0x01, 0x00]) {
		return Some("ico");
	}
	if bytes.starts_with(b"/* XPM */") {
		return Some("xpm");
	}
	let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]);
	head.contains("<svg").then_some("svg")
}

fn stat_key(path: &Path) -> Option<(i64, u64)> {
	let metadata = std::fs::metadata(path).ok()?;
	let mtime = metadata
		.modified()
		.ok()
		.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
		.map(|duration| duration.as_secs() as i64)
		.unwrap_or(0);
	Some((mtime, metadata.len()))
}

fn hash(input: &str) -> u64 {
	let mut hash = 0xcbf2_9ce4_8422_2325u64;
	for byte in input.as_bytes() {
		hash ^= *byte as u64;
		hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
	}
	hash
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn recognizes_appimage_paths() {
		assert!(is_appimage(Path::new("/tmp/app.AppImage")));
		assert!(is_appimage(Path::new("/tmp/app.appimage")));
		assert!(!is_appimage(Path::new("/tmp/app.desktop")));
	}

	#[test]
	fn parses_embedded_entry_fields() {
		let entry = parse_entry(
			b"[Desktop Entry]\nType=Application\nName=Aseprite\nComment=Pixel\\nart\nIcon=aseprite\n",
		)
		.unwrap();
		assert_eq!(entry.name.as_deref(), Some("Aseprite"));
		assert_eq!(entry.comment.as_deref(), Some("Pixel\nart"));
		assert_eq!(entry.icon.as_deref(), Some("aseprite"));
	}
}

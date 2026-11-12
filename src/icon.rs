//! MIME detection and themed icon names.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use crate::item::IconRef;

/// The MIME type for a path, by extension and then by content sniffing.
pub fn mime_for_path(path: &Path) -> Option<String> {
	let guessed = mime_guess::from_path(path).first_raw();
	// `mime_guess` only knows common extensions. The shared MIME database
	// covers the rest (disc images such as `.rvz`, emulator formats, ...), and
	// is the same source the desktop uses for "Open with". It also knows
	// container formats that `mime_guess` lumps into `application/octet-stream`
	// (`.iso`), which would otherwise bypass the registered launcher.
	if let Some(mime) = guessed.filter(|mime| *mime != "application/octet-stream") {
		return Some(mime.to_string());
	}
	if let Some(mime) = shared_mime_for_extension(path) {
		return Some(mime);
	}
	if let Some(mime) = guessed {
		return Some(mime.to_string());
	}
	infer::get_from_path(path)
		.ok()
		.flatten()
		.map(|kind| kind.mime_type().to_string())
}

/// Look up an extension in `/usr/share/mime/globs2`, cached for the process.
fn shared_mime_for_extension(path: &Path) -> Option<String> {
	static GLOBS: OnceLock<HashMap<String, String>> = OnceLock::new();
	let globs = GLOBS.get_or_init(|| {
		let text = std::fs::read_to_string("/usr/share/mime/globs2").unwrap_or_default();
		parse_globs2(&text)
	});
	let extension = path.extension()?.to_str()?.to_ascii_lowercase();
	globs.get(&extension).cloned()
}

/// Parse the `weight:mime:glob` lines of `globs2` into extension -> MIME.
/// Only simple `*.ext` globs are kept; the highest weight wins.
fn parse_globs2(text: &str) -> HashMap<String, String> {
	let mut best: HashMap<String, (u32, String)> = HashMap::new();
	for line in text.lines() {
		let line = line.trim();
		if line.is_empty() || line.starts_with('#') {
			continue;
		}
		let mut parts = line.splitn(3, ':');
		let (Some(weight), Some(mime), Some(glob)) = (parts.next(), parts.next(), parts.next())
		else {
			continue;
		};
		let Ok(weight) = weight.parse::<u32>() else {
			continue;
		};
		let Some(extension) = glob.strip_prefix("*.") else {
			continue;
		};
		if extension.is_empty() || !extension.chars().all(|c| c.is_ascii_alphanumeric()) {
			continue;
		}
		let key = extension.to_ascii_lowercase();
		let entry = best
			.entry(key)
			.or_insert_with(|| (weight, mime.to_string()));
		if weight > entry.0 {
			*entry = (weight, mime.to_string());
		}
	}
	best.into_iter()
		.map(|(key, (_, mime))| (key, mime))
		.collect()
}

/// The themed icon name for a path.
pub fn for_path(path: &Path) -> IconRef {
	match mime_for_path(path) {
		Some(mime) => for_mime(&mime),
		None => IconRef::Theme(String::from("text-x-generic")),
	}
}

/// The themed icon name for a MIME type.
pub fn for_mime(mime: &str) -> IconRef {
	let name = match mime {
		"inode/directory" => "folder",
		mime if mime.starts_with("image/") => "image-x-generic",
		mime if mime.starts_with("audio/") => "audio-x-generic",
		mime if mime.starts_with("video/") => "video-x-generic",
		mime if mime.starts_with("font/") => "font-x-generic",
		mime if mime.starts_with("text/") => match mime {
			"text/html" | "text/xml" | "application/json" | "application/xml" => "text-html",
			"text/x-shellscript" | "text/x-perl" | "text/x-python" => "text-x-script",
			_ => "text-x-generic",
		},
		"application/pdf" => "application-pdf",
		"application/zip"
		| "application/x-tar"
		| "application/gzip"
		| "application/x-bzip2"
		| "application/x-xz"
		| "application/x-7z-compressed"
		| "application/x-rar"
		| "application/zstd" => "package-x-generic",
		"application/x-executable"
		| "application/x-sharedlib"
		| "application/x-pie-executable"
		| "application/x-shellscript" => "application-x-executable",
		"application/vnd.oasis.opendocument.text"
		| "application/msword"
		| "application/rtf"
		| "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
			"x-office-document"
		}
		"application/vnd.oasis.opendocument.spreadsheet"
		| "application/vnd.ms-excel"
		| "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "x-office-spreadsheet",
		"application/vnd.oasis.opendocument.presentation"
		| "application/vnd.ms-powerpoint"
		| "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
			"x-office-presentation"
		}
		// The freedesktop convention: the icon name is the MIME type with
		// "/" replaced by "-". The theme lookup falls back to a generic file
		// icon when the theme has no such icon.
		"application/octet-stream" | "" => "text-x-generic",
		other => return IconRef::Theme(other.replace('/', "-")),
	};
	IconRef::Theme(String::from(name))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn theme(mime: &str) -> String {
		match for_mime(mime) {
			IconRef::Theme(name) => name,
			IconRef::Path(path) => path.to_string_lossy().into_owned(),
		}
	}

	#[test]
	fn common_types_have_icons() {
		assert_eq!(theme("image/png"), "image-x-generic");
		assert_eq!(theme("audio/mpeg"), "audio-x-generic");
		assert_eq!(theme("video/mp4"), "video-x-generic");
		assert_eq!(theme("application/pdf"), "application-pdf");
		assert_eq!(theme("application/zip"), "package-x-generic");
		assert_eq!(theme("text/html"), "text-html");
		assert_eq!(theme("application/octet-stream"), "text-x-generic");
		// Unknown application types keep their own themed icon name.
		assert_eq!(
			theme("application/x-gamecube-rom"),
			"application-x-gamecube-rom"
		);
	}

	#[test]
	fn shared_mime_globs_pick_the_heaviest_extension() {
		let globs = parse_globs2(
			"50:application/x-gamecube-rom:*.rvz\n\
			 50:application/x-rvz-image:*.rvz\n\
			 60:application/x-wii-rom:*.rvz\n\
			 50:image/png:*.png\n\
			 50:application/pdf:*.[pP][dD][fF]\n",
		);
		assert_eq!(
			globs.get("rvz").map(String::as_str),
			Some("application/x-wii-rom")
		);
		assert_eq!(globs.get("png").map(String::as_str), Some("image/png"));
		assert!(!globs.contains_key("pdf"));
	}

	#[test]
	fn content_sniffing_covers_extensionless_files() {
		let root = std::env::temp_dir().join(format!("hishell-icon-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("picture");
		std::fs::write(&path, b"\x89PNG\r\n\x1a\n").unwrap();
		assert_eq!(
			theme(mime_for_path(&path).unwrap().as_str()),
			"image-x-generic"
		);
		std::fs::remove_dir_all(root).unwrap();
	}
}

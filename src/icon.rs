//! MIME detection and themed icon names.

use std::path::Path;

use crate::item::IconRef;

/// The MIME type for a path, by extension and then by content sniffing.
pub fn mime_for_path(path: &Path) -> Option<String> {
	if let Some(mime) = mime_guess::from_path(path).first_raw() {
		return Some(mime.to_string());
	}
	infer::get_from_path(path)
		.ok()
		.flatten()
		.map(|kind| kind.mime_type().to_string())
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
		_ => "text-x-generic",
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

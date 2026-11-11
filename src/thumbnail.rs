//! Cached image thumbnails for the FreeDesktop platform.

use std::path::{Path, PathBuf};

use crate::platform::{PlatformError, Thumbnail, ThumbnailRequest, Thumbnailer};

#[derive(Default)]
pub struct FreeDesktopThumbnailer;

impl Thumbnailer for FreeDesktopThumbnailer {
	fn supports(&self, path: &Path) -> bool {
		is_image(path)
	}

	fn thumbnail(&self, request: &ThumbnailRequest) -> Result<Option<Thumbnail>, PlatformError> {
		if !is_image(&request.path) {
			return Ok(None);
		}
		let metadata = match std::fs::metadata(&request.path) {
			Ok(metadata) => metadata,
			Err(_) => return Ok(None),
		};
		let size = request.size.max(1);
		let target = cache_dir().join(format!(
			"{:x}.png",
			hash(&format!(
				"{}|{}|{}|{}",
				request.path.display(),
				metadata.len(),
				metadata
					.modified()
					.ok()
					.and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
					.map(|duration| duration.as_nanos())
					.unwrap_or(0),
				size
			))
		));
		if target.is_file() {
			return Ok(Some(Thumbnail { path: target, size }));
		}
		let image =
			image::open(&request.path).map_err(|error| PlatformError::Io(error.to_string()))?;
		let thumbnail = image.thumbnail(size, size);
		if let Some(parent) = target.parent() {
			std::fs::create_dir_all(parent)
				.map_err(|error| PlatformError::Io(error.to_string()))?;
		}
		thumbnail
			.save(&target)
			.map_err(|error| PlatformError::Io(error.to_string()))?;
		Ok(Some(Thumbnail { path: target, size }))
	}
}

fn is_image(path: &Path) -> bool {
	path.extension()
		.and_then(|extension| extension.to_str())
		.is_some_and(|extension| {
			matches!(
				extension.to_ascii_lowercase().as_str(),
				"png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "tiff"
			)
		})
}

fn cache_dir() -> PathBuf {
	dirs::cache_dir()
		.unwrap_or_else(|| PathBuf::from("/tmp"))
		.join("hishell")
		.join("thumbnails")
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
	fn an_image_is_cached_and_reused() {
		let root = std::env::temp_dir().join(format!("hishell-thumbnail-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("photo.png");
		image::RgbaImage::new(8, 8).save(&path).unwrap();
		let request = ThumbnailRequest {
			path: path.clone(),
			size: 4,
		};
		let first = FreeDesktopThumbnailer.thumbnail(&request).unwrap().unwrap();
		assert!(first.path.is_file());
		let second = FreeDesktopThumbnailer.thumbnail(&request).unwrap().unwrap();
		assert_eq!(first.path, second.path);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn a_non_image_is_not_thumbnailed() {
		let root =
			std::env::temp_dir().join(format!("hishell-thumbnail-text-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("notes.txt");
		std::fs::write(&path, b"x").unwrap();
		assert!(
			FreeDesktopThumbnailer
				.thumbnail(&ThumbnailRequest { path, size: 4 })
				.unwrap()
				.is_none()
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn only_images_are_supported() {
		assert!(FreeDesktopThumbnailer.supports(Path::new("/tmp/photo.png")));
		assert!(FreeDesktopThumbnailer.supports(Path::new("/tmp/photo.JPG")));
		assert!(!FreeDesktopThumbnailer.supports(Path::new("/tmp/notes.txt")));
	}
}

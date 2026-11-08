//! Lexical path helpers and navigation plans.

use std::path::{Path, PathBuf};

/// Resolve a leading `~/` against the home folder, leaving other paths alone.
pub fn expand_home(value: &str) -> PathBuf {
	let Some(rest) = value.strip_prefix("~/") else {
		return PathBuf::from(value);
	};
	match dirs::home_dir() {
		Some(home) => home.join(rest),
		None => PathBuf::from(value),
	}
}

/// Path components without the root, joined by newlines for the location bar.
pub fn segments(path: &str) -> String {
	let trimmed = path.trim();
	if trimmed.is_empty() || trimmed == "." {
		return String::from("/");
	}
	let trimmed = trimmed
		.strip_suffix('/')
		.filter(|p| !p.is_empty())
		.unwrap_or(trimmed);
	trimmed
		.split('/')
		.filter(|part| !part.is_empty())
		.collect::<Vec<_>>()
		.join("\n")
}

/// The absolute path formed by the first `index + 1` components.
pub fn path_for_index(current_path: &str, index: i32) -> String {
	let trimmed = current_path.trim();
	if trimmed.is_empty() || trimmed == "." {
		return String::from("/");
	}
	let trimmed = trimmed
		.strip_suffix('/')
		.filter(|p| !p.is_empty())
		.unwrap_or(trimmed);
	let parts: Vec<&str> = trimmed.split('/').filter(|part| !part.is_empty()).collect();
	if (index as usize) < parts.len() {
		format!("/{}", parts[..=index as usize].join("/"))
	} else {
		current_path.to_string()
	}
}

/// The last component of a path.
pub fn folder_name(path: &str) -> String {
	let trimmed = path.trim();
	if trimmed.is_empty() || trimmed == "/" || trimmed == "." {
		return String::from("/");
	}
	let trimmed = trimmed
		.strip_suffix('/')
		.filter(|p| !p.is_empty())
		.unwrap_or(trimmed);
	trimmed
		.rsplit('/')
		.next()
		.filter(|name| !name.is_empty())
		.unwrap_or("/")
		.to_string()
}

/// Collapse a trailing slash without touching the rest of the path.
pub fn normalize(path: &str) -> String {
	let trimmed = path.trim();
	if trimmed.is_empty() {
		return String::from(".");
	}
	if trimmed == "/" {
		return String::from("/");
	}
	trimmed
		.strip_suffix('/')
		.filter(|p| !p.is_empty())
		.unwrap_or(trimmed)
		.to_string()
}

/// Plan a folder transition between two absolute paths, lexically.
///
/// Returns `{ "relation", "close", "open" }`. `close` lists the folders to
/// leave one level at a time, from the current folder up to the nearest common
/// ancestor; `open` lists the folders to enter from that ancestor down to the
/// destination. Symlinks are not resolved.
pub fn navigation_plan(from: &str, to: &str) -> String {
	let from_parts = components(from);
	let to_parts = components(to);
	let mut close = Vec::new();
	let mut open = Vec::new();
	let relation;
	if from_parts == to_parts {
		relation = "same";
	} else if is_under(&to_parts, &from_parts) {
		relation = "child";
		for depth in from_parts.len()..to_parts.len() {
			open.push(join(&to_parts, depth + 1));
		}
	} else if is_under(&from_parts, &to_parts) {
		relation = "parent";
		for depth in (to_parts.len()..from_parts.len()).rev() {
			close.push(join(&from_parts, depth + 1));
		}
	} else {
		relation = "sibling";
		let mut common = 0;
		while common < from_parts.len()
			&& common < to_parts.len()
			&& from_parts[common] == to_parts[common]
		{
			common += 1;
		}
		for depth in (common..from_parts.len()).rev() {
			close.push(join(&from_parts, depth + 1));
		}
		for depth in common..to_parts.len() {
			open.push(join(&to_parts, depth + 1));
		}
	}
	serde_json::json!({
		"relation": relation,
		"close": close,
		"open": open,
	})
	.to_string()
}

pub fn parent(path: &str) -> String {
	match Path::new(path.trim()).parent() {
		Some(parent) => parent.to_string_lossy().into_owned(),
		None => String::from("/"),
	}
}

/// Whether `text` resolves to something that exists, with `base` resolving
/// relative text.
pub fn exists(base: &str, text: &str) -> bool {
	let text = text.trim();
	if text.is_empty() {
		return false;
	}
	Path::new(&resolve(base, text)).exists()
}

/// Resolve a layout entry against the folder the window shows. `~` and `~/sub`
/// expand against the home directory, absolute paths stay absolute, and
/// anything else joins `base`. `.` and `..` are collapsed.
pub fn resolve(base: &str, spec: &str) -> String {
	let spec = spec.trim();
	let base = base.trim();

	// Before the window's folder is known a relative entry is passed through
	// untouched, so the panes pick it up once the base is set.
	if base.is_empty() && !spec.starts_with('/') && !spec.starts_with('~') {
		return spec.to_string();
	}

	let joined = if spec == "~" || spec.starts_with("~/") {
		match dirs::home_dir().map(|home| home.to_string_lossy().into_owned()) {
			Some(home) if spec == "~" => home,
			Some(home) => format!("{}/{}", home, &spec[2..]),
			None => spec.to_string(),
		}
	} else if spec.starts_with('/') {
		spec.to_string()
	} else {
		format!("{}/{}", base, spec)
	};

	let mut parts: Vec<&str> = Vec::new();
	for part in joined.split('/') {
		match part {
			"" | "." => {}
			".." => {
				parts.pop();
			}
			other => parts.push(other),
		}
	}
	format!("/{}", parts.join("/"))
}

/// Directory entries matching the last segment of `text`.
///
/// Returns a JSON array of `{name, path, suffix, is_dir}`. Folders are listed
/// first and get a trailing slash.
pub fn complete(base: &str, text: &str, limit: i32) -> String {
	let text = text.trim();
	if text.is_empty() {
		return String::from("[]");
	}
	let limit = limit.clamp(1, 64) as usize;

	let (lookup_spec, display_prefix, prefix) = if text == "~" {
		("~/".to_string(), "~/".to_string(), String::new())
	} else if let Some(index) = text.rfind('/') {
		let split = text[..=index].to_string();
		(split.clone(), split, text[index + 1..].to_string())
	} else {
		(String::new(), String::new(), text.to_string())
	};

	let lookup = resolve(base, &lookup_spec);
	let Ok(read) = std::fs::read_dir(&lookup) else {
		return String::from("[]");
	};

	let prefix_lower = prefix.to_lowercase();
	let show_hidden = prefix.starts_with('.');
	let mut entries: Vec<(String, bool)> = Vec::new();
	for entry in read.flatten() {
		let name = entry.file_name().to_string_lossy().into_owned();
		if !show_hidden && name.starts_with('.') {
			continue;
		}
		if !name.to_lowercase().starts_with(&prefix_lower) {
			continue;
		}
		entries.push((name, entry.path().is_dir()));
	}
	entries.sort_by(|left, right| {
		right
			.1
			.cmp(&left.1)
			.then_with(|| left.0.to_lowercase().cmp(&right.0.to_lowercase()))
	});
	entries.truncate(limit);

	let suggestions: Vec<serde_json::Value> = entries
		.iter()
		.map(|(name, is_dir)| {
			let rest: String = name.chars().skip(prefix.chars().count()).collect();
			let path = if *is_dir {
				format!("{}{}/", display_prefix, name)
			} else {
				format!("{}{}", display_prefix, name)
			};
			let suffix = if *is_dir { format!("{}/", rest) } else { rest };
			serde_json::json!({
				"name": name,
				"path": path,
				"suffix": suffix,
				"is_dir": is_dir,
			})
		})
		.collect();
	serde_json::Value::Array(suggestions).to_string()
}

fn components(path: &str) -> Vec<String> {
	path.trim()
		.split('/')
		.filter(|part| !part.is_empty())
		.map(str::to_string)
		.collect()
}

fn is_under(parts: &[String], prefix: &[String]) -> bool {
	parts.len() >= prefix.len() && parts[..prefix.len()] == *prefix
}

fn join(parts: &[String], count: usize) -> String {
	if count == 0 {
		String::from("/")
	} else {
		format!("/{}", parts[..count].join("/"))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn navigation_plans_name_the_relation() {
		assert_eq!(
			serde_json::from_str::<serde_json::Value>(&navigation_plan("/a/b", "/a/b")).unwrap()["relation"],
			"same"
		);
		assert_eq!(
			serde_json::from_str::<serde_json::Value>(&navigation_plan("/a", "/a/b")).unwrap()["relation"],
			"child"
		);
		assert_eq!(
			serde_json::from_str::<serde_json::Value>(&navigation_plan("/a/b", "/a")).unwrap()["relation"],
			"parent"
		);
		assert_eq!(
			serde_json::from_str::<serde_json::Value>(&navigation_plan("/a/b", "/a/c")).unwrap()["relation"],
			"sibling"
		);
	}

	#[test]
	fn navigation_steps_walk_one_level_at_a_time() {
		let plan: serde_json::Value =
			serde_json::from_str(&navigation_plan("/a/b", "/a/c/d")).unwrap();
		assert_eq!(plan["close"], serde_json::json!(["/a/b"]));
		assert_eq!(plan["open"], serde_json::json!(["/a/c", "/a/c/d"]));
	}

	#[test]
	fn resolve_collapses_dots() {
		assert_eq!(resolve("/base", "../next"), String::from("/next"));
		assert_eq!(resolve("/base", "./here"), String::from("/base/here"));
	}

	#[test]
	fn segments_and_names_stay_lexical() {
		assert_eq!(segments("/a/b/"), String::from("a\nb"));
		assert_eq!(folder_name("/a/b/"), String::from("b"));
		assert_eq!(path_for_index("/a/b/c", 1), String::from("/a/b"));
	}
}

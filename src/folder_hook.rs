//! Folder hooks: commands a folder runs before it is listed.
//!
//! A definition is a small desktop entry installed under
//! `hishell/folder-hooks` in the XDG data directories, like a descriptor
//! generator:
//!
//! ```ini
//! [Folder Hook]
//! Exec=/usr/bin/bundle-converter --root %p
//! Marker=manifest.json
//! ```
//!
//! `%p` is the folder being listed. `Marker` makes the hook conditional: it
//! runs only while an immediate child folder contains that file and still
//! lacks the `.directory` that means the child was prepared. Without
//! `Marker` the hook runs for every folder.
//!
//! A hook is a control, not a launch: it runs in the folder before the scan,
//! so the items the scan reports are the ones it produced. A definition whose
//! program is not installed is ignored.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The file that means a child folder was already prepared.
const DONE: &str = ".directory";

const EXTENSION: &str = "folder-hook";

#[derive(Clone, Debug)]
struct Hook {
	exec: String,
	marker: Option<String>,
}

/// Run the hooks that apply to `folder`.
pub fn prepare(folder: &Path) -> bool {
	if !folder.is_dir() {
		return false;
	}
	let mut ran = false;
	for hook in hooks().iter().filter(|hook| hook.applies(folder)) {
		ran |= hook.run(folder);
	}
	ran
}

impl Hook {
	fn applies(&self, folder: &Path) -> bool {
		let Some(marker) = self.marker.as_deref() else {
			return true;
		};
		let Ok(entries) = fs::read_dir(folder) else {
			return false;
		};
		entries.flatten().any(|entry| {
			let child = entry.path();
			child.is_dir() && child.join(marker).is_file() && !child.join(DONE).is_file()
		})
	}

	fn run(&self, folder: &Path) -> bool {
		let Ok(args) = crate::entry::expand_hook(&self.exec, None, "", folder) else {
			return false;
		};
		let Some((program, rest)) = args.split_first() else {
			return false;
		};
		Command::new(program)
			.args(rest)
			.current_dir(folder)
			.status()
			.is_ok()
	}
}

fn hooks() -> &'static Vec<Hook> {
	HOOKS.get_or_init(|| {
		let mut out = Vec::new();
		for dir in hook_dirs() {
			let Ok(entries) = fs::read_dir(&dir) else {
				continue;
			};
			let mut files: Vec<PathBuf> = entries
				.flatten()
				.map(|entry| entry.path())
				.filter(|path| {
					path.extension()
						.is_some_and(|ext| ext.eq_ignore_ascii_case(EXTENSION))
				})
				.collect();
			files.sort();
			for file in files {
				if let Some(hook) = read_hook(&file) {
					out.push(hook);
				}
			}
		}
		out
	})
}

static HOOKS: OnceLock<Vec<Hook>> = OnceLock::new();

fn hook_dirs() -> Vec<PathBuf> {
	crate::descriptor_generator::data_dirs()
		.into_iter()
		.map(|dir| dir.join("hishell/folder-hooks"))
		.collect()
}

fn read_hook(file: &Path) -> Option<Hook> {
	let contents = fs::read_to_string(file).ok()?;
	parse_hook(&contents)
}

fn parse_hook(contents: &str) -> Option<Hook> {
	let mut exec = None;
	let mut try_exec = None;
	let mut marker = None;
	for line in contents.lines() {
		let line = line.trim();
		if let Some(rest) = line.strip_prefix("Exec=") {
			exec = Some(rest.trim().to_string()).filter(|exec| !exec.is_empty());
		} else if let Some(rest) = line.strip_prefix("TryExec=") {
			try_exec = Some(rest.trim().to_string()).filter(|exec| !exec.is_empty());
		} else if let Some(rest) = line.strip_prefix("Marker=") {
			marker = Some(rest.trim().to_string()).filter(|marker| !marker.is_empty());
		}
	}
	let exec = exec?;
	let gate = try_exec.or_else(|| exec.split_whitespace().next().map(String::from))?;
	if !crate::descriptor_generator::command_available(&gate) {
		return None;
	}
	Some(Hook { exec, marker })
}

#[cfg(test)]
mod tests {
	use super::*;

	fn hook(contents: &str) -> Option<Hook> {
		parse_hook(contents)
	}

	#[test]
	fn a_hook_needs_an_exec() {
		assert!(hook("[Folder Hook]\nMarker=manifest.json\n").is_none());
	}

	#[test]
	fn a_hook_reads_its_marker() {
		let parsed = hook("[Folder Hook]\nExec=/bin/sh -c true\nMarker=manifest.json\n").unwrap();
		assert_eq!(parsed.exec, "/bin/sh -c true");
		assert_eq!(parsed.marker.as_deref(), Some("manifest.json"));
	}

	#[test]
	fn a_missing_program_ignores_the_definition() {
		assert!(hook("[Folder Hook]\nExec=/nonexistent/converter %p\nMarker=x\n").is_none());
	}

	#[test]
	fn a_marker_waits_for_an_unprepared_child() {
		let root = std::env::temp_dir().join(format!("hishell-hook-{}", std::process::id()));
		let _ = fs::remove_dir_all(&root);
		fs::create_dir_all(root.join("Game")).unwrap();
		fs::write(root.join("Game/manifest.json"), "{}").unwrap();

		let parsed = Hook {
			exec: String::from("/bin/true"),
			marker: Some(String::from("manifest.json")),
		};
		assert!(parsed.applies(&root));

		fs::write(root.join("Game").join(DONE), "[Desktop Entry]\n").unwrap();
		assert!(!parsed.applies(&root));

		let _ = fs::remove_dir_all(&root);
	}

	#[test]
	fn a_hook_runs_in_the_folder() {
		let root = std::env::temp_dir().join(format!("hishell-hook-run-{}", std::process::id()));
		let _ = fs::remove_dir_all(&root);
		fs::create_dir_all(&root).unwrap();

		let parsed = Hook {
			exec: String::from("sh -c \"pwd > ran.txt\""),
			marker: None,
		};
		assert!(parsed.run(&root));
		assert!(root.join("ran.txt").is_file());

		let _ = fs::remove_dir_all(&root);
	}

	#[test]
	fn a_hook_without_a_marker_always_applies() {
		let parsed = Hook {
			exec: String::from("/bin/true"),
			marker: None,
		};
		assert!(parsed.applies(Path::new("/")));
	}
}

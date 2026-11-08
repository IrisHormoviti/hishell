//! Navigation history.

use std::path::PathBuf;

use crate::view::PaneId;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HistoryEntry {
	pub pane: PaneId,
	pub path: PathBuf,
	pub focus: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HistoryCommand {
	Back,
	Forward,
	Clear,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HistoryStatus {
	pub can_back: bool,
	pub can_forward: bool,
}

#[derive(Clone, Debug, Default)]
pub struct History {
	back: Vec<HistoryEntry>,
	forward: Vec<HistoryEntry>,
	current: Option<HistoryEntry>,
}

impl History {
	pub fn record(&mut self, entry: HistoryEntry) {
		if self.current.as_ref() == Some(&entry) {
			return;
		}
		if let Some(current) = self.current.replace(entry) {
			self.back.push(current);
		}
		self.forward.clear();
	}

	pub fn back(&mut self) -> Option<HistoryEntry> {
		let entry = self.back.pop()?;
		if let Some(current) = self.current.replace(entry.clone()) {
			self.forward.push(current);
		}
		Some(entry)
	}

	pub fn forward(&mut self) -> Option<HistoryEntry> {
		let entry = self.forward.pop()?;
		if let Some(current) = self.current.replace(entry.clone()) {
			self.back.push(current);
		}
		Some(entry)
	}

	pub fn clear(&mut self) {
		self.back.clear();
		self.forward.clear();
		self.current = None;
	}

	pub fn status(&self) -> HistoryStatus {
		HistoryStatus {
			can_back: !self.back.is_empty(),
			can_forward: !self.forward.is_empty(),
		}
	}

	pub fn current(&self) -> Option<&HistoryEntry> {
		self.current.as_ref()
	}

	/// A linear oldest-to-newest view with the cursor at `index`.
	pub fn entries(&self) -> (Vec<HistoryEntry>, usize) {
		let mut entries: Vec<HistoryEntry> = self.back.iter().rev().cloned().collect();
		let index = entries.len();
		if let Some(current) = &self.current {
			entries.push(current.clone());
		}
		entries.extend(self.forward.iter().rev().cloned());
		(entries, index)
	}

	pub fn restore(&mut self, entries: Vec<HistoryEntry>, index: usize) {
		self.clear();
		let index = index.min(entries.len());
		self.back = entries[..index].iter().rev().cloned().collect();
		self.current = entries.get(index).cloned();
		self.forward = entries[index.saturating_add(1)..]
			.iter()
			.rev()
			.cloned()
			.collect();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn entry(path: &str) -> HistoryEntry {
		HistoryEntry {
			pane: PaneId(0),
			path: PathBuf::from(path),
			focus: None,
		}
	}

	#[test]
	fn back_and_forward_walk_the_recorded_entries() {
		let mut history = History::default();
		history.record(entry("/a"));
		history.record(entry("/b"));
		assert!(history.status().can_back);
		assert_eq!(history.back().unwrap().path, PathBuf::from("/a"));
		assert!(history.status().can_forward);
		assert_eq!(history.forward().unwrap().path, PathBuf::from("/b"));
	}

	#[test]
	fn a_new_navigation_drops_forward_entries() {
		let mut history = History::default();
		history.record(entry("/a"));
		history.record(entry("/b"));
		history.back();
		history.record(entry("/c"));
		assert!(!history.status().can_forward);
	}

	#[test]
	fn entries_round_trip() {
		let mut history = History::default();
		for path in ["/a", "/b", "/c"] {
			history.record(entry(path));
		}
		history.back();
		let (entries, index) = history.entries();
		assert_eq!(index, 1);
		let mut restored = History::default();
		restored.restore(entries, index);
		assert_eq!(restored.current().unwrap().path, PathBuf::from("/b"));
		assert!(restored.status().can_back);
		assert!(restored.status().can_forward);
	}
}

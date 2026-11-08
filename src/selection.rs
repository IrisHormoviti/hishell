//! Folder selection state.

use std::collections::HashSet;

use crate::item::ItemId;

#[derive(Clone, Debug, Default)]
pub struct Selection {
	active: bool,
	selected: HashSet<ItemId>,
	order: Vec<ItemId>,
	anchor: Option<ItemId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionStatus {
	pub active: bool,
	pub count: usize,
	pub items: Vec<ItemId>,
}

impl Selection {
	pub fn active(&self) -> bool {
		self.active
	}

	pub fn count(&self) -> usize {
		self.order.len()
	}

	pub fn is_empty(&self) -> bool {
		self.order.is_empty()
	}

	pub fn contains(&self, id: ItemId) -> bool {
		self.selected.contains(&id)
	}

	pub fn items(&self) -> &[ItemId] {
		&self.order
	}

	pub fn status(&self) -> SelectionStatus {
		SelectionStatus {
			active: self.active,
			count: self.order.len(),
			items: self.order.clone(),
		}
	}

	pub fn set_active(&mut self, active: bool) {
		self.active = active;
	}

	pub fn clear(&mut self) {
		self.active = false;
		self.selected.clear();
		self.order.clear();
		self.anchor = None;
	}

	pub fn select_only(&mut self, id: ItemId) {
		self.active = true;
		self.selected.clear();
		self.selected.insert(id);
		self.order = vec![id];
		self.anchor = Some(id);
	}

	pub fn toggle(&mut self, id: ItemId) {
		self.active = true;
		if self.selected.remove(&id) {
			self.order.retain(|current| *current != id);
			if self.count() == 0 {
				self.active = false;
			}
		} else {
			self.selected.insert(id);
			self.order.push(id);
		}
		self.anchor = Some(id);
	}

	pub fn range_to(&mut self, visible: &[ItemId], id: ItemId) {
		let Some(target) = visible.iter().position(|current| *current == id) else {
			return;
		};
		let anchor = self
			.anchor
			.and_then(|anchor| visible.iter().position(|current| *current == anchor))
			.unwrap_or(target);
		let (start, end) = if anchor <= target {
			(anchor, target)
		} else {
			(target, anchor)
		};
		self.active = true;
		self.selected.clear();
		self.order.clear();
		for item in &visible[start..=end] {
			self.selected.insert(*item);
			self.order.push(*item);
		}
	}

	pub fn select_all(&mut self, visible: &[ItemId]) {
		self.active = !visible.is_empty();
		self.selected = visible.iter().copied().collect();
		self.order = visible.to_vec();
		self.anchor = visible.last().copied();
	}

	/// Drop ids that are no longer part of the folder.
	pub fn retain(&mut self, visible: &[ItemId]) {
		self.selected.retain(|id| visible.contains(id));
		self.order.retain(|id| visible.contains(id));
		if let Some(anchor) = self.anchor
			&& !visible.contains(&anchor)
		{
			self.anchor = None;
		}
		if self.order.is_empty() {
			self.active = false;
		}
	}

	/// Reorder the selected ids to follow the visible order.
	pub fn reorder(&mut self, visible: &[ItemId]) {
		self.order
			.retain(|id| self.selected.contains(id) && visible.contains(id));
		for id in visible {
			if self.selected.contains(id) && !self.order.contains(id) {
				self.order.push(*id);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn ids(count: u64) -> Vec<ItemId> {
		(0..count).map(ItemId::new).collect()
	}

	#[test]
	fn toggle_enters_and_leaves_selection_mode() {
		let mut selection = Selection::default();
		selection.toggle(ItemId::new(2));
		assert!(selection.active());
		assert!(selection.contains(ItemId::new(2)));
		selection.toggle(ItemId::new(2));
		assert!(!selection.active());
		assert!(selection.is_empty());
	}

	#[test]
	fn range_selection_follows_visible_order() {
		let visible = ids(5);
		let mut selection = Selection::default();
		selection.select_only(visible[1]);
		selection.range_to(&visible, visible[3]);
		assert_eq!(selection.items(), &visible[1..=3]);
	}

	#[test]
	fn stale_ids_are_dropped() {
		let mut selection = Selection::default();
		selection.select_only(ItemId::new(1));
		selection.retain(&ids(1));
		assert!(selection.is_empty());
	}
}

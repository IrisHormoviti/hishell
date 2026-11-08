//! Semantic focus navigation.

use std::path::PathBuf;

use crate::item::ItemId;
use crate::view::PaneId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Direction {
	Left,
	Right,
	Up,
	Down,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FocusCommand {
	First,
	Last,
	Move(Direction),
	Activate,
	Set { pane: PaneId, path: PathBuf },
	Clear,
}

#[derive(Default)]
pub struct Focus {
	pane: PaneId,
	item: Option<ItemId>,
	columns: usize,
}

impl Focus {
	pub fn pane(&self) -> PaneId {
		self.pane
	}

	pub fn item(&self) -> Option<ItemId> {
		self.item
	}

	pub fn columns(&self) -> usize {
		self.columns
	}

	pub fn set_columns(&mut self, columns: usize) {
		self.columns = columns.max(1);
	}

	pub fn set(&mut self, pane: PaneId, item: Option<ItemId>) {
		self.pane = pane;
		self.item = item;
	}

	pub fn first(&mut self, order: &[ItemId]) {
		self.item = order.first().copied();
	}

	pub fn last(&mut self, order: &[ItemId]) {
		self.item = order.last().copied();
	}

	pub fn move_focus(&mut self, direction: Direction, order: &[ItemId]) {
		if order.is_empty() {
			self.item = None;
			return;
		}
		let Some(current) = self
			.item
			.and_then(|item| order.iter().position(|candidate| *candidate == item))
		else {
			self.item = order.first().copied();
			return;
		};
		let columns = self.columns.max(1);
		let next = match direction {
			Direction::Left => current.saturating_sub(1),
			Direction::Right => (current + 1).min(order.len() - 1),
			Direction::Up => current.saturating_sub(columns),
			Direction::Down => (current + columns).min(order.len() - 1),
		};
		self.item = order.get(next).copied();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn order() -> Vec<ItemId> {
		(0..6).map(ItemId::new).collect()
	}

	#[test]
	fn first_last_and_clear() {
		let mut focus = Focus::default();
		let order = order();
		focus.first(&order);
		assert_eq!(focus.item(), Some(ItemId::new(0)));
		focus.last(&order);
		assert_eq!(focus.item(), Some(ItemId::new(5)));
		focus.set(PaneId(2), None);
		assert_eq!(focus.pane(), PaneId(2));
		assert_eq!(focus.item(), None);
	}

	#[test]
	fn movement_follows_the_grid_columns() {
		let mut focus = Focus::default();
		focus.set_columns(3);
		focus.first(&order());
		focus.move_focus(Direction::Right, &order());
		assert_eq!(focus.item(), Some(ItemId::new(1)));
		focus.move_focus(Direction::Down, &order());
		assert_eq!(focus.item(), Some(ItemId::new(4)));
		focus.move_focus(Direction::Up, &order());
		assert_eq!(focus.item(), Some(ItemId::new(1)));
		focus.move_focus(Direction::Right, &order());
		focus.move_focus(Direction::Right, &order());
		assert_eq!(focus.item(), Some(ItemId::new(3)));
	}
}

//! Semantic focus navigation and pane state.

use std::collections::HashMap;
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

#[derive(Clone, Debug, PartialEq)]
pub enum FocusCommand {
	Register(PaneId),
	Release(PaneId),
	ActivatePane(PaneId),
	SetOrder {
		pane: PaneId,
		order: Vec<ItemId>,
		columns: usize,
		horizontal: bool,
	},
	SetPositions {
		pane: PaneId,
		positions: Vec<(i32, i32)>,
	},
	SetGrid {
		pane: PaneId,
		columns: i32,
		rows: i32,
	},
	SetGeometry {
		pane: PaneId,
		x: f64,
		y: f64,
		width: f64,
		height: f64,
	},
	SetEmptySlots {
		pane: PaneId,
		enabled: bool,
	},
	SetIndex {
		pane: PaneId,
		index: i32,
	},
	SetCursor {
		pane: PaneId,
		cursor: Option<(i32, i32)>,
	},
	First,
	Last,
	Move(Direction),
	Activate,
	Set {
		pane: PaneId,
		path: PathBuf,
	},
	Clear,
}

#[derive(Clone, Debug)]
pub struct PaneFocus {
	order: Vec<ItemId>,
	positions: Vec<(i32, i32)>,
	grid_columns: i32,
	grid_rows: i32,
	empty_slots: bool,
	index: i32,
	cursor: Option<(i32, i32)>,
	active: bool,
	columns: usize,
	horizontal: bool,
	x: f64,
	y: f64,
	width: f64,
	height: f64,
}

impl Default for PaneFocus {
	fn default() -> Self {
		Self {
			order: Vec::new(),
			positions: Vec::new(),
			grid_columns: 1,
			grid_rows: 1,
			empty_slots: false,
			index: -1,
			cursor: None,
			active: false,
			columns: 1,
			horizontal: false,
			x: 0.0,
			y: 0.0,
			width: 0.0,
			height: 0.0,
		}
	}
}

impl PaneFocus {
	fn count(&self) -> usize {
		self.order.len()
	}

	fn has_positions(&self) -> bool {
		!self.positions.is_empty() && self.positions.len() == self.order.len()
	}

	fn item_at_cell(&self, column: i32, row: i32) -> i32 {
		self.positions
			.iter()
			.position(|&(c, r)| c == column && r == row)
			.map_or(-1, |index| index as i32)
	}

	fn cursor_cell(&self) -> (i32, i32) {
		if let Some(cell) = self.cursor {
			return cell;
		}
		if self.has_positions() {
			let index = self.index.max(0) as usize;
			return self.positions.get(index).copied().unwrap_or((0, 0));
		}
		(0, 0)
	}

	fn on_empty_slot(&self) -> bool {
		self.has_positions() && self.empty_slots && self.index < 0 && self.cursor.is_some()
	}

	fn place_cursor(&mut self, column: i32, row: i32) {
		self.cursor = Some((column, row));
		self.index = self.item_at_cell(column, row);
		self.active = true;
	}

	fn sync_cursor_to_index(&mut self) {
		if self.has_positions()
			&& self.index >= 0
			&& let Some(cell) = self.positions.get(self.index as usize)
		{
			self.cursor = Some(*cell);
		}
	}

	fn rows(&self) -> i32 {
		let count = self.count() as i32;
		if count == 0 {
			return 1;
		}
		if self.has_positions() {
			return self
				.positions
				.iter()
				.map(|(_, row)| row + 1)
				.max()
				.unwrap_or(1)
				.max(1);
		}
		let stride = self.columns.max(1) as i32;
		if self.horizontal {
			stride
		} else {
			(count + stride - 1) / stride
		}
	}

	fn grid_columns(&self) -> i32 {
		let count = self.count() as i32;
		if count == 0 {
			return 1;
		}
		if self.has_positions() {
			return self
				.positions
				.iter()
				.map(|(column, _)| column + 1)
				.max()
				.unwrap_or(1)
				.max(1);
		}
		let stride = self.columns.max(1) as i32;
		if self.horizontal {
			(count + stride - 1) / stride
		} else {
			stride
		}
	}

	fn row_column(&self) -> (i32, i32) {
		let count = self.count() as i32;
		if count == 0 {
			return (0, 0);
		}
		if self.has_positions() {
			if let Some((column, row)) = self.cursor
				&& self.index < 0
			{
				return (row, column);
			}
			let index = self.index.clamp(0, count - 1);
			let (column, row) = self.positions[index as usize];
			return (row, column);
		}
		let stride = self.columns.max(1) as i32;
		let index = self.index.clamp(0, count - 1);
		if self.horizontal {
			(index % stride, index / stride)
		} else {
			(index / stride, index % stride)
		}
	}

	fn index_at(&self, row: i32, column: i32) -> i32 {
		let count = self.count() as i32;
		if count == 0 {
			return -1;
		}
		if self.has_positions() {
			let mut best = 0;
			let mut best_distance = i64::MAX;
			for (index, &(c, r)) in self.positions.iter().enumerate() {
				let distance = (r - row).abs() as i64 + (c - column).abs() as i64;
				if distance < best_distance {
					best_distance = distance;
					best = index as i32;
				}
			}
			return best;
		}
		let row = row.clamp(0, self.rows() - 1);
		let column = column.clamp(0, self.grid_columns() - 1);
		let stride = self.columns.max(1) as i32;
		let index = if self.horizontal {
			column * stride + row
		} else {
			row * stride + column
		};
		index.clamp(0, count - 1)
	}

	fn set_index(&mut self, index: i32) -> bool {
		let count = self.count() as i32;
		if count == 0 {
			return false;
		}
		let index = index.clamp(0, count - 1);
		let changed = index != self.index || !self.active;
		self.index = index;
		self.active = true;
		self.sync_cursor_to_index();
		changed
	}

	fn first(&mut self) -> bool {
		let index = if self.has_positions() {
			visual_edge(&self.positions, true).map(|index| index as i32)
		} else if self.count() > 0 {
			Some(0)
		} else {
			None
		};
		index.is_some_and(|index| self.set_index(index))
	}

	fn last(&mut self) -> bool {
		let index = if self.has_positions() {
			visual_edge(&self.positions, false).map(|index| index as i32)
		} else if self.count() > 0 {
			Some(self.count() as i32 - 1)
		} else {
			None
		};
		index.is_some_and(|index| self.set_index(index))
	}

	fn move_focus(&mut self, direction: Direction) -> bool {
		let count = self.count();
		if count == 0 {
			return false;
		}
		if !self.active {
			if self.on_empty_slot() {
				let (column, row) = self.cursor_cell();
				self.place_cursor(column, row);
				return true;
			}
			let index = self.index.max(0);
			return self.set_index(index);
		}

		let stride = self.columns.max(1);
		let index = self.index.clamp(0, count as i32 - 1);

		let free_empty = self.has_positions() && self.empty_slots;
		if free_empty {
			let (column, row) = self.cursor_cell();
			let (next_column, next_row) = match direction {
				Direction::Right => (column + 1, row),
				Direction::Left => (column - 1, row),
				Direction::Down => (column, row + 1),
				Direction::Up => (column, row - 1),
			};
			if next_column >= 0
				&& next_row >= 0
				&& next_column < self.grid_columns
				&& next_row < self.grid_rows
				&& (next_column, next_row) != (column, row)
			{
				self.place_cursor(next_column, next_row);
				return true;
			}
		}

		let target = if free_empty {
			index
		} else if self.has_positions() {
			nearest_in_direction(&self.positions, index as usize, direction)
				.map_or(index, |target| target as i32)
		} else if self.horizontal {
			let row = index % stride as i32;
			let column = index / stride as i32;
			match direction {
				Direction::Down => {
					if row + 1 < stride as i32 && index + 1 < count as i32 {
						index + 1
					} else {
						index
					}
				}
				Direction::Up => {
					if row > 0 {
						index - 1
					} else {
						index
					}
				}
				Direction::Right => {
					if (index + stride as i32) < count as i32 {
						index + stride as i32
					} else {
						index
					}
				}
				Direction::Left => {
					if column > 0 {
						index - stride as i32
					} else {
						index
					}
				}
			}
		} else {
			let row = index / stride as i32;
			let column = index % stride as i32;
			match direction {
				Direction::Right => {
					if column + 1 < stride as i32 && index + 1 < count as i32 {
						index + 1
					} else {
						index
					}
				}
				Direction::Left => {
					if column > 0 {
						index - 1
					} else {
						index
					}
				}
				Direction::Down => {
					if (index + stride as i32) < count as i32 {
						index + stride as i32
					} else {
						index
					}
				}
				Direction::Up => {
					if row > 0 {
						index - stride as i32
					} else {
						index
					}
				}
			}
		};

		if target != index {
			self.index = target;
			self.sync_cursor_to_index();
			return true;
		}
		false
	}
}

#[derive(Clone, Debug)]
pub struct Focus {
	panes: HashMap<PaneId, PaneFocus>,
	active: PaneId,
}

impl Default for Focus {
	fn default() -> Self {
		let active = PaneId(0);
		let mut panes = HashMap::new();
		panes.insert(active, PaneFocus::default());
		Self { panes, active }
	}
}

impl Focus {
	pub fn pane(&self) -> PaneId {
		self.active
	}

	pub fn item(&self) -> Option<ItemId> {
		let pane = self.panes.get(&self.active)?;
		if pane.index < 0 {
			return None;
		}
		pane.order.get(pane.index as usize).copied()
	}

	pub fn index(&self) -> i32 {
		self.panes
			.get(&self.active)
			.map(|pane| pane.index)
			.unwrap_or(-1)
	}

	pub fn columns(&self) -> usize {
		self.panes
			.get(&self.active)
			.map(|pane| pane.columns.max(1))
			.unwrap_or(1)
	}

	pub fn cursor(&self) -> Option<(i32, i32)> {
		self.panes.get(&self.active).and_then(|pane| pane.cursor)
	}

	pub fn on_empty_slot(&self) -> bool {
		self.panes
			.get(&self.active)
			.is_some_and(PaneFocus::on_empty_slot)
	}

	pub fn index_at(&self, row: i32, column: i32) -> i32 {
		self.panes
			.get(&self.active)
			.map(|pane| pane.index_at(row, column))
			.unwrap_or(-1)
	}

	pub fn pane_index(&self, pane: PaneId) -> i32 {
		self.panes.get(&pane).map(|pane| pane.index).unwrap_or(-1)
	}

	pub fn pane_cursor(&self, pane: PaneId) -> Option<(i32, i32)> {
		self.panes.get(&pane).and_then(|pane| pane.cursor)
	}

	pub fn pane_columns(&self, pane: PaneId) -> usize {
		self.panes
			.get(&pane)
			.map(|pane| pane.columns.max(1))
			.unwrap_or(1)
	}

	pub fn row_column(&self, pane: PaneId) -> (i32, i32) {
		self.panes
			.get(&pane)
			.map(PaneFocus::row_column)
			.unwrap_or((0, 0))
	}

	pub fn pane_grid_columns(&self, pane: PaneId) -> i32 {
		self.panes
			.get(&pane)
			.map(PaneFocus::grid_columns)
			.unwrap_or(1)
	}

	pub fn pane_rows(&self, pane: PaneId) -> i32 {
		self.panes.get(&pane).map(PaneFocus::rows).unwrap_or(1)
	}

	pub fn pane_index_at(&self, pane: PaneId, row: i32, column: i32) -> i32 {
		self.panes
			.get(&pane)
			.map(|pane| pane.index_at(row, column))
			.unwrap_or(-1)
	}

	/// Move the focus into the pane on the requested side when the cursor
	/// cannot advance inside its own pane. Geometry is the adapter's layout,
	/// so the core only compares the rectangles it was given.
	fn handoff(&mut self, from: PaneId, direction: Direction) -> bool {
		let Some(pane) = self.panes.get(&from).cloned() else {
			return false;
		};
		if pane.count() == 0 {
			return false;
		}
		let overlaps = |a: f64, a_end: f64, b: f64, b_end: f64| a < b_end - 1.0 && b < a_end - 1.0;
		let from_x_end = pane.x + pane.width;
		let from_y_end = pane.y + pane.height;
		let mut best: Option<(f64, PaneId)> = None;
		for (&id, candidate) in &self.panes {
			if id == from || candidate.count() == 0 {
				continue;
			}
			let candidate_x_end = candidate.x + candidate.width;
			let candidate_y_end = candidate.y + candidate.height;
			let (valid, distance) = match direction {
				Direction::Right => (
					candidate.x >= from_x_end - 1.0
						&& overlaps(pane.y, from_y_end, candidate.y, candidate_y_end),
					candidate.x - from_x_end,
				),
				Direction::Left => (
					candidate_x_end <= pane.x + 1.0
						&& overlaps(pane.y, from_y_end, candidate.y, candidate_y_end),
					pane.x - candidate_x_end,
				),
				Direction::Down => (
					candidate.y >= from_y_end - 1.0
						&& overlaps(pane.x, from_x_end, candidate.x, candidate_x_end),
					candidate.y - from_y_end,
				),
				Direction::Up => (
					candidate_y_end <= pane.y + 1.0
						&& overlaps(pane.x, from_x_end, candidate.x, candidate_x_end),
					pane.y - candidate_y_end,
				),
			};
			if valid && best.is_none_or(|(best_distance, _)| distance < best_distance) {
				best = Some((distance, id));
			}
		}
		let Some((_, neighbour_id)) = best else {
			return false;
		};
		let (row, column) = pane.row_column();
		let Some(neighbour) = self.panes.get_mut(&neighbour_id) else {
			return false;
		};
		let entry = match direction {
			Direction::Right => neighbour.index_at(row, 0),
			Direction::Left => neighbour.index_at(row, neighbour.grid_columns() - 1),
			Direction::Down => neighbour.index_at(0, column),
			Direction::Up => neighbour.index_at(neighbour.rows() - 1, column),
		};
		neighbour.index = entry.max(0);
		neighbour.active = true;
		neighbour.sync_cursor_to_index();
		self.active = neighbour_id;
		true
	}

	pub fn apply(&mut self, command: FocusCommand) -> bool {
		match command {
			FocusCommand::Register(pane) => {
				self.panes.entry(pane).or_default();
				true
			}
			FocusCommand::Release(pane) => {
				if self.panes.remove(&pane).is_none() {
					return false;
				}
				if self.active == pane {
					self.active = self
						.panes
						.keys()
						.copied()
						.min_by_key(|pane| pane.0)
						.unwrap_or(PaneId(0));
					self.panes.entry(self.active).or_default();
				}
				true
			}
			FocusCommand::ActivatePane(pane) => {
				self.panes.entry(pane).or_default();
				if self.active == pane {
					return false;
				}
				self.active = pane;
				true
			}
			FocusCommand::SetOrder {
				pane,
				order,
				columns,
				horizontal,
			} => {
				let entry = self.panes.entry(pane).or_default();
				let previous = if entry.index >= 0 {
					entry.order.get(entry.index as usize).copied()
				} else {
					None
				};
				let was_active = entry.active;
				let was_empty = entry.index < 0 && entry.cursor.is_some();
				let empty = order.is_empty();
				entry.order = order;
				entry.columns = columns.max(1);
				entry.horizontal = horizontal;
				if empty {
					entry.active = false;
					entry.index = -1;
					entry.cursor = None;
					return true;
				}
				let count = entry.order.len() as i32;
				entry.index = match previous
					.and_then(|id| entry.order.iter().position(|candidate| *candidate == id))
				{
					Some(index) => index as i32,
					None if was_empty => -1,
					None => entry.index.clamp(0, count - 1),
				};
				entry.active = was_active;
				if entry.index >= 0 {
					entry.sync_cursor_to_index();
				}
				true
			}
			FocusCommand::SetPositions { pane, positions } => {
				let entry = self.panes.entry(pane).or_default();
				entry.positions = positions;
				if entry.index >= 0 {
					entry.sync_cursor_to_index();
				} else if let Some((column, row)) = entry.cursor {
					entry.cursor = Some((
						column.clamp(0, (entry.grid_columns - 1).max(0)),
						row.clamp(0, (entry.grid_rows - 1).max(0)),
					));
				}
				true
			}
			FocusCommand::SetGrid {
				pane,
				columns,
				rows,
			} => {
				let entry = self.panes.entry(pane).or_default();
				entry.grid_columns = columns.max(1);
				entry.grid_rows = rows.max(1);
				if let Some((column, row)) = entry.cursor {
					entry.cursor = Some((
						column.min(entry.grid_columns - 1),
						row.min(entry.grid_rows - 1),
					));
				}
				true
			}
			FocusCommand::SetGeometry {
				pane,
				x,
				y,
				width,
				height,
			} => {
				let entry = self.panes.entry(pane).or_default();
				entry.x = x;
				entry.y = y;
				entry.width = width;
				entry.height = height;
				true
			}
			FocusCommand::SetEmptySlots { pane, enabled } => {
				let entry = self.panes.entry(pane).or_default();
				entry.empty_slots = enabled;
				if enabled {
					entry.sync_cursor_to_index();
				} else if entry.index < 0 {
					entry.index = if entry.count() > 0 { 0 } else { -1 };
					entry.sync_cursor_to_index();
				}
				true
			}
			FocusCommand::SetIndex { pane, index } => {
				let entry = self.panes.entry(pane).or_default();
				entry.set_index(index)
			}
			FocusCommand::SetCursor { pane, cursor } => {
				let entry = self.panes.entry(pane).or_default();
				match cursor {
					Some((column, row)) => entry.place_cursor(column, row),
					None => {
						entry.cursor = None;
						entry.index = -1;
						entry.active = false;
					}
				}
				true
			}
			FocusCommand::First => self.panes.entry(self.active).or_default().first(),
			FocusCommand::Last => self.panes.entry(self.active).or_default().last(),
			FocusCommand::Move(direction) => {
				let current = self.active;
				if self.panes.entry(current).or_default().move_focus(direction) {
					return true;
				}
				self.handoff(current, direction)
			}
			FocusCommand::Activate => false,
			FocusCommand::Set { pane, path } => {
				let entry = self.panes.entry(pane).or_default();
				let id = ItemId::for_path(&path);
				match entry.order.iter().position(|candidate| *candidate == id) {
					Some(index) => {
						self.active = pane;
						entry.set_index(index as i32)
					}
					None => false,
				}
			}
			FocusCommand::Clear => {
				let entry = self.panes.entry(self.active).or_default();
				let changed = entry.index >= 0 || entry.active || entry.cursor.is_some();
				entry.index = -1;
				entry.cursor = None;
				entry.active = false;
				changed
			}
		}
	}

	/// Set the active pane's order without disturbing its cursor.
	pub fn set_order(&mut self, pane: PaneId, order: Vec<ItemId>) {
		self.apply(FocusCommand::SetOrder {
			pane,
			order,
			columns: self.pane_columns(pane),
			horizontal: false,
		});
	}

	pub fn set_columns(&mut self, columns: usize) {
		let pane = self.active;
		if let Some(entry) = self.panes.get_mut(&pane) {
			entry.columns = columns.max(1);
		}
	}

	pub fn set(&mut self, pane: PaneId, item: Option<ItemId>) {
		self.apply(FocusCommand::ActivatePane(pane));
		let entry = self.panes.entry(pane).or_default();
		match item.and_then(|id| entry.order.iter().position(|candidate| *candidate == id)) {
			Some(index) => {
				entry.index = index as i32;
				entry.active = true;
				entry.sync_cursor_to_index();
			}
			None => {
				entry.index = -1;
				entry.active = false;
				entry.cursor = None;
			}
		}
	}

	pub fn first(&mut self, order: &[ItemId]) {
		self.apply(FocusCommand::SetOrder {
			pane: self.active,
			order: order.to_vec(),
			columns: self.columns(),
			horizontal: false,
		});
		self.apply(FocusCommand::First);
	}

	pub fn last(&mut self, order: &[ItemId]) {
		self.apply(FocusCommand::SetOrder {
			pane: self.active,
			order: order.to_vec(),
			columns: self.columns(),
			horizontal: false,
		});
		self.apply(FocusCommand::Last);
	}

	pub fn move_focus(&mut self, direction: Direction, order: &[ItemId]) {
		self.apply(FocusCommand::SetOrder {
			pane: self.active,
			order: order.to_vec(),
			columns: self.columns(),
			horizontal: false,
		});
		self.apply(FocusCommand::Move(direction));
	}
}

fn nearest_in_direction(
	positions: &[(i32, i32)],
	index: usize,
	direction: Direction,
) -> Option<usize> {
	let (column, row) = positions[index];
	let mut best: Option<(i64, usize)> = None;
	for (candidate, &(c, r)) in positions.iter().enumerate() {
		if candidate == index {
			continue;
		}
		let (forward, aside) = match direction {
			Direction::Right => (c - column, (r - row).abs()),
			Direction::Left => (column - c, (r - row).abs()),
			Direction::Down => (r - row, (c - column).abs()),
			Direction::Up => (row - r, (c - column).abs()),
		};
		if forward <= 0 {
			continue;
		}
		let score = forward as i64 + aside as i64 * 3;
		if best.is_none_or(|(best_score, _)| score < best_score) {
			best = Some((score, candidate));
		}
	}
	best.map(|(_, candidate)| candidate)
}

fn visual_edge(positions: &[(i32, i32)], first: bool) -> Option<usize> {
	let mut best: Option<(i32, i32, usize)> = None;
	for (index, &(column, row)) in positions.iter().enumerate() {
		let better = match best {
			None => true,
			Some((best_row, best_column, _)) => {
				if first {
					(row, column) < (best_row, best_column)
				} else {
					(row, column) > (best_row, best_column)
				}
			}
		};
		if better {
			best = Some((row, column, index));
		}
	}
	best.map(|(_, _, index)| index)
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
		assert_eq!(focus.item(), Some(ItemId::new(2)));
		focus.move_focus(Direction::Down, &order());
		assert_eq!(focus.item(), Some(ItemId::new(5)));
		focus.move_focus(Direction::Left, &order());
		assert_eq!(focus.item(), Some(ItemId::new(4)));
	}

	#[test]
	fn panes_keep_their_own_cursor_and_order() {
		let mut focus = Focus::default();
		let first = PaneId(0);
		let second = PaneId(1);
		let left = order();
		let right = (10..14).map(ItemId::new).collect::<Vec<_>>();
		focus.apply(FocusCommand::Register(second));
		focus.apply(FocusCommand::SetOrder {
			pane: first,
			order: left.clone(),
			columns: 3,
			horizontal: false,
		});
		focus.apply(FocusCommand::SetOrder {
			pane: second,
			order: right.clone(),
			columns: 2,
			horizontal: false,
		});
		focus.apply(FocusCommand::ActivatePane(first));
		focus.apply(FocusCommand::First);
		focus.apply(FocusCommand::Move(Direction::Down));
		assert_eq!(focus.pane(), first);
		assert_eq!(focus.item(), Some(ItemId::new(3)));

		focus.apply(FocusCommand::ActivatePane(second));
		assert_eq!(focus.pane(), second);
		focus.apply(FocusCommand::First);
		assert_eq!(focus.item(), Some(ItemId::new(10)));
		assert_eq!(focus.pane_index(first), 3);
	}

	#[test]
	fn moving_past_an_edge_hands_off_to_the_neighbour() {
		let mut focus = Focus::default();
		let left = PaneId(0);
		let right = PaneId(1);
		focus.apply(FocusCommand::Register(right));
		focus.apply(FocusCommand::SetOrder {
			pane: left,
			order: (0..3).map(ItemId::new).collect(),
			columns: 3,
			horizontal: false,
		});
		focus.apply(FocusCommand::SetOrder {
			pane: right,
			order: (10..13).map(ItemId::new).collect(),
			columns: 3,
			horizontal: false,
		});
		focus.apply(FocusCommand::SetGeometry {
			pane: left,
			x: 0.0,
			y: 0.0,
			width: 100.0,
			height: 100.0,
		});
		focus.apply(FocusCommand::SetGeometry {
			pane: right,
			x: 100.0,
			y: 0.0,
			width: 100.0,
			height: 100.0,
		});
		focus.apply(FocusCommand::ActivatePane(left));
		focus.apply(FocusCommand::Last);
		assert!(focus.apply(FocusCommand::Move(Direction::Right)));
		assert_eq!(focus.pane(), right);
		assert_eq!(focus.item(), Some(ItemId::new(10)));
	}

	#[test]
	fn free_placement_moves_by_cell_and_can_rest_on_a_gap() {
		let mut focus = Focus::default();
		let pane = PaneId(0);
		let order = (0..4).map(ItemId::new).collect::<Vec<_>>();
		focus.apply(FocusCommand::SetOrder {
			pane,
			order,
			columns: 3,
			horizontal: false,
		});
		focus.apply(FocusCommand::SetPositions {
			pane,
			positions: vec![(0, 0), (2, 0), (0, 1), (2, 1)],
		});
		focus.apply(FocusCommand::SetGrid {
			pane,
			columns: 3,
			rows: 2,
		});
		focus.apply(FocusCommand::SetEmptySlots {
			pane,
			enabled: true,
		});
		focus.apply(FocusCommand::SetIndex { pane, index: 0 });
		assert!(focus.apply(FocusCommand::Move(Direction::Right)));
		assert_eq!(focus.index(), -1);
		assert!(focus.apply(FocusCommand::Move(Direction::Left)));
		assert_eq!(focus.index(), 0);
		assert!(focus.apply(FocusCommand::Move(Direction::Down)));
		assert_eq!(focus.index(), 2);
	}
}

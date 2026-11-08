//! Slot and header placement for grid, list, grouped, and free layouts.

use crate::config::Config;
use crate::item::Item;
use crate::view::{FolderMode, GridSpec, HeaderSpec, Rect, SlotId, SlotSpec, ViewRequest};

#[derive(Clone, Debug, Default)]
pub struct FolderLayout {
	pub slots: Vec<SlotSpec>,
	pub headers: Vec<HeaderSpec>,
}

pub fn folder(
	config: &Config,
	request: &ViewRequest,
	mode: &FolderMode,
	items: &[Item],
) -> FolderLayout {
	match mode {
		FolderMode::Grid(grid) => grid_layout(config, request, grid, items),
		FolderMode::List(_) => list_layout(config, request, items),
	}
}

fn list_layout(config: &Config, request: &ViewRequest, items: &[Item]) -> FolderLayout {
	let height = request.metrics.label_height.max(16.0) + 12.0;
	let mut result = FolderLayout::default();

	if config.sort == crate::view::SortKey::Free {
		return result;
	}

	let grouped = config.group_mode != crate::view::GroupMode::None || config.show_stash;
	if !grouped {
		result.slots = items
			.iter()
			.enumerate()
			.map(|(index, item)| SlotSpec {
				id: SlotId(item.id.get()),
				item: item.id,
				cell: (0, index as u32),
				rect: Rect {
					x: 0.0,
					y: index as f64 * height,
					width: request.viewport.width,
					height,
				},
			})
			.collect();
		return result;
	}

	let header_height = request.metrics.label_height.max(16.0) + request.metrics.small_spacing;
	let mut y = 0.0;
	let mut row = 0u32;
	for (label, group_items) in runs(items) {
		if !label.is_empty() {
			result.headers.push(HeaderSpec {
				title: label,
				item: None,
				group: group_items.0,
				cell: (0, row),
				rect: Rect {
					x: 0.0,
					y,
					width: request.viewport.width,
					height: header_height,
				},
			});
			y += header_height + request.metrics.small_spacing;
			row += 1;
		}
		for item in group_items.1 {
			result.slots.push(SlotSpec {
				id: SlotId(item.id.get()),
				item: item.id,
				cell: (0, row),
				rect: Rect {
					x: 0.0,
					y,
					width: request.viewport.width,
					height,
				},
			});
			y += height;
			row += 1;
		}
	}
	result
}

fn grid_layout(
	config: &Config,
	request: &ViewRequest,
	grid: &GridSpec,
	items: &[Item],
) -> FolderLayout {
	if items.is_empty() {
		return FolderLayout::default();
	}
	if config.sort == crate::view::SortKey::Free {
		return free_layout(config, request, grid, items);
	}

	let grouped = config.group_mode != crate::view::GroupMode::None || config.show_stash;
	if !grouped {
		return plain_grid(grid, items);
	}

	let stride = i32::from(grid.lines.max(1));
	let gap = grid.gap;
	if grid.horizontal {
		grouped_horizontal(request, grid, items, stride, gap)
	} else {
		grouped_vertical(request, grid, items, stride, gap)
	}
}

fn plain_grid(grid: &GridSpec, items: &[Item]) -> FolderLayout {
	let lines = grid.lines.max(1) as usize;
	let columns = if grid.horizontal {
		items.len().div_ceil(lines)
	} else {
		lines
	};
	FolderLayout {
		slots: items
			.iter()
			.enumerate()
			.map(|(index, item)| {
				let (column, row) = if grid.horizontal {
					(index / lines, index % lines)
				} else {
					(index % columns, index / columns)
				};
				SlotSpec {
					id: SlotId(item.id.get()),
					item: item.id,
					cell: (column as u32, row as u32),
					rect: Rect {
						x: column as f64 * (grid.cell.width + grid.gap),
						y: row as f64 * (grid.cell.height + grid.gap),
						width: grid.cell.width,
						height: grid.cell.height,
					},
				}
			})
			.collect(),
		headers: Vec::new(),
	}
}

fn grouped_vertical(
	request: &ViewRequest,
	grid: &GridSpec,
	items: &[Item],
	stride: i32,
	gap: f64,
) -> FolderLayout {
	let header_height = request.metrics.label_height.max(16.0) + request.metrics.small_spacing;
	let natural_width = stride as f64 * grid.cell.width + (stride - 1) as f64 * gap;
	let mut result = FolderLayout::default();
	let mut y = 0.0;
	let mut row = 0u32;

	for (label, (group, group_items)) in runs(items) {
		if !label.is_empty() {
			result.headers.push(HeaderSpec {
				title: label,
				item: None,
				group,
				cell: (0, row),
				rect: Rect {
					x: 0.0,
					y,
					width: natural_width,
					height: header_height,
				},
			});
			y += header_height + gap;
			row += 1;
		}
		let rows = ceil_div(i32::try_from(group_items.len()).unwrap_or(i32::MAX), stride).max(1);
		for (offset, item) in group_items.iter().enumerate() {
			let column = offset as i32 % stride;
			let item_row = offset as i32 / stride;
			result.slots.push(SlotSpec {
				id: SlotId(item.id.get()),
				item: item.id,
				cell: (column as u32, row + item_row as u32),
				rect: Rect {
					x: column as f64 * (grid.cell.width + gap),
					y: y + item_row as f64 * (grid.cell.height + gap),
					width: grid.cell.width,
					height: grid.cell.height,
				},
			});
		}
		y += rows as f64 * grid.cell.height + (rows - 1) as f64 * gap + gap;
		row += rows as u32;
	}
	result
}

fn grouped_horizontal(
	request: &ViewRequest,
	grid: &GridSpec,
	items: &[Item],
	stride: i32,
	gap: f64,
) -> FolderLayout {
	let header_width = request.metrics.label_height.max(16.0) + request.metrics.small_spacing;
	let group_height = stride as f64 * grid.cell.height + (stride - 1) as f64 * gap;
	let mut result = FolderLayout::default();
	let mut x = 0.0;
	let mut column = 0u32;

	for (label, (group, group_items)) in runs(items) {
		let has_header = !label.is_empty();
		if has_header {
			result.headers.push(HeaderSpec {
				title: label,
				item: None,
				group,
				cell: (column, 0),
				rect: Rect {
					x,
					y: 0.0,
					width: header_width,
					height: group_height,
				},
			});
		}
		let item_columns =
			ceil_div(i32::try_from(group_items.len()).unwrap_or(i32::MAX), stride).max(1);
		let header_columns: i32 = if has_header { 1 } else { 0 };
		let items_x = x + header_columns as f64 * (header_width + gap);
		for (offset, item) in group_items.iter().enumerate() {
			let item_column = offset as i32 / stride;
			let row = offset as i32 % stride;
			result.slots.push(SlotSpec {
				id: SlotId(item.id.get()),
				item: item.id,
				cell: (
					column + header_columns as u32 + item_column as u32,
					row as u32,
				),
				rect: Rect {
					x: items_x + item_column as f64 * (grid.cell.width + gap),
					y: row as f64 * (grid.cell.height + gap),
					width: grid.cell.width,
					height: grid.cell.height,
				},
			});
		}
		let span = header_columns + item_columns;
		x += header_columns as f64 * header_width
			+ item_columns as f64 * grid.cell.width
			+ span as f64 * gap;
		column += span as u32;
	}
	result
}

fn free_layout(
	config: &Config,
	request: &ViewRequest,
	grid: &GridSpec,
	items: &[Item],
) -> FolderLayout {
	let positions = &config.free_placement_positions;
	let horizontal = grid.horizontal;
	let gap = grid.gap.max(0.0);
	let item_width = grid.cell.width.max(1.0);
	let item_height = grid.cell.height.max(1.0);
	let mut min_columns = 1;
	let mut min_rows = 1;
	for item in items {
		if let Some(&(column, row)) = positions.get(&item.name) {
			min_columns = min_columns.max(column + 1);
			min_rows = min_rows.max(row + 1);
		}
	}

	let cross_fill =
		|view: f64, item: f64| -> i32 { (((view + gap) / (item + gap)).floor() as i32).max(1) };
	let count = items.len() as i32;
	let (columns, rows) = if horizontal {
		let rows = grid.lines.max(1) as i32;
		let mut columns = ceil_div(count, rows).max(min_columns);
		columns = columns.max(cross_fill(request.viewport.width, item_width));
		(columns.max(1), rows)
	} else {
		let columns = (grid.lines.max(1) as i32).max(min_columns);
		let mut rows = ceil_div(count, columns).max(min_rows);
		rows = rows.max(cross_fill(request.viewport.height, item_height));
		(columns.max(1), rows.max(1))
	};

	let mut occupied = vec![vec![false; columns as usize]; rows as usize];
	let mut slots = vec![(0i32, 0i32); items.len()];

	for (index, item) in items.iter().enumerate() {
		let Some(&(mut column, mut row)) = positions.get(&item.name) else {
			continue;
		};
		column = column.clamp(0, columns - 1);
		row = row.clamp(0, rows - 1);
		if occupied[row as usize][column as usize] {
			let (next_column, next_row) = nearest_available(&occupied, columns, rows, column, row);
			column = next_column;
			row = next_row;
		}
		occupied[row as usize][column as usize] = true;
		slots[index] = (column, row);
	}

	for (index, item) in items.iter().enumerate() {
		if positions.contains_key(&item.name) {
			continue;
		}
		let (column, row) = first_available(&occupied, columns, rows, horizontal);
		occupied[row as usize][column as usize] = true;
		slots[index] = (column, row);
	}

	FolderLayout {
		slots: items
			.iter()
			.zip(slots)
			.map(|(item, (column, row))| SlotSpec {
				id: SlotId(item.id.get()),
				item: item.id,
				cell: (column as u32, row as u32),
				rect: Rect {
					x: column as f64 * (item_width + gap),
					y: row as f64 * (item_height + gap),
					width: item_width,
					height: item_height,
				},
			})
			.collect(),
		headers: Vec::new(),
	}
}

/// Contiguous runs in the already-grouped item order.
fn runs(items: &[Item]) -> Vec<(String, (u16, Vec<&Item>))> {
	let mut result: Vec<(String, (u16, Vec<&Item>))> = Vec::new();
	for item in items {
		match result.last_mut() {
			Some((_, (group, group_items))) if *group == item.group => {
				group_items.push(item);
			}
			_ => result.push((item.group_label.clone(), (item.group, vec![item]))),
		}
	}
	result
}

fn first_available(
	occupied: &[Vec<bool>],
	columns: i32,
	rows: i32,
	horizontal: bool,
) -> (i32, i32) {
	if horizontal {
		for column in 0..columns {
			for row in 0..rows {
				if !occupied[row as usize][column as usize] {
					return (column, row);
				}
			}
		}
	} else {
		for row in 0..rows {
			for column in 0..columns {
				if !occupied[row as usize][column as usize] {
					return (column, row);
				}
			}
		}
	}
	(columns - 1, rows - 1)
}

fn nearest_available(
	occupied: &[Vec<bool>],
	columns: i32,
	rows: i32,
	column: i32,
	row: i32,
) -> (i32, i32) {
	let limit = (columns + rows).max(1);
	for distance in 1..=limit {
		for delta_column in -distance..=distance {
			let delta_row = distance - delta_column.abs();
			for candidate_row in [row - delta_row, row + delta_row] {
				let candidate_column = column + delta_column;
				if candidate_column >= 0
					&& candidate_column < columns
					&& candidate_row >= 0
					&& candidate_row < rows
					&& !occupied[candidate_row as usize][candidate_column as usize]
				{
					return (candidate_column, candidate_row);
				}
			}
		}
	}
	first_available(occupied, columns, rows, false)
}

fn ceil_div(value: i32, divisor: i32) -> i32 {
	(value + divisor - 1) / divisor
}

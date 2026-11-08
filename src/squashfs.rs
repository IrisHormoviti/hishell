//! Minimal read-only SquashFS 4.0 reader, enough to pull small files out of an
//! AppImage (the embedded desktop entry and icon) without external tools.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAGIC: u32 = 0x7371_7368;
const SUPERBLOCK_SIZE: usize = 96;
const METADATA_SIZE: usize = 8192;
const FRAGMENT_ENTRY_SIZE: usize = 16;
const FRAGMENT_ENTRIES_PER_BLOCK: usize = METADATA_SIZE / FRAGMENT_ENTRY_SIZE;
const MAX_SYMLINK_DEPTH: usize = 8;
const MAX_BLOCK_COUNT: u64 = 1 << 20;
/// The AppImage runtime in front of the filesystem stays well below this.
const BASE_SCAN_LIMIT: u64 = 16 * 1024 * 1024;
/// Files larger than this are never read; the desktop entry and icons are small.
pub const MAX_READ_SIZE: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Compressor {
	Gzip,
	Zstd,
}

pub struct DirEntry {
	pub name: String,
	pub inode_ref: u64,
}

pub enum Inode {
	Dir {
		block_index: u64,
		size: u32,
		offset: u16,
	},
	File {
		blocks_start: u64,
		file_size: u64,
		frag_index: u32,
		frag_offset: u32,
		block_sizes: Vec<u32>,
	},
	Symlink(String),
	Other,
}

pub struct Squashfs {
	file: File,
	base: u64,
	block_size: u32,
	compressor: Compressor,
	root_inode: u64,
	inode_table: u64,
	dir_table: u64,
	fragment_table: u64,
	cache: HashMap<u64, Vec<u8>>,
}

impl Squashfs {
	/// Opens a filesystem that starts somewhere inside `path` (an AppImage is an
	/// ELF runtime followed by the image) and validates its superblock.
	pub fn open(path: &Path) -> Option<Squashfs> {
		let mut file = File::open(path).ok()?;
		let len = file.metadata().ok()?.len();
		let base = find_base(&mut file, len)?;

		let mut sb = [0u8; SUPERBLOCK_SIZE];
		file.seek(SeekFrom::Start(base)).ok()?;
		file.read_exact(&mut sb).ok()?;

		let block_size = u32_at(&sb, 12)?;
		let compressor = match u16_at(&sb, 20)? {
			1 => Compressor::Gzip,
			6 => Compressor::Zstd,
			_ => return None,
		};
		if u16_at(&sb, 28)? != 4 || u16_at(&sb, 30)? != 0 {
			return None;
		}
		if !(4096..=1_048_576).contains(&block_size) || !block_size.is_power_of_two() {
			return None;
		}
		let bytes_used = u64_at(&sb, 40)?;
		if base + bytes_used > len {
			return None;
		}

		Some(Squashfs {
			file,
			base,
			block_size,
			compressor,
			root_inode: u64_at(&sb, 32)?,
			inode_table: u64_at(&sb, 64)?,
			dir_table: u64_at(&sb, 72)?,
			fragment_table: u64_at(&sb, 80)?,
			cache: HashMap::new(),
		})
	}

	pub fn root_inode(&self) -> u64 {
		self.root_inode
	}

	pub fn read_dir(&mut self, dir_ref: u64) -> Option<Vec<DirEntry>> {
		let inode = self.read_inode(dir_ref)?;
		let Inode::Dir {
			block_index,
			size,
			offset,
		} = inode
		else {
			return None;
		};
		if size < 4 {
			return Some(Vec::new());
		}

		let loc = self.base + self.dir_table + block_index;
		let listing = self.read_metadata(loc, offset as usize, (size - 3) as usize)?;

		let mut entries = Vec::new();
		let mut pos = 0usize;
		while pos + 12 <= listing.len() {
			let count = u32_at(&listing, pos)? as usize + 1;
			let start = u32_at(&listing, pos + 4)? as u64;
			pos += 12;

			for _ in 0..count {
				if pos + 8 > listing.len() {
					return None;
				}
				let inode_offset = u16_at(&listing, pos)?;
				let name_size = u16_at(&listing, pos + 6)? as usize + 1;
				pos += 8;
				if pos + name_size > listing.len() {
					return None;
				}
				entries.push(DirEntry {
					name: String::from_utf8_lossy(&listing[pos..pos + name_size]).to_string(),
					inode_ref: (start << 16) | inode_offset as u64,
				});
				pos += name_size;
			}
		}

		Some(entries)
	}

	/// Resolves `path` relative to the filesystem root, following symlinks.
	pub fn lookup_path(&mut self, path: &str) -> Option<Inode> {
		let mut dir = self.root_inode;
		let mut parents = Vec::new();
		let mut pending: Vec<String> = path
			.split('/')
			.filter(|part| !part.is_empty() && *part != ".")
			.map(str::to_string)
			.rev()
			.collect();
		let mut depth = 0usize;
		let mut resolved = None;

		while let Some(component) = pending.pop() {
			if component == ".." {
				if let Some(parent) = parents.pop() {
					dir = parent;
				}
				resolved = None;
				continue;
			}

			let entry = self
				.read_dir(dir)?
				.into_iter()
				.find(|entry| entry.name == component)?;
			let inode = self.read_inode(entry.inode_ref)?;

			match inode {
				Inode::Symlink(target) => {
					depth += 1;
					if depth > MAX_SYMLINK_DEPTH {
						return None;
					}
					if target.starts_with('/') {
						dir = self.root_inode;
						parents.clear();
					}
					for part in target.split('/').rev() {
						if !part.is_empty() && part != "." {
							pending.push(part.to_string());
						}
					}
				}
				other => {
					if !pending.is_empty() {
						if !matches!(other, Inode::Dir { .. }) {
							return None;
						}
						parents.push(dir);
						dir = entry.inode_ref;
					}
					resolved = Some(other);
				}
			}
		}

		resolved
	}

	pub fn read_file(&mut self, inode: &Inode) -> Option<Vec<u8>> {
		let Inode::File {
			blocks_start,
			file_size,
			frag_index,
			frag_offset,
			block_sizes,
		} = inode
		else {
			return None;
		};
		if *file_size > MAX_READ_SIZE {
			return None;
		}
		let file_size = *file_size as usize;

		let mut out = Vec::with_capacity(file_size);
		let mut loc = self.base + blocks_start;
		for &size_field in block_sizes {
			let block = self.read_data_block(loc, size_field, self.block_size)?;
			loc += (size_field & 0x00ff_ffff) as u64;
			out.extend_from_slice(&block);
		}

		if *frag_index != u32::MAX {
			let tail = file_size.saturating_sub(out.len());
			if tail > 0 {
				let fragment = self.read_fragment(*frag_index)?;
				let start = *frag_offset as usize;
				let end = start.checked_add(tail)?;
				out.extend_from_slice(fragment.get(start..end)?);
			}
		}

		out.truncate(file_size);
		if out.len() < file_size {
			return None;
		}
		Some(out)
	}

	fn read_inode(&mut self, inode_ref: u64) -> Option<Inode> {
		let loc = self.base + self.inode_table + (inode_ref >> 16);
		let offset = (inode_ref & 0xffff) as usize;
		let header = self.read_metadata(loc, offset, 16)?;
		let inode_type = u16_at(&header, 0)?;
		let body_offset = offset + 16;

		match inode_type {
			// Basic directory.
			1 => {
				let body = self.read_metadata(loc, body_offset, 16)?;
				Some(Inode::Dir {
					block_index: u32_at(&body, 0)? as u64,
					size: u16_at(&body, 8)? as u32,
					offset: u16_at(&body, 10)?,
				})
			}
			// Extended directory.
			8 => {
				let body = self.read_metadata(loc, body_offset, 24)?;
				Some(Inode::Dir {
					block_index: u32_at(&body, 8)? as u64,
					size: u32_at(&body, 4)?,
					offset: u16_at(&body, 18)?,
				})
			}
			// Basic file.
			2 => {
				let body = self.read_metadata(loc, body_offset, 16)?;
				let file_size = u32_at(&body, 12)? as u64;
				let frag_index = u32_at(&body, 4)?;
				let count = block_count(file_size, frag_index, self.block_size)?;
				Some(Inode::File {
					blocks_start: u32_at(&body, 0)? as u64,
					file_size,
					frag_index,
					frag_offset: u32_at(&body, 8)?,
					block_sizes: self.read_block_sizes(loc, body_offset + 16, count)?,
				})
			}
			// Extended file.
			9 => {
				let body = self.read_metadata(loc, body_offset, 40)?;
				let file_size = u64_at(&body, 8)?;
				let frag_index = u32_at(&body, 24)?;
				let count = block_count(file_size, frag_index, self.block_size)?;
				Some(Inode::File {
					blocks_start: u64_at(&body, 0)?,
					file_size,
					frag_index,
					frag_offset: u32_at(&body, 28)?,
					block_sizes: self.read_block_sizes(loc, body_offset + 40, count)?,
				})
			}
			// Basic and extended symlinks.
			3 | 10 => {
				let body = self.read_metadata(loc, body_offset, 8)?;
				let target_size = u32_at(&body, 4)? as usize;
				if target_size == 0 || target_size > 4096 {
					return None;
				}
				let target = self.read_metadata(loc, body_offset + 8, target_size)?;
				Some(Inode::Symlink(String::from_utf8_lossy(&target).to_string()))
			}
			_ => Some(Inode::Other),
		}
	}

	fn read_block_sizes(&mut self, loc: u64, offset: usize, count: u64) -> Option<Vec<u32>> {
		if count == 0 {
			return Some(Vec::new());
		}
		if count > MAX_BLOCK_COUNT {
			return None;
		}
		let bytes = self.read_metadata(loc, offset, count as usize * 4)?;
		let mut sizes = Vec::with_capacity(count as usize);
		for i in 0..count as usize {
			sizes.push(u32_at(&bytes, i * 4)?);
		}
		Some(sizes)
	}

	fn read_fragment(&mut self, index: u32) -> Option<Vec<u8>> {
		let block = index as usize / FRAGMENT_ENTRIES_PER_BLOCK;
		let within = (index as usize % FRAGMENT_ENTRIES_PER_BLOCK) * FRAGMENT_ENTRY_SIZE;

		// The superblock points at the list of metadata block locations.
		self.file
			.seek(SeekFrom::Start(
				self.base + self.fragment_table + block as u64 * 8,
			))
			.ok()?;
		let mut location = [0u8; 8];
		self.file.read_exact(&mut location).ok()?;
		let location = u64::from_le_bytes(location);

		let entry = self.read_metadata(self.base + location, within, FRAGMENT_ENTRY_SIZE)?;
		let start = u64_at(&entry, 0)?;
		let size_field = u32_at(&entry, 8)?;
		self.read_data_block(self.base + start, size_field, self.block_size)
	}

	fn read_data_block(&mut self, loc: u64, size_field: u32, capacity: u32) -> Option<Vec<u8>> {
		// A size of zero denotes a sparse block.
		if size_field == 0 {
			return Some(vec![0u8; capacity as usize]);
		}
		let size = (size_field & 0x00ff_ffff) as usize;
		if size == 0 || size > capacity as usize {
			return None;
		}

		self.file.seek(SeekFrom::Start(loc)).ok()?;
		let mut raw = vec![0u8; size];
		self.file.read_exact(&mut raw).ok()?;

		if size_field & 0x0100_0000 != 0 {
			return Some(raw);
		}
		decompress(self.compressor, &raw, capacity as usize)
	}

	/// Reads from a metadata stream, which is a sequence of compressed 8 KiB
	/// blocks; records may span block boundaries.
	fn read_metadata(&mut self, mut loc: u64, mut offset: usize, len: usize) -> Option<Vec<u8>> {
		let mut out = Vec::with_capacity(len);
		while out.len() < len {
			let block = self.metadata_block(loc)?;
			if offset >= block.len() {
				return None;
			}
			let take = (len - out.len()).min(block.len() - offset);
			out.extend_from_slice(&block[offset..offset + take]);
			offset += take;
			if offset >= block.len() && out.len() < len {
				loc = self.next_metadata_location(loc)?;
				offset = 0;
			}
		}
		Some(out)
	}

	fn metadata_block(&mut self, loc: u64) -> Option<Vec<u8>> {
		if let Some(block) = self.cache.get(&loc) {
			return Some(block.clone());
		}

		self.file.seek(SeekFrom::Start(loc)).ok()?;
		let mut header = [0u8; 2];
		self.file.read_exact(&mut header).ok()?;
		let header = u16::from_le_bytes(header);
		let size = (header & 0x7fff) as usize;
		if size == 0 {
			return None;
		}

		let mut raw = vec![0u8; size];
		self.file.read_exact(&mut raw).ok()?;
		let block = if header & 0x8000 != 0 {
			raw
		} else {
			decompress(self.compressor, &raw, METADATA_SIZE)?
		};
		if block.len() > METADATA_SIZE {
			return None;
		}

		self.cache.insert(loc, block.clone());
		Some(block)
	}

	fn next_metadata_location(&mut self, loc: u64) -> Option<u64> {
		self.file.seek(SeekFrom::Start(loc)).ok()?;
		let mut header = [0u8; 2];
		self.file.read_exact(&mut header).ok()?;
		let size = (u16::from_le_bytes(header) & 0x7fff) as u64;
		Some(loc + 2 + size)
	}
}

fn block_count(file_size: u64, frag_index: u32, block_size: u32) -> Option<u64> {
	let block_size = block_size as u64;
	if frag_index == u32::MAX {
		Some(file_size.div_ceil(block_size))
	} else {
		Some(file_size / block_size)
	}
}

fn find_base(file: &mut File, len: u64) -> Option<u64> {
	const CHUNK: usize = 1 << 20;

	let limit = len.min(BASE_SCAN_LIMIT);
	let mut base = 0u64;
	let mut tail: Vec<u8> = Vec::new();
	let mut buf = vec![0u8; CHUNK];

	while base < limit {
		let want = (limit - base).min(CHUNK as u64) as usize;
		file.seek(SeekFrom::Start(base)).ok()?;
		file.read_exact(&mut buf[..want]).ok()?;

		let window_start = base - tail.len() as u64;
		let mut window = std::mem::take(&mut tail);
		window.extend_from_slice(&buf[..want]);

		let mut i = 0usize;
		while let Some(pos) = find_magic(&window[i..]) {
			let at = i + pos;
			if at + SUPERBLOCK_SIZE <= window.len()
				&& valid_superblock(&window[at..at + SUPERBLOCK_SIZE])
			{
				return Some(window_start + at as u64);
			}
			i = at + 1;
		}

		tail = window[window.len().saturating_sub(SUPERBLOCK_SIZE - 1)..].to_vec();
		base += want as u64;
	}

	None
}

fn find_magic(data: &[u8]) -> Option<usize> {
	data.windows(4).position(|window| window == b"hsqs")
}

fn valid_superblock(sb: &[u8]) -> bool {
	if u32_at(sb, 0) != Some(MAGIC) {
		return false;
	}
	if u16_at(sb, 28) != Some(4) || u16_at(sb, 30) != Some(0) {
		return false;
	}
	let Some(block_size) = u32_at(sb, 12) else {
		return false;
	};
	(4096..=1_048_576).contains(&block_size) && block_size.is_power_of_two()
}

fn decompress(compressor: Compressor, raw: &[u8], capacity: usize) -> Option<Vec<u8>> {
	match compressor {
		Compressor::Gzip => {
			let mut out = Vec::new();
			let decoder = flate2::read::ZlibDecoder::new(raw);
			decoder
				.take(capacity as u64 + 1)
				.read_to_end(&mut out)
				.ok()?;
			if out.len() > capacity {
				return None;
			}
			Some(out)
		}
		Compressor::Zstd => zstd::bulk::decompress(raw, capacity).ok(),
	}
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
	Some(u16::from_le_bytes(
		data.get(offset..offset + 2)?.try_into().ok()?,
	))
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
	Some(u32::from_le_bytes(
		data.get(offset..offset + 4)?.try_into().ok()?,
	))
}

fn u64_at(data: &[u8], offset: usize) -> Option<u64> {
	Some(u64::from_le_bytes(
		data.get(offset..offset + 8)?.try_into().ok()?,
	))
}

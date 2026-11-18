//! Event sounds, best effort.
//!
//! An event is a name from the Sound Naming Specification — `item-focus`,
//! `window-new`, `dialog-ok` — resolved the way the rest of the system
//! resolves sounds: against the XDG sound theme the session names in
//! `$XDG_CURRENT_DESKTOP/sounds/theme` under the configuration and data
//! directories, with the freedesktop theme as the fallback. The theme keeps a
//! PCM `.wav` copy of the names Hishell plays next to each `.oga`, because the
//! image has no Vorbis decoder for `pw-cat`.
//!
//! A player per event is not usable. Starting a PipeWire client costs about
//! 200 ms on this hardware and `pw-cat` asks for 100 ms of stream latency by
//! default, both far longer than a UI tick. One thread therefore keeps one
//! `pw-cat` stream open with the sound files already decoded, writes silence
//! into it between events and splices each event in as it is played.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The event a key plays while it is held down.
pub const KEY_PRESS: &str = "button-pressed";
/// The event a key plays when it is released.
pub const KEY_RELEASE: &str = "button-released";
/// The cursor moved onto another item.
pub const ITEM_FOCUS: &str = "item-focus";
/// An item was opened.
pub const ITEM_SELECTED: &str = "item-selected";
/// A folder opened.
pub const WINDOW_OPEN: &str = "window-new";
/// A folder closed.
pub const WINDOW_CLOSE: &str = "window-close";
/// A dialog or a menu appeared.
pub const DIALOG_OPEN: &str = "dialog-open";
/// A dialog was accepted.
pub const DIALOG_OK: &str = "dialog-ok";
/// A dialog was cancelled.
pub const DIALOG_CANCEL: &str = "dialog-cancel";
/// An action failed.
pub const DIALOG_ERROR: &str = "dialog-error";
/// A drag lifted an item.
pub const DRAG_START: &str = "drag-start";
/// A drag dropped onto a target that took it.
pub const DRAG_ACCEPT: &str = "drag-accept";
/// A drag dropped where nothing takes it.
pub const DRAG_FAIL: &str = "drag-fail";
/// One row of a scroll.
pub const SCROLL_TICK: &str = "scroll-down";
/// A scroll reached an end.
pub const SCROLL_EDGE: &str = "scroll-edge";

/// The theme ClamOS ships.
const CLAMOS_THEME: &str = "3ds";
/// The theme every session is expected to have; the spec's last fallback.
const FALLBACK_THEME: &str = "freedesktop";

/// How close together two plays of the same event may be. Holding a key and
/// scrolling repeat far faster than the ear can separate ticks; different
/// events are never merged, so a press and its response both play.
const MIN_INTERVAL: Duration = Duration::from_millis(100);

/// How close together two plays of any event may be, so a burst of events
/// cannot queue a stutter of sound behind the stream.
const MAX_BACKLOG: Duration = Duration::from_millis(400);

/// What the stream is opened as. The theme's PCM copies are built to match.
const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const BYTES_PER_FRAME: usize = CHANNELS as usize * 2;
/// Frames per write: short enough that a tick starts within one, long enough
/// that the writer is not a syscall per millisecond.
const CHUNK_FRAMES: usize = 480;
const CHUNK_BYTES: usize = CHUNK_FRAMES * BYTES_PER_FRAME;
/// Asked of PipeWire, whose default is 100 ms — longer than the tick itself.
const LATENCY: &str = "20ms";

/// Play a sound event without waiting for it.
pub fn play(event: &str) {
	player().dispatch(Message::Play(event.to_string()));
}

/// The same, behind the [`crate::SoundPlayer`] trait, so a frontend can hand
/// a player to an engine's platform.
pub struct Events;

impl crate::SoundPlayer for Events {
	fn play(&self, event: &str) {
		play(event);
	}

	fn set_volume(&self, _volume: f32) {}
}

/// Open the playback stream, if it is not open already.
///
/// A frontend calls this when its windows come up, so the stream has settled
/// before the first event arrives.
pub fn open() {
	player().dispatch(Message::Open);
}

/// Close the playback stream.
///
/// A silent playback stream is not free: it is a node in the audio graph and
/// costs CPU whether or not it plays anything.
pub fn close() {
	player().dispatch(Message::Close);
}

enum Message {
	Open,
	Close,
	Play(String),
}

/// The channel to the player thread, and the most recent event handed to it.
struct Player {
	sender: Sender<Message>,
	/// Events sent from the UI thread are throttled here: a `send` to a thread
	/// that is blocked in `write` cannot be taken back, so filtering after the
	/// hand-off would only queue work.
	last: Mutex<Vec<(String, Instant)>>,
}

impl Player {
	fn dispatch(&self, message: Message) {
		if let Message::Play(event) = &message
			&& !self.rate_ok(event)
		{
			return;
		}
		let _ = self.sender.send(message);
	}

	fn rate_ok(&self, event: &str) -> bool {
		let Ok(mut last) = self.last.lock() else {
			return true;
		};
		let now = Instant::now();
		if last
			.iter()
			.any(|(name, at)| name == event && now.duration_since(*at) < MIN_INTERVAL)
		{
			return false;
		}
		last.retain(|(_, at)| now.duration_since(*at) < MAX_BACKLOG);
		last.push((event.to_string(), now));
		true
	}
}

fn player() -> &'static Player {
	static PLAYER: OnceLock<Player> = OnceLock::new();
	PLAYER.get_or_init(|| {
		let (sender, receiver) = channel::<Message>();
		let _ = std::thread::Builder::new()
			.name("hishell-sound".into())
			.spawn(move || run(receiver));
		Player {
			sender,
			last: Mutex::new(Vec::new()),
		}
	})
}

/// One event waiting its turn: its PCM frames and how many bytes are played.
struct Clip {
	frames: Vec<u8>,
	offset: usize,
}

/// The player thread: one open stream, silence between events.
fn run(receiver: Receiver<Message>) {
	let mut stream: Option<Stream> = None;
	let mut waiting: Vec<Clip> = Vec::new();
	let mut playing: Option<Clip> = None;
	let mut chunk = vec![0u8; CHUNK_BYTES];

	loop {
		// Take everything that has arrived. With no stream to pace the loop,
		// block for the first message instead of spinning.
		let mut block = stream.is_none();
		loop {
			let message = if block {
				block = false;
				match receiver.recv() {
					Ok(message) => message,
					Err(_) => return,
				}
			} else {
				match receiver.try_recv() {
					Ok(message) => message,
					Err(_) => break,
				}
			};
			match message {
				Message::Open => {
					if stream.is_none() {
						stream = Stream::open();
					}
				}
				Message::Close => {
					stream = None;
					waiting.clear();
					playing = None;
				}
				Message::Play(event) => {
					if let Some(frames) = load(&event) {
						waiting.push(Clip { frames, offset: 0 });
					}
				}
			}
		}

		let Some(active) = stream.as_mut() else {
			continue;
		};

		let mut filled = 0;
		while filled < CHUNK_BYTES {
			let clip = match playing.as_mut() {
				Some(clip) => clip,
				None => match waiting.pop() {
					Some(clip) => playing.insert(clip),
					None => break,
				},
			};
			if clip.offset >= clip.frames.len() {
				playing = None;
				continue;
			}
			let take = (clip.frames.len() - clip.offset).min(CHUNK_BYTES - filled);
			chunk[filled..filled + take]
				.copy_from_slice(&clip.frames[clip.offset..clip.offset + take]);
			clip.offset += take;
			filled += take;
		}
		chunk[filled..].fill(0);

		// This blocks while PipeWire's buffer is full, which is what paces the
		// stream to the sample rate.
		if active.stdin.write_all(&chunk).is_err() {
			stream = None;
		}
	}
}

/// The playback stream, killed when it is dropped.
struct Stream {
	child: Child,
	stdin: ChildStdin,
}

impl Stream {
	fn open() -> Option<Self> {
		let mut child = Command::new("pw-cat")
			.args([
				"--playback",
				"--raw",
				"--rate",
				"48000",
				"--channels",
				"2",
				"--format",
				"s16",
				"--latency",
				LATENCY,
				"-",
			])
			.stdin(Stdio::piped())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.ok()?;
		let stdin = child.stdin.take()?;
		// The pipe is where an event waits when the thread is behind: a full
		// one would hold a third of a second of silence ahead of it.
		shrink_pipe(&stdin);
		Some(Self { child, stdin })
	}
}

/// One page is the smallest a pipe can be.
#[cfg(target_os = "linux")]
fn shrink_pipe(stdin: &ChildStdin) {
	use std::os::fd::AsRawFd;

	const F_SETPIPE_SZ: libc::c_int = 1031;
	const PAGE: libc::c_int = 4096;
	unsafe {
		libc::fcntl(stdin.as_raw_fd(), F_SETPIPE_SZ, PAGE);
	}
}

#[cfg(not(target_os = "linux"))]
fn shrink_pipe(_stdin: &ChildStdin) {}

impl Drop for Stream {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

/// The PCM frames of a sound event, if the theme has a usable copy of it.
fn load(event: &str) -> Option<Vec<u8>> {
	let path = sound_file(event)?;
	let bytes = std::fs::read(&path).ok()?;
	pcm(&bytes)
}

/// The data directories, `/usr/share` always last.
fn data_dirs() -> Vec<PathBuf> {
	let value = std::env::var("XDG_DATA_DIRS")
		.ok()
		.filter(|value| !value.is_empty())
		.unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
	let mut dirs: Vec<PathBuf> = value
		.split(':')
		.filter(|dir| !dir.is_empty())
		.map(PathBuf::from)
		.collect();
	if !dirs.iter().any(|dir| dir == Path::new("/usr/share")) {
		dirs.push(PathBuf::from("/usr/share"));
	}
	dirs
}

/// The configuration directories, where the desktop declaration lives.
fn config_dirs() -> Vec<PathBuf> {
	let value = std::env::var("XDG_CONFIG_DIRS")
		.ok()
		.filter(|value| !value.is_empty())
		.unwrap_or_else(|| "/etc/xdg".to_string());
	let mut dirs: Vec<PathBuf> = value
		.split(':')
		.filter(|dir| !dir.is_empty())
		.map(PathBuf::from)
		.collect();
	if !dirs.iter().any(|dir| dir == Path::new("/etc/xdg")) {
		dirs.push(PathBuf::from("/etc/xdg"));
	}
	dirs
}

/// The themes to try, most specific first: the one the desktop declares, then
/// the theme ClamOS ships, then the spec's fallback.
fn themes() -> Vec<String> {
	let mut themes: Vec<String> = Vec::new();
	if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
		for desktop in desktop.split(':').filter(|name| !name.is_empty()) {
			if let Some(theme) = declared_theme(desktop) {
				themes.push(theme);
			}
		}
	}
	for theme in [CLAMOS_THEME, FALLBACK_THEME] {
		if !themes.iter().any(|name| name == theme) {
			themes.push(theme.to_string());
		}
	}
	themes
}

/// The theme a desktop environment declares, from its `sounds/theme` file.
fn declared_theme(desktop: &str) -> Option<String> {
	let relative = Path::new(desktop).join("sounds").join("theme");
	for dir in config_dirs() {
		let Ok(text) = std::fs::read_to_string(dir.join(&relative)) else {
			continue;
		};
		let theme = text.trim();
		if !theme.is_empty() && !theme.contains('/') {
			return Some(theme.to_string());
		}
	}
	None
}

/// The file backing a sound event, in the first theme that has it.
fn sound_file(event: &str) -> Option<PathBuf> {
	// An event name is a file name; one carrying a separator is a mistake.
	if event.is_empty() || event.contains('/') {
		return None;
	}
	let dirs = data_dirs();
	let themes = themes();
	for dir in &dirs {
		for theme in &themes {
			for extension in ["wav", "oga"] {
				let path = dir
					.join("sounds")
					.join(theme)
					.join("stereo")
					.join(format!("{event}.{extension}"));
				if path.is_file() {
					return Some(path);
				}
			}
		}
	}
	None
}

/// The `data` chunk of a WAV, if it matches the stream's format.
fn pcm(bytes: &[u8]) -> Option<Vec<u8>> {
	if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
		return None;
	}
	let mut at = 12;
	while at + 8 <= bytes.len() {
		let id = &bytes[at..at + 4];
		let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
		let body = bytes.get(at + 8..at + 8 + size)?;
		if id == b"fmt " {
			if size < 16
				|| u16::from_le_bytes(body[0..2].try_into().ok()?) != 1
				|| u16::from_le_bytes(body[2..4].try_into().ok()?) != CHANNELS
				|| u32::from_le_bytes(body[4..8].try_into().ok()?) != RATE
				|| u16::from_le_bytes(body[14..16].try_into().ok()?) != 16
			{
				return None;
			}
		} else if id == b"data" {
			return Some(body.to_vec());
		}
		// Chunks are word aligned.
		at += 8 + size + (size & 1);
	}
	None
}

#[cfg(test)]
mod tests {
	use super::pcm;

	fn wav(format: &[u8], data: &[u8]) -> Vec<u8> {
		let mut out = Vec::new();
		out.extend_from_slice(b"RIFF");
		out.extend_from_slice(&0u32.to_le_bytes());
		out.extend_from_slice(b"WAVE");
		out.extend_from_slice(b"fmt ");
		out.extend_from_slice(&(format.len() as u32).to_le_bytes());
		out.extend_from_slice(format);
		out.extend_from_slice(b"data");
		out.extend_from_slice(&(data.len() as u32).to_le_bytes());
		out.extend_from_slice(data);
		out
	}

	fn format(channels: u16, rate: u32, bits: u16) -> Vec<u8> {
		let mut out = Vec::new();
		out.extend_from_slice(&1u16.to_le_bytes());
		out.extend_from_slice(&channels.to_le_bytes());
		out.extend_from_slice(&rate.to_le_bytes());
		out.extend_from_slice(&(rate * u32::from(channels) * u32::from(bits) / 8).to_le_bytes());
		out.extend_from_slice(&(channels * bits / 8).to_le_bytes());
		out.extend_from_slice(&bits.to_le_bytes());
		out
	}

	#[test]
	fn reads_the_samples_of_a_matching_wav() {
		let bytes = wav(&format(2, 48_000, 16), &[1, 2, 3, 4]);
		assert_eq!(pcm(&bytes), Some(vec![1, 2, 3, 4]));
	}

	#[test]
	fn skips_a_chunk_before_the_samples() {
		let mut format = format(2, 48_000, 16);
		format.extend_from_slice(b"LIST");
		format.extend_from_slice(&2u32.to_le_bytes());
		format.extend_from_slice(b"ab");
		let bytes = wav(&format, &[5, 6]);
		assert_eq!(pcm(&bytes), Some(vec![5, 6]));
	}

	#[test]
	fn refuses_a_different_rate() {
		let bytes = wav(&format(2, 44_100, 16), &[1, 2, 3, 4]);
		assert_eq!(pcm(&bytes), None);
	}

	#[test]
	fn refuses_a_different_channel_count() {
		let bytes = wav(&format(1, 48_000, 16), &[1, 2]);
		assert_eq!(pcm(&bytes), None);
	}

	#[test]
	fn refuses_a_non_pcm_file() {
		assert_eq!(pcm(b"OggS not a wav"), None);
	}
}

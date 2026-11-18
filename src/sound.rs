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
/// A panel or menu expanded.
pub const MENU_OPEN: &str = "menu-popup";
/// A panel or menu collapsed.
pub const MENU_CLOSE: &str = "menu-popdown";
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
/// A slider, or a segmented control, stepped to a new value.
pub const SLIDER_TICK: &str = "slider-tick";
/// One row of a vertical scroll, away from the start and back to it.
pub const SCROLL_DOWN: &str = "scroll-down";
pub const SCROLL_UP: &str = "scroll-up";
/// One column of a horizontal scroll, away from the start and back to it.
pub const SCROLL_LEFT: &str = "scroll-left";
pub const SCROLL_RIGHT: &str = "scroll-right";
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
	let player = player();
	// A stream that died — PipeWire restarted, or a spawn that failed while
	// the server was still coming up — is replaced before the event is handed
	// to it, so the event is late once rather than lost.
	player.dispatch(Message::Open);
	player.dispatch(Message::Play(event.to_string()));
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

/// Stop whatever is sounding and drop it.
///
/// A frontend calls this when the session comes back from something that
/// held it: the guest that owned the screens is gone, and a confirmation that
/// was half played must not resume as if the shell had never left.
pub fn flush() {
	player().dispatch(Message::Flush);
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
	Flush,
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

/// How many events may sound at once. A press, its release and the action
/// they started can overlap; past this many, the oldest is dropped.
const VOICES: usize = 8;

/// The player thread: one open stream, every event mixed in as it arrives.
///
/// Events are summed, not queued: a confirmation that lasts two seconds must
/// not hold back the tick after it, and a release that follows its press a few
/// microseconds later must not silence the action they started.
fn run(receiver: Receiver<Message>) {
	let mut stream: Option<Stream> = None;
	let mut active: Vec<Clip> = Vec::new();
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
					active.clear();
				}
				Message::Flush => {
					active.clear();
					// The stream already holds silence up to its latency; the
					// voices are what would otherwise resume.
				}
				Message::Play(event) => {
					if let Some(frames) = load(&event) {
						if active.len() >= VOICES {
							active.remove(0);
						}
						active.push(Clip { frames, offset: 0 });
					}
				}
			}
		}

		let Some(active_stream) = stream.as_mut() else {
			continue;
		};

		// Sum every voice into one chunk; what is left is silence.
		chunk.fill(0);
		let mut finished = 0;
		for clip in active.iter_mut() {
			let take = (clip.frames.len() - clip.offset).min(CHUNK_BYTES);
			mix(
				&mut chunk[..take],
				&clip.frames[clip.offset..clip.offset + take],
			);
			clip.offset += take;
			if clip.offset >= clip.frames.len() {
				finished += 1;
			}
		}
		if finished > 0 {
			active.retain(|clip| clip.offset < clip.frames.len());
		}

		// This blocks while PipeWire's buffer is full, which is what paces the
		// stream to the sample rate.
		if active_stream.stdin.write_all(&chunk).is_err() {
			stream = None;
		}
	}
}

/// Add stereo 16-bit frames into `into`, saturating instead of wrapping.
fn mix(into: &mut [u8], frames: &[u8]) {
	for (target, source) in into.chunks_exact_mut(2).zip(frames.chunks_exact(2)) {
		let a = i16::from_le_bytes([target[0], target[1]]) as i32;
		let b = i16::from_le_bytes([source[0], source[1]]) as i32;
		let sum = (a + b).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
		target.copy_from_slice(&sum.to_le_bytes());
	}
}

/// The playback stream, killed when it is dropped.
struct Stream {
	child: Child,
	stdin: ChildStdin,
}

impl Stream {
	fn open() -> Option<Self> {
		let mut child = match Command::new("pw-cat")
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
		{
			Ok(child) => child,
			Err(error) => {
				// A session without a sound server is not a failure worth
				// stopping for, but it should not be invisible either.
				eprintln!("hishell: cannot start pw-cat: {error}");
				return None;
			}
		};
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

	/// The theme's committed PCM twins must match the stream. A regenerated
	/// theme that kept the 3DS source rate would otherwise be silent.
	#[test]
	fn the_shipped_theme_has_a_twin_for_every_event_the_shell_plays() {
		let root =
			std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/sounds/3ds/stereo");
		let events = [
			"item-focus",
			"window-new",
			"window-close",
			"dialog-open",
			"dialog-ok",
			"dialog-cancel",
			"dialog-error",
			"drag-start",
			"drag-accept",
			"drag-fail",
			"scroll-down",
			"scroll-up",
			"scroll-left",
			"scroll-right",
			"scroll-edge",
			"slider-tick",
			"menu-popup",
			"menu-popdown",
			"button-pressed",
			"button-released",
		];
		for event in events {
			let path = root.join(format!("{event}.wav"));
			let bytes = std::fs::read(&path).unwrap_or_else(|error| {
				panic!("{}: {error}", path.display());
			});
			assert!(
				pcm(&bytes).is_some_and(|frames| !frames.is_empty()),
				"{} is not 48 kHz stereo 16-bit PCM",
				path.display()
			);
		}
	}
}

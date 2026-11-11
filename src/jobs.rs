//! Worker pool for file operations.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::ops::{JobOutcome, Op, OpKind, ProgressSink};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct JobId(pub u64);

#[derive(Clone, Debug)]
pub struct CancelToken {
	cancel: Arc<AtomicBool>,
	paused: Arc<AtomicBool>,
}

impl CancelToken {
	pub fn cancel(&self) {
		self.cancel.store(true, Ordering::Relaxed);
	}

	pub fn is_cancelled(&self) -> bool {
		self.cancel.load(Ordering::Relaxed)
	}

	pub fn pause(&self) {
		self.paused.store(true, Ordering::Relaxed);
	}

	pub fn resume(&self) {
		self.paused.store(false, Ordering::Relaxed);
	}

	pub fn is_paused(&self) -> bool {
		self.paused.load(Ordering::Relaxed)
	}

	pub(crate) fn cancel_flag(&self) -> &AtomicBool {
		&self.cancel
	}

	pub(crate) fn pause_flag(&self) -> &AtomicBool {
		&self.paused
	}
}

impl Default for CancelToken {
	fn default() -> Self {
		Self {
			cancel: Arc::new(AtomicBool::new(false)),
			paused: Arc::new(AtomicBool::new(false)),
		}
	}
}

#[derive(Clone, Debug)]
pub struct JobHandle {
	id: JobId,
	cancel: CancelToken,
}

impl JobHandle {
	pub fn id(&self) -> JobId {
		self.id
	}

	pub fn cancel(&self) {
		self.cancel.cancel();
	}

	pub fn pause(&self) {
		self.cancel.pause();
	}

	pub fn resume(&self) {
		self.cancel.resume();
	}

	pub fn is_paused(&self) -> bool {
		self.cancel.is_paused()
	}
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum JobEvent {
	Started {
		id: JobId,
		kind: OpKind,
	},
	Progress {
		id: JobId,
		done: u64,
		total: u64,
		current: std::path::PathBuf,
	},
	Finished {
		id: JobId,
		outcome: JobOutcome,
	},
	Cancelled {
		id: JobId,
	},
}

struct Job {
	id: JobId,
	op: Op,
	cancel: CancelToken,
}

/// Writes the destination's `[Progress]` metadata and mirrors the same values
/// to the frontend as `JobEvent::Progress`.
struct JobProgressSink {
	id: JobId,
	events: Sender<JobEvent>,
	trackers: HashMap<PathBuf, crate::progress::Tracker>,
	last_sent: HashMap<PathBuf, u64>,
	last_event: HashMap<PathBuf, Instant>,
}

impl JobProgressSink {
	fn new(id: JobId, events: Sender<JobEvent>) -> Self {
		Self {
			id,
			events,
			trackers: HashMap::new(),
			last_sent: HashMap::new(),
			last_event: HashMap::new(),
		}
	}
}

impl ProgressSink for JobProgressSink {
	fn begin(&mut self, path: &Path, is_dir: bool, task: &str, target: u64) {
		let tracker = crate::progress::Tracker::begin(path, is_dir, task, target);
		self.trackers.insert(path.to_path_buf(), tracker);
		self.last_sent.insert(path.to_path_buf(), 0);
		self.last_event.insert(path.to_path_buf(), Instant::now());
		let _ = self.events.send(JobEvent::Progress {
			id: self.id,
			done: 0,
			total: target,
			current: path.to_path_buf(),
		});
	}

	fn update(&mut self, path: &Path, value: u64, total: u64) {
		if let Some(tracker) = self.trackers.get_mut(path) {
			tracker.update(value);
		}
		let now = Instant::now();
		let last = self.last_sent.get(path).copied().unwrap_or(0);
		let step = (total / 100).max(1);
		let due = value >= total
			|| value.saturating_sub(last) >= step
			|| now
				.duration_since(self.last_event.get(path).copied().unwrap_or(now))
				>= std::time::Duration::from_millis(200);
		if !due {
			return;
		}
		self.last_sent.insert(path.to_path_buf(), value);
		self.last_event.insert(path.to_path_buf(), now);
		let _ = self.events.send(JobEvent::Progress {
			id: self.id,
			done: value,
			total,
			current: path.to_path_buf(),
		});
	}

	fn finish(&mut self, path: &Path) {
		self.trackers.remove(path);
		self.last_sent.remove(path);
		self.last_event.remove(path);
	}
}

pub struct Jobs {
	job_tx: Mutex<Option<Sender<Job>>>,
	events: Receiver<JobEvent>,
	workers: Mutex<Vec<JoinHandle<()>>>,
	/// Outstanding tokens, so shutdown can unblock a paused worker.
	tokens: Mutex<Vec<(JobId, CancelToken)>>,
	next_id: AtomicU64,
}

impl Jobs {
	pub fn new(workers: usize) -> Self {
		let (job_tx, job_rx) = unbounded::<Job>();
		let (event_tx, events) = unbounded::<JobEvent>();
		let mut handles = Vec::with_capacity(workers.max(1));
		for index in 0..workers.max(1) {
			let jobs = job_rx.clone();
			let events = event_tx.clone();
			let handle = std::thread::Builder::new()
				.name(format!("hishell-job-{index}"))
				.spawn(move || {
					while let Ok(job) = jobs.recv() {
						let _ = events.send(JobEvent::Started {
							id: job.id,
							kind: job.op.kind(),
						});
						if job.cancel.is_cancelled() {
							let _ = events.send(JobEvent::Cancelled { id: job.id });
							continue;
						}
						let mut progress = JobProgressSink::new(job.id, events.clone());
						let outcome = crate::ops::run_with(
							&job.op,
							job.cancel.cancel_flag(),
							Some(job.cancel.pause_flag()),
							&mut progress,
						);
						if outcome.cancelled {
							let _ = events.send(JobEvent::Cancelled { id: job.id });
						} else {
							let _ = events.send(JobEvent::Finished {
								id: job.id,
								outcome,
							});
						}
					}
				})
				.expect("spawn job worker");
			handles.push(handle);
		}

		Self {
			job_tx: Mutex::new(Some(job_tx)),
			events,
			workers: Mutex::new(handles),
			tokens: Mutex::new(Vec::new()),
			next_id: AtomicU64::new(1),
		}
	}

	pub fn submit(&self, op: Op) -> JobHandle {
		let id = JobId(self.next_id.fetch_add(1, Ordering::Relaxed));
		let cancel = CancelToken::default();
		let job = Job {
			id,
			op,
			cancel: cancel.clone(),
		};
		self.tokens
			.lock()
			.unwrap()
			.push((id, cancel.clone()));
		let sender = self.job_tx.lock().unwrap();
		if let Some(sender) = sender.as_ref() {
			let _ = sender.send(job);
		}
		JobHandle { id, cancel }
	}

	pub fn drain(&self) -> Vec<JobEvent> {
		let events: Vec<JobEvent> = self.events.try_iter().collect();
		let finished: HashSet<JobId> = events
			.iter()
			.filter_map(|event| match event {
				JobEvent::Finished { id, .. } | JobEvent::Cancelled { id } => Some(*id),
				_ => None,
			})
			.collect();
		if !finished.is_empty() {
			self.tokens
				.lock()
				.unwrap()
				.retain(|(id, _)| !finished.contains(id));
		}
		events
	}

	pub fn shutdown(&self) {
		// A paused worker waits for Resume or Stop. Closing the shell must not
		// block joining it, so every outstanding job is cancelled first.
		for (_, token) in self.tokens.lock().unwrap().iter() {
			token.cancel();
		}
		let mut sender = self.job_tx.lock().unwrap();
		sender.take();
		drop(sender);
		let mut workers = self.workers.lock().unwrap();
		for worker in workers.drain(..) {
			let _ = worker.join();
		}
	}
}

impl Default for Jobs {
	fn default() -> Self {
		let workers = std::thread::available_parallelism()
			.map(|count| count.get())
			.unwrap_or(1)
			.clamp(1, 4);
		Self::new(workers)
	}
}

impl Drop for Jobs {
	fn drop(&mut self) {
		self.shutdown();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ops::CopyOptions;
	use std::path::PathBuf;

	fn tree(name: &str) -> PathBuf {
		let root = std::env::temp_dir().join(format!("hishell-jobs-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		root
	}

	#[test]
	fn a_job_runs_and_reports_completion() {
		let root = tree("copy");
		let source = root.join("a.txt");
		std::fs::write(&source, b"x").unwrap();
		let dest = root.join("dest");
		std::fs::create_dir_all(&dest).unwrap();
		let jobs = Jobs::new(1);
		let handle = jobs.submit(Op::Copy {
			sources: vec![source],
			dest: dest.clone(),
			options: CopyOptions::default(),
		});
		let mut finished = None;
		for _ in 0..200 {
			for event in jobs.drain() {
				if let JobEvent::Finished { id, outcome } = event
					&& id == handle.id()
				{
					finished = Some(outcome);
				}
			}
			if finished.is_some() {
				break;
			}
			std::thread::sleep(std::time::Duration::from_millis(10));
		}
		let outcome = finished.expect("job finished");
		assert!(outcome.failed.is_empty());
		assert!(dest.join("a.txt").exists());
		jobs.shutdown();
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn shutdown_joins_idle_workers() {
		let jobs = Jobs::new(2);
		jobs.shutdown();
	}
}

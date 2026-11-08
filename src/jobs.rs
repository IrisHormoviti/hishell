//! Worker pool for file operations.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::ops::{JobOutcome, Op, OpKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JobId(pub u64);

#[derive(Clone, Debug)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
	pub fn cancel(&self) {
		self.0.store(true, Ordering::Relaxed);
	}

	pub fn is_cancelled(&self) -> bool {
		self.0.load(Ordering::Relaxed)
	}
}

impl Default for CancelToken {
	fn default() -> Self {
		Self(Arc::new(AtomicBool::new(false)))
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

pub struct Jobs {
	job_tx: Mutex<Option<Sender<Job>>>,
	events: Receiver<JobEvent>,
	workers: Mutex<Vec<JoinHandle<()>>>,
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
						let outcome = crate::ops::run(&job.op, &job.cancel.0);
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
		let sender = self.job_tx.lock().unwrap();
		if let Some(sender) = sender.as_ref() {
			let _ = sender.send(job);
		}
		JobHandle { id, cancel }
	}

	pub fn drain(&self) -> Vec<JobEvent> {
		self.events.try_iter().collect()
	}

	pub fn shutdown(&self) {
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

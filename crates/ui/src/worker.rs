use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

/// A handle that can cancel a job running on a [`JobQueue`].
///
/// Call [`Self::cancel`] from the main thread; the worker thread
/// checks the flag between work units and exits early when set.
#[derive(Clone)]
#[allow(dead_code)]
pub struct JobToken {
    cancelled: Arc<AtomicBool>,
}

#[allow(dead_code)]
impl JobToken {
    /// Signal the worker that this job should be abandoned.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Check whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// A single background thread for fire-and-forget batch work.
///
/// Jobs run sequentially. Tasks are free to use `rayon::par_iter`
/// internally for data parallelism — the [`JobQueue`] just provides
/// the off-main-thread context.
///
/// Every `std::thread::spawn` call for batch work should go through
/// this type (preview work uses [`PreviewService`](crate::latest::PreviewService)).
pub struct JobQueue {
    task_tx: Option<async_channel::Sender<Box<dyn FnOnce() + Send + 'static>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl JobQueue {
    pub fn new() -> Self {
        let (tx, rx) = async_channel::unbounded::<Box<dyn FnOnce() + Send + 'static>>();
        let handle = thread::spawn(move || {
            while let Ok(task) = rx.recv_blocking() {
                task();
            }
        });
        Self {
            task_tx: Some(tx),
            handle: Some(handle),
        }
    }

    /// Schedule a closure to run on the worker thread.
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        if let Some(tx) = &self.task_tx {
            let _ = tx.send_blocking(Box::new(f));
        }
    }

    /// Create a cancellation token that can be used to signal the
    /// in-flight job to stop early.
    #[allow(dead_code)]
    pub fn job_token(&self) -> JobToken {
        JobToken {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Drop for JobQueue {
    fn drop(&mut self) {
        drop(self.task_tx.take());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

use std::thread;

/// A single background thread for fire-and-forget batch work.
///
/// Jobs run sequentially. Tasks are free to use `rayon::par_iter`
/// internally for data parallelism — the [`JobQueue`] just provides
/// the off-main-thread context.
///
/// Every `std::thread::spawn` call for batch work should go through
/// this type (preview work uses [`PreviewService`](crate::background::preview_service::PreviewService)).
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
}

impl Drop for JobQueue {
    fn drop(&mut self) {
        drop(self.task_tx.take());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

use std::thread;

/// A single background thread that executes fire-and-forget closures
/// sequentially.  Tasks are free to use `rayon::par_iter` internally for
/// parallelism — the [`Worker`] just provides the off-main-thread context.
///
/// Every `std::thread::spawn` in the codebase should go through this type
/// instead of spawning ad-hoc threads.
pub struct Worker {
    task_tx: Option<async_channel::Sender<Box<dyn FnOnce() + Send + 'static>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Worker {
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

impl Drop for Worker {
    fn drop(&mut self) {
        // Dropping the sender closes the channel, causing recv_blocking to
        // return Err and the worker thread to exit.
        drop(self.task_tx.take());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

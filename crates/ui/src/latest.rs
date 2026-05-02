use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::thread;

/// A request sender that assigns monotonically increasing IDs for
/// latest-wins request patterns. Cloning shares the same ID counter
/// and channel.
pub struct RequestDedup<T> {
    id: Rc<Cell<u64>>,
    tx: async_channel::Sender<(u64, T)>,
}

impl<T> Clone for RequestDedup<T> {
    fn clone(&self) -> Self {
        Self {
            id: Rc::clone(&self.id),
            tx: self.tx.clone(),
        }
    }
}

impl<T> RequestDedup<T> {
    pub fn new(tx: async_channel::Sender<(u64, T)>) -> Self {
        Self {
            id: Rc::new(Cell::new(0)),
            tx,
        }
    }

    /// Bump the ID so all in-flight requests become stale.
    /// Use this when no request should be processed (e.g. no selection).
    pub fn send_dummy(&self) {
        self.id.set(self.id.get().wrapping_add(1));
    }

    /// Send a request with the next ID. Returns the assigned ID.
    pub fn send(&self, data: T) -> u64 {
        let id = self.id.get().wrapping_add(1);
        self.id.set(id);
        let _ = self.tx.try_send((id, data));
        id
    }

    /// Check whether a response ID matches the latest issued request.
    pub fn is_current(&self, id: u64) -> bool {
        id == self.id.get()
    }
}

/// Receive the latest request from a channel, draining stale ones.
/// Blocks until at least one request is available, then returns
/// the most recent (discarding superseded requests).
pub fn recv_latest<T>(rx: &async_channel::Receiver<(u64, T)>) -> Option<(u64, T)> {
    let first = rx.recv_blocking().ok()?;
    let mut latest = first;
    while let Ok(next) = rx.try_recv() {
        latest = next;
    }
    Some(latest)
}

/// A dedicated worker thread for latest-wins preview rendering.
///
/// Spawns one `std::thread` that drains a request channel and
/// sends results back on a response channel. The returned
/// [`RequestDedup`] is used by the main thread to issue requests,
/// and the [`Receiver`](async_channel::Receiver) delivers results
/// tagged with the request ID.
///
/// Dropping the service closes the request channel, which causes
/// the worker to exit (after `recv_latest` returns `None`).
/// `Drop` joins the worker thread, which may block briefly if a
/// render is in flight — bounded by one image render in practice.
pub struct PreviewService<Req, Res> {
    _req_tx: Option<async_channel::Sender<(u64, Req)>>,
    _handle: Option<thread::JoinHandle<()>>,
    _marker: PhantomData<Res>,
}

impl<Req: Send + 'static, Res: Send + 'static> PreviewService<Req, Res> {
    /// Create a new preview service.
    ///
    /// `process` is called on the worker thread for each request.
    /// It receives the request ID and payload; return `Some(Res)` to
    /// send a result tagged with the request ID, or `None` to skip.
    pub fn new<F>(
        process: F,
    ) -> (Self, RequestDedup<Req>, async_channel::Receiver<(u64, Res)>)
    where
        F: Fn(u64, Req) -> Option<Res> + Send + 'static,
    {
        let (req_tx, req_rx) = async_channel::unbounded::<(u64, Req)>();
        let (res_tx, res_rx) = async_channel::unbounded::<(u64, Res)>();
        let dedup = RequestDedup::new(req_tx.clone());
        let handle = thread::spawn(move || {
            while let Some((id, req)) = recv_latest(&req_rx) {
                if let Some(res) = process(id, req) {
                    let _ = res_tx.send_blocking((id, res));
                }
            }
        });
        (
            Self {
                _req_tx: Some(req_tx),
                _handle: Some(handle),
                _marker: PhantomData,
            },
            dedup,
            res_rx,
        )
    }
}

impl<Req, Res> Drop for PreviewService<Req, Res> {
    fn drop(&mut self) {
        drop(self._req_tx.take());
        if let Some(h) = self._handle.take() {
            let _ = h.join();
        }
    }
}

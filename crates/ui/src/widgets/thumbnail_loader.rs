use std::path::PathBuf;

use crate::pixbuf::PixelData;
use crate::types::PageId;

pub struct ThumbReq {
    pub id: PageId,
    pub path: PathBuf,
}

pub struct ThumbData {
    pub pixels: PixelData,
    /// Full image size in its EXIF-corrected frame.
    pub orig_width: u32,
    pub orig_height: u32,
}

impl ThumbData {
    pub fn to_pixbuf(&self) -> gtk::gdk_pixbuf::Pixbuf {
        self.pixels.to_pixbuf()
    }
}

pub enum LoadMsg {
    Progress(PageId, ThumbData),
    Finished,
}

/// Encapsulates the rayon thumbnail loader behind a simple `enqueue_batch` API.
/// Create with [`ThumbnailService::new`]; keep the service alive to keep the
/// worker thread alive.  Drop it to shut down the worker and close the result
/// channel.
pub struct ThumbnailService {
    job_queue: crate::worker::JobQueue,
    msg_tx: async_channel::Sender<LoadMsg>,
}

impl ThumbnailService {
    pub fn new() -> (Self, async_channel::Receiver<LoadMsg>) {
        let (msg_tx, msg_rx) = async_channel::unbounded();
        (
            Self {
                job_queue: crate::worker::JobQueue::new(),
                msg_tx,
            },
            msg_rx,
        )
    }

    pub fn enqueue_batch(&self, items: Vec<ThumbReq>) {
        if items.is_empty() {
            return;
        }
        let tx = self.msg_tx.clone();
        self.job_queue.spawn(move || {
            use rayon::prelude::*;
            items.into_par_iter().for_each(|req| {
                // User rotation is applied by the PageItem, so load unrotated.
                if let Some(pb) = crate::pixbuf::load_page(&req.path, 0, Some(256)) {
                    let (orig_width, orig_height) =
                        recto_core::io::oriented_dimensions(&req.path).unwrap_or((0, 0));
                    let data = ThumbData {
                        pixels: PixelData::from_pixbuf(&pb),
                        orig_width,
                        orig_height,
                    };
                    let _ = tx.send_blocking(LoadMsg::Progress(req.id, data));
                }
                let _ = tx.send_blocking(LoadMsg::Finished);
            });
        });
    }
}

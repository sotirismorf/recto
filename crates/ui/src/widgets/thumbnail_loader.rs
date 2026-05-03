use gtk::prelude::*;
use gtk::{gdk_pixbuf, glib};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::app::State;
use crate::types::PageId;

pub struct ThumbReq {
    pub id: PageId,
    pub path: PathBuf,
}

pub struct ThumbData {
    pub bytes: glib::Bytes,
    pub width: i32,
    pub height: i32,
    pub rowstride: i32,
    pub has_alpha: bool,
    pub orig_width: u32,
    pub orig_height: u32,
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
                let file_dims = gdk_pixbuf::Pixbuf::file_info(&req.path)
                    .map(|(_, w, h)| (w as u32, h as u32))
                    .unwrap_or((0, 0));
                if let Ok(pb) = gdk_pixbuf::Pixbuf::from_file_at_scale(&req.path, 256, 256, true) {
                    let pb = pb.apply_embedded_orientation().unwrap_or(pb);
                    let (orig_width, orig_height) =
                        exif_corrected_dims(file_dims, pb.width(), pb.height());
                    let bytes = pb.read_pixel_bytes();
                    let data = ThumbData {
                        bytes,
                        width: pb.width(),
                        height: pb.height(),
                        rowstride: pb.rowstride(),
                        has_alpha: pb.has_alpha(),
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

pub(crate) fn exif_corrected_dims(file_dims: (u32, u32), thumb_w: i32, thumb_h: i32) -> (u32, u32) {
    let (fw, fh) = file_dims;
    if fw == 0 || fh == 0 || thumb_w == 0 || thumb_h == 0 {
        return file_dims;
    }
    let (fw, fh) = (fw as f64, fh as f64);
    let (tw, th) = (thumb_w as f64, thumb_h as f64);
    if (tw * fh - th * fw).abs() <= (tw * fw - th * fh).abs() {
        (fw as u32, fh as u32)
    } else {
        (fh as u32, fw as u32)
    }
}

pub(crate) fn decrement_pending(counter: &Rc<Cell<usize>>, spinner: &gtk::Spinner) {
    let val = counter.get();
    if val > 0 {
        counter.set(val - 1);
    }
    if counter.get() == 0 {
        spinner.set_visible(false);
        spinner.stop();
    }
}

pub(crate) fn update_count(count: &gtk::Label, state: &State) {
    let n = state.project().pages.len();
    count.set_label(&format!("{} images", n));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exif_orientation_portrait() {
        let dims = exif_corrected_dims((6000, 4000), 256, 384);
        assert_eq!(dims, (4000, 6000));
    }

    #[test]
    fn exif_orientation_landscape() {
        let dims = exif_corrected_dims((6000, 4000), 384, 256);
        assert_eq!(dims, (6000, 4000));
    }

    #[test]
    fn exif_zero_dims_fallback() {
        let dims = exif_corrected_dims((0, 0), 256, 256);
        assert_eq!(dims, (0, 0));
    }

    #[test]
    fn exif_square_image() {
        let dims = exif_corrected_dims((4000, 4000), 256, 256);
        assert_eq!(dims, (4000, 4000));
    }
}

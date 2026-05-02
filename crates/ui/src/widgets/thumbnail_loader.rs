use gtk::prelude::*;
use gtk::glib;
use std::cell::Cell;
use std::rc::Rc;

use crate::app::State;
use crate::types::PageId;

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

pub(crate) fn exif_corrected_dims(
    file_dims: (u32, u32),
    thumb_w: i32,
    thumb_h: i32,
) -> (u32, u32) {
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

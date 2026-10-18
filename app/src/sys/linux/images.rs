//! Image previews (import.md#design) with gdk-pixbuf, which comes with
//! GTK: an image decoded, turned upright by its EXIF orientation, scaled
//! down to fit a box, and encoded as JPEG for the page, as WIC does on
//! Windows. gdk-pixbuf reads the formats it has loaders for (JPEG, PNG,
//! GIF, BMP, TIFF; WebP with its loader).

use std::path::Path;

use av_utils::error::{Error, Result};
use gtk::gdk_pixbuf::{InterpType, Pixbuf};
use gtk::glib;

use crate::sys::fit;

// ----------------------------------------------------------------------------
/// What a worker holds while it makes previews: nothing on Linux (COM on
/// Windows).
pub struct ImageThread;

impl ImageThread {
    // ------------------------------------------------------------------------
    pub fn new() -> Self {
        Self
    }
}

// ----------------------------------------------------------------------------
/// The image `path` as a JPEG of at most `max_w` x `max_h` pixels, upright
/// by its EXIF orientation, keeping its aspect; never scaled up.
pub fn preview(path: &Path, max_w: u32, max_h: u32) -> Result<Vec<u8>> {
    let pixbuf = Pixbuf::from_file(path).map_err(gtk_error)?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    let (width, height) = fit(pixbuf.width() as u32, pixbuf.height() as u32, max_w, max_h);
    let scaled = pixbuf
        .scale_simple(width as i32, height as i32, InterpType::Hyper)
        .ok_or_else(|| Error::Gtk {
            msg: format!("could not scale {}", path.display()),
        })?;
    // The quality WIC's JPEG encoder takes by default.
    scaled
        .save_to_bufferv("jpeg", &[("quality", "90")])
        .map_err(gtk_error)
}

// ----------------------------------------------------------------------------
fn gtk_error(e: glib::Error) -> Error {
    Error::Gtk { msg: e.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::gdk_pixbuf::{Colorspace, PixbufLoader};
    use gtk::prelude::*;
    use std::fs;

    /// The size of the image in `bytes`.
    fn size_of(bytes: &[u8]) -> (u32, u32) {
        let loader = PixbufLoader::new();
        loader.write(bytes).unwrap();
        loader.close().unwrap();
        let pixbuf = loader.pixbuf().unwrap();
        (pixbuf.width() as u32, pixbuf.height() as u32)
    }

    /// A gray `width` x `height` image file of `format` (`png`, `jpeg`).
    fn image(width: u32, height: u32, format: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "avelor-image-test-{}.{format}",
            av_utils::id::Id::generate()
        ));
        let pixbuf = Pixbuf::new(Colorspace::Rgb, false, 8, width as i32, height as i32).unwrap();
        pixbuf.fill(0x808080ff);
        pixbuf.savev(&path, format, &[]).unwrap();
        path
    }

    // ------------------------------------------------------------------------
    #[test]
    fn previews_are_jpegs_that_fit() {
        let (large, small) = (image(400, 200, "png"), image(100, 50, "jpeg"));
        let large_preview = preview(&large, 360, 508);
        let small_preview = preview(&small, 360, 508);
        let missing = preview(&large.with_extension("gone"), 360, 508);
        let _ = (fs::remove_file(&large), fs::remove_file(&small));

        let large_preview = large_preview.unwrap();
        assert_eq!(&large_preview[..2], [0xFF, 0xD8], "a JPEG");
        assert_eq!(size_of(&large_preview), (360, 180));
        assert_eq!(size_of(&small_preview.unwrap()), (100, 50));
        assert!(missing.is_err());
    }
}

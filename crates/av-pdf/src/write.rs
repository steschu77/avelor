//! Writing a PDF file: objects numbered as they are added, then the
//! cross-reference table and trailer; and a PDF of image pages (JPEG,
//! PNG).

use crate::Error;
use crate::jpeg;
use crate::object::{Dict, Object, Ref, Stream};
use crate::png;

/// A4, in points.
const A4: (f64, f64) = (595.0, 842.0);

// ----------------------------------------------------------------------------
pub struct Writer {
    out: Vec<u8>,
    /// Each object's offset, by number less one; `None` while reserved.
    offsets: Vec<Option<usize>>,
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

impl Writer {
    // ------------------------------------------------------------------------
    pub fn new() -> Writer {
        Writer {
            out: b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec(),
            offsets: Vec::new(),
        }
    }

    // ------------------------------------------------------------------------
    /// A number for an object written later with `put`, so others can refer
    /// to it first.
    pub fn reserve(&mut self) -> Ref {
        self.offsets.push(None);
        Ref::new(self.offsets.len() as u32)
    }

    // ------------------------------------------------------------------------
    /// Write `object` as the reserved `r`.
    pub fn put(&mut self, r: Ref, object: Object) {
        if let Some(slot) = self.offsets.get_mut(r.num as usize - 1) {
            *slot = Some(self.out.len());
        }
        self.out
            .extend_from_slice(format!("{} 0 obj\n", r.num).as_bytes());
        object.write_to(&mut self.out);
        self.out.extend_from_slice(b"\nendobj\n");
    }

    // ------------------------------------------------------------------------
    pub fn add(&mut self, object: Object) -> Ref {
        let r = self.reserve();
        self.put(r, object);
        r
    }

    // ------------------------------------------------------------------------
    /// The file, with `root` as its catalog.
    pub fn finish(mut self, root: Ref) -> Vec<u8> {
        let xref = self.out.len();
        let size = self.offsets.len() + 1;
        let mut table = format!("xref\n0 {size}\n0000000000 65535 f \n");
        for offset in &self.offsets {
            match offset {
                Some(offset) => table.push_str(&format!("{offset:010} 00000 n \n")),
                None => table.push_str("0000000000 65535 f \n"),
            }
        }
        self.out.extend_from_slice(table.as_bytes());
        self.out.extend_from_slice(b"trailer\n");
        let trailer = Dict::default()
            .with("Size", Object::Int(size as i64))
            .with("Root", root.into());
        Object::Dict(trailer).write_to(&mut self.out);
        self.out
            .extend_from_slice(format!("\nstartxref\n{xref}\n%%EOF\n").as_bytes());
        self.out
    }
}

// ----------------------------------------------------------------------------
/// A PDF whose pages are the images `pages`, each as it is: a JPEG
/// (`DCTDecode`), or a PNG's image data (`FlateDecode` with its predictors,
/// `png.rs`). A page is sized by the image's density, else fitted into A4,
/// and turned by a JPEG's EXIF orientation (`/Rotate`; a mirrored one as
/// the turn nearest it). `NotImage` if one is neither, or a PNG a PDF
/// can't take.
pub fn image_pages(pages: &[&[u8]]) -> Result<Vec<u8>, Error> {
    let images: Vec<PageImage> = pages
        .iter()
        .map(|data| PageImage::of(data).ok_or(Error::NotImage))
        .collect::<Result<_, _>>()?;
    let mut w = Writer::new();
    let tree = w.reserve();
    let mut kids = Vec::new();
    for image in images {
        let (width, height) = (image.width, image.height);
        let size = match image.dpi {
            Some((x, y)) => (width * 72.0 / x, height * 72.0 / y),
            None => {
                // Fitted as it is shown, turned or not.
                let turned = image.turns == 90 || image.turns == 270;
                let (shown_w, shown_h) = if turned {
                    (height, width)
                } else {
                    (width, height)
                };
                let (page_w, page_h) = if shown_w > shown_h { (A4.1, A4.0) } else { A4 };
                let scale = (page_w / shown_w).min(page_h / shown_h);
                (width * scale, height * scale)
            }
        };
        let dict = image
            .dict
            .with("Type", Object::name("XObject"))
            .with("Subtype", Object::name("Image"))
            .with("Width", Object::Int(width as i64))
            .with("Height", Object::Int(height as i64));
        let xobject = w.add(Object::Stream(Stream {
            dict,
            data: image.data,
        }));
        let content = format!("q {} 0 0 {} 0 0 cm /Im0 Do Q", real(size.0), real(size.1));
        let content = w.add(Object::Stream(Stream {
            dict: Dict::default(),
            data: content.into_bytes(),
        }));
        let media = [0.0, 0.0, size.0, size.1].map(Object::Real).to_vec();
        let xobjects = Dict::default().with("Im0", xobject.into());
        let mut page = Dict::default()
            .with("Type", Object::name("Page"))
            .with("Parent", tree.into())
            .with("MediaBox", Object::Array(media))
            .with(
                "Resources",
                Dict::default().with("XObject", xobjects.into()).into(),
            )
            .with("Contents", content.into());
        if image.turns != 0 {
            page.set("Rotate", Object::Int(image.turns));
        }
        kids.push(w.add(page.into()).into());
    }
    let count = Object::Int(kids.len() as i64);
    let pages = Dict::default()
        .with("Type", Object::name("Pages"))
        .with("Kids", Object::Array(kids))
        .with("Count", count);
    w.put(tree, pages.into());
    let catalog = Dict::default()
        .with("Type", Object::name("Catalog"))
        .with("Pages", tree.into());
    let catalog = w.add(catalog.into());
    Ok(w.finish(catalog))
}

// ----------------------------------------------------------------------------
/// An image to put on a page: its size, density and turn, its image
/// dictionary's colors and filter, and its data as it goes in.
struct PageImage {
    width: f64,
    height: f64,
    dpi: Option<(f64, f64)>,
    /// Degrees clockwise.
    turns: i64,
    dict: Dict,
    data: Vec<u8>,
}

impl PageImage {
    // ------------------------------------------------------------------------
    fn of(data: &[u8]) -> Option<PageImage> {
        if let Some(info) = jpeg::info(data) {
            let space = match info.components {
                1 => "DeviceGray",
                4 => "DeviceCMYK",
                _ => "DeviceRGB",
            };
            let mut dict = Dict::default()
                .with("ColorSpace", Object::name(space))
                .with("BitsPerComponent", Object::Int(8))
                .with("Filter", Object::name("DCTDecode"));
            // Adobe stores CMYK inverted.
            if info.components == 4 && info.adobe {
                let decode = [1, 0, 1, 0, 1, 0, 1, 0].map(Object::Int).to_vec();
                dict.set("Decode", Object::Array(decode));
            }
            let turns = match info.orientation {
                3 | 4 => 180,
                5 | 6 => 90,
                7 | 8 => 270,
                _ => 0,
            };
            return Some(PageImage {
                width: info.width as f64,
                height: info.height as f64,
                dpi: info.dpi,
                turns,
                dict,
                data: data.to_vec(),
            });
        }
        let info = png::info(data)?;
        let space = match (&info.palette, info.colors) {
            (Some(palette), _) => Object::Array(vec![
                Object::name("Indexed"),
                Object::name("DeviceRGB"),
                Object::Int((palette.len() / 3).max(1) as i64 - 1),
                Object::Str(palette.clone()),
            ]),
            (None, 1) => Object::name("DeviceGray"),
            (None, _) => Object::name("DeviceRGB"),
        };
        let params = Dict::default()
            .with("Predictor", Object::Int(15))
            .with("Colors", Object::Int(info.colors as i64))
            .with("BitsPerComponent", Object::Int(info.bits as i64))
            .with("Columns", Object::Int(info.width as i64));
        let dict = Dict::default()
            .with("ColorSpace", space)
            .with("BitsPerComponent", Object::Int(info.bits as i64))
            .with("Filter", Object::name("FlateDecode"))
            .with("DecodeParms", params.into());
        Some(PageImage {
            width: info.width as f64,
            height: info.height as f64,
            dpi: info.dpi,
            turns: 0,
            dict,
            data: info.data,
        })
    }
}

// ----------------------------------------------------------------------------
/// `value` as PDF writes a number: at most four decimals.
fn real(value: f64) -> String {
    let mut out = Vec::new();
    Object::Real(value).write_to(&mut out);
    String::from_utf8(out).unwrap_or_default()
}

// ----------------------------------------------------------------------------
/// `data` as zlib data in stored (uncompressed) blocks: what `FlateDecode`
/// takes, without a compressor.
pub fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut blocks = data.chunks(0xffff).peekable();
    if blocks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        let len = block.len() as u16;
        out.push(u8::from(last));
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&(b << 16 | a).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;
    use crate::jpeg::tests::header;
    use crate::png::tests::png;
    use av_utils::inflate;

    // ------------------------------------------------------------------------
    /// The objects of the first page of `pdf`: its dictionary, and its
    /// image's.
    fn first_page(pdf: &[u8]) -> (String, String) {
        let text = String::from_utf8_lossy(pdf).into_owned();
        let page = text
            .split("obj\n")
            .find(|o| o.contains("/Type /Page "))
            .unwrap();
        let image = text
            .split("obj\n")
            .find(|o| o.contains("/Subtype /Image"))
            .unwrap();
        (page.to_string(), image.chars().take(300).collect())
    }

    // ------------------------------------------------------------------------
    #[test]
    fn image_pages_are_sized_by_their_density_and_turned_by_exif() {
        let a4 = header(2480, 3508, 3, Some(300), None, false);
        let (page, image) = first_page(&image_pages(&[&a4]).unwrap());
        assert!(page.contains("/MediaBox [0 0 595.2 841.92]"), "{page}");
        assert!(!page.contains("/Rotate"), "{page}");
        assert!(image.contains("/ColorSpace /DeviceRGB"), "{image}");

        // A photo without density, taken sideways: fitted into A4 as shown.
        let photo = header(4000, 3000, 3, None, Some(6), false);
        let (page, _) = first_page(&image_pages(&[&photo]).unwrap());
        assert!(page.contains("/MediaBox [0 0 793.3333 595]"), "{page}");
        assert!(page.contains("/Rotate 90"), "{page}");

        let cmyk = header(100, 100, 4, Some(100), None, true);
        let (_, image) = first_page(&image_pages(&[&cmyk]).unwrap());
        assert!(image.contains("/ColorSpace /DeviceCMYK"), "{image}");
        assert!(image.contains("/Decode [1 0 1 0 1 0 1 0]"), "{image}");

        assert_eq!(image_pages(&[&a4, b"GIF89a"]), Err(Error::NotImage));

        // A PNG goes in as it is: its data, its predictors, its density.
        let rgb = png(4, 2, None, Some(300));
        let (page, image) = first_page(&image_pages(&[&rgb]).unwrap());
        assert!(page.contains("/MediaBox [0 0 0.96 0.48]"), "{page}");
        assert!(image.contains("/Filter /FlateDecode"), "{image}");
        assert!(
            image.contains("/Predictor 15 /Colors 3 /BitsPerComponent 8 /Columns 4"),
            "{image}"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_png_page_decodes_to_its_pixels_and_palette() {
        let palette = b"\r\0\0\xff(\\";
        let pdf = image_pages(&[&png(3, 2, Some(palette), None)]).unwrap();
        let doc = Document::parse(&pdf).unwrap();
        let image = (1..10)
            .filter_map(|num| doc.get(num))
            .find(|o| o.as_dict().and_then(|d| d.name("Subtype")) == Some("Image"))
            .unwrap();
        let stream = image.as_stream().unwrap();
        assert_eq!(doc.decoded(stream).data, [0, 1, 2, 1, 2, 3]);
        let space = doc.entry(&stream.dict, "ColorSpace");
        let space = space.as_array().unwrap();
        assert_eq!(space[2], Object::Int(1));
        assert_eq!(
            space[3],
            Object::Str(palette.to_vec()),
            "read back as written"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn jpeg_pages_read_back_as_scans() {
        let one = header(2480, 3508, 1, Some(300), None, false);
        let two = header(1240, 1754, 3, Some(150), Some(3), false);
        let pdf = image_pages(&[&one, &two]).unwrap();
        let doc = Document::parse(&pdf).unwrap();
        assert_eq!(doc.page_count(), 2);
        let scan = doc.page(0).unwrap().scan.unwrap();
        assert_eq!(scan.jpeg, one);
        assert!(scan.upright);
        assert!(!doc.page(1).unwrap().scan.unwrap().upright, "turned 180");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn stored_zlib_inflates_back() {
        for data in [Vec::new(), b"Hello".to_vec(), vec![7u8; 70_000]] {
            let mut out = Vec::new();
            inflate::zlib(&mut out, &zlib_stored(&data), 1 << 20).unwrap();
            assert_eq!(out, data);
        }
        assert_eq!(&zlib_stored(b"a")[8..], &[0x00, 0x62, 0x00, 0x62]);
    }
}

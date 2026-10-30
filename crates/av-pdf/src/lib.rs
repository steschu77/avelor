//! Reading a PDF's text and its scanned pages, and writing PDFs, by hand
//! (import.md#reading-pdfs). Not a general-purpose PDF library. Scope
//! decisions, each documented where it matters:
//! - The file: cross-reference tables and streams, `/Prev` updates, object
//!   streams; when the offsets don't match the file, its objects are found
//!   by scanning it. An encrypted file is refused (`Error::Encrypted`):
//!   decrypting is left out.
//! - Streams: Flate (with PNG predictors), ASCIIHex and ASCII85 are undone;
//!   other filters are named, a `DCTDecode` image handed out as its JPEG.
//! - Text: the text operators, through the fonts' `ToUnicode` maps, else a
//!   simple font's encoding with its `/Differences`; laid out into lines and
//!   paragraphs by position. Type3 glyphs, predefined CJK CMaps, the reading
//!   order of columns and form fields' values are left out.
//! - Writing: objects and a cross-reference table, no compression.

mod cmap;
mod content;
mod document;
mod encoding;
mod filter;
mod font;
mod layout;
mod lexer;
mod object;
mod write;

pub use content::{Page, SCAN_COVER, Scan};
pub use document::Document;
pub use layout::{Line, to_text};
pub use object::{Dict, Object, Ref, Stream};
pub use write::{Writer, zlib_stored};

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Error {
    /// No `%PDF-` in the file.
    NotPdf,
    Encrypted,
    /// What the file lacks to be read.
    Broken(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::NotPdf => f.write_str("not a PDF"),
            Error::Encrypted => f.write_str("encrypted"),
            Error::Broken(what) => write!(f, "broken: {what}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    /// A document whose pages have the content streams `pages`, with the
    /// resources `resources` (built with the writer's numbers).
    fn document(pages: &[&[u8]], resources: impl Fn(&mut Writer) -> Dict) -> Vec<u8> {
        let mut w = Writer::new();
        let tree = w.reserve();
        let resources = resources(&mut w);
        let kids: Vec<Object> = pages
            .iter()
            .map(|content| {
                let content = w.add(Object::Stream(Stream {
                    dict: Dict::default().with("Filter", Object::name("FlateDecode")),
                    data: zlib_stored(content),
                }));
                let page = Dict::default()
                    .with("Type", Object::name("Page"))
                    .with("Parent", tree.into())
                    .with("Contents", content.into());
                w.add(page.into()).into()
            })
            .collect();
        let count = Object::Int(kids.len() as i64);
        let media = Object::Array([0, 0, 595, 842].map(Object::Int).to_vec());
        let tree_dict = Dict::default()
            .with("Type", Object::name("Pages"))
            .with("Kids", Object::Array(kids))
            .with("Count", count)
            .with("MediaBox", media)
            .with("Resources", resources.into());
        w.put(tree, tree_dict.into());
        let catalog = Dict::default()
            .with("Type", Object::name("Catalog"))
            .with("Pages", tree.into());
        let catalog = w.add(catalog.into());
        w.finish(catalog)
    }

    // ------------------------------------------------------------------------
    /// Helvetica in WinAnsi with `/Differences` naming 0x80 `adieresis`.
    fn helvetica(w: &mut Writer) -> Object {
        let encoding = Dict::default()
            .with("BaseEncoding", Object::name("WinAnsiEncoding"))
            .with(
                "Differences",
                Object::Array(vec![Object::Int(0x80), Object::name("adieresis")]),
            );
        let font = Dict::default()
            .with("Type", Object::name("Font"))
            .with("Subtype", Object::name("Type1"))
            .with("BaseFont", Object::name("Helvetica"))
            .with("Encoding", encoding.into());
        w.add(font.into()).into()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_page_reads_to_its_text_in_lines_and_paragraphs() {
        let content: &[u8] = b"BT /F1 12 Tf 72 720 Td (Stadtwerke M\x80nster) Tj\n\
            0 -14 Td [(Rechnung)-300(Nr.)-250(4711)] TJ\n\
            14 TL (Betrag: 42,00 EUR) '\n\
            0 -40 Td (Mit freundlichen Gr\xfc\xdfen) Tj ET";
        let data = document(&[content], |w| {
            let font = helvetica(w);
            Dict::default().with("Font", Dict::default().with("F1", font).into())
        });
        let doc = Document::parse(&data).unwrap();
        let page = doc.page(0).unwrap();
        assert_eq!(
            page.text,
            "Stadtwerke M\u{e4}nster\nRechnung Nr. 4711\nBetrag: 42,00 EUR\n\n\
             Mit freundlichen Gr\u{fc}\u{df}en"
        );
        assert!(page.usable() && !page.blank && page.scan.is_none());
        assert_eq!(page.unmapped, 0);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn identity_fonts_read_through_their_to_unicode_map() {
        let content: &[u8] = b"BT /F2 10 Tf 50 800 Td <00010002000300030004> Tj ET\n\
            q 1 0 0 1 0 -100 cm /X1 Do Q";
        let data = document(&[content], |w| {
            let map = b"1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
                2 beginbfchar <0001> <0048> <0002> <0065> endbfchar\n\
                1 beginbfrange <0003> <0004> [<006C> <006F>] endbfrange";
            let map = w.add(Object::Stream(Stream {
                dict: Dict::default(),
                data: map.to_vec(),
            }));
            let font = Dict::default()
                .with("Subtype", Object::name("Type0"))
                .with("Encoding", Object::name("Identity-H"))
                .with("ToUnicode", map.into());
            let font = w.add(font.into());
            let fonts = Dict::default().with("F2", font.into());
            // A form showing the same font's glyph 5, which the map lacks.
            let form = w.add(Object::Stream(Stream {
                dict: Dict::default().with("Subtype", Object::name("Form")).with(
                    "Resources",
                    Dict::default().with("Font", fonts.clone().into()).into(),
                ),
                data: b"BT /F2 10 Tf 50 650 Td <00030005> Tj ET".to_vec(),
            }));
            Dict::default()
                .with("Font", fonts.into())
                .with("XObject", Dict::default().with("X1", form.into()).into())
        });
        let doc = Document::parse(&data).unwrap();
        let page = doc.page(0).unwrap();
        assert_eq!(page.text, "Hello\n\nl");
        assert_eq!((page.glyphs, page.unmapped), (7, 1));
        assert!(!page.usable(), "too few letters");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_full_page_jpeg_is_a_scan_and_a_logo_is_not() {
        let jpeg = b"\xff\xd8\xff\xe0 a JPEG \xff\xd9";
        let image = |w: &mut Writer| {
            let image = Dict::default()
                .with("Subtype", Object::name("Image"))
                .with("Filter", Object::name("DCTDecode"))
                .with("ColorSpace", Object::name("DeviceRGB"));
            let image = w.add(Object::Stream(Stream {
                dict: image,
                data: jpeg.to_vec(),
            }));
            Dict::default().with("XObject", Dict::default().with("Im0", image.into()).into())
        };
        let pages: [&[u8]; 5] = [
            b"q 595 0 0 842 0 0 cm /Im0 Do Q",
            b"q 100 0 0 50 20 780 cm /Im0 Do Q",
            b"q 0 842 -595 0 595 0 cm /Im0 Do Q",
            b"",
            b"q 565 0 0 782.343 15 29.8286 cm /Im0 Do Q",
        ];
        let data = document(&pages, image);
        let doc = Document::parse(&data).unwrap();
        let scan = doc.page(0).unwrap().scan.unwrap();
        assert_eq!(scan.jpeg, jpeg);
        assert!(scan.upright && !scan.cmyk);
        let logo = doc.page(1).unwrap();
        assert!(logo.scan.is_none() && !logo.blank);
        assert!(logo.cover > 0.0 && logo.cover < 0.02, "{}", logo.cover);
        assert!(!doc.page(2).unwrap().scan.unwrap().upright, "turned");
        assert!(doc.page(3).unwrap().blank);
        let margin = doc.page(4).unwrap();
        assert!(
            margin.scan.is_some(),
            "a scan fitted with a margin: {}",
            margin.cover
        );
    }
}

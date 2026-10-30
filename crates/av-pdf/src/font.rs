//! A font as far as its text goes: how its strings split into codes, each
//! code's text (`ToUnicode`, else a simple font's encoding) and width.
//! Predefined CJK CMaps and Type3 glyph procedures are left out: their codes
//! are read as two bytes and one, their text only through `ToUnicode`.

use std::collections::HashMap;

use crate::cmap::CMap;
use crate::document::Document;
use crate::encoding::{Base, base_char, glyph_text};
use crate::object::{Dict, Object};

/// A glyph's width where the font gives none, in thousandths of the size.
const DEFAULT_WIDTH: f64 = 500.0;

// ----------------------------------------------------------------------------
/// One code of a string shown.
#[derive(Debug, PartialEq)]
pub struct Glyph {
    /// `None` when the font doesn't say.
    pub text: Option<String>,
    /// In thousandths of the font size.
    pub width: f64,
    /// A single-byte code 32, which word spacing widens.
    pub space: bool,
}

// ----------------------------------------------------------------------------
#[derive(Debug)]
enum Codes {
    /// One byte per code, its text from the encoding where `ToUnicode` has
    /// none.
    Simple { encoding: Vec<Option<String>> },
    /// Codes by the embedded encoding's code space, else two bytes.
    Composite { space: Option<CMap> },
}

// ----------------------------------------------------------------------------
#[derive(Debug)]
pub struct Font {
    codes: Codes,
    to_unicode: Option<CMap>,
    widths: HashMap<u32, f64>,
    default_width: f64,
}

impl Font {
    // ------------------------------------------------------------------------
    /// The font of the dictionary `dict`.
    pub fn load(doc: &Document, dict: &Dict) -> Font {
        let to_unicode = doc
            .entry(dict, "ToUnicode")
            .as_stream()
            .map(|s| CMap::parse(&doc.decoded(s).data));
        let mut widths = HashMap::new();
        if dict.name("Subtype") == Some("Type0") {
            let encoding = doc.entry(dict, "Encoding");
            let space = encoding
                .as_stream()
                .map(|s| CMap::parse(&doc.decoded(s).data));
            let descendants = doc.entry(dict, "DescendantFonts");
            let descendant = descendants
                .as_array()
                .and_then(|d| d.first())
                .map(|d| doc.resolve(d));
            let mut default_width = 1000.0;
            if let Some(descendant) = descendant.as_deref().and_then(Object::as_dict) {
                if let Some(dw) = doc.entry(descendant, "DW").as_f64() {
                    default_width = dw;
                }
                if let Some(w) = doc.entry(descendant, "W").as_array() {
                    cid_widths(doc, w, &mut widths);
                }
            }
            return Font {
                codes: Codes::Composite { space },
                to_unicode,
                widths,
                default_width,
            };
        }

        let first = doc.entry(dict, "FirstChar").as_int().unwrap_or(0);
        if let Some(w) = doc.entry(dict, "Widths").as_array() {
            for (i, width) in w.iter().enumerate() {
                if let Some(width) = doc.resolve(width).as_f64() {
                    widths.insert((first + i as i64) as u32, width);
                }
            }
        }
        let descriptor = doc.entry(dict, "FontDescriptor");
        let missing = descriptor
            .as_dict()
            .and_then(|d| doc.entry(d, "MissingWidth").as_f64())
            .filter(|w| *w > 0.0);
        Font {
            codes: Codes::Simple {
                encoding: simple_encoding(doc, dict),
            },
            to_unicode,
            widths,
            default_width: missing.unwrap_or(DEFAULT_WIDTH),
        }
    }

    // ------------------------------------------------------------------------
    /// The glyphs of the string `bytes`.
    pub fn glyphs(&self, bytes: &[u8]) -> Vec<Glyph> {
        let mut glyphs = Vec::new();
        let mut rest = bytes;
        while !rest.is_empty() {
            let len = match &self.codes {
                Codes::Simple { .. } => 1,
                Codes::Composite { space } => space
                    .as_ref()
                    .and_then(|s| s.code_len(rest))
                    .or_else(|| self.to_unicode.as_ref()?.code_len(rest))
                    .unwrap_or(2),
            };
            let len = len.min(rest.len());
            let code = rest[..len].iter().fold(0u32, |v, &b| v << 8 | b as u32);
            rest = &rest[len..];

            let mapped = self.to_unicode.as_ref().and_then(|m| m.get(len, code));
            let text = match (&self.codes, mapped) {
                (_, Some(text)) => Some(text.to_string()),
                (Codes::Simple { encoding }, None) => encoding[code as usize].clone(),
                (Codes::Composite { .. }, None) => None,
            };
            let width = self
                .widths
                .get(&code)
                .copied()
                .unwrap_or(self.default_width);
            glyphs.push(Glyph {
                text,
                width,
                space: len == 1 && code == 32,
            });
        }
        glyphs
    }
}

// ----------------------------------------------------------------------------
/// A simple font's encoding: its base (named, or the dictionary's
/// `/BaseEncoding`; else Standard for Type1 fonts and WinAnsi for others),
/// with its `/Differences`.
fn simple_encoding(doc: &Document, dict: &Dict) -> Vec<Option<String>> {
    let encoding = doc.entry(dict, "Encoding");
    let base_name = match &*encoding {
        Object::Name(name) => Some(name.clone()),
        Object::Dict(d) => doc.entry(d, "BaseEncoding").as_name().map(str::to_string),
        _ => None,
    };
    let base = match base_name.as_deref() {
        Some("WinAnsiEncoding") => Base::WinAnsi,
        Some("MacRomanEncoding") => Base::MacRoman,
        Some("StandardEncoding") => Base::Standard,
        _ if dict.name("Subtype") == Some("Type1") => Base::Standard,
        _ => Base::WinAnsi,
    };
    let mut table: Vec<Option<String>> = (0..=255u8)
        .map(|code| base_char(base, code).map(String::from))
        .collect();
    if let Some(d) = encoding.as_dict() {
        if let Some(differences) = doc.entry(d, "Differences").as_array() {
            let mut code = 0usize;
            for item in differences {
                match doc.resolve(item).as_ref() {
                    Object::Int(start) => code = (*start).clamp(0, 255) as usize,
                    Object::Name(name) => {
                        if code < 256 {
                            table[code] = glyph_text(name);
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
    }
    table
}

// ----------------------------------------------------------------------------
/// A CID font's `/W`: `c [w1 w2 …]` or `c_first c_last w`.
fn cid_widths(doc: &Document, w: &[Object], widths: &mut HashMap<u32, f64>) {
    let items: Vec<Object> = w.iter().map(|o| doc.resolve(o).as_ref().clone()).collect();
    let mut i = 0;
    while i < items.len() {
        let Some(first) = items[i].as_int() else {
            i += 1;
            continue;
        };
        match (items.get(i + 1), items.get(i + 2)) {
            (Some(Object::Array(list)), _) => {
                for (k, width) in list.iter().enumerate() {
                    if let Some(width) = width.as_f64() {
                        widths.insert((first + k as i64) as u32, width);
                    }
                }
                i += 2;
            }
            (Some(last), Some(width)) => {
                if let (Some(last), Some(width)) = (last.as_int(), width.as_f64()) {
                    for cid in first..=last.min(first + 0xffff) {
                        widths.insert(cid as u32, width);
                    }
                }
                i += 3;
            }
            _ => break,
        }
    }
}

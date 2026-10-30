//! A CMap's code space and its codes' text: a font's `ToUnicode` map
//! (`bfchar`, `bfrange` with a start or a list), and a composite font's
//! embedded encoding, whose code space splits its strings into codes.

use std::collections::HashMap;

use crate::lexer::{Lexer, Token};
use crate::object::Object;

/// The most codes one `bfrange` maps.
const MAX_RANGE: u32 = 0x10000;

// ----------------------------------------------------------------------------
#[derive(Debug, Default)]
pub struct CMap {
    /// The code space: low and high code of a length in bytes.
    pub space: Vec<(u32, u32, usize)>,
    /// A code (its length and value) to its text.
    map: HashMap<(usize, u32), String>,
}

// ----------------------------------------------------------------------------
fn code_of(bytes: &[u8]) -> (usize, u32) {
    let value = bytes.iter().take(4).fold(0u32, |v, &b| v << 8 | b as u32);
    (bytes.len().min(4), value)
}

// ----------------------------------------------------------------------------
/// UTF-16BE text, or a single byte as Latin-1, which broken maps write.
fn text_of(bytes: &[u8]) -> String {
    if bytes.len() == 1 {
        return (bytes[0] as char).to_string();
    }
    let units = bytes
        .chunks(2)
        .map(|u| (u[0] as u16) << 8 | *u.get(1).unwrap_or(&0) as u16);
    char::decode_utf16(units)
        .map(|c| c.unwrap_or('\u{FFFD}'))
        .collect()
}

impl CMap {
    // ------------------------------------------------------------------------
    /// The map in the stream's text `data`; what can't be read is skipped.
    pub fn parse(data: &[u8]) -> CMap {
        let mut cmap = CMap::default();
        let mut lexer = Lexer::new(data, 0);
        while let Some(token) = lexer.next_token() {
            let Token::Keyword(word) = token else {
                continue;
            };
            match word {
                b"begincodespacerange" => {
                    for [lo, hi] in operands(&mut lexer).as_chunks().0 {
                        if let (Some(lo), Some(hi)) = (lo.as_str(), hi.as_str()) {
                            let ((len, lo), (_, hi)) = (code_of(lo), code_of(hi));
                            cmap.space.push((lo, hi, len));
                        }
                    }
                }
                b"beginbfchar" => {
                    for [src, dst] in operands(&mut lexer).as_chunks().0 {
                        let Some(src) = src.as_str() else {
                            continue;
                        };
                        let text = match dst {
                            Object::Str(dst) => text_of(dst),
                            Object::Name(name) => match crate::encoding::glyph_text(name) {
                                Some(text) => text,
                                None => continue,
                            },
                            _ => continue,
                        };
                        cmap.map.insert(code_of(src), text);
                    }
                }
                b"beginbfrange" => {
                    for [lo, hi, dst] in operands(&mut lexer).as_chunks().0 {
                        cmap.add_range(lo, hi, dst);
                    }
                }
                _ => {}
            }
        }
        cmap
    }

    // ------------------------------------------------------------------------
    fn add_range(&mut self, lo: &Object, hi: &Object, dst: &Object) {
        let (Some(lo_bytes), Some(hi)) = (lo.as_str(), hi.as_str()) else {
            return;
        };
        let ((len, lo), (_, hi)) = (code_of(lo_bytes), code_of(hi));
        if hi < lo || hi - lo >= MAX_RANGE {
            return;
        }
        match dst {
            Object::Str(start) if start.len() >= 2 => {
                let mut units: Vec<u16> = start
                    .chunks(2)
                    .map(|u| (u[0] as u16) << 8 | *u.get(1).unwrap_or(&0) as u16)
                    .collect();
                let last = units.len() - 1;
                let first = units[last];
                for (i, code) in (lo..=hi).enumerate() {
                    units[last] = first.wrapping_add(i as u16);
                    let text = char::decode_utf16(units.iter().copied())
                        .map(|c| c.unwrap_or('\u{FFFD}'))
                        .collect();
                    self.map.insert((len, code), text);
                }
            }
            Object::Str(start) => {
                let first = start.first().copied().unwrap_or(0);
                for (i, code) in (lo..=hi).enumerate() {
                    let c = first.wrapping_add(i as u8) as char;
                    self.map.insert((len, code), c.to_string());
                }
            }
            Object::Array(items) => {
                for (code, item) in (lo..=hi).zip(items) {
                    if let Some(dst) = item.as_str() {
                        self.map.insert((len, code), text_of(dst));
                    }
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------------
    /// The text of the code `code` of `len` bytes.
    pub fn get(&self, len: usize, code: u32) -> Option<&str> {
        self.map.get(&(len, code)).map(String::as_str)
    }

    // ------------------------------------------------------------------------
    /// The length of the code at the start of `bytes`, by the code space;
    /// `None` if it has none or no range matches.
    pub fn code_len(&self, bytes: &[u8]) -> Option<usize> {
        self.space.iter().find_map(|&(lo, hi, len)| {
            let (_, value) = code_of(bytes.get(..len)?);
            (lo..=hi).contains(&value).then_some(len)
        })
    }
}

// ----------------------------------------------------------------------------
/// The objects up to the next keyword (its `end…`).
fn operands(lexer: &mut Lexer) -> Vec<Object> {
    let mut items = Vec::new();
    while let Some(object) = lexer.object() {
        items.push(object);
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn a_to_unicode_map_reads_chars_ranges_and_lists() {
        let text = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
            1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
            2 beginbfchar <0003> <0020> <0011> <00660069> endbfchar\n\
            2 beginbfrange <0024> <0026> <0041> <0030> <0031> [<00E4> <D83DDE00>] endbfrange\n\
            endcmap CMapName currentdict /CMap defineresource pop end end";
        let cmap = CMap::parse(text);
        assert_eq!(cmap.get(2, 0x03), Some(" "));
        assert_eq!(cmap.get(2, 0x11), Some("fi"));
        assert_eq!(cmap.get(2, 0x26), Some("C"));
        assert_eq!(cmap.get(2, 0x30), Some("\u{e4}"));
        assert_eq!(cmap.get(2, 0x31), Some("\u{1F600}"));
        assert_eq!(cmap.get(1, 0x03), None, "a code of another length");
        assert_eq!(cmap.code_len(&[0x00, 0x24]), Some(2));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn mixed_code_lengths_split_by_their_space() {
        let cmap = CMap::parse(b"2 begincodespacerange <00> <80> <8140> <FFFF> endcodespacerange");
        assert_eq!(cmap.code_len(&[0x41, 0x81]), Some(1));
        assert_eq!(cmap.code_len(&[0x81, 0x40]), Some(2));
    }
}

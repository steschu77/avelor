//! A stream's filters: Flate (with PNG predictors), ASCIIHex and ASCII85
//! are undone here; the first other one (an image's `DCTDecode`,
//! `CCITTFaxDecode`, …) is handed back by name with the bytes before it.

use crate::object::Dict;
use av_utils::inflate;

/// The most a stream inflates to: more is a broken or hostile file.
const LIMIT: usize = 64 << 20;

// ----------------------------------------------------------------------------
pub struct Decoded {
    pub data: Vec<u8>,
    /// The first filter not undone, with the rest after it.
    pub rest: Option<String>,
}

// ----------------------------------------------------------------------------
/// `data` with `filters` (name and parameters, in order) undone as far as
/// they can be. A broken Flate stream keeps what inflated before the break.
pub fn decode(filters: &[(String, Option<Dict>)], data: &[u8]) -> Decoded {
    let mut data = data.to_vec();
    for (name, params) in filters {
        data = match name.as_str() {
            "FlateDecode" | "Fl" => {
                let mut out = Vec::new();
                let _ = inflate::zlib(&mut out, &data, LIMIT);
                match params {
                    Some(params) => predicted(params, out),
                    None => out,
                }
            }
            "ASCIIHexDecode" | "AHx" => ascii_hex(&data),
            "ASCII85Decode" | "A85" => ascii85(&data),
            _ => {
                return Decoded {
                    data,
                    rest: Some(name.clone()),
                };
            }
        };
    }
    Decoded { data, rest: None }
}

// ----------------------------------------------------------------------------
/// Undo a PNG predictor (10 to 15) per row; TIFF's (2) and none (1) leave
/// the data as it is.
fn predicted(params: &Dict, data: Vec<u8>) -> Vec<u8> {
    let int = |key: &str, default: i64| params.get(key).and_then(|o| o.as_int()).unwrap_or(default);
    if int("Predictor", 1) < 10 {
        return data;
    }
    let colors = int("Colors", 1).clamp(1, 32) as usize;
    let bits = int("BitsPerComponent", 8).clamp(1, 16) as usize;
    let columns = int("Columns", 1).clamp(1, 1 << 20) as usize;
    let bpp = (colors * bits).div_ceil(8);
    let row = (colors * bits * columns).div_ceil(8);

    let mut out = Vec::with_capacity(data.len());
    let mut prior = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        let (kind, line) = (chunk[0], &chunk[1..]);
        let mut current = vec![0u8; row];
        for i in 0..line.len() {
            let left = if i >= bpp { current[i - bpp] } else { 0 };
            let up = prior[i];
            let upper_left = if i >= bpp { prior[i - bpp] } else { 0 };
            let base = match kind {
                1 => left,
                2 => up,
                3 => ((left as u16 + up as u16) / 2) as u8,
                4 => paeth(left, up, upper_left),
                _ => 0,
            };
            current[i] = line[i].wrapping_add(base);
        }
        out.extend_from_slice(&current[..line.len()]);
        prior = current;
    }
    out
}

// ----------------------------------------------------------------------------
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = (
        (p - a as i16).abs(),
        (p - b as i16).abs(),
        (p - c as i16).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

// ----------------------------------------------------------------------------
fn ascii_hex(data: &[u8]) -> Vec<u8> {
    let digits: Vec<u8> = data
        .iter()
        .take_while(|&&b| b != b'>')
        .filter_map(|&b| (b as char).to_digit(16).map(|d| d as u8))
        .collect();
    digits
        .chunks(2)
        .map(|pair| pair[0] << 4 | pair.get(1).copied().unwrap_or(0))
        .collect()
}

// ----------------------------------------------------------------------------
fn ascii85(data: &[u8]) -> Vec<u8> {
    let data = data.strip_prefix(b"<~").unwrap_or(data);
    let mut out = Vec::new();
    let mut group = [0u8; 5];
    let mut n = 0;
    for &b in data {
        match b {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group[n] = b - b'!';
                n += 1;
                if n == 5 {
                    out.extend_from_slice(&base85(&group).to_be_bytes());
                    n = 0;
                }
            }
            _ => {}
        }
    }
    if n > 1 {
        for slot in group.iter_mut().skip(n) {
            *slot = b'u' - b'!';
        }
        out.extend_from_slice(&base85(&group).to_be_bytes()[..n - 1]);
    }
    out
}

// ----------------------------------------------------------------------------
fn base85(group: &[u8; 5]) -> u32 {
    group.iter().fold(0u32, |value, &d| {
        value.wrapping_mul(85).wrapping_add(d as u32)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::Object;

    // ------------------------------------------------------------------------
    fn filters(names: &[&str]) -> Vec<(String, Option<Dict>)> {
        names.iter().map(|n| (n.to_string(), None)).collect()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn ascii_filters_decode_and_others_are_handed_back() {
        let decoded = decode(&filters(&["ASCIIHexDecode"]), b"48 65 6c 6C 6F>");
        assert_eq!(decoded.data, b"Hello");
        let decoded = decode(&filters(&["ASCII85Decode"]), b"<~87cURD]i,\"Ebo80~>");
        assert_eq!(decoded.data, b"Hello World!");
        assert_eq!(decode(&filters(&["A85"]), b"z~>").data, [0; 4]);

        let decoded = decode(&filters(&["AHx", "DCTDecode"]), b"FFD8>");
        assert_eq!(decoded.data, [0xff, 0xd8]);
        assert_eq!(decoded.rest.as_deref(), Some("DCTDecode"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn png_predictors_undo_per_row() {
        let params = Dict::default()
            .with("Predictor", Object::Int(12))
            .with("Columns", Object::Int(3));
        // Row 1 with Sub, row 2 with Up, row 3 with Paeth.
        let data = vec![1, 1, 1, 1, 2, 1, 1, 1, 4, 0, 0, 0];
        assert_eq!(predicted(&params, data), vec![1, 2, 3, 2, 3, 4, 2, 3, 4]);
        let none = Dict::default().with("Predictor", Object::Int(1));
        assert_eq!(predicted(&none, vec![2, 7]), vec![2, 7]);
    }
}

//! What a PNG's chunks say, for putting it on a PDF page as it is: PDF's
//! Flate filter with PNG predictors takes its image data (the `IDAT`
//! chunks) without decoding. Only what that takes is read: gray, RGB or a
//! palette, not interlaced, at most 8 bits; alpha, interlacing and 16 bits
//! are left out (`None`). Checksums aren't checked.

// ----------------------------------------------------------------------------
#[derive(Debug, PartialEq)]
pub struct Png {
    pub width: u32,
    pub height: u32,
    /// 1 for gray or a palette's index, 3 for RGB.
    pub colors: u8,
    pub bits: u8,
    /// A palette image's RGB entries.
    pub palette: Option<Vec<u8>>,
    /// Dots per inch, x and y, if its `pHYs` gives a plausible one.
    pub dpi: Option<(f64, f64)>,
    /// The `IDAT` chunks' data, joined: zlib data of filtered rows.
    pub data: Vec<u8>,
}

const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

// ----------------------------------------------------------------------------
fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

// ----------------------------------------------------------------------------
/// The PNG `data`, if a PDF can take it as it is.
pub fn info(data: &[u8]) -> Option<Png> {
    if !data.starts_with(SIGNATURE) {
        return None;
    }
    let mut png = Png {
        width: 0,
        height: 0,
        colors: 0,
        bits: 0,
        palette: None,
        dpi: None,
        data: Vec::new(),
    };
    let mut kind = 0;
    let mut at = SIGNATURE.len();
    while at + 8 <= data.len() {
        let len = u32_at(data, at)? as usize;
        let chunk = data.get(at + 4..at + 8)?;
        let body = data.get(at + 8..at + 8 + len)?;
        match chunk {
            b"IHDR" => {
                png.width = u32_at(body, 0)?;
                png.height = u32_at(body, 4)?;
                png.bits = *body.get(8)?;
                kind = *body.get(9)?;
                let interlaced = *body.get(12)? != 0;
                png.colors = match kind {
                    0 | 3 => 1,
                    2 => 3,
                    _ => return None,
                };
                if interlaced || png.bits > 8 || (kind == 2 && png.bits != 8) {
                    return None;
                }
            }
            b"PLTE" => png.palette = Some(body.to_vec()),
            b"pHYs" if body.get(8) == Some(&1) => {
                let dpi = |ppm: u32| ppm as f64 * 0.0254;
                let (x, y) = (dpi(u32_at(body, 0)?), dpi(u32_at(body, 4)?));
                let plausible = |dpi: f64| (50.0..=2400.0).contains(&dpi);
                png.dpi = (plausible(x) && plausible(y)).then_some((x, y));
            }
            b"IDAT" => png.data.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    // A palette is the image's only for a palette image; another's is a
    // suggestion.
    if kind != 3 {
        png.palette = None;
    }
    let complete = png.width > 0 && png.height > 0 && !png.data.is_empty();
    (complete && (kind != 3 || png.palette.is_some())).then_some(png)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::write::zlib_stored;

    // ------------------------------------------------------------------------
    /// A PNG of `width` x `height`: RGB, or gray with `palette`, of rows
    /// filtered with None; `dpi` as its `pHYs`.
    pub fn png(width: u32, height: u32, palette: Option<&[u8]>, dpi: Option<u32>) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        let mut chunk = |kind: &[u8], body: &[u8]| {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(body);
            out.extend_from_slice(&[0; 4]);
        };
        let colors = if palette.is_some() { 1 } else { 3 };
        let mut ihdr = width.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, if palette.is_some() { 3 } else { 2 }, 0, 0, 0]);
        chunk(b"IHDR", &ihdr);
        if let Some(palette) = palette {
            chunk(b"PLTE", palette);
        }
        if let Some(dpi) = dpi {
            let ppm = (dpi as f64 / 0.0254).round() as u32;
            let mut phys = ppm.to_be_bytes().to_vec();
            phys.extend_from_slice(&ppm.to_be_bytes());
            phys.push(1);
            chunk(b"pHYs", &phys);
        }
        let mut rows = Vec::new();
        for y in 0..height {
            rows.push(0);
            for x in 0..width * colors {
                rows.push((x + y) as u8);
            }
        }
        chunk(b"IDAT", &zlib_stored(&rows));
        chunk(b"IEND", &[]);
        out
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_png_gives_its_size_colors_density_and_data() {
        let rgb = info(&png(4, 2, None, Some(300))).unwrap();
        assert_eq!((rgb.width, rgb.height, rgb.colors, rgb.bits), (4, 2, 3, 8));
        let (x, y) = rgb.dpi.unwrap();
        assert!((x - 300.0).abs() < 0.1 && (y - 300.0).abs() < 0.1, "{x}");
        assert!(rgb.palette.is_none() && !rgb.data.is_empty());

        let indexed = info(&png(2, 2, Some(b"\r\0\0\xff\xff\xff"), None)).unwrap();
        assert_eq!(indexed.colors, 1);
        assert_eq!(indexed.palette.as_deref(), Some(&b"\r\0\0\xff\xff\xff"[..]));
        assert_eq!(indexed.dpi, None);

        let mut alpha = png(2, 2, None, None);
        alpha[SIGNATURE.len() + 8 + 9] = 6;
        assert_eq!(info(&alpha), None, "with alpha");
        assert_eq!(info(b"GIF89a"), None);
    }
}

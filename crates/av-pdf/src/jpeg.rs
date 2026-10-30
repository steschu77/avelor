//! What a JPEG's headers say, for placing it on a PDF page: its size and
//! components (SOF), its density (JFIF), its EXIF orientation and whether
//! Adobe wrote it (APP14, whose CMYK is stored inverted). The image data
//! itself goes into the PDF as it is.

// ----------------------------------------------------------------------------
#[derive(Debug, PartialEq)]
pub struct Jpeg {
    pub width: u16,
    pub height: u16,
    /// 1 gray, 3 RGB (YCbCr), 4 CMYK.
    pub components: u8,
    /// Dots per inch, x and y, if the JFIF header gives a plausible one.
    pub dpi: Option<(f64, f64)>,
    /// EXIF's orientation, 1 (upright) to 8.
    pub orientation: u8,
    /// An Adobe APP14 marker.
    pub adobe: bool,
}

// ----------------------------------------------------------------------------
fn u16_at(data: &[u8], at: usize, big: bool) -> Option<u16> {
    let bytes = [*data.get(at)?, *data.get(at + 1)?];
    Some(if big {
        u16::from_be_bytes(bytes)
    } else {
        u16::from_le_bytes(bytes)
    })
}

// ----------------------------------------------------------------------------
fn u32_at(data: &[u8], at: usize, big: bool) -> Option<u32> {
    let bytes: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
    Some(if big {
        u32::from_be_bytes(bytes)
    } else {
        u32::from_le_bytes(bytes)
    })
}

// ----------------------------------------------------------------------------
/// The headers of the JPEG `data`, up to its scan; `None` if it isn't one
/// or has no frame header before it.
pub fn info(data: &[u8]) -> Option<Jpeg> {
    if data.get(..2) != Some(&[0xff, 0xd8]) {
        return None;
    }
    let mut jpeg = Jpeg {
        width: 0,
        height: 0,
        components: 0,
        dpi: None,
        orientation: 1,
        adobe: false,
    };
    let mut frame = false;
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xff {
            return None;
        }
        let marker = data[at + 1];
        // Fill bytes, and markers without a segment.
        if marker == 0xff {
            at += 1;
            continue;
        }
        if matches!(marker, 0x01 | 0xd0..=0xd7) {
            at += 2;
            continue;
        }
        if matches!(marker, 0xd9 | 0xda) {
            break;
        }
        let len = u16_at(data, at + 2, true)? as usize;
        let segment = data.get(at + 4..at + 2 + len.max(2))?;
        match marker {
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                jpeg.height = u16_at(segment, 1, true)?;
                jpeg.width = u16_at(segment, 3, true)?;
                jpeg.components = *segment.get(5)?;
                frame = true;
            }
            0xe0 if segment.starts_with(b"JFIF\0") => jpeg.dpi = density(segment),
            0xe1 if segment.starts_with(b"Exif\0\0") => {
                if let Some(orientation) = orientation(&segment[6..]) {
                    jpeg.orientation = orientation;
                }
            }
            0xee if segment.starts_with(b"Adobe") => jpeg.adobe = true,
            _ => {}
        }
        at += 2 + len;
    }
    (frame && jpeg.width > 0 && jpeg.height > 0).then_some(jpeg)
}

// ----------------------------------------------------------------------------
/// A JFIF header's density in dots per inch; none for an aspect ratio only,
/// or one too far from a scan's.
fn density(segment: &[u8]) -> Option<(f64, f64)> {
    let unit = match *segment.get(7)? {
        1 => 1.0,
        2 => 2.54,
        _ => return None,
    };
    let x = u16_at(segment, 8, true)? as f64 * unit;
    let y = u16_at(segment, 10, true)? as f64 * unit;
    let plausible = |dpi: f64| (50.0..=2400.0).contains(&dpi);
    (plausible(x) && plausible(y)).then_some((x, y))
}

// ----------------------------------------------------------------------------
/// The orientation tag (0x0112) of an EXIF block's first directory.
fn orientation(tiff: &[u8]) -> Option<u8> {
    let big = match tiff.get(..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let ifd = u32_at(tiff, 4, big)? as usize;
    let count = u16_at(tiff, ifd, big)? as usize;
    (0..count.min(256))
        .find_map(|i| {
            let entry = ifd + 2 + i * 12;
            (u16_at(tiff, entry, big)? == 0x0112).then(|| u16_at(tiff, entry + 8, big))?
        })
        .map(|o| o.clamp(1, 8) as u8)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    /// A JPEG's headers: `width` x `height` of `components`, a JFIF density
    /// of `dpi`, an EXIF `orientation`, an Adobe marker.
    pub fn header(
        width: u16,
        height: u16,
        components: u8,
        dpi: Option<u16>,
        orientation: Option<u16>,
        adobe: bool,
    ) -> Vec<u8> {
        let mut out = vec![0xff, 0xd8];
        let mut segment = |marker: u8, body: &[u8]| {
            out.extend_from_slice(&[0xff, marker]);
            out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
            out.extend_from_slice(body);
        };
        if let Some(dpi) = dpi {
            let mut jfif = b"JFIF\0\x01\x02\x01".to_vec();
            jfif.extend_from_slice(&dpi.to_be_bytes());
            jfif.extend_from_slice(&dpi.to_be_bytes());
            jfif.extend_from_slice(&[0, 0]);
            segment(0xe0, &jfif);
        }
        if let Some(orientation) = orientation {
            let mut exif = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
            exif.extend_from_slice(&orientation.to_be_bytes());
            exif.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
            segment(0xe1, &exif);
        }
        if adobe {
            segment(0xee, b"Adobe\0\x64\0\0\0\0\0");
        }
        let mut sof = vec![8];
        sof.extend_from_slice(&height.to_be_bytes());
        sof.extend_from_slice(&width.to_be_bytes());
        sof.push(components);
        for c in 0..components {
            sof.extend_from_slice(&[c + 1, 0x11, 0]);
        }
        segment(0xc0, &sof);
        out.extend_from_slice(&[0xff, 0xda, 0, 2, 0xff, 0xd9]);
        out
    }

    // ------------------------------------------------------------------------
    #[test]
    fn headers_give_size_density_orientation_and_adobe() {
        let data = header(2480, 3508, 3, Some(300), Some(6), false);
        assert_eq!(
            info(&data),
            Some(Jpeg {
                width: 2480,
                height: 3508,
                components: 3,
                dpi: Some((300.0, 300.0)),
                orientation: 6,
                adobe: false,
            })
        );
        let cmyk = info(&header(100, 50, 4, None, None, true)).unwrap();
        assert_eq!((cmyk.components, cmyk.dpi, cmyk.orientation), (4, None, 1));
        assert!(cmyk.adobe);
        assert_eq!(
            info(&header(10, 10, 1, Some(1), None, false)).unwrap().dpi,
            None
        );
        assert_eq!(info(b"\x89PNG\r\n"), None);
        assert_eq!(info(&[0xff, 0xd8, 0xff, 0xd9]), None, "no frame");
    }
}

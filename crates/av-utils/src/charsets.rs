// ----------------------------------------------------------------------------
pub fn from_utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

// ----------------------------------------------------------------------------
pub fn from_windows1252(bytes: &[u8]) -> String {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}',
        '\u{017D}', '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}',
        '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}',
        '\u{9D}', '\u{017E}', '\u{0178}',
    ];
    bytes
        .iter()
        .map(|&b| {
            if (0x80..0xA0).contains(&b) {
                HIGH[(b - 0x80) as usize]
            } else {
                b as char
            }
        })
        .collect()
}

// ----------------------------------------------------------------------------
/// ISO-8859-15 (Latin-9): Latin-1 with the euro sign and seven other letters
/// replacing eight rarely used symbols.
pub fn from_iso8859_15(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0xA4 => '\u{20AC}',
            0xA6 => '\u{0160}',
            0xA8 => '\u{0161}',
            0xB4 => '\u{017D}',
            0xB8 => '\u{017E}',
            0xBC => '\u{0152}',
            0xBD => '\u{0153}',
            0xBE => '\u{0178}',
            b => b as char,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn iso8859_15_differs_from_latin1_only_where_specified() {
        assert_eq!(
            from_iso8859_15(&[0x41, 0xA4, 0xE4, 0xBD]),
            "A\u{20AC}\u{E4}\u{153}"
        );
        assert_eq!(from_windows1252(&[0x80, 0xE4]), "\u{20AC}\u{E4}");
    }
}
